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
