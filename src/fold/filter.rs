//! The `/` filter: narrowing one level to the rows that match what was typed.
//!
//! A ranked fuzzy match over the visible rows, the same matcher STAR/CORD's
//! quick switcher uses. `rank` takes the indices `sort::order` produced and
//! hands back the subset that matches, best first, so a filter is a view over
//! the sorted listing and never a second copy of it.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::entry::Entry;

/// The rows of `visible` that match `query`, best match first. An empty
/// query is the identity: every row, in the order given -- typing nothing
/// is not a query that happens to match everything, it is "no filter yet".
///
/// A fresh [`Matcher`] is built on every call rather than threaded through
/// `State`. Its own doc comment says construction pays for the matcher's
/// ~135 KB of scratch memory, which sounds like the kind of thing to keep
/// around, but that allocation is well under a millisecond and `rank` runs
/// once per keystroke over at most `max_entries` (50,000 by default) names,
/// not once per row per frame -- cheap enough that giving `Frame` a field
/// just to dodge it would be solving a problem that has not shown up yet.
pub fn rank(query: &str, entries: &[Entry], visible: &[usize]) -> Vec<usize> {
    if query.is_empty() {
        return visible.to_vec();
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);

    let mut buf = Vec::new();
    let mut scored: Vec<(usize, u32)> = visible
        .iter()
        .filter_map(|&i| {
            let haystack = Utf32Str::new(&entries[i].display, &mut buf);
            pattern
                .score(haystack, &mut matcher)
                .map(|score| (i, score))
        })
        .collect();

    // A stable sort: two rows the matcher scores identically keep the order
    // `visible` gave them, rather than shuffling on every keystroke as ties
    // come and go.
    scored.sort_by_key(|&(_, score)| std::cmp::Reverse(score));
    scored.into_iter().map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::entry::EntryKind;

    fn entry(name: &str) -> Entry {
        Entry {
            path: name.into(),
            display: name.to_string(),
            kind: EntryKind::File,
            link_kind: None,
            len: 0,
            modified: None,
            mode: 0,
            executable: false,
            hidden: name.starts_with('.'),
        }
    }

    #[test]
    fn an_empty_query_keeps_every_row_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let entries: Vec<Entry> = ["b", "a"]
            .iter()
            .map(|n| crate::fold::entry::stat(&dir.path().join(n)))
            .collect();
        assert_eq!(rank("", &entries, &[1, 0]), vec![1, 0]);
    }

    #[test]
    fn a_tighter_contiguous_match_ranks_above_a_looser_subsequence() {
        // Both names contain "cat" as a subsequence in order, but
        // `cat.rs` matches it as a prefix with no gaps at all, while
        // `concatenate.rs` only has it buried after `con`. The matcher's
        // gap penalty should put the tighter match first.
        let entries = vec![entry("cat.rs"), entry("concatenate.rs")];
        let visible = vec![0, 1];
        assert_eq!(
            rank("cat", &entries, &visible),
            vec![0, 1],
            "the prefix match should outscore the same letters buried mid-word"
        );
    }

    #[test]
    fn matching_is_case_insensitive() {
        let entries = vec![entry("Cargo.toml")];
        assert_eq!(rank("cargo", &entries, &[0]), vec![0]);
        assert_eq!(rank("CARGO", &entries, &[0]), vec![0]);
    }

    #[test]
    fn a_query_that_matches_nothing_returns_an_empty_list() {
        let entries = vec![entry("Cargo.toml"), entry("README.md")];
        assert!(rank("zzzz", &entries, &[0, 1]).is_empty());
    }

    #[test]
    fn rows_the_matcher_scores_identically_keep_the_order_visible_gave_them() {
        // Two rows with the same display name score identically no matter
        // what the matcher's bonus rules do, so this isolates the sort's
        // stability from the scoring itself.
        let entries = vec![entry("readme.txt"), entry("readme.txt")];
        assert_eq!(rank("read", &entries, &[0, 1]), vec![0, 1]);
        assert_eq!(rank("read", &entries, &[1, 0]), vec![1, 0]);
    }
}
