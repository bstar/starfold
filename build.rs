fn main() {
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
