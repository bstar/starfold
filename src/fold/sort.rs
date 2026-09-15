//! Ordering a listing's rows.
//!
//! `order` never mutates a [`Entry`]; it hands back the indices that would
//! draw it in the chosen order, filtering out the hidden ones itself, so a
//! panel holds one `Vec<Entry>` from the listing and one `Vec<usize>` from
//! here rather than re-sorting or re-allocating a filtered copy on every
//! frame.

use serde::{Deserialize, Serialize};

use super::entry::{Entry, EntryKind};

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

fn is_dir_like(e: &Entry) -> bool {
    // A symlink to a directory sorts with the directories: what matters to
    // someone drilling down is where `l` takes them, not the row's own type.
    matches!(e.kind, EntryKind::Dir)
        || matches!(e.kind, EntryKind::Symlink if e.link_kind == Some(EntryKind::Dir))
}

fn key_of(e: &Entry) -> (String, &str) {
    let lower = e.display.to_lowercase();
    let ext = std::path::Path::new(&e.display)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    (lower, ext)
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
            let (da, db) = (is_dir_like(ea), is_dir_like(eb));
            if da != db {
                return db.cmp(&da);
            }
        }
        let ordering = match o.key {
            SortKey::Name => {
                let (na, nb) = (key_of(ea).0, key_of(eb).0);
                na.cmp(&nb)
            }
            SortKey::Size => ea.len.cmp(&eb.len),
            SortKey::Time => ea.modified.cmp(&eb.modified),
            SortKey::Ext => {
                let (ka, kb) = (key_of(ea), key_of(eb));
                ka.1.cmp(kb.1).then_with(|| ka.0.cmp(&kb.0))
            }
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
        Entry {
            path: name.into(),
            display: name.to_string(),
            kind,
            link_kind: None,
            len,
            modified: None,
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
    fn size_and_time_are_the_other_two_keys() {
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
            names in proptest::collection::vec("[a-zA-Z._]{1,8}", 0..20),
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
