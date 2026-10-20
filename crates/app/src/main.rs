//! `transom`: the console CLI. With no subcommand it opens the graphical app (in a build with
//! the `gui` feature), keeping this console for its log; otherwise it prints help and exits 2.

use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use transom_app::cli::Cli;

fn exit_code(r: anyhow::Result<i32>) -> ExitCode {
    match r {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(e) => {
            eprintln!("transom: {e:#}");
            ExitCode::from(1)
        }
    }
}

#[cfg(feature = "gui")]
fn no_command(config_dir: Option<PathBuf>, verbose: u8) -> ExitCode {
    exit_code(transom_app::gui::run(transom_app::gui::GuiArgs {
        config_dir,
        verbose,
        link: None,
    }))
}

#[cfg(not(feature = "gui"))]
fn no_command(_config_dir: Option<PathBuf>, _verbose: u8) -> ExitCode {
    use clap::CommandFactory;
    eprint!("{}", Cli::command().render_help());
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        return no_command(cli.config_dir, cli.verbose);
    };
    let level = match cli.verbose {
        0 => log::LevelFilter::Warn,
        1 => log::LevelFilter::Info,
        _ => log::LevelFilter::Debug,
    };
    env_logger::Builder::new()
        .filter_level(level)
        .parse_default_env()
        .init();
    exit_code(transom_app::run_command(cli.config_dir, command))
}
