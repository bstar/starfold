//! STAR/FOLD — a stack-based terminal file manager.

mod audio_embed;
mod bundled_amp;
mod bundled_preview;
mod cli;
mod config;
mod fold;
#[cfg(feature = "terminal-graphics")]
mod graphical;
mod paths;
mod session;
mod ui;
mod updates;
#[cfg(feature = "terminal-graphics")]
mod video_transport;

use std::io::Write as _;
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser as _;

use paths::PATHS;

fn main() -> Result<()> {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--bundled-amp-version"))
    {
        anyhow::ensure!(
            cfg!(bundled_staramp),
            "This executable does not contain a bundled AMP helper"
        );
        anyhow::ensure!(
            bundled_amp::command().arg("--version").status()?.success(),
            "Bundled AMP helper failed its version check"
        );
        return Ok(());
    }
    #[cfg(feature = "terminal-graphics")]
    if graphical::handles_args() {
        updates::startup();
        return graphical::main();
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--preview-worker")) {
        return fold::preview::connection::child_main();
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--archive-worker")) {
        return fold::archive::connection::child_main(&PathBuf::from(
            std::env::args_os()
                .nth(2)
                .ok_or_else(|| anyhow::anyhow!("Missing archive request"))?,
        ));
    }
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--elevated-delete")) {
        let path = PathBuf::from(
            std::env::args_os()
                .nth(2)
                .ok_or_else(|| anyhow::anyhow!("Missing delete target"))?,
        );
        if std::env::args_os().nth(3).is_some() {
            anyhow::bail!("Unexpected elevated-delete argument");
        }
        return fold::elevated::delete_one(&path).map_err(anyhow::Error::msg);
    }
    let cli = cli::Cli::parse();
    #[cfg(feature = "terminal-graphics")]
    if cli.command.is_none() {
        updates::startup();
        return graphical::main();
    }

    PATHS.init_private_dirs();
    // The guard must outlive everything that logs; dropping it early loses
    // whatever the writer thread had buffered.
    let _log = starkit::logging::init(&PATHS, cli.verbose)?;

    match cli.command {
        Some(cli::Command::List { dir, hidden, sort }) => run_list(dir, hidden, sort),
        Some(cli::Command::Update { action }) => updates::command(action),
        None => {
            updates::startup();
            anyhow::ensure!(
                !cli.graphical,
                "This build has no graphical frontend; use --cells"
            );
            run_tui(cli.dir)
        }
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
/// restores its own last directory. A worker checks saved locations and
/// falls back to the working directory when a mount has disappeared.
/// `session.show_hidden`/`sort` are applied right after `Handle::spawn` --
/// before the window ever draws a frame -- so the first thing on screen is
/// already how the last session left it rather than the defaults for one
/// redraw.
fn run_tui(dir: Option<PathBuf>) -> Result<()> {
    let (core, cfg, config_path, session_path) = window_parts(dir, None)?;
    ui::app::App::run(core, cfg, config_path, Some(session_path))
}

fn window_parts(
    dir: Option<PathBuf>,
    session_override: Option<PathBuf>,
) -> Result<(fold::Handle, config::Config, PathBuf, PathBuf)> {
    let config_path = PATHS.config_file()?;
    match config::Config::write_template(&config_path) {
        Ok(true) => tracing::info!("wrote a starting config.toml to {}", config_path.display()),
        Ok(false) => {}
        Err(e) => tracing::warn!("could not write a starting config.toml: {e}"),
    }
    let cfg = config::Config::load(&config_path)?;

    let session_path = session_override.unwrap_or(PATHS.session_file()?);
    let session = session::load(&session_path);

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let explicit = dir.clone();
    let restored = restore_locations(dir, &session, &cwd);

    let home = std::env::home_dir()
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| restored.fold.path.clone());

    let core = fold::Handle::spawn_startup(cfg.core(), restored, home, fold::Handle::probe_trash());
    if let Some(hidden) = session.show_hidden {
        core.send(fold::Command::SetHidden(hidden));
    }
    core.send(fold::Command::RestoreSorts {
        fold: session.sort,
        panes: [session.commander_left_sort, session.commander_right_sort],
    });
    if !session.tabs.is_empty() {
        let (tabs, active) = restore_workspace(&session, explicit);
        core.send(fold::Command::RestoreTabs(tabs, active));
    }
    Ok((core, cfg, config_path, session_path))
}

