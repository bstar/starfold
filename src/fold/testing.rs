//! The fixture tree every core test and every drawn frame is built on.
//!
//! Built in a temporary directory rather than checked in, because a checked-in
//! tree cannot carry a symlink on every platform git runs on, cannot have its
//! mtimes pinned, and would put the one picture it needs beside forty empty
//! files. `testdata/media/harbour.png` is the one thing that cannot be
//! written from here and is copied in.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// A fixture home directory. Dropping it removes the tree.
pub struct Fixture {
    dir: tempfile::TempDir,
}

/// The one instant every fixture mtime is measured from: 2026-09-11 12:00:00
/// UTC (a Friday). Tests that format times pass this as `now`, so "14:02" is
/// the same string on every machine and on every day the test is run.
pub fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_789_128_000)
}

impl Fixture {
    /// The tree:
    ///
    /// ```text
    /// home/
    ///   projects/
    ///     starwire/
    ///       src/main.rs            "fn main() {}\n"
    ///       Cargo.toml             a few lines of toml
    ///       README.md              markdown, 31 lines
    ///       .gitignore             hidden
    ///       target/                empty directory
    ///   pictures/
    ///     harbour.png              testdata/media/harbour.png, 64x48
    ///   notes.txt -> projects/starwire/README.md   symlink
    ///   dangling -> nowhere        broken symlink
    ///   blob.bin                   256 bytes with NULs in them
    ///   empty/                     empty directory
    /// ```
    ///
    /// mtimes: `src/main.rs` two minutes before [`now`], `Cargo.toml` an hour
    /// before, everything else a day before. Sizes are whatever the bytes
    /// written come to; a test that needs one reads it back rather than
    /// hard-coding it.
    pub fn tree() -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let home = dir.path();
        let starwire = home.join("projects/starwire");
        fs::create_dir_all(starwire.join("src")).unwrap();
        fs::create_dir_all(starwire.join("target")).unwrap();
        fs::create_dir_all(home.join("pictures")).unwrap();
        fs::create_dir_all(home.join("empty")).unwrap();

        write(&starwire.join("src/main.rs"), b"fn main() {}\n");
        write(
            &starwire.join("Cargo.toml"),
            b"[package]\nname = \"starwire\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nanyhow = \"1\"\n",
        );
        let readme: String = (1..=31)
            .map(|n| {
                if n == 1 {
                    "# STAR/WIRE\n".to_string()
                } else {
                    format!("line {n}\n")
                }
            })
            .collect();
        write(&starwire.join("README.md"), readme.as_bytes());
        write(&starwire.join(".gitignore"), b"/target\n");

        let png = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/media/harbour.png");
        fs::copy(&png, home.join("pictures/harbour.png")).expect("testdata/media/harbour.png");

        let mut blob = Vec::with_capacity(256);
        for i in 0..256u32 {
            blob.push(if i % 7 == 0 { 0 } else { (i % 251) as u8 });
        }
        write(&home.join("blob.bin"), &blob);

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("projects/starwire/README.md", home.join("notes.txt"))
                .unwrap();
            std::os::unix::fs::symlink("nowhere", home.join("dangling")).unwrap();
        }

        let day = Duration::from_secs(86_400);
        for path in walk(home) {
            touch(&path, now() - day);
        }
        touch(
            &starwire.join("src/main.rs"),
            now() - Duration::from_secs(120),
        );
        touch(
            &starwire.join("Cargo.toml"),
            now() - Duration::from_secs(3_600),
        );

        Self { dir }
    }

    /// The fixture's home directory.
    pub fn home(&self) -> &Path {
        self.dir.path()
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }
}

/// Read one directory under `fixture` with a default [`crate::fold::listing::
/// ListConfig`] -- the common case for a test that only cares about one
/// level's rows and would otherwise repeat `listing::read(&f.path(...),
/// &ListConfig::default())` at every call site.
pub fn listing(fixture: &Fixture, relative: &str) -> crate::fold::listing::Listing {
    let dir = if relative.is_empty() {
        fixture.home().to_path_buf()
    } else {
        fixture.path(relative)
    };
    crate::fold::listing::read(&dir, &crate::fold::listing::ListConfig::default())
}

fn write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
}

fn touch(path: &Path, at: SystemTime) {
    // A symlink's own mtime is not what any listing shows, and `File::open`
    // would follow it to the target, which has its own time; skip them.
    if fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return;
    }
    let file = fs::File::open(path).unwrap();
    let times = fs::FileTimes::new().set_modified(at).set_accessed(at);
    file.set_times(times).unwrap();
}

/// Every path under `dir`, files and directories, `dir` itself included --
/// without following symlinks.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![dir.to_path_buf()];
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_dir())
            .unwrap_or(false);
        if is_dir {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tree_is_what_the_doc_comment_says() {
        let f = Fixture::tree();
        assert!(f.path("projects/starwire/src/main.rs").is_file());
        assert!(f.path("projects/starwire/target").is_dir());
        assert!(f.path("pictures/harbour.png").is_file());
        assert!(f.path("empty").is_dir());
        assert_eq!(fs::read(f.path("blob.bin")).unwrap().len(), 256);
        assert!(fs::read(f.path("blob.bin")).unwrap().contains(&0));
        #[cfg(unix)]
        {
            assert!(fs::symlink_metadata(f.path("notes.txt"))
                .unwrap()
                .file_type()
                .is_symlink());
            assert!(
                fs::metadata(f.path("dangling")).is_err(),
                "the dangling link resolves"
            );
        }
        let main_rs = fs::metadata(f.path("projects/starwire/src/main.rs")).unwrap();
        assert_eq!(
            main_rs.modified().unwrap(),
            now() - Duration::from_secs(120)
        );
    }
}
