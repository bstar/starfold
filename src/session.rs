//! What STAR/FOLD remembers between runs.
//!
//! Not settings -- those are `config.toml`, which a person edits. This is
//! where the fold was left, so the next run opens where this one closed
//! rather than back at the current directory every time. A session file that
//! will not parse is not an error worth reporting: it is a cache of one
//! convenience, and losing it costs a directory.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::fold::sort::SortOrder;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    /// The Fold stack's directory. Kept under its original key so sessions
    /// written before Commander existed still restore normally.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_dir: Option<PathBuf>,
    pub version: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<TabSession>,
    pub active_tab: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commander: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commander_left: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commander_right: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commander_active: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show_hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<SortOrder>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commander_left_sort: Option<SortOrder>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commander_right_sort: Option<SortOrder>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabSession {
    pub id: u64,
    pub name: Option<String>,
    pub stacks: Vec<StackSession>,
    pub active_stack: usize,
    pub commander: bool,
    pub commander_pane: usize,
    pub sort: SortOrder,
    pub pane_sorts: [SortOrder; 2],
    pub show_hidden: bool,
    pub search: Option<SearchSession>,
    pub focus: String,
    pub preview_open: bool,
    pub preview_scroll: usize,
    pub preview_page: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StackSession {
    pub frames: Vec<FrameSession>,
    pub active: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameSession {
    pub dir: PathBuf,
    pub cursor: usize,
    pub cursor_name: Option<String>,
    pub filter: String,
    pub scroll: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchSession {
    pub query: String,
    pub contents: bool,
    pub root: Option<PathBuf>,
    pub cursor: usize,
    pub cursor_path: Option<PathBuf>,
    pub scroll: usize,
}

/// Read the session, or the defaults if there is none, or it will not parse.
/// Either is logged, not returned as an error -- nothing in the caller would
/// do anything with one but fall back to the same defaults anyway.
pub fn load(path: &Path) -> Session {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Session::default();
    };
    match toml::from_str(&text) {
        Ok(session) => session,
        Err(e) => {
            tracing::warn!("ignoring an unreadable session file: {e}");
            Session::default()
        }
    }
}

impl Session {
    /// Write it, through a temporary file in the same directory so an
    /// interrupted write cannot leave a truncated session behind.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).context("serialising the session")?;
        starkit::fs::write_private(path, text.as_bytes())
            .with_context(|| format!("writing {}", path.display()))
    }
}

type PendingSession = (
    std::sync::Mutex<(Option<Session>, bool)>,
    std::sync::Condvar,
);

/// A private, process-lifetime lock and a coalescing writer. Other windows
/// may read the workspace but cannot overwrite its owner's saved tabs.
pub struct Writer {
    lock: std::fs::File,
    pending: std::sync::Arc<PendingSession>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Writer {
    pub fn acquire(path: PathBuf) -> Result<Option<Self>> {
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path.with_extension("lock"))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        let pending = std::sync::Arc::new((
            std::sync::Mutex::new((None::<Session>, false)),
            std::sync::Condvar::new(),
        ));
        let worker = pending.clone();
        let thread = std::thread::Builder::new()
            .name("starfold-session".into())
            .spawn(move || loop {
                let (session, stop) = {
                    let (mutex, ready) = &*worker;
                    let mut state = mutex.lock().unwrap_or_else(|e| e.into_inner());
                    while state.0.is_none() && !state.1 {
                        state = ready.wait(state).unwrap_or_else(|e| e.into_inner());
                    }
                    (state.0.take(), state.1)
                };
                if let Some(session) = session {
                    if let Err(error) = session.save(&path) {
                        tracing::warn!("could not save workspace: {error:#}");
                    }
                }
                if stop {
                    break;
                }
            })?;
        Ok(Some(Self {
            lock,
            pending,
            thread: Some(thread),
        }))
    }

    pub fn save(&self, session: Session) {
        let (mutex, ready) = &*self.pending;
        mutex.lock().unwrap_or_else(|e| e.into_inner()).0 = Some(session);
        ready.notify_one();
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        let (mutex, ready) = &*self.pending;
        mutex.lock().unwrap_or_else(|e| e.into_inner()).1 = true;
        ready.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // A concurrently spawned child may briefly inherit the descriptor
        // before exec. Release ownership explicitly after flushing instead
        // of waiting for every inherited descriptor to close.
        if let Err(error) = self.lock.unlock() {
            tracing::warn!("could not release workspace lock: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");

        let session = Session {
            last_dir: Some(PathBuf::from("/home/bob/projects")),
            commander: Some(true),
            commander_left: Some(PathBuf::from("/media/bob/usb")),
            commander_right: Some(PathBuf::from("/mnt/share")),
            commander_active: Some(1),
            show_hidden: Some(true),
            sort: Some(SortOrder::default()),
            commander_left_sort: Some(SortOrder::default()),
            commander_right_sort: Some(SortOrder::default()),
            ..Default::default()
        };
        session.save(&path).unwrap();

        let read = load(&path);
        assert_eq!(read, session);
    }

    #[test]
    fn a_missing_file_loads_as_the_default_session() {
        let dir = tempfile::tempdir().unwrap();
        let session = load(&dir.path().join("nothing.toml"));
        assert_eq!(session, Session::default());
    }

    #[test]
    fn an_old_session_defaults_to_fold_without_commander_locations() {
        let old = "last_dir = '/home/bob/projects'\nshow_hidden = true\n";
        let session: Session = toml::from_str(old).unwrap();
        assert_eq!(session.last_dir, Some(PathBuf::from("/home/bob/projects")));
        assert_eq!(session.commander, None);
        assert_eq!(session.commander_left, None);
        assert_eq!(session.commander_right, None);
        assert_eq!(session.commander_active, None);
        assert_eq!(session.commander_left_sort, None);
        assert_eq!(session.commander_right_sort, None);
    }

    #[test]
    fn an_unparsable_file_is_ignored_rather_than_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        std::fs::write(&path, "this is not [ toml").unwrap();
        assert_eq!(load(&path), Session::default());
    }

    #[test]
    fn a_missing_parent_directory_is_created_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("session.toml");
        Session::default().save(&path).unwrap();
        assert!(path.is_file());
    }
    #[test]
    fn workspace_writer_has_one_owner_flushes_latest_and_releases_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        let writer = Writer::acquire(path.clone()).unwrap().unwrap();
        // Model a descriptor inherited by a child during process startup.
        let inherited_lock = writer.lock.try_clone().unwrap();
        assert!(Writer::acquire(path.clone()).unwrap().is_none());
        for n in 0..25 {
            writer.save(Session {
                active_tab: n,
                ..Default::default()
            });
        }
        let mut latest = Session {
            version: Some(2),
            active_tab: 1,
            ..Default::default()
        };
        latest.tabs = vec![TabSession {
            name: Some("work".into()),
            stacks: vec![StackSession {
                active: 0,
                frames: vec![FrameSession {
                    dir: "/unmounted/projects".into(),
                    cursor: 12,
                    cursor_name: Some("current.txt".into()),
                    filter: "txt".into(),
                    scroll: 9,
                }],
            }],
            preview_scroll: 6,
            preview_open: true,
            focus: "preview".into(),
            ..Default::default()
        }];
        writer.save(latest.clone());
        drop(writer);
        assert_eq!(load(&path), latest);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(Writer::acquire(path).unwrap().is_some());
        drop(inherited_lock);
    }
    #[test]
    fn legacy_session_migrates_without_requiring_workspace_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.toml");
        std::fs::write(
            &path,
            "last_dir = '/old/project'\ncommander = true\ncommander_active = 1\n",
        )
        .unwrap();
        let saved = load(&path);
        assert!(saved.tabs.is_empty());
        assert_eq!(saved.last_dir, Some("/old/project".into()));
        assert_eq!(saved.commander_active, Some(1));
    }
}
