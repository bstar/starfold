//! Real-process startup/CLI checks with isolated standalone installations.
#![cfg(unix)]
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn installation() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("starfold");
    std::fs::copy(env!("CARGO_BIN_EXE_starfold"), &binary).unwrap();
    (root, binary)
}
fn run(root: &Path, binary: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(binary);
    command.args(args).env("STARFOLD_DIR", root.join("state"));
    if cfg!(target_os = "linux") {
        command.env("APPIMAGE", root.join("starfold"));
    }
    command.output().unwrap()
}
fn cache(root: &Path) -> PathBuf {
    root.join("state/cache/updates")
        .join(if cfg!(feature = "terminal-graphics") {
            "graphical"
        } else {
            "terminal"
        })
}
#[test]
fn policy_commands_persist() {
    let (root, binary) = installation();
    for (action, expected) in [
        ("enable", "automatic"),
        ("notify", "notify"),
        ("disable", "off"),
    ] {
        let output = run(root.path(), &binary, &["update", action]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let policy = std::fs::read_to_string(cache(root.path()).join("policy.json")).unwrap();
        assert!(policy.contains(expected));
    }
    let output = run(root.path(), &binary, &["update", "status"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Policy: Off"));
}
#[test]
fn pending_updates_activate_before_ui_and_preserve_the_command_symlink() {
    use sha2::{Digest, Sha256};
    let (root, binary) = installation();
    let command = root.path().join("command");
    std::os::unix::fs::symlink(&binary, &command).unwrap();
    let target = std::fs::canonicalize(&binary).unwrap();
    let directory = cache(root.path());
    std::fs::create_dir_all(&directory).unwrap();
    let bytes = b"#!/bin/sh\nif [ \"${1-}\" = --version ]; then echo 'starfold 99.0.0'; else printf 'ACTIVATED %s\\n' \"$*\"; fi\n";
    std::fs::write(directory.join("pending.bin"), bytes).unwrap();
    let prefix = if cfg!(feature = "terminal-graphics") {
        "starfold-graphical"
    } else {
        "starfold"
    };
    let asset = if cfg!(target_os = "macos") {
        format!(
            "{prefix}-99.0.0-{}-apple-darwin.tar.gz",
            std::env::consts::ARCH
        )
    } else {
        format!("{prefix}-99.0.0-{}.AppImage", std::env::consts::ARCH)
    };
    let metadata = serde_json::json!({
        "target": target,
        "candidate": { "tag": "v99.0.0", "version": "99.0.0", "asset": asset, "url": "https://github.com/bstar/starfold/releases/download/v99.0.0/fixture", "size": bytes.len(), "sha256": "ab".repeat(32), "release_url": "https://github.com/bstar/starfold/releases/tag/v99.0.0" },
        "executable_sha256": format!("{:x}", Sha256::digest(bytes)), "executable_size": bytes.len()
    });
    std::fs::write(
        directory.join("pending.json"),
        serde_json::to_vec(&metadata).unwrap(),
    )
    .unwrap();
    let help = run(root.path(), &command, &["--help"]);
    assert!(help.status.success());
    assert!(directory.join("pending.json").exists());
    #[cfg(feature = "terminal-graphics")]
    {
        let help = run(root.path(), &command, &["graphical", "--help"]);
        assert!(help.status.success());
        assert!(directory.join("pending.json").exists());
    }
    // Headless listing does not activate or check updates.
    let listing = run(
        root.path(),
        &command,
        &["list", root.path().to_str().unwrap()],
    );
    assert!(
        listing.status.success(),
        "{}",
        String::from_utf8_lossy(&listing.stderr)
    );
    assert!(directory.join("pending.json").exists());
    let output = run(root.path(), &command, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("ACTIVATED"));
    assert!(std::fs::symlink_metadata(&command).unwrap().is_symlink());
    assert!(!directory.join("pending.json").exists());
    assert_eq!(std::fs::read(binary).unwrap(), bytes);
    assert_eq!(
        std::fs::metadata(root.path().join(".starfold.previous"))
            .unwrap()
            .len(),
        std::fs::metadata(env!("CARGO_BIN_EXE_starfold"))
            .unwrap()
            .len()
    );
}
