//! Application identity and startup/CLI glue for KIT's shared release updater.
use std::path::PathBuf;

use anyhow::{Context, Result};
use starkit::update::{Application, Package, Policy, Updater};

fn updater() -> Result<Updater> {
    let package = if cfg!(target_os = "macos") {
        Package::MacArchive
    } else {
        Package::AppImage
    };
    Updater::new(
        Application {
            owner: "bstar".into(),
            repository: "starfold".into(),
            executable: "starfold".into(),
            asset_prefix: if cfg!(feature = "terminal-graphics") {
                "starfold-graphical"
            } else {
                "starfold"
            }
            .into(),
            version: env!("CARGO_PKG_VERSION").into(),
            package,
            architecture: std::env::consts::ARCH.into(),
        },
        crate::PATHS
            .cache_dir()?
            .join("updates")
            .join(if cfg!(feature = "terminal-graphics") {
                "graphical"
            } else {
                "terminal"
            }),
    )
}

fn target() -> Result<PathBuf> {
    // Inside an AppImage current_exe is the disposable mounted payload. The
    // runtime names the real installation in APPIMAGE; only that can update.
    if cfg!(target_os = "linux") {
        return std::env::var_os("APPIMAGE").map(PathBuf::from)
            .context("Automatic Linux updates require a standalone AppImage; use Nix or pacman for a package-managed installation");
    }
    std::env::current_exe().context("Cannot locate STAR/FOLD installation")
}

/// Runs only for a user-facing browser/client launch, before Kitty/terminal
/// setup. Worker/internal/list/help invocations never check or apply updates.
pub fn startup() {
    if std::env::args_os().skip(1).any(|arg| {
        matches!(
            arg.to_str(),
            Some("--help" | "-h" | "--version" | "-V" | "--sessions" | "--capabilities")
        )
    }) {
        return;
    }
    // Graphical server is the persistent browsing host. Relay/terminal bridge
    // processes are only attachment plumbing and do not start update workers.
    if std::env::args_os().nth(1).is_some_and(|arg| {
        let arg = arg.to_string_lossy();
        arg.starts_with("--graphical-") && arg != "--graphical-server"
    }) {
        return;
    }
    let result = (|| -> Result<()> {
        let target = starkit::update::standalone_target(&target()?)?;
        let updater = updater()?;
        if updater.policy()? == Policy::Off {
            return Ok(());
        }
        if updater.activate(&target)? {
            use std::os::unix::process::CommandExt;
            let mut command = std::process::Command::new(&target);
            command.args(std::env::args_os().skip(1));
            // New AppImage runtime must establish its own mount/environment.
            if cfg!(target_os = "linux") {
                command.env_remove("APPIMAGE").env_remove("APPDIR");
            }
            return Err(command.exec().into());
        }
        updater.spawn_background(target)
    })();
    // Package/development installs are expected to skip self-update; no
    // terminal output and no networking. Explicit commands explain the reason.
    if let Err(error) = result {
        tracing::debug!("Standalone update skipped: {error:#}");
    }
}

