fn main() {
    println!("cargo:rustc-check-cfg=cfg(bundled_previews)");
    let variables = [
        "STARFOLD_BUNDLE_PREVIEW_PDF",
        "STARFOLD_BUNDLE_PREVIEW_VIDEO",
    ];
    let mut bundled = 0;
    for variable in variables {
        println!("cargo:rerun-if-env-changed={variable}");
        if let Some(path) = std::env::var_os(variable) {
            let path = std::fs::canonicalize(path).expect("Bundled preview helper must exist");
            println!("cargo:rerun-if-changed={}", path.display());
            println!("cargo:rustc-env={variable}={}", path.display());
            bundled += 1;
        }
    }
    assert!(
        bundled == 0 || bundled == 2,
        "Bundle both preview helpers together"
    );
    if bundled == 2 {
        println!("cargo:rustc-cfg=bundled_previews");
    }

    println!("cargo:rustc-check-cfg=cfg(bundled_archive)");
    println!("cargo:rerun-if-env-changed=STARFOLD_BUNDLE_ARCHIVE");
    if let Some(path) = std::env::var_os("STARFOLD_BUNDLE_ARCHIVE") {
        let path = std::fs::canonicalize(path).expect("Bundled archive extension must exist");
        println!("cargo:rerun-if-changed={}", path.display());
        println!("cargo:rustc-env=STARFOLD_BUNDLE_ARCHIVE={}", path.display());
        println!("cargo:rustc-cfg=bundled_archive");
    }

    println!("cargo:rustc-check-cfg=cfg(bundled_staramp)");
    println!("cargo:rerun-if-env-changed=STARFOLD_BUNDLE_STARAMP");
    if let Some(path) = std::env::var_os("STARFOLD_BUNDLE_STARAMP") {
        assert!(
            std::env::var_os("CARGO_FEATURE_TERMINAL_GRAPHICS").is_some(),
            "Bundled STAR/AMP requires terminal-graphics"
        );
        let path = std::fs::canonicalize(path).expect("Bundled STAR/AMP must exist");
        println!("cargo:rerun-if-changed={}", path.display());
        println!("cargo:rustc-env=STARFOLD_BUNDLE_STARAMP={}", path.display());
        println!("cargo:rustc-cfg=bundled_staramp");
    }
}
