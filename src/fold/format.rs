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

/// Stood in for a modification time [`jiff::Timestamp`] cannot represent --
/// a `SystemTime` so far in the past or future it falls outside jiff's
/// supported range. Nine characters, well under the ten-column budget.
const UNREPRESENTABLE: &str = "unknown";

/// A modification time, relative to `now` in `tz`: the clock time
/// (`"14:02"`) for something modified today, the month and day (`"Sep 12"`)
/// for this year, and the full date (`"2024-03-01"`) for anything older --
/// the same three-tier scheme `ls -l` and every desktop file manager use,
/// because a bare `14:02` for a file a year old would read as "two minutes
/// ago" to anyone skimming the column.
///
/// A time in the future -- a clock skewed by a network share, a file
/// extracted from an archive with a bogus timestamp -- is shown as a date
/// rather than a clock time: "future at 14:02" invites the reader to trust a
/// number that is not this year's, and the date form carries no such claim.
/// `SystemTime` also has no lower or upper bound of its own, so a value
/// jiff's `Timestamp` cannot represent is reported as [`UNREPRESENTABLE`]
/// rather than panicking -- this is the one path in `starfold` a hostile or
/// corrupt filesystem can hand a raw, unchecked number to.
pub fn when(time: SystemTime, tz: &jiff::tz::TimeZone, now: SystemTime) -> String {
    let Ok(ts) = jiff::Timestamp::try_from(time) else {
        return UNREPRESENTABLE.to_string();
    };
    let zoned = ts.to_zoned(tz.clone());

    let Ok(now_ts) = jiff::Timestamp::try_from(now) else {
        // `now` itself is out of range (a badly set system clock, most
        // plausibly); fall back to the date form, which needs no comparison.
        return zoned.strftime("%Y-%m-%d").to_string();
    };
    if ts > now_ts {
        return zoned.strftime("%Y-%m-%d").to_string();
    }

    let now_zoned = now_ts.to_zoned(tz.clone());
    if zoned.date() == now_zoned.date() {
        zoned.strftime("%H:%M").to_string()
    } else if zoned.year() == now_zoned.year() {
        zoned.strftime("%b %-d").to_string()
    } else {
        zoned.strftime("%Y-%m-%d").to_string()
    }
}

/// `drwxr-x---`, from the raw Unix mode bits: a type character, then nine
/// permission characters, with setuid/setgid/sticky folded into the
/// executable position of their triad the way `ls -l` draws them (`s`/`S`
/// when the bit is set and the plain executable bit is on/off, `t`/`T` for
/// the sticky bit on the "other" triad).
pub fn mode(bits: u32) -> String {
    let mut out = String::with_capacity(10);
    out.push(type_char(bits));

    out.push(perm_char(bits & 0o400 != 0, 'r'));
    out.push(perm_char(bits & 0o200 != 0, 'w'));
    out.push(special_char(
        bits & 0o100 != 0,
        bits & 0o4000 != 0,
        's',
        'S',
    ));

    out.push(perm_char(bits & 0o040 != 0, 'r'));
    out.push(perm_char(bits & 0o020 != 0, 'w'));
    out.push(special_char(
        bits & 0o010 != 0,
        bits & 0o2000 != 0,
        's',
        'S',
    ));

    out.push(perm_char(bits & 0o004 != 0, 'r'));
    out.push(perm_char(bits & 0o002 != 0, 'w'));
    out.push(special_char(
        bits & 0o001 != 0,
        bits & 0o1000 != 0,
        't',
        'T',
    ));

    out
}

/// The leading type character, read from the `S_IFMT` bits (`0o170000`)
/// rather than from [`super::entry::EntryKind`]: `mode` is handed raw bits,
/// not an `Entry`, so a file manager can format a mode read from anywhere
/// (an archive member, a remote listing) without building one first.
///
/// Bits that name none of the seven known types -- most often all zero,
/// since nothing ever sets `S_IFMT` to a reserved combination in practice --
/// fall back to `'-'` rather than inventing an eighth character `ls -l`
/// never prints.
fn type_char(bits: u32) -> char {
    match bits & 0o170000 {
        0o040000 => 'd',
        0o120000 => 'l',
        0o020000 => 'c',
        0o060000 => 'b',
        0o010000 => 'p',
        0o140000 => 's',
        _ => '-',
    }
}

fn perm_char(set: bool, c: char) -> char {
    if set {
        c
    } else {
        '-'
    }
}

