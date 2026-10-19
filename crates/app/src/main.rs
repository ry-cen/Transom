//! Transom: view and control a Mac running Sill.

mod cli;
mod commands;
mod pair_cmd;
mod target;
mod text;
#[cfg(feature = "video")]
mod viewer;

use clap::Parser;
use cli::{Cli, Command};
use std::process::ExitCode;
use transom_net::store::Store;

fn run(cli: Cli) -> anyhow::Result<i32> {
    let store = Store::open(cli.config_dir.clone()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    match cli.command {
        Command::Identity => commands::identity(&store),
        Command::Discover { timeout } => commands::discover(&store, timeout),
        Command::Pair {
            target,
            code,
            url,
            name,
        } => pair_cmd::run(
            pair_cmd::PairArgs {
                target: &target,
                code: code.as_deref(),
                url: url.as_deref(),
                name: &name,
            },
            &store,
            &rt,
        ),
        Command::List => commands::list(&store),
        Command::Forget { mac } => commands::forget(&store, &mac),
        Command::Windows {
            target,
            hello_version,
            name,
        } => commands::windows(&target, &hello_version, &name, &store, &rt),
        Command::Connect { target, opts } => connect(&target, opts, store, &rt),
    }
}

#[cfg(feature = "video")]
fn connect(
    target: &cli::TargetArgs,
    opts: cli::ConnectOpts,
    store: Store,
    rt: &tokio::runtime::Runtime,
) -> anyhow::Result<i32> {
    let route = target::resolve(target, &store, rt)?;
    viewer::run(route, opts, store, rt)
}

#[cfg(not(feature = "video"))]
fn connect(
    _target: &cli::TargetArgs,
    _opts: cli::ConnectOpts,
    _store: Store,
    _rt: &tokio::runtime::Runtime,
) -> anyhow::Result<i32> {
    eprintln!("this build has no viewer (built without the `video` feature)");
    Ok(2)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let level = match cli.verbose {
        0 => log::LevelFilter::Warn,
        1 => log::LevelFilter::Info,
        _ => log::LevelFilter::Debug,
    };
    env_logger::Builder::new()
        .filter_level(level)
        .parse_default_env()
        .init();
    match run(cli) {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(e) => {
            eprintln!("transom: {e:#}");
            ExitCode::from(1)
        }
    }
}
