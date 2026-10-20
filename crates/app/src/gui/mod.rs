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

/// Runs the app until its window closes; returns the process exit code.
pub fn run(args: GuiArgs) -> anyhow::Result<i32> {
    use anyhow::anyhow;
    use model::{IdentityInfo, Prefs};
    use transom_net::store::Store;
    use transom_proto::identity::b64url_encode;

    let store = Store::open(args.config_dir).map_err(|e| anyhow!("{e}"))?;
    let prefs = Prefs::load(&store);
    let identity = store.identity().map_err(|e| anyhow!("{e}"))?;
    crate::logging::init(args.verbose, &prefs, store.dir());
    let info = IdentityInfo {
        fingerprint_b64: b64url_encode(&identity.fingerprint),
        fingerprint_hex: identity
            .fingerprint
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        config_dir: store.dir().display().to_string(),
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let result = shell::run(&rt, store, identity, prefs, info);
    // Do not hang on a blocked task at exit.
    rt.shutdown_timeout(std::time::Duration::from_millis(500));
    result
}