pub fn command(action: Option<crate::cli::UpdateAction>) -> Result<()> {
    use crate::cli::UpdateAction;
    let updater = updater()?;
    match action.unwrap_or(UpdateAction::Check) {
        UpdateAction::Check => {
            match updater.check()? {
                Some(candidate) => println!("STAR/FOLD {} is available: {}\nRun starfold update install to stage it for the next launch.", candidate.version, candidate.release_url),
                None => println!("No newer compatible published release is available."),
            }
        }
        UpdateAction::Install => {
            let target = starkit::update::standalone_target(&target()?)?;
            anyhow::ensure!(updater.policy()? != Policy::Off, "Updates are disabled; run starfold update enable first");
            match updater.check()? {
                Some(candidate) => {
                    updater.stage(&candidate, &target)?;
                    println!("STAR/FOLD {} is verified and staged. It will be installed on the next launch.", candidate.version);
                }
                None => println!("No newer compatible published release is available."),
            }
        }
        UpdateAction::Enable => {
            starkit::update::standalone_target(&target()?)?;
            updater.set_policy(Policy::Automatic)?;
            println!("Automatic updates enabled. Stable releases are checked in the background every six hours and installed on the next launch.");
        }
        UpdateAction::Notify => {
            starkit::update::standalone_target(&target()?)?;
            updater.set_policy(Policy::Notify)?;
            println!("Notify-only updates enabled. Check starfold update status for available releases; install them with starfold update install.");
        }
        UpdateAction::Disable => {
            updater.set_policy(Policy::Off)?;
            println!("Automatic updates disabled; any staged update was removed.");
        }
        UpdateAction::Status => {
            println!("Build: {} {} ({})", if cfg!(feature = "terminal-graphics") { "graphical" } else { "terminal" }, env!("CARGO_PKG_VERSION"), std::env::consts::ARCH);
            println!("Policy: {:?}", updater.policy()?);
            match target().and_then(|target| starkit::update::standalone_target(&target)) {
                Ok(target) => println!("Installation: {}", target.display()),
                Err(error) => println!("Self-update unavailable: {error:#}"),
            }
            if let Some(version) = updater.pending_version()? { println!("Staged for next launch: {version}"); }
            if let Some(applied) = updater.applied()? {
                println!("{}", notice_lines(&applied, "Last applied:", "").join("\n"));
            }
            let status = updater.status()?;
            if let Some(candidate) = status.available { println!("Available: {} ({})", candidate.version, candidate.release_url); }
            if let Some(error) = status.error { println!("Last check failed: {error}"); }
            if cfg!(feature = "terminal-graphics") {
                println!("Graphical updates require matching graphical release assets with their bundled AMP helper; ordinary terminal releases are never substituted.");
            }
        }
        UpdateAction::Rollback => {
            let target = starkit::update::standalone_target(&target()?)?;
            updater.rollback(&target)?;
            updater.set_policy(Policy::Off)?;
            println!("Restored the previous executable and disabled automatic updates. Relaunch STAR/FOLD to use it.");
        }
    }
    Ok(())
}

