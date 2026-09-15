//! Ordering a listing's rows.
//!
//! `order` never mutates a [`Entry`]; it hands back the indices that would
//! draw it in the chosen order, filtering out the hidden ones itself, so a
//! panel holds one `Vec<Entry>` from the listing and one `Vec<usize>` from
//! here rather than re-sorting or re-allocating a filtered copy on every
//! frame.

use serde::{Deserialize, Serialize};

use super::entry::Entry;

/// What a listing is ordered by.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Time,
    /// The extension, then the name -- what a directory of mixed source files
    /// groups by when nothing else is asked for.
    Ext,
}

impl SortKey {
    /// The next one round, for the key that cycles them.
    pub fn next(self) -> Self {
        match self {
            SortKey::Name => SortKey::Size,
            SortKey::Size => SortKey::Time,
            SortKey::Time => SortKey::Ext,
            SortKey::Ext => SortKey::Name,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Time => "time",
            SortKey::Ext => "ext",
        }
    }
}

/// A sort key, a direction, and whether directories lead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SortOrder {
    pub key: SortKey,
    pub reverse: bool,
    pub dirs_first: bool,
}

impl Default for SortOrder {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            reverse: false,
            dirs_first: true,
        }
    }
}

/// The lowercased name, for a comparison that does not care whether someone
/// wrote `README` or `readme.md`.
fn lower_name(e: &Entry) -> String {
    e.display.to_lowercase()
}

/// Compare two names the way a person reading a directory of `file2.txt`,
/// `file10.txt`, `file9.txt` expects: runs of digits compare by their
/// numeric value rather than character by character, so `file2` sorts
/// before `file10` instead of after it (`'1' < '2'` as characters, which is
/// what a plain `str::cmp` would do). Everything between the digit runs
/// still compares as plain text, case already folded by the caller.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    let mut a = a.chars().peekable();
    let mut b = b.chars().peekable();
    loop {
        return match (a.peek(), b.peek()) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(&ca), Some(&cb)) if ca.is_ascii_digit() && cb.is_ascii_digit() => {
                let na = take_digits(&mut a);
                let nb = take_digits(&mut b);
                match compare_digit_runs(&na, &nb) {
                    Ordering::Equal => continue,
                    other => other,
                }
            }
            (Some(&ca), Some(&cb)) => {
                if ca != cb {
                    ca.cmp(&cb)
                } else {
                    a.next();
                    b.next();
                    continue;
                }
            }
        };
    }
}

fn take_digits(iter: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut out = String::new();
    while let Some(&c) = iter.peek() {
        if c.is_ascii_digit() {
            out.push(c);
            iter.next();
        } else {
            break;
        }
    }
    out
}

/// Two runs of digits, compared as the numbers they spell rather than
/// parsed into an integer that a long enough run could overflow: strip
/// leading zeros, then the longer remaining run is the bigger number, and
/// equal lengths compare lexicographically (which is exactly numeric
/// comparison once the lengths match).
fn compare_digit_runs(a: &str, b: &str) -> std::cmp::Ordering {
    let ta = a.trim_start_matches('0');
    let tb = b.trim_start_matches('0');
    ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb))
}

