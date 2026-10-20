//! prefs.json in the config folder (docs/ui-spec.md section 8.6): versioned, every field
//! defaulted, unknown fields ignored, written atomically with mode 0600 by `Store::write_file`.

use crate::gui::model::Prefs;
use transom_input::KeymapMode;
use transom_net::store::Store;
use transom_net::NetError;
use transom_proto::compat::{default_device_label, COMPAT_APP_VERSION};
use transom_proto::safetext::{label, LABEL_LIMIT};

pub const PREFS_FILE: &str = "prefs.json";

impl Prefs {
    /// The saved preferences; defaults (with a warning in the log) when the file is missing,
    /// unreadable or not valid JSON.
    pub fn load(store: &Store) -> Prefs {
        match store.read_file(PREFS_FILE) {
            Ok(Some(bytes)) => match serde_json::from_slice::<Prefs>(&bytes) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("{PREFS_FILE} is not valid, using defaults: {e}");
                    Prefs::default()
                }
            },
            Ok(None) => Prefs::default(),
            Err(e) => {
                log::warn!("could not read {PREFS_FILE}, using defaults: {e}");
                Prefs::default()
            }
        }
    }

    pub fn save(&self, store: &Store) -> Result<(), NetError> {
        let json = serde_json::to_vec_pretty(self).map_err(|e| NetError::Store(e.to_string()))?;
        store.write_file(PREFS_FILE, &json)
    }

    /// "pc" or "mac"; anything else (or None) is the OS default.
    pub fn keymap_mode(&self) -> KeymapMode {
        match self.keymap.as_deref() {
            Some("pc") => KeymapMode::Pc,
            Some("mac") => KeymapMode::Mac,
            _ => KeymapMode::default_for_os(),
        }
    }

    /// The name sent in the hello and the pairing request: the preference, safe-text filtered,
    /// else "Transom (‹OS›)".
    pub fn device_name(&self) -> String {
        let n = self
            .device_name
            .as_deref()
            .map(|n| label(n, LABEL_LIMIT))
            .unwrap_or_default();
        if n.is_empty() {
            default_device_label()
        } else {
            n
        }
    }

    /// The Sill version reported in the hello: the preference, else the compatibility version.
    pub fn hello_version(&self) -> String {
        let v = self
            .hello_version
            .as_deref()
            .map(|v| label(v, 32))
            .unwrap_or_default();
        if v.is_empty() {
            COMPAT_APP_VERSION.to_string()
        } else {
            v
        }
    }

    /// The frame rate to ask for: 120 when set to 120, else 60.
    pub fn fps_to_ask(&self) -> i64 {
        if self.fps == 120 {
            120
        } else {
            60
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::model::MacPrefs;
    use std::path::PathBuf;

    fn temp_store() -> (Store, PathBuf) {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("transom-prefs-test-{}-{n}", std::process::id()));
        (Store::open(Some(dir.clone())).unwrap(), dir)
    }

    #[test]
    fn missing_file_gives_defaults() {
        let (store, dir) = temp_store();
        assert_eq!(Prefs::load(&store), Prefs::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn round_trip() {
        let (store, dir) = temp_store();
        let mut p = Prefs {
            fps: 120,
            keymap: Some("mac".into()),
            ..Default::default()
        };
        p.macs.insert(
            "AAAA".into(),
            MacPrefs {
                remote_host: Some("100.1.2.3:7455".into()),
            },
        );
        p.save(&store).unwrap();
        assert_eq!(Prefs::load(&store), p);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_file_gives_defaults() {
        let (store, dir) = temp_store();
        store.write_file(PREFS_FILE, b"not json").unwrap();
        assert_eq!(Prefs::load(&store), Prefs::default());
        store
            .write_file(PREFS_FILE, br#"{"fps":120,"unknown":[1,2]}"#)
            .unwrap();
        assert_eq!(Prefs::load(&store).fps, 120);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn derived_values() {
        let p = Prefs::default();
        assert_eq!(p.keymap_mode(), KeymapMode::default_for_os());
        assert_eq!(p.device_name(), default_device_label());
        assert_eq!(p.hello_version(), COMPAT_APP_VERSION);
        assert_eq!(p.fps_to_ask(), 60);
        let p = Prefs {
            keymap: Some("pc".into()),
            device_name: Some("  Desk\npc ".into()),
            hello_version: Some("0.6.0".into()),
            fps: 90,
            ..Default::default()
        };
        assert_eq!(p.keymap_mode(), KeymapMode::Pc);
        assert_eq!(p.device_name(), "Desk pc");
        assert_eq!(p.hello_version(), "0.6.0");
        assert_eq!(p.fps_to_ask(), 60);
    }
}