/// An explicit location starts fresh in only the active pane. A restored
/// search or preview must not keep displaying its previous location.
fn restore_workspace(
    session: &session::Session,
    explicit: Option<PathBuf>,
) -> (Vec<session::TabSession>, usize) {
    let mut tabs = session.tabs.clone();
    let active = session.active_tab.min(tabs.len().saturating_sub(1));
    if let (Some(dir), Some(tab)) = (explicit, tabs.get_mut(active)) {
        let stack = if tab.commander {
            tab.commander_pane.min(1) + 1
        } else {
            0
        };
        if let Some(saved) = tab.stacks.get_mut(stack) {
            *saved = fold::stack::Stack::new(dir).snapshot();
            tab.search = None;
            tab.preview_scroll = 0;
            tab.preview_page = None;
        }
    }
    (tabs, active)
}

fn restore_locations(
    explicit_dir: Option<PathBuf>,
    session: &session::Session,
    cwd: &std::path::Path,
) -> fold::handle::Startup {
    let mut fold = saved_location(session.last_dir.as_ref(), cwd, "Fold");
    let active = session.commander_active.unwrap_or(0).min(1);
    let enabled = session.commander.unwrap_or(false);
    let has_commander =
        enabled || session.commander_left.is_some() || session.commander_right.is_some();
    let mut commander = has_commander.then(|| {
        let dirs = [
            saved_location(
                session
                    .commander_left
                    .as_ref()
                    .or(session.last_dir.as_ref()),
                cwd,
                "left pane",
            ),
            saved_location(
                session
                    .commander_right
                    .as_ref()
                    .or(session.last_dir.as_ref()),
                cwd,
                "right pane",
            ),
        ];
        (dirs, active, enabled)
    });

    if let Some(dir) = explicit_dir {
        if let Some((dirs, active, enabled)) = commander.as_mut() {
            if *enabled {
                dirs[*active] = fold::handle::StartupLocation {
                    path: dir,
                    fallback: None,
                    label: if *active == 0 {
                        "left pane"
                    } else {
                        "right pane"
                    },
                };
            } else {
                fold.path = dir;
                fold.fallback = None;
            }
        } else {
            fold.path = dir;
            fold.fallback = None;
        }
    }

    fold::handle::Startup { fold, commander }
}

fn saved_location(
    saved: Option<&PathBuf>,
    cwd: &std::path::Path,
    label: &'static str,
) -> fold::handle::StartupLocation {
    match saved {
        Some(dir) => fold::handle::StartupLocation {
            path: dir.clone(),
            fallback: Some(cwd.to_path_buf()),
            label,
        },
        None => fold::handle::StartupLocation {
            path: cwd.to_path_buf(),
            fallback: None,
            label,
        },
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
        assert_eq!(restored.fold.path, dir.path());
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
        assert_eq!(restored.fold.path, dir.path());
        let (dirs, active, enabled) = restored.commander.unwrap();
        assert_eq!(
            [dirs[0].path.clone(), dirs[1].path.clone()],
            [left, requested]
        );
        assert_eq!((active, enabled), (1, true));
        assert_eq!(dirs[1].fallback, None);
    }

    #[test]
    fn missing_saved_location_is_deferred_for_worker_validation() {
        let dir = tempfile::tempdir().unwrap();
        let session = session::Session {
            commander: Some(true),
            commander_left: Some(dir.path().join("missing")),
            ..Default::default()
        };
        let restored = restore_locations(None, &session, dir.path());
        let (dirs, active, enabled) = restored.commander.unwrap();
        assert_eq!(dirs[0].path, dir.path().join("missing"));
        assert_eq!(dirs[0].fallback.as_deref(), Some(dir.path()));
        assert_eq!(dirs[1].path, dir.path());
        assert_eq!((active, enabled), (0, true));
    }
    #[test]
    fn cli_override_clears_only_active_pane_search_and_preview() {
        let saved = session::TabSession {
            commander: true,
            commander_pane: 1,
            active_stack: 2,
            stacks: ["/fold", "/left", "/right"]
                .iter()
                .map(|path| fold::stack::Stack::new(PathBuf::from(path)).snapshot())
                .collect(),
            search: Some(session::SearchSession {
                query: "old query".into(),
                root: Some("/right".into()),
                ..Default::default()
            }),
            preview_scroll: 40,
            preview_page: Some(8),
            ..Default::default()
        };
        let session = session::Session {
            active_tab: 1,
            tabs: vec![saved.clone(), saved.clone()],
            ..Default::default()
        };
        let (tabs, active) = restore_workspace(&session, Some("/new/project".into()));
        assert_eq!(active, 1);
        assert_eq!(tabs[0], saved);
        assert_eq!(tabs[1].stacks[0], saved.stacks[0]);
        assert_eq!(tabs[1].stacks[1], saved.stacks[1]);
        assert_eq!(
            tabs[1].stacks[2].frames[0].dir,
            PathBuf::from("/new/project")
        );
        assert!(tabs[1].search.is_none());
        assert_eq!(tabs[1].preview_scroll, 0);
        assert_eq!(tabs[1].preview_page, None);
    }
}
