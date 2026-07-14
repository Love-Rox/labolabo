//! GRDB `Date` <-> SQLite TEXT compatibility, factored out of `database.rs`
//! because it is the single trickiest piece of this port (see
//! `rust/README.md`'s wave-4c section for the empirical notes this is
//! derived from).
//!
//! ## Write side
//!
//! GRDB's `Date: DatabaseValueConvertible` extension
//! (`GRDB/Core/Support/Foundation/Date.swift`) formats every `Date` it
//! writes with a fixed `DateFormatter`:
//!
//! ```text
//! dateFormat = "yyyy-MM-dd HH:mm:ss.SSS"
//! locale     = en_US_POSIX
//! timeZone   = UTC (secondsFromGMT: 0)
//! ```
//!
//! i.e. always a space separator (never `T`), always exactly 3 fractional
//! digits, always UTC, never a trailing zone suffix. `format_grdb_datetime`
//! reproduces this exactly via chrono's `%.3f` (which — per chrono's own
//! docs — is fixed-width, unlike plain `%f`).
//!
//! ## Read side
//!
//! GRDB's read path (`Date.fromDatabaseValue`) is more lenient than its
//! write path, by necessity: a column can hold rows written by older
//! `DateFormatter`/`DatabaseDateComponents` conventions, or (rarely) a
//! numeric SQLite storage class. Concretely, verified against
//! `GRDB/Core/Support/Foundation/{Date,DatabaseDateComponents,
//! SQLiteDateParser}.swift`:
//!
//! 1. If the column's SQLite storage class is TEXT, it's parsed as one of
//!    `YYYY-MM-DD`, `YYYY-MM-DD HH:MM`, `YYYY-MM-DD HH:MM:SS`, or
//!    `YYYY-MM-DD HH:MM:SS.SSS` — `T` accepted in place of the space, and an
//!    optional trailing `Z` / `+HH:MM` / `-HH:MM` zone offset — with missing
//!    time components defaulting to zero. **Time-only formats (`HH:MM...`,
//!    with no date) are recognized by the grammar but always rejected**:
//!    `Date(databaseDateComponents:)` guards on `format.hasYMDComponents`
//!    and returns `nil` otherwise. So this port only implements the
//!    YMD-having branch of `SQLiteDateParser` — the time-only branch would
//!    never produce a `Some` here either.
//! 2. Otherwise (INTEGER/REAL storage class — i.e. the column looks like a
//!    plain number to SQLite and TEXT decoding failed), the value is
//!    interpreted as `timeIntervalSince1970` **seconds** (not
//!    milliseconds).
//!
//! `parse_grdb_datetime` implements branch 1. `database.rs` implements
//! branch 2 directly against `rusqlite::types::ValueRef` (it needs the raw
//! SQLite storage class, which is exactly what `ValueRef`'s variants are).
//!
//! `parse_grdb_datetime`'s structure deliberately differs from
//! `SQLiteDateParser`'s: the Swift parser is a strict incremental
//! state machine (`guard parser.parseDigit(...) else { return nil }` at
//! each step, so encountering something unparseable mid-field fails the
//! *whole* parse immediately). This port instead greedily consumes what it
//! can at each optional field and defers all-or-nothing validation to a
//! single trailing-input check at the end. These are behaviorally
//! equivalent for every input: both ultimately require the entire string to
//! be consumed by `date [sep time [":" seconds ["." fraction]]] [zone]`, so
//! "stop early and let the final leftover-check reject it" and "fail
//! immediately inside the field" reach the same accept/reject verdict, and
//! for any input both accept, they consume the identical digits (see the
//! `fraction_extra_digits_are_discarded` / `garbage_after_partial_fraction`
//! tests below for the specific cases this equivalence was checked against).
//! Calendar validity itself uses `chrono::NaiveDate::from_ymd_opt`, which
//! rejects out-of-range month/day (e.g. month `13`) outright — Swift's
//! `Calendar(identifier: .gregorian).date(from:)` would instead *roll over*
//! such components into a valid date. This divergence is a deliberate,
//! documented simplification: every real `addedAt` value in production was
//! itself written by `format_grdb_datetime`'s Swift counterpart, so it is
//! always in-range; a hand-edited or corrupted out-of-range date is the
//! only input where the two disagree, and rejecting it here (rather than
//! silently reinterpreting it as a different date) is the safer failure
//! mode for a persistence layer.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Utc};

/// Formats `dt` exactly as GRDB's `Date.databaseValue` would:
/// `"yyyy-MM-dd HH:mm:ss.SSS"` in UTC, always 3 fractional digits.
pub(crate) fn format_grdb_datetime(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M:%S%.3f").to_string()
}

