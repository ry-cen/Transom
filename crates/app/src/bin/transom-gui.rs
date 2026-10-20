//! `transom-gui`: the graphical app, with no console window on Windows (docs/ui-spec.md
//! sections 7.2 and 8.1). The desktop entry and Start-menu shortcut start this one.
#![cfg_attr(windows, windows_subsystem = "windows")]

use clap::Parser;
use std::process::ExitCode;
use transom_app::cli::GuiCli;
use transom_app::gui::{run, GuiArgs};

fn main() -> ExitCode {
    let cli = GuiCli::parse();
    match run(GuiArgs {
        config_dir: cli.config_dir,
        verbose: cli.verbose,
        link: cli.link,
    }) {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(e) => {
            eprintln!("transom-gui: {e:#}");
            ExitCode::from(1)
        }
    }
}
