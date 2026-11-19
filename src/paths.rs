//! Where STAR/FOLD keeps its files.
//!
//! A name for [`starcore::paths::Paths`], and nothing else -- the layout
//! itself is documented there: everything under one directory,
//! `~/.local/starfold` by default, rather than scattered across the three
//! XDG roots, so the config, the session and the log can be backed up, moved
//! between machines, or deleted by moving one folder.

// `own_dir` has no caller yet -- a private trash or a themes directory would
// use it, and neither exists until a later phase -- but it is part of the
// paths vocabulary this file exists to name.
#[allow(unused_imports)]
pub use starcore::paths::{own_dir, Paths};

/// This application's directories. `$STARFOLD_DIR` moves everything;
/// `$STARFOLD_CONFIG_DIR` moves only the configuration, which is what a
/// dotfile manager wants.
pub const PATHS: Paths = Paths::new("starfold", "STARFOLD_DIR", "STARFOLD_CONFIG_DIR");

/// A helper executable installed beside STAR/FOLD (archive engine, preview
/// extensions). Test executables sit deeper inside Cargo's target directory --
/// `deps/`, or `build/<package>/<hash>/out/` with newer Cargo -- so tests look
/// for the helper in that directory's ancestors.
pub(crate) fn helper(name: &str) -> std::path::PathBuf {
    let executable = std::env::current_exe().unwrap_or_default();
    let directory = executable
        .parent()
        .unwrap_or(std::path::Path::new("/"))
        .to_path_buf();
    #[cfg(test)]
    if let Some(found) = directory
        .ancestors()
        .map(|ancestor| ancestor.join(name))
        .find(|path| path.is_file())
    {
        return found;
    }
    if directory.file_name().is_some_and(|n| n == "deps") {
        directory.parent().unwrap_or(&directory).join(name)
    } else {
        directory.join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// STAR/CORE's own tests cover the layout; what is worth asserting here is
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
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let old = std::env::var_os(key);
        // SAFETY: serialised by `ENV_LOCK` above, which every caller of this
        // helper takes before touching the environment.
        unsafe { std::env::set_var(key, value) };
        f();
        match old {
            Some(v) => unsafe { std::env::set_var(key, v) },
            None => unsafe { std::env::remove_var(key) },
        }
    }
}