/// Parses a GRDB-written (or GRDB-compatible) `DATETIME` TEXT value. Returns
/// `None` for anything not matching the YMD-having grammar described in this
/// module's doc comment (including a bare time-only string, matching
/// Swift's `hasYMDComponents` guard).
pub(crate) fn parse_grdb_datetime(s: &str) -> Option<DateTime<Utc>> {
    // Every valid format is pure ASCII (digits, '-', ':', '.', ' ', 'T', 'Z',
    // '+'). Rejecting non-ASCII input up front means every fixed-byte-offset
    // slice below lands on a char boundary by construction — this string
    // can't panic on the `s[a..b]` indexing used throughout, no matter how
    // malformed the input is.
    if !s.is_ascii() || s.len() < 10 {
        return None;
    }
    let (date_part, mut rest) = s.split_at(10);
    let year: i32 = parse_fixed_digits(&date_part[0..4])?;
    if date_part.as_bytes()[4] != b'-' {
        return None;
    }
    let month: u32 = parse_fixed_digits(&date_part[5..7])?;
    if date_part.as_bytes()[7] != b'-' {
        return None;
    }
    let day: u32 = parse_fixed_digits(&date_part[8..10])?;
    let date = NaiveDate::from_ymd_opt(year, month, day)?;

    let mut hour = 0u32;
    let mut minute = 0u32;
    let mut second = 0u32;
    let mut milli = 0u32;
    let mut offset_seconds: i32 = 0;

    if !rest.is_empty() {
        let sep = rest.as_bytes()[0];
        if sep != b' ' && sep != b'T' {
            return None;
        }
        rest = &rest[1..];

        // HH:MM is mandatory once a separator is present.
        if rest.len() < 5 || rest.as_bytes()[2] != b':' {
            return None;
        }
        hour = parse_fixed_digits(&rest[0..2])?;
        minute = parse_fixed_digits(&rest[3..5])?;
        rest = &rest[5..];

        if let Some((offset, remainder)) = try_parse_zone(rest) {
            offset_seconds = offset;
            rest = remainder;
        } else if let Some(after_colon) = rest.strip_prefix(':') {
            if after_colon.len() < 2 {
                return None;
            }
            second = parse_fixed_digits(&after_colon[0..2])?;
            rest = &after_colon[2..];

            if let Some((offset, remainder)) = try_parse_zone(rest) {
                offset_seconds = offset;
                rest = remainder;
            } else if let Some(after_dot) = rest.strip_prefix('.') {
                let mut digits = 0usize;
                let mut value = 0u32;
                let mut cursor = after_dot;
                while digits < 3 {
                    match cursor.as_bytes().first() {
                        Some(b) if b.is_ascii_digit() => {
                            value = value * 10 + u32::from(b - b'0');
                            cursor = &cursor[1..];
                            digits += 1;
                        }
                        _ => break,
                    }
                }
                if digits == 0 {
                    return None;
                }
                milli = match digits {
                    1 => value * 100,
                    2 => value * 10,
                    _ => value,
                };
                // Extra fractional digits beyond 3 are consumed and
                // discarded (matches `SQLiteDateParser`'s trailing
                // `while parser.parseDigit() != nil {}`).
                while cursor.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                    cursor = &cursor[1..];
                }
                rest = cursor;
                if let Some((offset, remainder)) = try_parse_zone(rest) {
                    offset_seconds = offset;
                    rest = remainder;
                }
            }
        }
    }

    if !rest.is_empty() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    let time = NaiveTime::from_hms_milli_opt(hour, minute, second, milli)?;
    let naive = NaiveDateTime::new(date, time);
    let utc_naive = naive.checked_sub_signed(TimeDelta::seconds(i64::from(offset_seconds)))?;
    Some(DateTime::<Utc>::from_naive_utc_and_offset(utc_naive, Utc))
}

/// `Z`, `+HH:MM`, or `-HH:MM`. Returns the offset in seconds east of UTC
/// (so it can be *subtracted* from the naive local time to get UTC) plus
/// whatever remains of the input.
fn try_parse_zone(s: &str) -> Option<(i32, &str)> {
    if let Some(rest) = s.strip_prefix('Z') {
        return Some((0, rest));
    }
    let sign = match s.as_bytes().first() {
        Some(b'+') => 1,
        Some(b'-') => -1,
        _ => return None,
    };
    let body = &s[1..];
    if body.len() < 5 || body.as_bytes()[2] != b':' {
        return None;
    }
    let hour: i32 = parse_fixed_digits(&body[0..2])?;
    let minute: i32 = parse_fixed_digits(&body[3..5])?;
    Some((sign * (hour * 3600 + minute * 60), &body[5..]))
}

