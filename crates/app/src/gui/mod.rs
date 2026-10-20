//! The graphical app (docs/ui-spec.md). `transom-gui`, and `transom` with no subcommand, call
//! `run`.

pub mod effects;
pub mod images;
pub mod keyboard;
pub mod macs;
pub mod model;
pub mod overlay;
pub mod pairing;
pub mod prefs;
pub mod shell;
pub mod texts;
pub mod view;

use std::path::PathBuf;

/// What both binaries pass to `run`.
#[derive(Clone, Debug, Default)]
pub struct GuiArgs {
    /// `--config-dir`: the device key, the saved Macs and prefs.json.
    pub config_dir: Option<PathBuf>,
    /// `-v` count: 1 info, 2 or more debug.
    pub verbose: u8,
    /// A sill://pair link from the command line (Phase 3; ignored before).
    pub link: Option<String>,
}

/// Runs the app until its window closes; returns the process exit code. WP-C.
pub fn run(args: GuiArgs) -> anyhow::Result<i32> {
    let _ = args;
    eprintln!("transom: the graphical app is not built yet");
    Ok(1)
}
