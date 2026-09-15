//! Handing a file to whatever the desktop opens it with.
//!
//! Runs the configured argv, or `open` on macOS and `xdg-open` elsewhere, with
//! every fd on `/dev/null` and the child detached -- never through a shell, so
//! a file called `-x` or one with a space or a semicolon in its name is a
//! single argument, not a chance to inject one.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

use super::OpenConfig;

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
    let mut args = argv(path, cfg);
    // `args` is never empty: `argv` always pushes at least the default
    // opener or the first configured word, then the path.
    let program = args.remove(0);

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
}
