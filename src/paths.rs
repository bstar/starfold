//! Where STAR/FOLD keeps its files.
//!
//! A name for [`starkit::paths::Paths`], and nothing else -- the layout
//! itself is documented there: everything under one directory,
//! `~/.local/starfold` by default, rather than scattered across the three
//! XDG roots, so the config, the session and the log can be backed up, moved
//! between machines, or deleted by moving one folder.

// `own_dir` has no caller yet -- a private trash or a themes directory would
// use it, and neither exists until a later phase -- but it is part of the
// paths vocabulary this file exists to name.
#[allow(unused_imports)]
pub use starkit::paths::{own_dir, Paths};

/// This application's directories. `$STARFOLD_DIR` moves everything;
/// `$STARFOLD_CONFIG_DIR` moves only the configuration, which is what a
/// dotfile manager wants.
pub const PATHS: Paths = Paths::new("starfold", "STARFOLD_DIR", "STARFOLD_CONFIG_DIR");

#[cfg(test)]
mod tests {
    use super::*;

    /// STAR/KIT's own tests cover the layout; what is worth asserting here is
    /// that this application's own three strings reach it -- an environment
    /// variable spelled for another program would relocate nothing.
    #[test]
    fn the_directory_override_wins() {
        let dir = tempfile::tempdir().unwrap();
        temp_env("STARFOLD_DIR", dir.path().as_os_str(), || {
            assert_eq!(PATHS.app(), "starfold");
            assert_eq!(PATHS.base_dir().unwrap(), dir.path());
            assert_eq!(PATHS.config_file().unwrap(), dir.path().join("config.toml"));
            assert_eq!(
                PATHS.log_dir().unwrap(),
                dir.path().join("cache"),
                "the log belongs in cache, not beside the config"
            );
        });
    }

    #[test]
    fn everything_hangs_off_one_base_directory() {
        if std::env::var_os("STARFOLD_DIR").is_some()
            || std::env::var_os("STARFOLD_CONFIG_DIR").is_some()
        {
            return;
        }
        let Ok(base) = PATHS.base_dir() else {
            return;
        };
        assert!(base.ends_with("starfold"));
        assert!(PATHS.config_file().unwrap().starts_with(&base));
        assert!(PATHS.cache_dir().unwrap().starts_with(&base));
        assert!(PATHS.log_dir().unwrap().starts_with(&base));
        assert!(PATHS.session_file().unwrap().starts_with(&base));
    }

    /// `std::env::set_var` is unsafe in edition 2024 and racy in any edition;
    /// this is its only user, and it still runs on the process-wide table.
    fn temp_env(key: &str, value: &std::ffi::OsStr, f: impl FnOnce()) {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var_os(key);
        // SAFETY: serialised by `LOCK` above, which every caller of this
        // helper takes before touching the environment.
        unsafe { std::env::set_var(key, value) };
        f();
        match old {
            Some(v) => unsafe { std::env::set_var(key, v) },
            None => unsafe { std::env::remove_var(key) },
        }
    }
}
