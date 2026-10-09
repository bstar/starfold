//! Archive extension integration: navigation and transactional publication.
pub mod browser;
pub mod connection;
pub mod edit;
pub mod operation;
pub mod service;
pub use service::list;
pub use starfold_archive_protocol::{Entry, Format};
use std::path::{Component, Path, PathBuf};
pub fn destination(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    path.with_file_name(
        suffix(&name)
            .map(|s| &name[..name.len() - s.len()])
            .filter(|s| !s.is_empty())
            .unwrap_or("extracted"),
    )
}
fn suffix(name: &str) -> Option<&str> {
    let lower = name.to_ascii_lowercase();
    let suffix = [
        ".tar.gz", ".tar.zst", ".tar.xz", ".tar.bz2", ".tgz", ".tbz2", ".tzst", ".txz", ".zip",
        ".tar", ".7z", ".rar",
    ]
    .into_iter()
    .find(|s| lower.ends_with(s));
    suffix.map(|s| &name[name.len() - s.len()..])
}
/// Never repair hostile names into different names: reject the archive.
fn safe_path(name: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !name.is_empty() && !name.contains(['\\', ':', '\0']),
        "Unsafe archive member name"
    );
    let p = Path::new(name);
    anyhow::ensure!(
        p.components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
            && p.components().any(|c| matches!(c, Component::Normal(_))),
        "Unsafe archive member path: {name}"
    );
    Ok(p.components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect())
}
const MAX_ENTRIES: usize = 100_000;
const MAX_OUTPUT: u64 = 64 * 1024 * 1024 * 1024;
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    #[test]
    fn rejects_traversal() {
        for n in ["../escape", "/tmp/x", "a/../../x", "C:\\x", "a\\..\\x", ""] {
            assert!(safe_path(n).is_err(), "{n}");
        }
    }
    proptest! { #[test] fn accepted_names_are_relative(s in ".{0,200}") { if let Ok(p)=safe_path(&s) { prop_assert!(!p.is_absolute()); prop_assert!(p.components().all(|c|matches!(c,Component::Normal(_)))); } } }
}
