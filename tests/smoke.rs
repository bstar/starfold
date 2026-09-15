//! End-to-end smoke tests, driving the built binary with no terminal
//! attached.
//!
//! `starfold` is a binary crate, so an integration test cannot reach
//! `Handle` or `run_list` directly the way an in-crate test can; this runs
//! the binary itself, the same way a person or a script would run
//! `starfold list`. Unlike STAR/CORD's `tests/smoke.rs`, nothing here needs
//! a live account or a network, so there is no environment variable to gate
//! these on -- they run on every `cargo test`, including CI.

use std::process::Command;

/// `$STARFOLD_DIR` pointed at a fresh temporary directory, so the test never
/// touches a real `~/.local/starfold` -- `main` calls
/// `PATHS.init_private_dirs()` before it even looks at the subcommand, and a
/// test run should not create or write to a real user's config, session or
/// log.
fn command(home: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_starfold"));
    cmd.env("STARFOLD_DIR", home);
    cmd
}

#[test]
fn listing_a_directory_prints_its_entries_and_exits_cleanly() {
    let home = tempfile::tempdir().expect("a temporary home");
    let target = tempfile::tempdir().expect("a temporary directory to list");
    std::fs::write(target.path().join("a.txt"), b"hello").unwrap();
    std::fs::write(target.path().join("b.txt"), b"world").unwrap();
    std::fs::create_dir(target.path().join("sub")).unwrap();

    let output = command(home.path())
        .args(["list", target.path().to_str().expect("a utf-8 path")])
        .output()
        .expect("running starfold list");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "starfold list exited with {}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        output.status
    );
    assert!(stdout.contains("a.txt"), "{stdout}");
    assert!(stdout.contains("b.txt"), "{stdout}");
    assert!(stdout.contains("sub"), "{stdout}");
}

#[test]
fn listing_a_directory_that_does_not_exist_exits_with_a_failure_code() {
    let home = tempfile::tempdir().expect("a temporary home");
    let missing = home.path().join("nowhere-at-all");

    let output = command(home.path())
        .args(["list", missing.to_str().expect("a utf-8 path")])
        .output()
        .expect("running starfold list");

    assert!(
        !output.status.success(),
        "listing a missing directory must not exit cleanly"
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "the documented exit code for a listing error"
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).is_empty(),
        "the reason belongs on stderr"
    );
}

#[test]
fn version_reports_the_crates_own_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_starfold"))
        .arg("--version")
        .output()
        .expect("running starfold --version");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "expected the crate version in: {stdout}"
    );
}
