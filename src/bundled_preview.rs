//! macOS standalone updates replace one executable. Carry matching compressed
//! helpers there as a fallback; Nix and AppImage retain adjacent executables.
#[cfg(any(bundled_previews, test))]
use std::path::Path;
use std::path::PathBuf;
pub fn executable(id: &str) -> anyhow::Result<Option<PathBuf>> {
    #[cfg(bundled_previews)]
    {
        let bytes: &[u8] = match id {
            "pdf" => include_bytes!(env!("STARFOLD_BUNDLE_PREVIEW_PDF")),
            "video" => include_bytes!(env!("STARFOLD_BUNDLE_PREVIEW_VIDEO")),
            _ => return Ok(None),
        };
        let root = crate::PATHS.cache_dir()?.join("bundled-previews");
        materialize(&root, id, bytes).map(Some)
    }
    #[cfg(not(bundled_previews))]
    {
        let _ = id;
        Ok(None)
    }
}
#[cfg(any(bundled_previews, test))]
fn materialize(root: &Path, id: &str, compressed: &[u8]) -> anyhow::Result<PathBuf> {
    use sha2::{Digest, Sha256};
    use std::{
        io::{Read, Write},
        os::unix::fs::PermissionsExt,
    };
    let mut bytes = vec![];
    flate2::read::GzDecoder::new(compressed)
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 32 * 1024 * 1024,
        "Bundled preview helper exceeds limit"
    );
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let directory = root.join(hash);
    std::fs::create_dir_all(&directory)?;
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    let path = directory.join(format!("starfold-preview-{id}"));
    if std::fs::read(&path).ok().as_deref() == Some(&bytes) {
        return Ok(path);
    }
    let mut file = tempfile::NamedTempFile::new_in(&directory)?;
    file.write_all(&bytes)?;
    file.as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o700))?;
    file.as_file().sync_all()?;
    file.persist(&path)?;
    Ok(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    fn compress(bytes: &[u8]) -> Vec<u8> {
        let mut out = flate2::write::GzEncoder::new(vec![], Default::default());
        out.write_all(bytes).unwrap();
        out.finish().unwrap()
    }
    #[test]
    fn versioned_helpers_restore_tampering_without_replacing_other_versions() {
        let temp = tempfile::tempdir().unwrap();
        let first = materialize(temp.path(), "pdf", &compress(b"first")).unwrap();
        let second = materialize(temp.path(), "pdf", &compress(b"second")).unwrap();
        assert_ne!(first, second);
        std::fs::write(&first, b"changed").unwrap();
        assert_eq!(
            materialize(temp.path(), "pdf", &compress(b"first")).unwrap(),
            first
        );
        assert_eq!(std::fs::read(&first).unwrap(), b"first");
        assert_eq!(std::fs::read(&second).unwrap(), b"second");
    }
}
