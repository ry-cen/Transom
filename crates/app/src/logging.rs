//! The graphical app's logging: standard error at the `-v` level, or `transom.log` in the
//! config folder when Preferences › Advanced asks for it (docs/ui-spec.md section 3.8).
#![cfg(feature = "gui")]

use crate::gui::model::Prefs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

/// The log level: Debug for `-vv` or the debug preference, Info for `-v` or the log file, else
/// Warn.
fn level(verbose: u8, prefs: &Prefs) -> log::LevelFilter {
    if verbose >= 2 || prefs.log_debug {
        log::LevelFilter::Debug
    } else if verbose == 1 || prefs.log_file {
        log::LevelFilter::Info
    } else {
        log::LevelFilter::Warn
    }
}

/// Opens `transom.log` for appending (mode 0600 on Unix).
fn open_log(path: &Path) -> std::io::Result<std::fs::File> {
    let mut o = OpenOptions::new();
    o.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

/// Starts logging. Returns the log file's path when logging goes to `transom.log`. Never
/// panics: a file error falls back to standard error with a warning.
pub fn init(verbose: u8, prefs: &Prefs, config_dir: &Path) -> Option<PathBuf> {
    let l = level(verbose, prefs);
    let mut builder = env_logger::Builder::new();
    builder.filter_level(l).parse_default_env();
    let mut path = None;
    let mut file_error = None;
    if prefs.log_file {
        let p = config_dir.join("transom.log");
        match open_log(&p) {
            Ok(f) => {
                builder.target(env_logger::Target::Pipe(Box::new(f)));
                path = Some(p);
            }
            Err(e) => file_error = Some(e),
        }
    }
    // A second init in one process (tests) is not an error worth reporting.
    if builder.try_init().is_err() {
        return None;
    }
    if let Some(e) = file_error {
        log::warn!("could not open the log file, logging to standard error: {e}");
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        let mut p = Prefs::default();
        assert_eq!(level(0, &p), log::LevelFilter::Warn);
        assert_eq!(level(1, &p), log::LevelFilter::Info);
        assert_eq!(level(2, &p), log::LevelFilter::Debug);
        p.log_file = true;
        assert_eq!(level(0, &p), log::LevelFilter::Info);
        p.log_debug = true;
        assert_eq!(level(0, &p), log::LevelFilter::Debug);
    }
}