fn parse_fixed_digits<T: std::str::FromStr>(s: &str) -> Option<T> {
    // Callers only ever slice an already-`is_ascii()`-checked string at
    // fixed byte offsets, so `s` here is always plain ASCII digits — this
    // just rejects a non-digit character (e.g. the `-`/`:` separators
    // landing in the wrong place for a malformed input).
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, ms: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap() + TimeDelta::milliseconds(i64::from(ms))
    }

    #[test]
    fn formats_with_fixed_three_fractional_digits() {
        assert_eq!(
            format_grdb_datetime(&utc(2026, 7, 13, 9, 5, 3, 7)),
            "2026-07-13 09:05:03.007"
        );
        assert_eq!(
            format_grdb_datetime(&utc(2026, 1, 1, 0, 0, 0, 0)),
            "2026-01-01 00:00:00.000"
        );
    }

    #[test]
    fn round_trips_format_output() {
        let dt = utc(2026, 12, 31, 23, 59, 59, 999);
        assert_eq!(parse_grdb_datetime(&format_grdb_datetime(&dt)), Some(dt));
    }

    #[test]
    fn date_only_defaults_time_to_midnight() {
        assert_eq!(
            parse_grdb_datetime("2026-07-13"),
            Some(utc(2026, 7, 13, 0, 0, 0, 0))
        );
    }

    #[test]
    fn accepts_t_separator() {
        assert_eq!(
            parse_grdb_datetime("2026-07-13T09:05:03.007"),
            Some(utc(2026, 7, 13, 9, 5, 3, 7))
        );
    }

    #[test]
    fn hm_and_hms_granularity() {
        assert_eq!(
            parse_grdb_datetime("2026-07-13 09:05"),
            Some(utc(2026, 7, 13, 9, 5, 0, 0))
        );
        assert_eq!(
            parse_grdb_datetime("2026-07-13 09:05:03"),
            Some(utc(2026, 7, 13, 9, 5, 3, 0))
        );
    }

    #[test]
    fn fraction_1_and_2_digits_scale_to_milliseconds() {
        assert_eq!(
            parse_grdb_datetime("2026-07-13 09:05:03.5"),
            Some(utc(2026, 7, 13, 9, 5, 3, 500))
        );
        assert_eq!(
            parse_grdb_datetime("2026-07-13 09:05:03.56"),
            Some(utc(2026, 7, 13, 9, 5, 3, 560))
        );
    }

    #[test]
    fn fraction_extra_digits_are_discarded() {
        // 6-digit microsecond precision: only the first 3 digits count.
        assert_eq!(
            parse_grdb_datetime("2026-07-13 09:05:03.123456"),
            Some(utc(2026, 7, 13, 9, 5, 3, 123))
        );
    }

    #[test]
    fn zone_offset_normalizes_to_utc() {
        // 09:05:03+02:00 is 07:05:03 UTC.
        assert_eq!(
            parse_grdb_datetime("2026-07-13 09:05:03+02:00"),
            Some(utc(2026, 7, 13, 7, 5, 3, 0))
        );
        // 09:05:03-02:00 is 11:05:03 UTC.
        assert_eq!(
            parse_grdb_datetime("2026-07-13 09:05:03-02:00"),
            Some(utc(2026, 7, 13, 11, 5, 3, 0))
        );
        assert_eq!(
            parse_grdb_datetime("2026-07-13 09:05:03Z"),
            Some(utc(2026, 7, 13, 9, 5, 3, 0))
        );
    }

    #[test]
    fn zone_offset_after_fraction() {
        assert_eq!(
            parse_grdb_datetime("2026-07-13 09:05:03.007+02:00"),
            Some(utc(2026, 7, 13, 7, 5, 3, 7))
        );
    }

    #[test]
    fn time_only_is_rejected_matching_has_ymd_components_guard() {
        assert_eq!(parse_grdb_datetime("09:05:03"), None);
        assert_eq!(parse_grdb_datetime("09:05"), None);
    }

    #[test]
    fn garbage_after_partial_fraction_is_rejected() {
        // A non-digit, non-zone character right after 1 or 2 fractional
        // digits must reject the whole parse, not silently truncate.
        assert_eq!(parse_grdb_datetime("2026-07-13 09:05:03.5X"), None);
        assert_eq!(parse_grdb_datetime("2026-07-13 09:05:03.55X"), None);
    }

    #[test]
    fn invalid_calendar_date_is_rejected() {
        assert_eq!(parse_grdb_datetime("2026-13-01"), None);
        assert_eq!(parse_grdb_datetime("2026-02-30"), None);
    }

    #[test]
    fn non_ascii_input_does_not_panic() {
        assert_eq!(parse_grdb_datetime("日本語のごみデータ"), None);
        assert_eq!(parse_grdb_datetime("2026-07-13 09:05:03.007€"), None);
    }

    #[test]
    fn malformed_inputs_are_rejected() {
        assert_eq!(parse_grdb_datetime(""), None);
        assert_eq!(parse_grdb_datetime("not-a-date"), None);
        assert_eq!(parse_grdb_datetime("2026/07/13"), None);
        assert_eq!(parse_grdb_datetime("2026-07-13 "), None);
        assert_eq!(parse_grdb_datetime("2026-07-13 09"), None);
        assert_eq!(parse_grdb_datetime("2026-07-13 09:05:"), None);
        assert_eq!(parse_grdb_datetime("2026-07-13 09:05:03."), None);
    }
}
