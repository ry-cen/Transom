//! Where Transom keeps its device key and the Macs it is paired with.

use crate::identity::DeviceIdentity;
use crate::NetError;
use std::io::Write;
use std::path::{Path, PathBuf};
use transom_proto::identity::b64url_decode;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SavedAddress {
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    pub kind: String,
    pub via: String,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SavedMac {
    #[serde(rename = "macID")]
    pub mac_id: String,
    pub name: String,
    /// base64url, 32 bytes.
    pub fingerprint: String,
    /// base64url, 32 bytes.
    #[serde(rename = "recognitionKey")]
    pub recognition_key: String,
    #[serde(rename = "remotePort", default)]
    pub remote_port: Option<u16>,
    #[serde(default)]
    pub addresses: Vec<SavedAddress>,
    #[serde(rename = "issuedAt", default)]
    pub issued_at: Option<f64>,
    /// "ip:port" of the last session.
    #[serde(rename = "lastAddress", default)]
    pub last_address: Option<String>,
}

impl SavedMac {
    pub fn fingerprint_bytes(&self) -> Option<[u8; 32]> {
        b64url_decode(&self.fingerprint)?.try_into().ok()
    }

    pub fn recognition_key_bytes(&self) -> Option<Vec<u8>> {
        b64url_decode(&self.recognition_key)
    }
}

#[derive(Serialize, Deserialize)]
struct MacsFile {
    version: i64,
    macs: Vec<SavedMac>,
}

use serde::{Deserialize, Serialize};

/// A handle on the config folder. Cloning it gives another handle on the same folder.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

fn store_err<E: std::fmt::Display>(e: E) -> NetError {
    NetError::Store(e.to_string())
}

/// Writes `<name>.tmp` next to the target, syncs it, then renames it over the target.
fn write_atomic(path: &Path, data: &[u8]) -> Result<(), NetError> {
    let mut tmp_name = path.as_os_str().to_os_string();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp).map_err(store_err)?;
    f.write_all(data).map_err(store_err)?;
    f.sync_all().map_err(store_err)?;
    drop(f);
    std::fs::rename(&tmp, path).map_err(store_err)
}

