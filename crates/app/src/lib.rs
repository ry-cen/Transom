//! Transom: view and control a Mac running Sill. The library behind the `transom` (CLI) and
//! `transom-gui` (graphical app) binaries.

pub mod cli;
pub mod commands;
pub mod logging;
pub mod pair_cmd;
pub mod pairflow;
pub mod session_state;
#[cfg(feature = "video")]
pub mod stream;
pub mod target;
pub mod text;
#[cfg(feature = "video")]
pub mod viewer;

#[cfg(feature = "gui")]
pub mod gui;

use cli::Command;
use std::path::PathBuf;
use transom_net::store::Store;

/// Runs one CLI subcommand; returns the process exit code.
pub fn run_command(config_dir: Option<PathBuf>, command: Command) -> anyhow::Result<i32> {
    let store = Store::open(config_dir).map_err(|e| anyhow::anyhow!("{e}"))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    match command {
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
