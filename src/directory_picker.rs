//! Read-only directory listing contract for embedded folder choosers.
use anyhow::{ensure, Context, Result};
use std::path::Path;

pub fn document(path: &Path, hidden: bool) -> Result<serde_json::Value> {
    let path = path.canonicalize().context("Open directory")?;
    ensure!(path.is_dir(), "Choose a directory");
    let cfg = crate::fold::listing::ListConfig {
        show_hidden: hidden,
        ..Default::default()
    };
    let listing = crate::fold::listing::read(&path, &cfg);
    if let Some(error) = listing.error {
        anyhow::bail!("{error}");
    }
    let order = crate::fold::sort::order(&listing.entries, cfg.sort, hidden);
    let mut directories = vec![];
    let mut truncated = listing.truncated;
    let mut bytes = 0;
    for i in order {
        let entry = &listing.entries[i];
        if !entry.is_dir_like() {
            continue;
        }
        // JSON paths must round-trip; never open a lossy display name.
        let Some(exact) = entry.path.to_str() else {
            continue;
        };
        bytes += exact.len();
        if directories.len() == 1000 || bytes > 512 * 1024 {
            truncated = true;
            break;
        }
        directories.push(serde_json::json!({"name":entry.display,"path":exact}));
    }
    ensure!(path.to_str().is_some(), "Directory path is not UTF-8");
    Ok(
        serde_json::json!({"version":1,"path":path,"parent":path.parent(),"directories":directories,"truncated":truncated}),
    )
}
fn places() -> (Vec<serde_json::Value>, Vec<String>) {
    use crate::fold::places::{self, LocationKind};
    let mut entries = vec![];
    let mut errors = vec![];
    let mut add = |name: &str, path: &Path, group: &str| {
        if path.is_absolute() && path.to_str().is_some() && entries.len() < 1000 {
            entries.push(serde_json::json!({"name":name,"path":path,"group":group}));
        }
    };
    if let Some(home) = std::env::var_os("HOME") {
        add("Home", Path::new(&home), "Standard");
    }
    add("Root", Path::new("/"), "Standard");
    match crate::paths::PATHS
        .config_file()
        .map_err(|e| e.to_string())
        .and_then(|p| places::load_bookmarks(&p.with_file_name("bookmarks.toml")))
    {
        Ok(bookmarks) => {
            for bookmark in bookmarks {
                add(&bookmark.name, &bookmark.path, "Bookmarks");
            }
        }
        Err(error) => errors.push(error),
    }
    match places::discover_paths() {
        Ok(locations) => {
            for location in locations {
                add(
                    &location.name,
                    &location.path,
                    match location.kind {
                        LocationKind::Device => "Devices",
                        LocationKind::Volume => "Volumes",
                        LocationKind::Network => "Network",
                    },
                );
            }
        }
        Err(error) => errors.push(error),
    }
    (entries, errors)
}
pub fn run() -> Result<()> {
    let args = std::env::args_os().skip(2).collect::<Vec<_>>();
    ensure!(
        !args.is_empty() && args.len() <= 3,
        "Usage: starfold --directory-list PATH [--hidden] [--places]"
    );
    ensure!(
        args[1..].iter().all(|a| a == "--hidden" || a == "--places"),
        "Unknown directory listing option"
    );
    let mut result = document(
        Path::new(&args[0]),
        args[1..].iter().any(|a| a == "--hidden"),
    )?;
    if args[1..].iter().any(|a| a == "--places") {
        let (entries, errors) = places();
        result["places"] = serde_json::json!(entries);
        result["places_errors"] = serde_json::json!(errors);
    }
    serde_json::to_writer(std::io::stdout().lock(), &result)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directories_only_exact_paths_hidden_and_symlinks() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("Movie folder ☄")).unwrap();
        std::fs::create_dir(root.path().join(".hidden")).unwrap();
        std::fs::write(root.path().join("film.mp4"), b"movie").unwrap();
        std::os::unix::fs::symlink(
            root.path().join("Movie folder ☄"),
            root.path().join("linked"),
        )
        .unwrap();
        let doc = document(root.path(), false).unwrap();
        assert_eq!(doc["directories"].as_array().unwrap().len(), 2);
        for row in doc["directories"].as_array().unwrap() {
            assert!(Path::new(row["path"].as_str().unwrap()).is_dir());
        }
        assert_eq!(
            document(root.path(), true).unwrap()["directories"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert!(document(&root.path().join("film.mp4"), false).is_err());
    }
}
