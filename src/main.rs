//! STAR/FOLD — a stack-based terminal file manager.

mod cli;
mod config;
mod fold;
mod paths;
mod session;
mod ui;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser as _;

use paths::PATHS;

fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    PATHS.init_private_dirs();
    // The guard must outlive everything that logs; dropping it early loses
    // whatever the writer thread had buffered.
    let _log = starkit::logging::init(&PATHS, cli.verbose)?;

    match cli.command {
        Some(cli::Command::List { dir, hidden, sort }) => run_list(dir, hidden, sort),
        None => run_tui(cli.dir),
    }
}

/// `starfold list <dir>`: a headless listing, with no terminal involved.
///
/// `// TODO(1e)`: this is the bootstrap stub. `fold::listing::read` is a
/// stub of its own until Phase 1a, so there is nothing real to print yet.
fn run_list(dir: PathBuf, hidden: bool, sort: cli::SortArg) -> Result<()> {
    let _ = (dir, hidden, sort);
    anyhow::bail!("starfold list is not built yet")
}

/// The window.
///
/// `// TODO(3a)`: this is the bootstrap stub. `ui::app` is Phase 3a's file;
/// until it exists there is nothing to hand the terminal to.
fn run_tui(dir: Option<PathBuf>) -> Result<()> {
    let _ = dir;
    anyhow::bail!("the STAR/FOLD window is not built yet")
}
