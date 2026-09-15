//! The `/` filter: narrowing one level to the rows that match what was typed.
//!
//! A ranked fuzzy match over the visible rows, the same matcher STAR/CORD's
//! quick switcher uses. `rank` takes the indices `sort::order` produced and
//! hands back the subset that matches, best first, so a filter is a view over
//! the sorted listing and never a second copy of it.

use super::entry::Entry;

/// The rows of `visible` that match `query`, best match first. An empty
/// query is the identity: every row, in the order given.
///
/// `// TODO(1a)`: the bootstrap stub is a case-insensitive substring match in
/// the original order; the nucleo ranking is Phase 1a's.
pub fn rank(query: &str, entries: &[Entry], visible: &[usize]) -> Vec<usize> {
    if query.is_empty() {
        return visible.to_vec();
    }
    let needle = query.to_lowercase();
    visible
        .iter()
        .copied()
        .filter(|&i| entries[i].display.to_lowercase().contains(&needle))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_query_keeps_every_row_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let entries: Vec<Entry> = ["b", "a"]
            .iter()
            .map(|n| crate::fold::entry::stat(&dir.path().join(n)))
            .collect();
        assert_eq!(rank("", &entries, &[1, 0]), vec![1, 0]);
    }
}
