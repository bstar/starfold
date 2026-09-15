//! Sizes, times and modes as a person reads them.
//!
//! Formatting lives in the core rather than the panels because `starfold
//! list` prints the same columns with no terminal attached, and two copies of
//! "what does 14200000 bytes look like" would drift.

use std::time::SystemTime;

/// A byte count as a person reads it: `14.2 MB`, decimal rather than binary,
/// which is what every desktop file manager already shows and what makes
/// `size(14_200_000) == "14.2 MB"` the obvious round trip.
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "KB", "MB", "GB", "TB", "PB", "EB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// A modification time, relative to `now` in `tz`: the clock time for today,
/// the month and day for this year, the date for anything older.
///
/// `// TODO(1a)`: the bootstrap stub prints the date only.
pub fn when(time: SystemTime, tz: &jiff::tz::TimeZone, now: SystemTime) -> String {
    let _ = now;
    let ts = jiff::Timestamp::try_from(time).unwrap_or_default();
    ts.to_zoned(tz.clone()).strftime("%Y-%m-%d").to_string()
}

/// `drwxr-x---`, from the raw mode bits.
///
/// `// TODO(1a)`: the bootstrap stub prints the octal.
pub fn mode(bits: u32) -> String {
    format!("{:o}", bits & 0o7777)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn small_sizes_are_shown_in_plain_bytes() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(999), "999 B");
        assert_eq!(size(1000), "1.0 KB");
        assert_eq!(size(14_200_000), "14.2 MB");
    }

    proptest! {
        /// Whatever the byte count, the string stays short enough for the
        /// status row: at most an EB-scale number, one decimal and a unit.
        #[test]
        fn size_never_grows_past_a_status_row(bytes: u64) {
            prop_assert!(size(bytes).chars().count() <= 10, "{}", size(bytes));
        }
    }
}