/// One `rwx` triad's executable slot, where a setuid/setgid/sticky bit can
/// replace `x` (or the empty `-`) with a letter that says the special bit is
/// set: lowercase when the plain executable bit is also set, uppercase when
/// it is the special bit alone.
fn special_char(exec: bool, special: bool, lower: char, upper: char) -> char {
    match (exec, special) {
        (true, true) => lower,
        (true, false) => 'x',
        (false, true) => upper,
        (false, false) => '-',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::time::Duration;

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

    #[test]
    fn a_time_earlier_today_is_shown_as_a_clock() {
        let now = crate::fold::testing::now();
        let earlier = now - Duration::from_secs(120);
        assert_eq!(when(earlier, &jiff::tz::TimeZone::UTC, now), "11:58");
    }

    #[test]
    fn a_time_two_days_ago_in_the_same_year_is_shown_as_month_and_day() {
        // `testing::now()` is 2026-09-11; two days back is still September.
        let now = crate::fold::testing::now();
        let two_days_ago = now - Duration::from_secs(2 * 86_400);
        assert_eq!(when(two_days_ago, &jiff::tz::TimeZone::UTC, now), "Sep 9");
    }

    #[test]
    fn a_time_from_a_previous_year_is_shown_as_a_full_date() {
        let now = crate::fold::testing::now();
        let over_a_year_ago = now - Duration::from_secs(400 * 86_400);
        let s = when(over_a_year_ago, &jiff::tz::TimeZone::UTC, now);
        assert_eq!(s.len(), 10, "{s}");
        assert!(s.starts_with("2025-"), "{s}");
    }

    #[test]
    fn a_time_in_the_future_is_shown_as_a_date_not_a_clock() {
        // `testing::now()` is 2026-09-11.
        let now = crate::fold::testing::now();
        let tomorrow = now + Duration::from_secs(86_400);
        assert_eq!(when(tomorrow, &jiff::tz::TimeZone::UTC, now), "2026-09-12");
    }

    #[test]
    fn a_time_jiff_cannot_represent_falls_back_rather_than_panicking() {
        let now = crate::fold::testing::now();
        // Nobody's disk reports a mtime this far out, but nothing stops a
        // corrupt filesystem from returning one; `SystemTime` has no bound
        // of its own to stop it either.
        let absurd = std::time::UNIX_EPOCH
            .checked_add(Duration::from_secs(u64::MAX / 2))
            .expect("half of u64::MAX seconds fits in a SystemTime");
        assert_eq!(when(absurd, &jiff::tz::TimeZone::UTC, now), UNREPRESENTABLE);
    }

    proptest! {
        /// Whatever `SystemTime` a filesystem hands back -- including ones
        /// jiff cannot represent -- `when` returns a short string rather
        /// than panicking.
        #[test]
        fn when_never_panics_and_stays_within_ten_columns(
            secs in proptest::num::u64::ANY,
            before_epoch: bool,
        ) {
            let now = crate::fold::testing::now();
            let time = if before_epoch {
                std::time::UNIX_EPOCH.checked_sub(Duration::from_secs(secs))
            } else {
                std::time::UNIX_EPOCH.checked_add(Duration::from_secs(secs))
            };
            if let Some(time) = time {
                let s = when(time, &jiff::tz::TimeZone::UTC, now);
                prop_assert!(s.chars().count() <= 10, "{}", s);
            }
        }
    }

    #[test]
    fn a_typical_directory_mode() {
        assert_eq!(mode(0o040755), "drwxr-xr-x");
    }

    #[test]
    fn a_typical_file_mode() {
        assert_eq!(mode(0o100644), "-rw-r--r--");
    }

    #[test]
    fn a_symlinks_mode_is_wide_open() {
        assert_eq!(mode(0o120777), "lrwxrwxrwx");
    }

    #[test]
    fn no_permission_bits_at_all_is_ten_dashes_after_the_type_char() {
        assert_eq!(mode(0), "----------");
    }

    #[test]
    fn setuid_shows_as_a_lowercase_s_when_the_owner_can_also_execute() {
        assert_eq!(mode(0o104755), "-rwsr-xr-x");
    }

    #[test]
    fn setgid_shows_as_an_uppercase_s_when_the_group_cannot_execute() {
        assert_eq!(mode(0o102644), "-rw-r-Sr--");
    }

    #[test]
    fn the_sticky_bit_on_a_world_writable_directory_is_the_classic_tmp_mode() {
        assert_eq!(mode(0o041777), "drwxrwxrwt");
    }

    proptest! {
        /// Every mode is exactly ten characters: a type char and nine
        /// permission chars, however the bits above them are set.
        #[test]
        fn mode_is_always_ten_characters(bits: u32) {
            prop_assert_eq!(mode(bits).chars().count(), 10);
        }
    }
}
