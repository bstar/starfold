//! Non-destructive creation of one file or directory in the active location.
//! The UI gathers a name; the IO worker performs the filesystem call.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use super::stack::FrameId;
use super::tab::TabId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Directory,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
        }
    }
}

/// The frame that asked for creation, so an eventual listing selects the new
/// item there even if the user switched panes while the worker was busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    pub tab: TabId,
    pub stack: usize,
    pub frame: FrameId,
}

pub fn validate_name(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        return Err("a name cannot be empty");
    }
    if name == "." || name == ".." {
        return Err("that is not a name");
    }
    if name.contains('/') {
        return Err("a name cannot contain /");
    }
    if name.contains('\0') {
        return Err("a name cannot contain NUL");
    }
    Ok(())
}

/// Both calls refuse an existing path, including a symlink, at the point of
/// creation. An earlier `exists()` check would race another process.
pub fn create(kind: Kind, dir: &Path, name: &str) -> Result<PathBuf, String> {
    validate_name(name).map_err(str::to_owned)?;
    let path = dir.join(name);
    let result = match kind {
        Kind::File => OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map(|_| ()),
        Kind::Directory => fs::create_dir(&path),
    };
    result
        .map(|()| path.clone())
        .map_err(|error| format!("creating {} {}: {error}", kind.label(), path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_refuses_existing_files_directories_and_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let path = create(Kind::File, root.path(), "note.txt").unwrap();
        fs::write(&path, "keep this").unwrap();
        assert!(create(Kind::File, root.path(), "note.txt").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep this");

        let dir = create(Kind::Directory, root.path(), "folder").unwrap();
        assert!(dir.is_dir());
        assert!(create(Kind::Directory, root.path(), "folder").is_err());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("missing", root.path().join("link")).unwrap();
            assert!(create(Kind::File, root.path(), "link").is_err());
        }
    }

    #[test]
    fn names_are_single_nonempty_components() {
        for name in ["", "  ", ".", "..", "a/b", "a\0b"] {
            assert!(validate_name(name).is_err(), "{name:?}");
        }
        for name in ["note.txt", "with spaces", ".hidden"] {
            assert!(validate_name(name).is_ok(), "{name:?}");
        }
    }
}
