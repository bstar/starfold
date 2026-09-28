//! Handing a file to whatever the desktop opens it with.
//!
//! Runs the configured argv, or `open` on macOS and `xdg-open` elsewhere, with
//! every fd on `/dev/null` and the child detached -- never through a shell, so
//! a file called `-x` or one with a space or a semicolon in its name is a
//! single argument, not a chance to inject one.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use super::OpenConfig;
use crate::fold::file_type::{self, FileType};

/// The argv `open_external` would run, without running it -- so the choice of
/// command can be tested without ever spawning a process.
pub fn argv(path: &Path, cfg: &OpenConfig) -> Vec<OsString> {
    let mut out: Vec<OsString> = if cfg.command.is_empty() {
        vec![OsString::from(default_opener())]
    } else {
        cfg.command.iter().map(OsString::from).collect()
    };
    // The path is always exactly one more argument, whatever it contains --
    // never appended to an existing argument, never split.
    out.push(path.as_os_str().to_os_string());
    out
}

#[cfg(target_os = "macos")]
fn default_opener() -> &'static str {
    "open"
}

#[cfg(not(target_os = "macos"))]
fn default_opener() -> &'static str {
    "xdg-open"
}

/// Spawn whatever `argv` names on `path`, and forget about it.
///
/// The child is never waited on: an external viewer is meant to outlive this
/// process, and blocking here would turn "open a file" into "open a file and
/// freeze until its viewer closes". Stdin, stdout and stderr are all nulled
/// so the child cannot read this program's keystrokes or write into the
/// alternate screen.
pub fn open_external(path: &Path, cfg: &OpenConfig) -> std::io::Result<()> {
    // Content-based MIME detection calls an empty .mkv "inode/x-empty", which
    // can route it to a browser. For an empty video, use the desktop's video
    // association instead. The viewer can then report the invalid content.
    #[cfg(target_os = "linux")]
    if cfg.command.is_empty() {
        if let Some(desktop) = empty_video_desktop(path) {
            match spawn_detached("gtk-launch", [OsStr::new(&desktop), path.as_os_str()]) {
                Ok(()) => return Ok(()),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
        }
    }

    let mut args = argv(path, cfg);
    // `args` is never empty: `argv` always pushes at least the default
    // opener or the first configured word, then the path.
    let program = args.remove(0);

    spawn_detached(program, args)
}

fn spawn_detached(
    program: impl AsRef<OsStr>,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
) -> std::io::Result<()> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Dropped rather than waited on: this is the detach.
    drop(child);
    Ok(())
}

#[cfg(target_os = "linux")]
fn empty_video_desktop(path: &Path) -> Option<String> {
    let mime = empty_video_mime(path)?;
    let output = Command::new("xdg-mime")
        .args(["query", "default", &mime])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let desktop = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    desktop.ends_with(".desktop").then_some(desktop)
}

#[cfg(target_os = "linux")]
fn empty_video_mime(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file()
        || metadata.len() != 0
        || file_type::classify(path, &[]) != FileType::Video
    {
        return None;
    }
    Some(
        mime_guess::from_path(path)
            .first()?
            .essence_str()
            .to_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configured_command_wins_over_the_platform_default() {
        let cfg = OpenConfig {
            command: vec!["myviewer".into(), "--flag".into()],
        };
        let got = argv(Path::new("/tmp/file.txt"), &cfg);
        assert_eq!(
            got,
            vec![
                OsString::from("myviewer"),
                OsString::from("--flag"),
                OsString::from("/tmp/file.txt"),
            ]
        );
    }

    #[test]
    fn a_path_with_spaces_and_a_leading_dash_stays_one_argument() {
        let cfg = OpenConfig {
            command: vec!["myviewer".into()],
        };
        let path = Path::new("-x weird name.txt");
        let got = argv(path, &cfg);
        assert_eq!(got.len(), 2, "the command word and the path, nothing split");
        assert_eq!(got[1], OsString::from("-x weird name.txt"));
    }

    #[test]
    fn no_configured_command_falls_back_to_the_platform_opener() {
        let cfg = OpenConfig::default();
        let got = argv(Path::new("/tmp/file.txt"), &cfg);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], OsString::from(default_opener()));
        assert_eq!(got[1], OsString::from("/tmp/file.txt"));
    }

    #[test]
    fn the_platform_default_is_open_on_macos_and_xdg_open_elsewhere() {
        #[cfg(target_os = "macos")]
        assert_eq!(default_opener(), "open");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(default_opener(), "xdg-open");
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn an_empty_video_uses_its_video_mime_for_the_desktop_association() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.mkv");
        std::fs::write(&path, []).unwrap();
        assert_eq!(empty_video_mime(&path).as_deref(), Some("video/x-matroska"));
        let text = dir.path().join("empty.txt");
        std::fs::write(&text, []).unwrap();
        assert_eq!(empty_video_mime(&text), None);
    }
}
