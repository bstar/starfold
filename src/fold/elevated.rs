//! The narrowly scoped delete performed by a fresh STAR/FOLD process under sudo.

use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Component, Path};
use std::process::{Command, Stdio};
use std::time::Duration;

use super::ops::progress::Progress;
use super::ops::Outcome;

pub fn is_permission_error(reason: &str) -> bool {
    let reason = reason.to_ascii_lowercase();
    reason.contains("permission denied") || reason.contains("operation not permitted")
}

/// Run one narrowly scoped privileged process per failed source. The worker
/// cannot prompt: `sudo -v` was completed on the UI's terminal first.
pub fn run(sources: &[std::path::PathBuf], progress: &Progress) -> Outcome {
    let mut outcome = Outcome::default();
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            outcome.failed.extend(
                sources
                    .iter()
                    .cloned()
                    .map(|path| (path, format!("cannot locate STAR/FOLD executable: {error}"))),
            );
            return outcome;
        }
    };
    for path in sources {
        if progress.is_cancelled() {
            outcome.cancelled = true;
            break;
        }
        let mut child = match Command::new("sudo")
            .args(["-n", "--"])
            .arg(&exe)
            .arg("--elevated-delete")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                outcome
                    .failed
                    .push((path.clone(), format!("{}: {error}", path.display())));
                progress.add(1);
                continue;
            }
        };
        loop {
            if progress.is_cancelled() {
                let _ = child.kill();
                outcome.cancelled = true;
                break;
            }
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(error) => {
                    let _ = child.kill();
                    outcome
                        .failed
                        .push((path.clone(), format!("{}: {error}", path.display())));
                    break;
                }
            }
        }
        let result = child.wait_with_output();
        if outcome.cancelled {
            break;
        }
        match result {
            Ok(output) if output.status.success() => outcome.done += 1,
            Ok(output) => {
                let reason = String::from_utf8_lossy(&output.stderr).trim().to_string();
                outcome.failed.push((
                    path.clone(),
                    if reason.is_empty() {
                        format!("{}: administrator delete failed", path.display())
                    } else {
                        format!("{}: {reason}", path.display())
                    },
                ));
            }
            Err(error) => outcome
                .failed
                .push((path.clone(), format!("{}: {error}", path.display()))),
        }
        progress.add(1);
    }
    outcome
}

/// Called only by the hidden `--elevated-delete` entry point. It does not
/// load a user config or run the TUI as root.
pub fn delete_one(path: &Path) -> Result<(), String> {
    // SAFETY: geteuid only reads process credentials.
    if unsafe { libc::geteuid() } != 0 {
        return Err("administrator privileges are required".into());
    }
    validate_target(path)?;

    let metadata = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if metadata.is_dir() {
        let parent = path.parent().ok_or("delete target has no parent")?;
        let parent_device = fs::metadata(parent)
            .map_err(|e| format!("{}: {e}", parent.display()))?
            .dev();
        if metadata.dev() != parent_device {
            return Err(format!(
                "{}: refusing to delete a mount root",
                path.display()
            ));
        }
        reject_nested_mounts(path, metadata.dev())?;
        fs::remove_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))
    } else {
        fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

fn validate_target(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("administrator delete requires an absolute path".into());
    }
    let mut names = 0;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(_) => names += 1,
            _ => return Err("administrator delete refuses . and .. in the path".into()),
        }
    }
    if names < 2 {
        return Err("administrator delete refuses a filesystem root".into());
    }
    Ok(())
}

fn reject_nested_mounts(root: &Path, device: u64) -> Result<(), String> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if metadata.is_dir() {
                if metadata.dev() != device {
                    return Err(format!("{}: refusing to cross a mount", path.display()));
                }
                pending.push(path);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevated_target_must_be_a_specific_absolute_entry() {
        for path in ["/", "/home", "relative/file", "/tmp/../etc/file"] {
            assert!(validate_target(Path::new(path)).is_err(), "{path}");
        }
        assert!(validate_target(Path::new("/tmp/a")).is_ok());
    }

    #[test]
    fn mount_scan_does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("child");
        fs::create_dir(&child).unwrap();
        std::os::unix::fs::symlink("/", child.join("root-link")).unwrap();
        let device = fs::metadata(dir.path()).unwrap().dev();
        assert!(reject_nested_mounts(dir.path(), device).is_ok());
    }
}
