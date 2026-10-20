//! The command line (clap derive).

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use transom_input::KeymapMode;
use transom_net::session::SourceChoice;
use transom_proto::compat::{default_device_label, COMPAT_APP_VERSION};

#[derive(Parser, Debug)]
#[command(
    name = "transom",
    version,
    about = "View and control a Mac running Sill from Linux, Windows or macOS."
)]
pub struct Cli {
    /// Use this folder for the device key and the saved Macs.
    #[arg(long, global = true, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,
    /// Log more: -v for info, -vv for debug.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// None: open the graphical app (in a build with the `gui` feature), else print help.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// `transom-gui`'s command line: the graphical app only.
#[cfg(feature = "gui")]
#[derive(Parser, Debug)]
#[command(
    name = "transom-gui",
    version,
    about = "View and control a Mac running Sill from Linux, Windows or macOS."
)]
pub struct GuiCli {
    /// Use this folder for the device key, the saved Macs and the preferences.
    #[arg(long, value_name = "DIR")]
    pub config_dir: Option<PathBuf>,
    /// Log more: -v for info, -vv for debug.
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub verbose: u8,
    /// A sill://pair link to open (handled from Phase 3; ignored before).
    #[arg(value_name = "LINK")]
    pub link: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Print this computer's key fingerprint and the config folder.
    Identity,
    /// List Macs running Sill on this network.
    Discover {
        /// How long to listen, in seconds.
        #[arg(long, default_value_t = 3, value_name = "SECS")]
        timeout: u64,
    },
    /// Pair with a Mac.
    Pair {
        #[command(flatten)]
        target: TargetArgs,
        /// The 12-digit code the Mac shows.
        #[arg(long, conflicts_with = "url")]
        code: Option<String>,
        /// A sill://pair link.
        #[arg(long, conflicts_with = "code")]
        url: Option<String>,
        /// How this computer is named in the Mac's device list.
        #[arg(long, default_value_t = default_device_label(), value_name = "LABEL")]
        name: String,
    },
    /// List saved Macs.
    List,
    /// Remove a saved Mac.
    Forget {
        /// Mac ID, name or Mac ID prefix.
        mac: String,
    },
    /// List the windows a Mac is showing.
    Windows {
        #[command(flatten)]
        target: TargetArgs,
        /// The Sill version to report in the hello.
        #[arg(long, default_value = COMPAT_APP_VERSION, value_name = "V")]
        hello_version: String,
        /// How this computer is named in the Mac's device list.
        #[arg(long, default_value_t = default_device_label(), value_name = "LABEL")]
        name: String,
    },
    /// Show the Mac's desktop or one of its windows and send input back.
    Connect {
        #[command(flatten)]
        target: TargetArgs,
        #[command(flatten)]
        opts: ConnectOpts,
    },
}

/// Which Mac to talk to.
#[derive(Args, Clone, Debug, Default)]
pub struct TargetArgs {
    /// The Mac's name (a saved Mac, or a Bonjour instance name).
    #[arg(long, value_name = "NAME")]
    pub mac: Option<String>,
    /// An address to dial instead of searching the network.
    #[arg(long, value_name = "HOST")]
    pub host: Option<String>,
    /// The port for --host (7455 with --remote).
    #[arg(long)]
    pub port: Option<u16>,
    /// Use the Mac's Remote Access door (port 7455 unless --port is given).
    #[arg(long)]
    pub remote: bool,
    /// With --host: trust whatever key the Mac presents (unsafe).
    #[arg(long, conflicts_with = "plain")]
    pub any_key: bool,
    /// With --host or --mac: connect without TLS to a plain door.
    #[arg(long)]
    pub plain: bool,
}

/// What the viewer needs besides the target.
#[derive(Args, Clone, Debug)]
#[cfg_attr(not(feature = "video"), allow(dead_code))]
pub struct ConnectOpts {
    /// What to show: auto, desktop, keep or window:ID.
    #[arg(long, default_value = "auto", value_parser = parse_source)]
    pub source: SourceChoice,
    /// Modifier layout: pc (Ctrl acts as Command) or mac. Default: mac on macOS, else pc.
    #[arg(long, value_parser = parse_keymap)]
    pub keymap: Option<KeymapMode>,
    /// The frame rate to ask the Mac for.
    #[arg(long, default_value_t = 60, value_name = "N")]
    pub fps: i64,
    /// Forward the pointer even when the Mac is this computer.
    #[arg(long)]
    pub allow_local_pointer: bool,
    /// Forward the keyboard even when the Mac is this computer.
    #[arg(long)]
    pub allow_local_keyboard: bool,
    /// The Sill version to report in the hello.
    #[arg(long, default_value = COMPAT_APP_VERSION, value_name = "V")]
    pub hello_version: String,
    /// How this computer is named in the Mac's device list.
    #[arg(long, default_value_t = default_device_label(), value_name = "LABEL")]
    pub name: String,
}

#[cfg_attr(not(feature = "video"), allow(dead_code))]
impl ConnectOpts {
    pub fn keymap_mode(&self) -> KeymapMode {
        self.keymap.unwrap_or_else(KeymapMode::default_for_os)
    }
}

pub fn parse_source(s: &str) -> Result<SourceChoice, String> {
    const BAD: &str = "expected auto, desktop, keep or window:ID";
    match s {
        "auto" => Ok(SourceChoice::Auto),
        "desktop" => Ok(SourceChoice::Desktop),
        "keep" => Ok(SourceChoice::Keep),
        _ => s
            .strip_prefix("window:")
            .and_then(|id| id.parse::<u32>().ok())
            .map(SourceChoice::Window)
            .ok_or_else(|| BAD.to_string()),
    }
}

fn parse_keymap(s: &str) -> Result<KeymapMode, String> {
    match s {
        "pc" => Ok(KeymapMode::Pc),
        "mac" => Ok(KeymapMode::Mac),
        _ => Err("expected pc or mac".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_parses() {
        assert_eq!(parse_source("auto"), Ok(SourceChoice::Auto));
        assert_eq!(parse_source("desktop"), Ok(SourceChoice::Desktop));
        assert_eq!(parse_source("keep"), Ok(SourceChoice::Keep));
        assert_eq!(parse_source("window:42"), Ok(SourceChoice::Window(42)));
        assert!(parse_source("window:x").is_err());
    }

    #[test]
    fn target_flags_conflict() {
        let r = Cli::try_parse_from(["transom", "connect", "--any-key", "--plain"]);
        assert!(r.is_err());
        let ok = Cli::try_parse_from(["transom", "connect", "--any-key"]);
        assert!(ok.is_ok());
    }

    #[test]
    fn cli_definition_is_valid() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn no_subcommand_parses() {
        let c = Cli::try_parse_from(["transom"]).unwrap();
        assert!(c.command.is_none());
        let c = Cli::try_parse_from(["transom", "-v", "--config-dir", "/tmp/x"]).unwrap();
        assert!(c.command.is_none());
        assert_eq!(c.verbose, 1);
        let c = Cli::try_parse_from(["transom", "list"]).unwrap();
        assert!(matches!(c.command, Some(Command::List)));
    }

    #[cfg(feature = "gui")]
    #[test]
    fn gui_cli_parses() {
        use clap::CommandFactory;
        GuiCli::command().debug_assert();
        let g = GuiCli::try_parse_from(["transom-gui", "-vv", "sill://pair?x"]).unwrap();
        assert_eq!(g.verbose, 2);
        assert_eq!(g.link.as_deref(), Some("sill://pair?x"));
        assert!(GuiCli::try_parse_from(["transom-gui", "list", "extra"]).is_err());
    }
}
