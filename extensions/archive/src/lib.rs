mod server;
pub use server::serve_stdio;
pub mod native;
pub mod progress;
mod rar;
mod read;
mod write;
pub use read::{copy_member, extract, list};
pub use starfold_archive_protocol::{Entry, Format};

use std::path::{Component, Path, PathBuf};
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

pub fn detect(path: &Path) -> anyhow::Result<Format> {
    use std::io::Read;
    let mut head = [0; 512];
    let n = std::fs::File::open(path)?.read(&mut head)?;
    let h = &head[..n];
    if h.starts_with(b"PK\x03\x04") || h.starts_with(b"PK\x05\x06") {
        return Ok(Format::Zip);
    }
    if h.starts_with(b"7z\xbc\xaf\x27\x1c") {
        return Ok(Format::SevenZip);
    }
    if h.starts_with(b"Rar!\x1a\x07") {
        return Ok(Format::Rar);
    }
    if h.starts_with(&[0x1f, 0x8b]) {
        return Ok(Format::TarGz);
    }
    if h.starts_with(b"BZh") {
        return Ok(Format::TarBz2);
    }
    if h.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0]) {
        return Ok(Format::TarXz);
    }
    if h.starts_with(&[0x04, 0x22, 0x4d, 0x18]) || h.starts_with(&[0x02, 0x21, 0x4c, 0x18]) {
        return Ok(Format::TarLz4);
    }
    if h.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return Ok(Format::TarZst);
    }
    if h.get(257..262) == Some(b"ustar") {
        return Ok(Format::Tar);
    }
    Format::from_path(path).ok_or_else(|| anyhow::anyhow!("Unsupported archive format"))
}
pub mod ops {
    pub use starfold_archive_protocol::{Item, ItemKind};
    pub mod progress {
        pub use crate::progress::Progress;
    }
}