/// The indices that draw `entries` in the order `o` describes, with the
/// hidden ones removed unless `hidden` says to keep them.
///
/// A stable sort, so two entries the key cannot tell apart -- two
/// directories modified the same second, most often -- keep the order
/// `read_dir` gave them rather than shuffling on every redraw.
pub fn order(entries: &[Entry], o: SortOrder, hidden: bool) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..entries.len())
        .filter(|&i| hidden || !entries[i].hidden)
        .collect();

    indices.sort_by(|&a, &b| {
        let (ea, eb) = (&entries[a], &entries[b]);
        if o.dirs_first {
            let (da, db) = (ea.is_dir_like(), eb.is_dir_like());
            if da != db {
                return db.cmp(&da);
            }
        }
        // `reverse` flips the ordering below, so each key's "forward"
        // direction has to be the one that already reads as ascending to a
        // person, not just whatever a raw field comparison happens to give:
        // `Size` ascending is smallest-first like `Name`'s A-before-Z, but
        // `Time` ascending would put the oldest file first, and nobody
        // browsing by time wants to scroll past years of history to find
        // what they touched five minutes ago. So `Time`'s forward direction
        // is newest-first, and `reverse` (like every other key) is what
        // gets you the other way round.
        let ordering = match o.key {
            SortKey::Name => natural_cmp(&lower_name(ea), &lower_name(eb)),
            SortKey::Size => ea.len.cmp(&eb.len),
            SortKey::Time => eb.modified.cmp(&ea.modified),
            SortKey::Ext => ea
                .ext()
                .cmp(&eb.ext())
                .then_with(|| natural_cmp(&lower_name(ea), &lower_name(eb))),
        };
        if o.reverse {
            ordering.reverse()
        } else {
            ordering
        }
    });

    indices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::entry::EntryKind;
    use proptest::prelude::*;

    fn entry(name: &str, kind: EntryKind, len: u64) -> Entry {
        entry_full(name, kind, len, None, None)
    }

    fn entry_full(
        name: &str,
        kind: EntryKind,
        len: u64,
        link_kind: Option<EntryKind>,
        modified: Option<std::time::SystemTime>,
    ) -> Entry {
        Entry {
            path: name.into(),
            display: name.to_string(),
            kind,
            link_kind,
            len,
            modified,
            mode: 0,
            executable: false,
            hidden: name.starts_with('.'),
        }
    }

    #[test]
    fn directories_lead_when_asked_to() {
        let entries = vec![
            entry("b.txt", EntryKind::File, 0),
            entry("a-dir", EntryKind::Dir, 0),
        ];
        let o = SortOrder {
            key: SortKey::Name,
            reverse: false,
            dirs_first: true,
        };
        let order = order(&entries, o, true);
        assert_eq!(order, vec![1, 0], "the directory sorts first");
    }

    #[test]
    fn a_symlink_to_a_directory_sorts_with_the_directories() {
        let entries = vec![
            entry("z-file.txt", EntryKind::File, 0),
            entry_full("a-link", EntryKind::Symlink, 0, Some(EntryKind::Dir), None),
        ];
        let o = SortOrder::default();
        assert_eq!(
            order(&entries, o, true),
            vec![1, 0],
            "the symlink-to-a-directory leads, same as a real directory"
        );
    }

    #[test]
    fn reverse_flips_the_order_within_each_group_but_directories_still_lead() {
        let entries = vec![
            entry("b-dir", EntryKind::Dir, 0),
            entry("a-dir", EntryKind::Dir, 0),
            entry("b-file.txt", EntryKind::File, 0),
            entry("a-file.txt", EntryKind::File, 0),
        ];
        let o = SortOrder {
            key: SortKey::Name,
            reverse: true,
            dirs_first: true,
        };
        assert_eq!(
            order(&entries, o, true),
            vec![0, 1, 2, 3],
            "b before a within the directories, then b before a within the files"
        );
    }

    #[test]
    fn names_are_compared_without_regard_to_case() {
        let entries = vec![
            entry("Banana", EntryKind::File, 0),
            entry("apple", EntryKind::File, 0),
        ];
        let o = SortOrder {
            dirs_first: false,
            ..SortOrder::default()
        };
        assert_eq!(order(&entries, o, true), vec![1, 0]);
    }

    #[test]
    fn digits_within_a_name_compare_numerically_not_lexicographically() {
        let entries = vec![
            entry("file10.txt", EntryKind::File, 0),
            entry("file2.txt", EntryKind::File, 0),
            entry("file1.txt", EntryKind::File, 0),
        ];
        let o = SortOrder {
            dirs_first: false,
            ..SortOrder::default()
        };
        let got = order(&entries, o, true);
        let names: Vec<&str> = got.iter().map(|&i| entries[i].display.as_str()).collect();
        assert_eq!(
            names,
            vec!["file1.txt", "file2.txt", "file10.txt"],
            "a plain string compare would put file10 before file2"
        );
    }

    #[test]
    fn hidden_entries_are_dropped_unless_asked_for() {
        let entries = vec![
            entry(".git", EntryKind::Dir, 0),
            entry("src", EntryKind::Dir, 0),
        ];
        let o = SortOrder::default();
        assert_eq!(order(&entries, o, false), vec![1]);
        assert_eq!(order(&entries, o, true).len(), 2);
    }

    #[test]
    fn size_ascending_is_smallest_first_and_reverse_flips_it() {
        let entries = vec![
            entry("big", EntryKind::File, 100),
            entry("small", EntryKind::File, 1),
        ];
        let o = SortOrder {
            key: SortKey::Size,
            reverse: false,
            dirs_first: false,
        };
        assert_eq!(order(&entries, o, true), vec![1, 0]);

        let o = SortOrder {
            key: SortKey::Size,
            reverse: true,
            dirs_first: false,
        };
        assert_eq!(order(&entries, o, true), vec![0, 1]);
    }

    #[test]
    fn time_sorts_newest_first_by_default_and_reverse_gives_oldest_first() {
        let now = crate::fold::testing::now();
        let day = std::time::Duration::from_secs(86_400);
        let entries = vec![
            entry_full("old.txt", EntryKind::File, 0, None, Some(now - day)),
            entry_full("new.txt", EntryKind::File, 0, None, Some(now)),
        ];
        let o = SortOrder {
            key: SortKey::Time,
            reverse: false,
            dirs_first: false,
        };
        assert_eq!(
            order(&entries, o, true),
            vec![1, 0],
            "the file touched most recently comes first"
        );

        let o = SortOrder {
            key: SortKey::Time,
            reverse: true,
            dirs_first: false,
        };
        assert_eq!(order(&entries, o, true), vec![0, 1]);
    }

    #[test]
    fn ext_key_groups_by_extension_then_falls_back_to_name() {
        let entries = vec![
            entry("b.rs", EntryKind::File, 0),
            entry("a.txt", EntryKind::File, 0),
            entry("a.rs", EntryKind::File, 0),
        ];
        let o = SortOrder {
            key: SortKey::Ext,
            reverse: false,
            dirs_first: false,
        };
        let got = order(&entries, o, true);
        let names: Vec<&str> = got.iter().map(|&i| entries[i].display.as_str()).collect();
        assert_eq!(names, vec!["a.rs", "b.rs", "a.txt"]);
    }

    #[test]
    fn the_key_cycles_round() {
        assert_eq!(SortKey::Name.next(), SortKey::Size);
        assert_eq!(SortKey::Ext.next(), SortKey::Name);
    }

    proptest! {
        /// Whatever order comes back, it is a permutation of the visible
        /// indices -- nothing invented, nothing dropped twice, nothing
        /// silently skipped that should have been kept.
        #[test]
        fn the_result_is_a_permutation_of_the_visible_indices(
            names in proptest::collection::vec("[a-zA-Z0-9._]{1,8}", 0..20),
            key in 0u8..4,
            reverse: bool,
            dirs_first: bool,
            hidden: bool,
        ) {
            let entries: Vec<Entry> = names
                .iter()
                .enumerate()
                .map(|(i, n)| entry(n, if i % 3 == 0 { EntryKind::Dir } else { EntryKind::File }, i as u64))
                .collect();
            let key = match key {
                0 => SortKey::Name,
                1 => SortKey::Size,
                2 => SortKey::Time,
                _ => SortKey::Ext,
            };
            let o = SortOrder { key, reverse, dirs_first };
            let got = order(&entries, o, hidden);

            let mut want: Vec<usize> = (0..entries.len())
                .filter(|&i| hidden || !entries[i].hidden)
                .collect();
            let mut sorted_got = got.clone();
            sorted_got.sort_unstable();
            want.sort_unstable();
            prop_assert_eq!(sorted_got, want);
        }
    }
}
