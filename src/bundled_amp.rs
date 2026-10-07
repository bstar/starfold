//! Release-owned AMP helper. Development/Nix wrappers retain their private PATH.
use std::path::PathBuf;

pub fn executable() -> PathBuf {
    #[cfg(bundled_staramp)]
    {
        match materialize(include_bytes!(env!("STARFOLD_BUNDLE_STARAMP"))) {
            Ok(path) => return path,
            Err(error) => {
                tracing::warn!("Bundled STAR/AMP unavailable: {error:#}");
                // Never substitute an unrelated installed AMP after a bundled
                // helper failed to materialize.
                return PathBuf::from("/nonexistent/starfold-bundled-staramp");
            }
        }
    }
    #[cfg(not(bundled_staramp))]
    {
        if let Some(path) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("starfold-staramp")))
            .filter(|p| p.is_file())
        {
            return path;
        }
        PathBuf::from("staramp")
    }
}

pub fn command() -> std::process::Command {
    let mut command = std::process::Command::new(executable());
    // The helper is extracted to a private cache. Only this child receives the
    // AppImage's bundled library search path; external opens retain the user's
    // environment.
    if cfg!(all(bundled_staramp, target_os = "linux")) {
        if let Some(appdir) = std::env::var_os("APPDIR") {
            let libs = PathBuf::from(appdir).join("usr/lib");
            command.env("LD_LIBRARY_PATH", libs);
        }
    }
    command
}

#[cfg(bundled_staramp)]
fn materialize(bytes: &[u8]) -> anyhow::Result<PathBuf> {
    materialize_in(&crate::PATHS.cache_dir()?.join("bundled-amp"), bytes)
}

#[cfg(any(bundled_staramp, test))]
fn materialize_in(root: &std::path::Path, bytes: &[u8]) -> anyhow::Result<PathBuf> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let hash = format!("{:x}", Sha256::digest(bytes));
    let dir = root.join(hash);
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700))?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    let target = dir.join("staramp");
    if std::fs::read(&target).ok().as_deref() == Some(bytes) {
        return Ok(target);
    }
    let mut file = tempfile::NamedTempFile::new_in(&dir)?;
    file.write_all(bytes)?;
    file.as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o700))?;
    file.as_file().sync_all()?;
    file.persist(&target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helpers_are_private_versioned_and_restore_tampered_bytes() {
        let root = tempfile::tempdir().unwrap();
        let first = materialize_in(root.path(), b"first helper").unwrap();
        let second = materialize_in(root.path(), b"second helper").unwrap();
        assert_ne!(first, second);
        std::fs::write(&first, "tampered").unwrap();
        assert_eq!(materialize_in(root.path(), b"first helper").unwrap(), first);
        assert_eq!(std::fs::read(first).unwrap(), b"first helper");
        assert_eq!(std::fs::read(second).unwrap(), b"second helper");
    }
}
