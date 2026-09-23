//! Bookmarks and mounted places, kept independent of terminal rendering.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationKind {
    Device,
    Volume,
    Network,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub name: String,
    pub path: PathBuf,
    pub kind: LocationKind,
}

#[derive(Debug, Clone, Default)]
pub struct PlacesState {
    pub bookmarks: Vec<Bookmark>,
    pub locations: Vec<Location>,
    pub loading: bool,
    pub error: Option<String>,
    pub(crate) bookmarks_error: Option<String>,
    pub(crate) locations_error: Option<String>,
    pub(crate) file_path: Option<PathBuf>,
    pub(crate) requested_path: Option<PathBuf>,
    pub(crate) revision: u64,
}

impl PlacesState {
    pub(crate) fn sync_error(&mut self) {
        self.error = self
            .bookmarks_error
            .clone()
            .or_else(|| self.locations_error.clone());
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BookmarksFile {
    #[serde(default)]
    bookmark: Vec<Bookmark>,
}

pub fn load_bookmarks(path: &Path) -> Result<Vec<Bookmark>, String> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("reading {}: {error}", path.display())),
    };
    let file: BookmarksFile = toml::from_str(&contents)
        .map_err(|error| format!("parsing {}: {error}", path.display()))?;
    let mut seen = std::collections::HashSet::new();
    for bookmark in &file.bookmark {
        if !bookmark.path.is_absolute() || bookmark.name.trim().is_empty() {
            return Err(format!("{} contains an invalid bookmark", path.display()));
        }
        if !seen.insert(bookmark.path.clone()) {
            return Err(format!(
                "{} contains duplicate bookmark paths",
                path.display()
            ));
        }
    }
    Ok(file.bookmark)
}

pub fn save_bookmarks(path: &Path, bookmarks: &[Bookmark]) -> Result<(), String> {
    let text = toml::to_string_pretty(&BookmarksFile {
        bookmark: bookmarks.to_vec(),
    })
    .map_err(|error| format!("serializing bookmarks: {error}"))?;
    starkit::fs::write_atomic(path, text.as_bytes())
        .map_err(|error| format!("saving {}: {error}", path.display()))
}

#[derive(Debug, Clone)]
struct Mount {
    source: String,
    target: PathBuf,
    fs_type: String,
}

pub fn discover_locations() -> Result<Vec<Location>, String> {
    #[cfg(target_os = "linux")]
    let mounts = {
        let text = std::fs::read("/proc/self/mountinfo")
            .map_err(|error| format!("reading mount information: {error}"))?;
        text.split(|byte| *byte == b'\n')
            .filter_map(parse_linux_mount)
            .collect::<Vec<_>>()
    };
    #[cfg(target_os = "macos")]
    let mounts = mac_mounts()?;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let mounts: Vec<Mount> = Vec::new();

    Ok(classify_mounts(mounts))
}

#[cfg(target_os = "linux")]
fn parse_linux_mount(line: &[u8]) -> Option<Mount> {
    use std::os::unix::ffi::OsStringExt;
    let pivot = line.windows(3).position(|bytes| bytes == b" - ")?;
    let (before, after) = (&line[..pivot], &line[pivot + 3..]);
    let target = ascii_fields(before).nth(4)?;
    let mut rest = ascii_fields(after);
    let fs_type = String::from_utf8_lossy(rest.next()?).into_owned();
    let source = rest.next()?;
    Some(Mount {
        source: String::from_utf8_lossy(&decode_mount_bytes(source)).into_owned(),
        target: PathBuf::from(std::ffi::OsString::from_vec(decode_mount_bytes(target))),
        fs_type,
    })
}

#[cfg(target_os = "linux")]
fn ascii_fields(part: &[u8]) -> impl Iterator<Item = &[u8]> {
    part.split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty())
}