impl Store {
    /// `dir`, or the per-user config directory; created (on unix with mode 0700) when missing.
    pub fn open(dir: Option<PathBuf>) -> Result<Store, NetError> {
        let dir = match dir {
            Some(d) => d,
            None => directories::ProjectDirs::from("", "", "transom")
                .ok_or_else(|| NetError::Store("no home directory".into()))?
                .config_dir()
                .to_path_buf(),
        };
        let existed = dir.exists();
        std::fs::create_dir_all(&dir).map_err(store_err)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if !existed {
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                    .map_err(store_err)?;
            } else if let Ok(md) = std::fs::metadata(&dir) {
                if md.permissions().mode() & 0o077 != 0 {
                    log::warn!(
                        "{} is accessible to other users; consider chmod 700",
                        dir.display()
                    );
                }
            }
        }
        #[cfg(not(unix))]
        let _ = existed;
        Ok(Store { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// device-key.pem: loaded, or generated and written on first use.
    pub fn identity(&self) -> Result<DeviceIdentity, NetError> {
        let path = self.dir.join("device-key.pem");
        match std::fs::read_to_string(&path) {
            Ok(pem) => DeviceIdentity::from_pem(&pem),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let (id, pem) = DeviceIdentity::generate()?;
                write_atomic(&path, pem.as_bytes())?;
                Ok(id)
            }
            Err(e) => Err(store_err(e)),
        }
    }

    /// macs.json = {"version":1,"macs":[...]}; a missing file is an empty list.
    pub fn macs(&self) -> Result<Vec<SavedMac>, NetError> {
        let path = self.dir.join("macs.json");
        match std::fs::read(&path) {
            Ok(bytes) => {
                let file: MacsFile = serde_json::from_slice(&bytes)
                    .map_err(|e| NetError::Store(format!("macs.json: {e}")))?;
                Ok(file.macs)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(store_err(e)),
        }
    }

    fn write_macs(&self, macs: Vec<SavedMac>) -> Result<(), NetError> {
        let file = MacsFile { version: 1, macs };
        let json = serde_json::to_vec_pretty(&file).map_err(store_err)?;
        write_atomic(&self.dir.join("macs.json"), &json)
    }

    /// Replaces the entry with the same macID, or appends.
    pub fn save_mac(&self, mac: SavedMac) -> Result<(), NetError> {
        let mut macs = self.macs()?;
        match macs.iter_mut().find(|m| m.mac_id == mac.mac_id) {
            Some(slot) => *slot = mac,
            None => macs.push(mac),
        }
        self.write_macs(macs)
    }

    pub fn forget(&self, query: &str) -> Result<Option<SavedMac>, NetError> {
        let Some(found) = self.find(query)? else {
            return Ok(None);
        };
        let macs: Vec<SavedMac> = self
            .macs()?
            .into_iter()
            .filter(|m| m.mac_id != found.mac_id)
            .collect();
        self.write_macs(macs)?;
        Ok(Some(found))
    }

    /// Exact macID; else case-insensitive exact name; else the unique macID with this prefix
    /// (at least 4 characters); else None.
    pub fn find(&self, query: &str) -> Result<Option<SavedMac>, NetError> {
        let macs = self.macs()?;
        if let Some(m) = macs.iter().find(|m| m.mac_id == query) {
            return Ok(Some(m.clone()));
        }
        let wanted = name_key(query);
        if let Some(m) = macs.iter().find(|m| name_key(&m.name) == wanted) {
            return Ok(Some(m.clone()));
        }
        if wanted.chars().count() >= 3 {
            let mut hits = macs
                .iter()
                .filter(|m| name_key(&m.name).starts_with(&wanted));
            if let (Some(first), None) = (hits.next(), hits.next()) {
                return Ok(Some(first.clone()));
            }
        }
        if query.chars().count() >= 4 {
            let upper = query.to_uppercase();
            let mut hits = macs.iter().filter(|m| m.mac_id.starts_with(&upper));
            if let (Some(first), None) = (hits.next(), hits.next()) {
                return Ok(Some(first.clone()));
            }
        }
        Ok(None)
    }

    /// A small named file in the config folder (the GUI's `prefs.json`); None when missing.
    pub fn read_file(&self, name: &str) -> Result<Option<Vec<u8>>, NetError> {
        let path = self.dir.join(checked_name(name)?);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(store_err(e)),
        }
    }

    /// Writes a small named file in the config folder: atomically, mode 0600 on unix.
    pub fn write_file(&self, name: &str, data: &[u8]) -> Result<(), NetError> {
        let path = self.dir.join(checked_name(name)?);
        write_atomic(&path, data)
    }
}

/// Files `read_file` and `write_file` never touch: they have their own methods.
const RESERVED_FILES: [&str; 2] = ["device-key.pem", "macs.json"];

/// `name` when it matches `^[a-z0-9][a-z0-9._-]*$`, is at most 64 bytes, does not end in ".tmp"
/// (`write_atomic`'s temporary) and is not reserved; else "bad file name".
fn checked_name(name: &str) -> Result<&str, NetError> {
    let bytes = name.as_bytes();
    let ok_first = bytes
        .first()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    let ok_rest = bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'));
    if !ok_first
        || !ok_rest
        || bytes.len() > 64
        || name.ends_with(".tmp")
        || RESERVED_FILES.contains(&name)
    {
        return Err(NetError::Store("bad file name".into()));
    }
    Ok(name)
}

/// A Mac name as `find` compares it: trimmed, lowercased, and with curly apostrophes made
/// straight, so "Ryan's MacBook Pro" typed in a terminal matches Bonjour's "Ryan’s MacBook Pro".
fn name_key(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            '\u{2019}' | '\u{2018}' | '\u{02BC}' => '\'',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_proto::identity::b64url_encode;

    fn temp_dir() -> PathBuf {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap();
        let suffix: String = r.iter().map(|b| format!("{b:02x}")).collect();
        std::env::temp_dir().join(format!("transom-store-test-{suffix}"))
    }

    fn mac(id: &str, name: &str) -> SavedMac {
        SavedMac {
            mac_id: id.into(),
            name: name.into(),
            fingerprint: b64url_encode(&[1u8; 32]),
            recognition_key: b64url_encode(&[2u8; 32]),
            remote_port: None,
            addresses: vec![],
            issued_at: None,
            last_address: None,
        }
    }

    #[test]
    fn identity_created_once() {
        let dir = temp_dir();
        let store = Store::open(Some(dir.clone())).unwrap();
        let a = store.identity().unwrap();
        let b = store.identity().unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn save_find_forget() {
        let dir = temp_dir();
        let store = Store::open(Some(dir.clone())).unwrap();
        assert!(store.macs().unwrap().is_empty());
        store.save_mac(mac("AAAA1111BBBB2222", "Studio")).unwrap();
        let mut changed = mac("AAAA1111BBBB2222", "Studio Mac");
        changed.remote_port = Some(7455);
        store.save_mac(changed.clone()).unwrap();
        assert_eq!(store.macs().unwrap(), vec![changed.clone()]);
        assert_eq!(
            store.find("AAAA1111BBBB2222").unwrap(),
            Some(changed.clone())
        );
        assert_eq!(store.forget("AAAA1111BBBB2222").unwrap(), Some(changed));
        assert!(store.macs().unwrap().is_empty());
        assert_eq!(store.forget("nothing").unwrap(), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn find_by_prefix_and_name() {
        let dir = temp_dir();
        let store = Store::open(Some(dir.clone())).unwrap();
        store.save_mac(mac("AAAA1111BBBB2222", "Studio")).unwrap();
        store.save_mac(mac("AAAA9999CCCC3333", "Laptop")).unwrap();
        assert_eq!(
            store.find("studio").unwrap().unwrap().mac_id,
            "AAAA1111BBBB2222"
        );
        assert_eq!(
            store.find("AAAA1").unwrap().unwrap().mac_id,
            "AAAA1111BBBB2222"
        );
        assert!(store.find("AAAA").unwrap().is_none());
        assert!(store.find("AAA").unwrap().is_none());
        assert!(store.find("zzzz").unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn find_tolerates_apostrophes_and_prefixes() {
        let dir = temp_dir();
        let store = Store::open(Some(dir.clone())).unwrap();
        store
            .save_mac(mac("AAAA1111BBBB2222", "Ryan\u{2019}s MacBook Pro"))
            .unwrap();
        store.save_mac(mac("CCCC1111DDDD2222", "Studio")).unwrap();
        let want = Some("AAAA1111BBBB2222".to_string());
        let id = |q: &str| store.find(q).unwrap().map(|m| m.mac_id);
        assert_eq!(id("Ryan's MacBook Pro"), want);
        assert_eq!(id("  ryan's macbook pro "), want);
        assert_eq!(id("ryan"), want);
        assert_eq!(id("ry"), None);
        assert_eq!(id("stu"), Some("CCCC1111DDDD2222".to_string()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn file_modes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir();
        let store = Store::open(Some(dir.clone())).unwrap();
        store.identity().unwrap();
        store.save_mac(mac("AAAA1111BBBB2222", "Studio")).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("device-key.pem")), 0o600);
        assert_eq!(mode(&dir.join("macs.json")), 0o600);
        store.write_file("prefs.json", b"{}").unwrap();
        assert_eq!(mode(&dir.join("prefs.json")), 0o600);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn named_files_round_trip() {
        let dir = temp_dir();
        let store = Store::open(Some(dir.clone())).unwrap();
        assert_eq!(store.read_file("prefs.json").unwrap(), None);
        store.write_file("prefs.json", b"{\"version\":1}").unwrap();
        assert_eq!(
            store.read_file("prefs.json").unwrap().as_deref(),
            Some(&b"{\"version\":1}"[..])
        );
        store.write_file("prefs.json", b"{}").unwrap();
        assert_eq!(
            store.read_file("prefs.json").unwrap().as_deref(),
            Some(&b"{}"[..])
        );
        assert!(!dir.join("prefs.json.tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_file_names_are_refused() {
        let dir = temp_dir();
        let store = Store::open(Some(dir.clone())).unwrap();
        for bad in [
            "",
            "../x",
            "a/b",
            "a\\b",
            ".hidden",
            "-x",
            "Prefs.json",
            "x.tmp",
            "macs.json",
            "device-key.pem",
            "caf\u{e9}.json",
            &"a".repeat(65),
        ] {
            assert!(
                matches!(store.write_file(bad, b"x"), Err(NetError::Store(ref m)) if m == "bad file name"),
                "{bad:?}"
            );
            assert!(store.read_file(bad).is_err(), "{bad:?}");
        }
        assert!(store.write_file("crash.log", b"x").is_ok());
        assert!(store.write_file("0-a_b.c", b"x").is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }
}
