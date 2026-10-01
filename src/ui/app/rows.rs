//! Reuse presentation rows when only cursor, progress, or preview state changes.
use super::*;
use crate::fold::{listing::Listing, stack::Frame, State};

pub(super) struct Stamp {
    listing: Option<Arc<Listing>>,
    indices: Vec<usize>,
    selection: Selection,
    minute: u64,
}

#[derive(Default)]
pub(super) struct Snapshot {
    pub rows: Vec<panels::stack::Row>,
    #[cfg(feature = "visual")]
    pub paths: Vec<PathBuf>,
}

pub(super) fn refresh(
    stamp: &mut Option<Stamp>,
    old: Snapshot,
    state: &State,
    frame: &Frame,
    selection: &Selection,
    tz: &jiff::tz::TimeZone,
    now: std::time::SystemTime,
) -> Snapshot {
    let listing = state.listing_of(&frame.dir);
    let minute = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 60;
    let same_listing = |stamp: &Stamp| match (&stamp.listing, listing) {
        (Some(old), Some(current)) => Arc::ptr_eq(old, current),
        (None, None) => true,
        _ => false,
    };
    if stamp.as_ref().is_some_and(|stamp| {
        same_listing(stamp)
            && stamp.indices == frame.rows
            && stamp.selection == *selection
            && stamp.minute == minute
    }) {
        return old;
    }
    *stamp = Some(Stamp {
        listing: listing.cloned(),
        indices: frame.rows.clone(),
        selection: selection.clone(),
        minute,
    });
    let mut result = Snapshot::default();
    if let Some(listing) = listing {
        result.rows.reserve(frame.rows.len());
        #[cfg(feature = "visual")]
        result.paths.reserve(frame.rows.len());
        for &index in &frame.rows {
            let Some(entry) = listing.entries.get(index) else {
                continue;
            };
            result.rows.push(build_row(entry, selection, tz, now));
            #[cfg(feature = "visual")]
            result.paths.push(entry.path.clone());
        }
    }
    result
}