#[cfg(target_os = "linux")]
fn decode_mount_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\'
            && i + 3 < bytes.len()
            && bytes[i + 1..i + 4]
                .iter()
                .all(|byte| (b'0'..=b'7').contains(byte))
        {
            let decoded = u16::from(bytes[i + 1] - b'0') * 64
                + u16::from(bytes[i + 2] - b'0') * 8
                + u16::from(bytes[i + 3] - b'0');
            if let Ok(byte) = u8::try_from(decoded) {
                out.push(byte);
                i += 4;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn mac_mounts() -> Result<Vec<Mount>, String> {
    use std::ffi::CStr;
    use std::mem::{size_of, MaybeUninit};
    use std::os::unix::ffi::OsStringExt;

    for _ in 0..3 {
        // SAFETY: a null buffer asks the kernel for the number of mounts.
        let count = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
        if count < 0 {
            return Err(format!(
                "reading mount information: {}",
                std::io::Error::last_os_error()
            ));
        }
        let capacity = (count as usize).saturating_add(8);
        let bytes = capacity
            .checked_mul(size_of::<libc::statfs>())
            .and_then(|size| libc::c_int::try_from(size).ok())
            .ok_or_else(|| "mount table is too large".to_string())?;
        let mut storage: Vec<MaybeUninit<libc::statfs>> = Vec::with_capacity(capacity);
        // SAFETY: MaybeUninit elements may be uninitialized before getfsstat writes them.
        unsafe { storage.set_len(capacity) };
        // SAFETY: storage owns `bytes` writable bytes and lives through the call.
        let read = unsafe { libc::getfsstat(storage.as_mut_ptr().cast(), bytes, libc::MNT_NOWAIT) };
        if read < 0 {
            return Err(format!(
                "reading mount information: {}",
                std::io::Error::last_os_error()
            ));
        }
        if read as usize >= capacity {
            continue;
        }
        // SAFETY: getfsstat initialized the first `read` statfs records.
        let records = unsafe {
            std::slice::from_raw_parts(storage.as_ptr().cast::<libc::statfs>(), read as usize)
        };
        return Ok(records
            .iter()
            .map(|record| {
                // SAFETY: these fixed-size statfs fields are NUL-terminated C strings.
                let source = unsafe { CStr::from_ptr(record.f_mntfromname.as_ptr()) }
                    .to_string_lossy()
                    .into_owned();
                let target = unsafe { CStr::from_ptr(record.f_mntonname.as_ptr()) }.to_bytes();
                let fs_type = unsafe { CStr::from_ptr(record.f_fstypename.as_ptr()) }
                    .to_string_lossy()
                    .into_owned();
                Mount {
                    source,
                    target: PathBuf::from(std::ffi::OsString::from_vec(target.to_vec())),
                    fs_type,
                }
            })
            .collect());
    }
    Err("mount table changed while reading it".into())
}

fn classify_mounts(mounts: Vec<Mount>) -> Vec<Location> {
    let mut by_path = BTreeMap::<PathBuf, Location>::new();
    for mount in mounts {
        let target = &mount.target;
        if target == Path::new("/")
            || !target.is_absolute()
            || target.starts_with("/proc")
            || target.starts_with("/sys")
            || target.starts_with("/dev")
        {
            continue;
        }
        let fs = mount.fs_type.to_ascii_lowercase();
        let network = matches!(
            fs.as_str(),
            "nfs"
                | "nfs4"
                | "cifs"
                | "smbfs"
                | "sshfs"
                | "fuse.sshfs"
                | "afpfs"
                | "webdav"
                | "davfs"
                | "davfs2"
                | "fuse.gvfsd-fuse"
                | "fuse.kio-fuse"
        ) || fs.starts_with("fuse.rclone")
            || mount.source.starts_with("//");
        let user_mount = target.starts_with("/media")
            || target.starts_with("/run/media")
            || target.starts_with("/mnt")
            || target.starts_with("/Volumes");
        let is_device = mount.source.starts_with("/dev/") || fs.starts_with("fuse.");
        if !network && !(user_mount && is_device) {
            continue;
        }
        let kind = if network {
            LocationKind::Network
        } else if target.starts_with("/media") || target.starts_with("/run/media") {
            LocationKind::Device
        } else {
            LocationKind::Volume
        };
        let name = target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| mount.source.clone());
        let location = Location {
            name,
            path: target.clone(),
            kind,
        };
        by_path
            .entry(target.clone())
            .and_modify(|prior| {
                if kind == LocationKind::Network {
                    *prior = location.clone();
                }
            })
            .or_insert(location);
    }
    by_path.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn bookmarks_round_trip_and_bad_file_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bookmarks.toml");
        let marks = vec![Bookmark {
            name: "Work".into(),
            path: PathBuf::from("/tmp/work"),
        }];
        save_bookmarks(&path, &marks).unwrap();
        assert_eq!(load_bookmarks(&path).unwrap(), marks);
        std::fs::write(&path, "[broken").unwrap();
        assert!(load_bookmarks(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[broken");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mountinfo_decodes_and_classifies() {
        let lines = [
            "1 0 8:1 / /run/media/user/My\\040Drive rw - vfat /dev/sdb1 rw",
            "2 0 0:1 / /mnt/share rw - cifs //host/share rw",
            "3 0 0:2 / /proc rw - proc proc rw",
        ];
        let places = classify_mounts(
            lines
                .iter()
                .filter_map(|line| parse_linux_mount(line.as_bytes()))
                .collect(),
        );
        assert_eq!(places.len(), 2);
        assert_eq!(places[0].kind, LocationKind::Network);
        assert_eq!(places[1].name, "My Drive");
    }

    #[test]
    fn duplicate_mounts_keep_the_network_kind_and_skip_system_mounts() {
        let mounts = vec![
            Mount {
                source: "/dev/sdb1".into(),
                target: "/mnt/share".into(),
                fs_type: "ext4".into(),
            },
            Mount {
                source: "//host/share".into(),
                target: "/mnt/share".into(),
                fs_type: "cifs".into(),
            },
            Mount {
                source: "proc".into(),
                target: "/proc".into(),
                fs_type: "proc".into(),
            },
        ];
        let places = classify_mounts(mounts);
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].kind, LocationKind::Network);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn raw_non_utf8_mount_path_keeps_its_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let line = b"1 0 8:1 / /mnt/raw-\xff rw - ext4 /dev/sdb1 rw";
        let mount = parse_linux_mount(line).unwrap();
        assert_eq!(mount.target.as_os_str().as_bytes(), b"/mnt/raw-\xff");
        let escaped = b"1 0 8:1 / /mnt/escaped-\\377 rw - ext4 /dev/sdb1 rw";
        let mount = parse_linux_mount(escaped).unwrap();
        assert_eq!(mount.target.as_os_str().as_bytes(), b"/mnt/escaped-\xff");
    }

    #[test]
    fn successful_refresh_clears_only_the_location_error() {
        use crate::fold::state::{apply, Change, State};
        use crate::fold::worker::Done;

        let dir = tempfile::tempdir().unwrap();
        let mut state = State::new(
            &crate::fold::FoldConfig::default(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            true,
        );
        apply(
            &mut state,
            Change::Done(Done::PlacesLoaded {
                path: dir.path().join("bookmarks.toml"),
                bookmarks: Err("bad bookmarks".into()),
                locations: Err("mount error".into()),
            }),
        );
        apply(
            &mut state,
            Change::Done(Done::PlacesRefreshed(Ok(Vec::new()))),
        );
        assert_eq!(state.places.error.as_deref(), Some("bad bookmarks"));

        apply(
            &mut state,
            Change::Done(Done::PlacesLoaded {
                path: dir.path().join("bookmarks.toml"),
                bookmarks: Ok(Vec::new()),
                locations: Err("mount error".into()),
            }),
        );
        apply(
            &mut state,
            Change::Done(Done::PlacesRefreshed(Ok(Vec::new()))),
        );
        assert!(state.places.error.is_none());

        let revision = state.places.revision;
        apply(
            &mut state,
            Change::Done(Done::BookmarksSaved {
                revision,
                result: Err("save error".into()),
            }),
        );
        apply(
            &mut state,
            Change::Done(Done::PlacesRefreshed(Ok(Vec::new()))),
        );
        assert_eq!(state.places.error.as_deref(), Some("save error"));
    }

    #[test]
    fn failed_bookmark_load_does_not_enable_overwriting_the_file() {
        use crate::fold::handle::Command;
        use crate::fold::state::{apply, Change, State};
        use crate::fold::worker::{Done, Job};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bookmarks.toml");
        std::fs::write(&path, "[broken").unwrap();
        let mut state = State::new(
            &crate::fold::FoldConfig::default(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            true,
        );
        apply(
            &mut state,
            Change::Command(Command::LoadPlaces(path.clone())),
        );
        let result = load_bookmarks(&path);
        assert!(result.is_err());
        apply(
            &mut state,
            Change::Done(Done::PlacesLoaded {
                path: path.clone(),
                bookmarks: result,
                locations: Ok(Vec::new()),
            }),
        );
        let effects = apply(
            &mut state,
            Change::Command(Command::SaveBookmark {
                name: "Safe".into(),
                path: dir.path().to_path_buf(),
            }),
        );
        assert!(!effects
            .jobs
            .iter()
            .any(|job| matches!(job, Job::SaveBookmarks { .. })));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "[broken");
    }

    #[test]
    fn bookmark_commands_persist_sequential_edits_and_report_failed_saves() {
        use crate::fold::handle::Command;
        use crate::fold::state::{apply, Change, State};
        use crate::fold::worker::{self, Done, IoOutcome, Job};
        use std::sync::atomic::AtomicBool;
        use std::sync::{Arc, RwLock};

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks.toml");
        let cfg = crate::fold::FoldConfig::default();
        let make_state = || {
            State::new(
                &cfg,
                dir.path().to_path_buf(),
                dir.path().to_path_buf(),
                true,
            )
        };
        let mut state = make_state();
        apply(
            &mut state,
            Change::Done(Done::PlacesLoaded {
                path: file.clone(),
                bookmarks: Ok(Vec::new()),
                locations: Ok(Vec::new()),
            }),
        );
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        let mut first = apply(
            &mut state,
            Change::Command(Command::SaveBookmark {
                name: "A".into(),
                path: a.clone(),
            }),
        )
        .jobs;
        let mut second = apply(
            &mut state,
            Change::Command(Command::SaveBookmark {
                name: "B".into(),
                path: b.clone(),
            }),
        )
        .jobs;
        assert_eq!(state.places.bookmarks.len(), 2);
        let worker_state = Arc::new(RwLock::new(make_state()));
        let cancel = AtomicBool::new(false);
        let run = |job| match worker::perform_io(job, &cfg, &worker_state, &cancel) {
            IoOutcome::Done(done) => done,
            _ => panic!("bookmark write must return a result"),
        };
        apply(&mut state, Change::Done(run(first.remove(0))));
        apply(&mut state, Change::Done(run(second.remove(0))));
        assert_eq!(load_bookmarks(&file).unwrap().len(), 2);

        for command in [
            Command::SaveBookmark {
                name: "A again".into(),
                path: a.clone(),
            },
            Command::RenameBookmark {
                path: b.clone(),
                name: "B again".into(),
            },
            Command::RemoveBookmark(a.clone()),
        ] {
            let mut effects = apply(&mut state, Change::Command(command));
            assert!(matches!(effects.jobs[0], Job::SaveBookmarks { .. }));
            apply(&mut state, Change::Done(run(effects.jobs.remove(0))));
        }
        assert_eq!(
            load_bookmarks(&file).unwrap(),
            vec![Bookmark {
                name: "B again".into(),
                path: b
            }]
        );

        let obstruction = dir.path().join("not-a-directory");
        std::fs::write(&obstruction, "blocked").unwrap();
        let mut failed_state = make_state();
        apply(
            &mut failed_state,
            Change::Done(Done::PlacesLoaded {
                path: obstruction.join("bookmarks.toml"),
                bookmarks: Ok(Vec::new()),
                locations: Ok(Vec::new()),
            }),
        );
        let mut effects = apply(
            &mut failed_state,
            Change::Command(Command::SaveBookmark {
                name: "Failed".into(),
                path: a,
            }),
        );
        apply(&mut failed_state, Change::Done(run(effects.jobs.remove(0))));
        assert!(failed_state.places.error.is_some());
    }

    #[cfg(target_os = "linux")]
    proptest! {
        #[test]
        fn mount_parser_never_panics(input in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = parse_linux_mount(&input);
        }
    }
}
