//! STAR/FOLD — a stack-based terminal file manager.

mod audio_embed;
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
/// A command-line directory opens in the active view. Otherwise each view
/// restores its own last directory, falling back to the working directory
/// when a saved mount has disappeared.
/// `session.show_hidden`/`sort` are applied right after `Handle::spawn` --
/// before the window ever draws a frame -- so the first thing on screen is
/// already how the last session left it rather than the defaults for one
/// redraw.
fn run_tui(dir: Option<PathBuf>) -> Result<()> {
    let config_path = PATHS.config_file()?;
    match config::Config::write_template(&config_path) {
        Ok(true) => tracing::info!("wrote a starting config.toml to {}", config_path.display()),
        Ok(false) => {}
        Err(e) => tracing::warn!("could not write a starting config.toml: {e}"),
    }
    let cfg = config::Config::load(&config_path)?;

    let session_path = PATHS.session_file()?;
    let session = session::load(&session_path);

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let restored = restore_locations(dir, &session, &cwd);
    let start = restored.fold_dir;

    let home = std::env::home_dir()
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| start.clone());

    let core = fold::Handle::spawn(cfg.core(), start, home, fold::Handle::probe_trash());
    if let Some(hidden) = session.show_hidden {
        core.send(fold::Command::SetHidden(hidden));
    }
    if let Some(sort) = session.sort {
        core.send(fold::Command::SetSort(sort));
    }
    if let Some((dirs, active, enabled)) = restored.commander {
        core.send(fold::Command::RestoreCommander {
            dirs,
            active,
            enabled,
        });
    }
    for notice in restored.notices {
        tracing::warn!("{notice}");
        core.send(fold::Command::Notify(notice));
    }

    ui::app::App::run(core, cfg, config_path, Some(session_path))
}

struct RestoredLocations {
    fold_dir: PathBuf,
    commander: Option<([PathBuf; 2], usize, bool)>,
    notices: Vec<String>,
}

fn restore_locations(
    explicit_dir: Option<PathBuf>,
    session: &session::Session,
    cwd: &std::path::Path,
) -> RestoredLocations {
    let mut notices = Vec::new();
    let mut fold_dir = restored_dir(session.last_dir.as_ref(), cwd, "Fold", &mut notices);
    let active = session.commander_active.unwrap_or(0).min(1);
    let enabled = session.commander.unwrap_or(false);
    let has_commander =
        enabled || session.commander_left.is_some() || session.commander_right.is_some();
    let mut commander = has_commander.then(|| {
        let dirs = [
            session.commander_left.as_ref().map_or_else(
                || fold_dir.clone(),
                |dir| restored_dir(Some(dir), cwd, "left pane", &mut notices),
            ),
            session.commander_right.as_ref().map_or_else(
                || fold_dir.clone(),
                |dir| restored_dir(Some(dir), cwd, "right pane", &mut notices),
            ),
        ];
        (dirs, active, enabled)
    });

    if let Some(dir) = explicit_dir {
        let dir = dir.canonicalize().unwrap_or(dir);
        if let Some((dirs, active, enabled)) = commander.as_mut() {
            if *enabled {
                dirs[*active] = dir;
            } else {
                fold_dir = dir;
            }
        } else {
            fold_dir = dir;
        }
    }

    RestoredLocations {
        fold_dir,
        commander,
        notices,
    }
}

fn restored_dir(
    saved: Option<&PathBuf>,
    cwd: &std::path::Path,
    label: &str,
    notices: &mut Vec<String>,
) -> PathBuf {
    match saved {
        Some(dir) if dir.is_dir() => dir.canonicalize().unwrap_or_else(|_| dir.clone()),
        Some(dir) => {
            notices.push(format!(
                "saved {label} location {} is unavailable; opened {}",
                dir.display(),
                cwd.display()
            ));
            cwd.to_path_buf()
        }
        None => cwd.to_path_buf(),
    }
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[test]
    fn legacy_session_starts_in_fold() {
        let dir = tempfile::tempdir().unwrap();
        let session = session::Session {
            last_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let restored = restore_locations(None, &session, dir.path());
        assert_eq!(restored.fold_dir, dir.path());
        assert!(restored.commander.is_none());
    }

    #[test]
    fn explicit_directory_replaces_only_the_active_commander_pane() {
        let dir = tempfile::tempdir().unwrap();
        let left = dir.path().join("left");
        let right = dir.path().join("right");
        let requested = dir.path().join("requested");
        for path in [&left, &right, &requested] {
            std::fs::create_dir(path).unwrap();
        }
        let session = session::Session {
            last_dir: Some(dir.path().to_path_buf()),
            commander: Some(true),
            commander_left: Some(left.clone()),
            commander_right: Some(right),
            commander_active: Some(1),
            ..Default::default()
        };
        let restored = restore_locations(Some(requested.clone()), &session, dir.path());
        assert_eq!(restored.fold_dir, dir.path());
        assert_eq!(restored.commander, Some(([left, requested], 1, true)));
    }

    #[test]
    fn missing_saved_location_uses_working_directory_with_notice() {
        let dir = tempfile::tempdir().unwrap();
        let session = session::Session {
            commander: Some(true),
            commander_left: Some(dir.path().join("missing")),
            ..Default::default()
        };
        let restored = restore_locations(None, &session, dir.path());
        assert_eq!(
            restored.commander,
            Some((
                [dir.path().to_path_buf(), dir.path().to_path_buf()],
                0,
                true
            ))
        );
        assert_eq!(restored.notices.len(), 1);
    }
}
