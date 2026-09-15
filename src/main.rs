//! STAR/FOLD — a stack-based terminal file manager.

mod cli;
mod config;
mod fold;
mod paths;
mod session;
mod ui;

use std::io::Write as _;
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
/// No `Handle`, no worker thread, no `State` -- this runs the same pure
/// functions the io thread would (`listing::read`, `sort::order`,
/// `format::{size,when,mode}`) directly on the calling thread, because there
/// is nothing here that benefits from a second thread: the process reads one
/// directory and exits.
fn run_list(dir: PathBuf, hidden: bool, sort: cli::SortArg) -> Result<()> {
    let cfg = fold::listing::ListConfig {
        show_hidden: hidden,
        sort: fold::sort::SortOrder {
            key: sort.key(),
            ..fold::sort::SortOrder::default()
        },
        ..fold::listing::ListConfig::default()
    };

    let listing = fold::listing::read(&dir, &cfg);

    // Printed and exited here, rather than returned as an `Err` for `main`
    // to report: `anyhow`'s own `Debug` rendering of an error chain is not
    // the one clean line a script parsing this output wants, and the exit
    // code is the only part of that machinery this command needs.
    if let Some(reason) = &listing.error {
        eprintln!("{}: {reason}", dir.display());
        std::process::exit(1);
    }

    let tz = jiff::tz::TimeZone::system();
    let now = std::time::SystemTime::now();

    // Through a locked handle rather than `println!`, which panics when the
    // reader has gone -- `starfold list | head` is the ordinary way to look
    // at a big directory, and a closed pipe is the reader being done, not an
    // error worth a backtrace.
    let mut out = std::io::stdout().lock();
    for index in fold::sort::order(&listing.entries, cfg.sort, cfg.show_hidden) {
        let entry = &listing.entries[index];
        let name = if entry.is_dir_like() {
            format!("{}/", entry.display)
        } else {
            entry.display.clone()
        };
        // A directory's own `len` is the size of its inode, which nobody
        // wants to read as a size; the window shows `-` there too.
        let size = if entry.is_dir_like() {
            "-".to_string()
        } else {
            fold::format::size(entry.len)
        };
        let when = entry
            .modified
            .map(|t| fold::format::when(t, &tz, now))
            .unwrap_or_else(|| "-".to_string());
        let line = format!(
            "{}  {:>8}  {:<10}  {name}",
            fold::format::mode(entry.mode),
            size,
            when,
        );
        if writeln!(out, "{line}").is_err() {
            return Ok(());
        }
    }

    if listing.truncated {
        let _ = writeln!(out, "(truncated)");
    }

    Ok(())
}

/// The window.
///
/// `// TODO(3a)`: this is the bootstrap stub. `ui::app` is Phase 3a's file;
/// until it exists there is nothing to hand the terminal to.
fn run_tui(dir: Option<PathBuf>) -> Result<()> {
    let _ = dir;
    anyhow::bail!("the STAR/FOLD window is not built yet")
}