/// Filesystem polling stays off the UI thread. Notices wait for existing dialogs.
pub struct Notices {
    receiver: std::sync::mpsc::Receiver<Vec<String>>,
    acknowledge: std::sync::mpsc::SyncSender<()>,
}
impl Notices {
    pub fn start() -> Option<Self> {
        target()
            .and_then(|p| starkit::update::standalone_target(&p))
            .ok()?;
        let updater = updater().ok()?;
        let marker = crate::PATHS
            .cache_dir()
            .ok()?
            .join(if cfg!(feature = "terminal-graphics") {
                "update-notice-seen-graphical.json"
            } else {
                "update-notice-seen-terminal.json"
            });
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let (acknowledge, ack) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new().name("starfold-update-notices".into()).spawn(move || {
            let mut applied_seen: String = std::fs::read_to_string(&marker).unwrap_or_default();
            let mut seen = String::new();
            loop {
                let notice = (|| -> Result<Option<(String, Vec<String>)>> {
                    if let Some(candidate) = updater.applied()? {
                        let key = format!("applied:{}", candidate.version);
                        if applied_seen != key && candidate.version == env!("CARGO_PKG_VERSION") {
                            return Ok(Some((key, notice_lines(&candidate, "Updated to", "The update has been applied."))));
                        }
                    }
                    if updater.policy()? == Policy::Off { return Ok(None); }
                    if let Some(candidate) = updater.status()?.available {
                        if candidate.version == env!("CARGO_PKG_VERSION") { return Ok(None); }
                        let staged = updater.pending_version()?.as_deref() == Some(candidate.version.as_str());
                        let key = format!("available:{}:{staged}", candidate.version);
                        if seen != key {
                            return Ok(Some((key, notice_lines(&candidate, "Update available:", if staged {
                                "Downloaded and verified. Relaunch to apply this update."
                            } else { "Run starfold update install to download this update." }))));
                        }
                    }
                    Ok(None)
                })();
                if let Ok(Some((key, lines))) = notice {
                    if sender.send(lines).is_err() || ack.recv().is_err() { break; }
                    seen = key;
                    // Only applied receipts persist; available notices recur per launch.
                    if seen.starts_with("applied:") {
                        applied_seen = seen.clone();
                        if let Ok(mut file) = tempfile::NamedTempFile::new_in(marker.parent().unwrap()) {
                            use std::io::Write;
                            if file.write_all(seen.as_bytes()).is_ok() { let _ = file.persist(&marker); }
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }).ok()?;
        Some(Self {
            receiver,
            acknowledge,
        })
    }
    pub fn poll(&self) -> Option<Vec<String>> {
        self.receiver.try_recv().ok()
    }
    pub fn shown(&self) {
        let _ = self.acknowledge.try_send(());
    }
}

fn notice_lines(
    candidate: &starkit::update::Candidate,
    heading: &str,
    detail: &str,
) -> Vec<String> {
    let mut lines = vec![
        format!("{heading} STAR/FOLD {}", candidate.version),
        detail.into(),
        String::new(),
        "Changelog".into(),
    ];
    let notes = if candidate.release_notes.trim().is_empty() {
        "No release notes were provided."
    } else {
        &candidate.release_notes
    };
    // Plain text only: release content cannot inject terminal controls.
    let notes: String = notes.chars().take(24000).collect();
    for line in notes.lines() {
        let clean: String = line.chars().filter(|c| !c.is_control()).collect();
        let chars: Vec<char> = clean.chars().collect();
        for chunk in chars.chunks(64) {
            lines.push(chunk.iter().collect());
        }
    }
    lines.push(String::new());
    lines.push(
        candidate
            .release_url
            .chars()
            .filter(|c| !c.is_control())
            .collect(),
    );
    lines
}

#[cfg(feature = "terminal-graphics")]
pub fn frontend_notice() -> Option<String> {
    static NOTICES: std::sync::OnceLock<std::sync::Mutex<Option<Notices>>> =
        std::sync::OnceLock::new();
    let guard = NOTICES
        .get_or_init(|| std::sync::Mutex::new(Notices::start()))
        .lock()
        .ok()?;
    let notices = guard.as_ref()?;
    let mut lines = notices.poll()?;
    lines.insert(0, "Presentation machine update".into());
    notices.shown();
    Some(format!("STARFOLD_UPDATE\n{}", lines.join("\n")))
}

#[cfg(test)]
mod notice_tests {
    use super::*;
    #[test]
    fn changelog_is_plain_text_bounded_and_preserves_version_and_link() {
        let candidate: starkit::update::Candidate = serde_json::from_value(serde_json::json!({
            "tag":"v9.0.0", "version":"9.0.0", "asset":"test", "url":"", "size":0,
            "sha256":"", "release_url":"https://github.com/bstar/starfold/releases/tag/v9.0.0",
            "release_notes":format!("Playback fixed\n\u{1b}]2;bad\u{7}{}", "x".repeat(50000))
        }))
        .unwrap();
        let lines = notice_lines(&candidate, "Updated to", "Applied on this launch.");
        assert!(lines[0].contains("9.0.0"));
        assert!(lines.iter().any(|line| line == "Playback fixed"));
        assert!(!lines.iter().any(|line| line.chars().any(char::is_control)));
        assert!(lines.join("\n").len() < 26000);
        assert_eq!(lines.last().unwrap(), &candidate.release_url);
    }
    proptest::proptest! {
        #[test]
        fn foreign_release_notes_cannot_emit_terminal_controls(notes in ".{0,1024}") {
            let candidate: starkit::update::Candidate = serde_json::from_value(serde_json::json!({
                "tag":"v9.0.0", "version":"9.0.0", "asset":"test", "url":"", "size":0,
                "sha256":"", "release_url":"https://github.com/bstar/starfold/releases/tag/v9.0.0",
                "release_notes":notes
            })).unwrap();
            let lines = notice_lines(&candidate, "Updated to", "Applied.");
            proptest::prop_assert!(lines.iter().all(|line| !line.chars().any(char::is_control)));
        }
    }
}
