//! Archive service shared by previews and operations. No UI or state access.
pub mod connection;
pub mod operation;
mod rar;
mod read;
mod write;
pub use read::{extract, list};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
pub use write::create;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub bytes: Option<u64>,
    pub directory: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Format {
    Zip,
    Tar,
    TarGz,
    TarZst,
    TarXz,
    TarBz2,
    SevenZip,
    Rar,
}
impl Format {
    pub fn detect(path: &Path) -> anyhow::Result<Self> {
        use std::io::Read;
        let mut head = [0; 512];
        let n = std::fs::File::open(path)?.read(&mut head)?;
        let h = &head[..n];
        if h.starts_with(b"PK\x03\x04") || h.starts_with(b"PK\x05\x06") {
            return Ok(Self::Zip);
        }
        if h.starts_with(b"7z\xbc\xaf\x27\x1c") {
            return Ok(Self::SevenZip);
        }
        if h.starts_with(b"Rar!\x1a\x07") {
            return Ok(Self::Rar);
        }
        if h.get(257..262) == Some(b"ustar") {
            return Ok(Self::Tar);
        }
        Self::from_path(path).ok_or_else(|| anyhow::anyhow!("Unsupported archive format"))
    }
    pub fn from_path(path: &Path) -> Option<Self> {
        let n = path.file_name()?.to_string_lossy().to_ascii_lowercase();
        [
            (".tar.gz", Self::TarGz),
            (".tgz", Self::TarGz),
            (".tar.zst", Self::TarZst),
            (".tzst", Self::TarZst),
            (".tar.xz", Self::TarXz),
            (".txz", Self::TarXz),
            (".tar.bz2", Self::TarBz2),
            (".tbz2", Self::TarBz2),
            (".tar", Self::Tar),
            (".zip", Self::Zip),
            (".7z", Self::SevenZip),
            (".rar", Self::Rar),
        ]
        .into_iter()
        .find(|(ext, _)| n.ends_with(ext))
        .map(|(_, f)| f)
    }
    pub fn writable(self) -> bool {
        !matches!(self, Self::Rar | Self::TarXz | Self::TarBz2)
    }
}
pub fn destination(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let lower = name.to_ascii_lowercase();
    let suffix = [
        ".tar.gz", ".tar.zst", ".tar.xz", ".tar.bz2", ".tgz", ".tbz2", ".tzst", ".txz", ".zip",
        ".tar", ".7z", ".rar",
    ]
    .into_iter()
    .find(|s| lower.ends_with(s));
    path.with_file_name(
        suffix
            .map(|s| &name[..name.len() - s.len()])
            .filter(|s| !s.is_empty())
            .unwrap_or("extracted"),
    )
}
/// Never repair hostile names into different names: reject the archive.
fn safe_path(name: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !name.is_empty() && !name.contains(['\\', ':', '\0']),
        "Unsafe archive member name"
    );
    let p = Path::new(name);
    anyhow::ensure!(
        p.components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
            && p.components().any(|c| matches!(c, Component::Normal(_))),
        "Unsafe archive member path: {name}"
    );
    Ok(p.components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect())
}
const MAX_ENTRIES: usize = 100_000;
const MAX_OUTPUT: u64 = 64 * 1024 * 1024 * 1024;
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    #[test]
    fn rejects_traversal() {
        for n in ["../escape", "/tmp/x", "a/../../x", "C:\\x", "a\\..\\x", ""] {
            assert!(safe_path(n).is_err(), "{n}");
        }
    }
    proptest! { #[test] fn accepted_names_are_relative(s in ".{0,200}") { if let Ok(p)=safe_path(&s) { prop_assert!(!p.is_absolute()); prop_assert!(p.components().all(|c|matches!(c,Component::Normal(_)))); } } }
}
