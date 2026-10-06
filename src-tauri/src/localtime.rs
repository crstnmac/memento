//! Calendar math in the user's local timezone, with the UTC offset injected so
//! date logic (summaries, "yesterday" in questions, snooze presets) can be
//! tested deterministically — including across DST changes.
//!
//! A "day number" is the count of civil days since 1970-01-01 in local time.

use chrono::{Local, TimeZone};

pub const DAY_MS: i64 = 86_400_000;
pub const HOUR_MS: i64 = 3_600_000;

/// UTC offset (ms) at a UTC instant. Takes the instant so DST is respected.
pub type Offset<'a> = &'a dyn Fn(i64) -> i64;

/// The machine's real offset at a UTC instant.
pub fn system_offset(utc_ms: i64) -> i64 {
    Local
        .timestamp_millis_opt(utc_ms)
        .single()
        .map(|dt| dt.offset().local_minus_utc() as i64 * 1000)
        .unwrap_or(0)
}

pub fn day_number(utc_ms: i64, off: Offset) -> i64 {
    (utc_ms + off(utc_ms)).div_euclid(DAY_MS)
}

/// UTC instant of local midnight at the start of `day`. Handles DST by
/// re-resolving the offset at the guessed instant.
pub fn start_of_day(day: i64, off: Offset) -> i64 {
    at_hour(day, 0, off)
}

/// UTC instant of `hour`:00 local on `day`.
pub fn at_hour(day: i64, hour: i64, off: Offset) -> i64 {
    let local = day * DAY_MS + hour * HOUR_MS;
    let guess = local - off(local);
    local - off(guess)
}

/// `[start, end)` of a local civil day as UTC instants; 23h or 25h on DST days.
pub fn day_bounds(day: i64, off: Offset) -> (i64, i64) {
    (start_of_day(day, off), start_of_day(day + 1, off))
}

/// Monday = 0 .. Sunday = 6.
pub fn weekday_mon0(day: i64) -> i64 {
    // 1970-01-01 was a Thursday (3 when Monday = 0).
    (day + 3).rem_euclid(7)
}

pub fn week_start_day(day: i64) -> i64 {
    day - weekday_mon0(day)
}

pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn is_valid_civil(y: i64, m: u32, d: u32) -> bool {
    (1..=12).contains(&m) && d >= 1 && {
        let (y2, m2, d2) = civil_from_days(days_from_civil(y, m, d));
        (y2, m2, d2) == (y, m, d)
    }
}

pub fn format_ymd(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Parse a strict `YYYY-MM-DD` into a day number.
pub fn parse_ymd(s: &str) -> Option<i64> {
    let mut parts = s.trim().split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1900..=2200).contains(&y) || !is_valid_civil(y, m, d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// Local hour of day (0-23) for a UTC instant.
pub fn local_hour(utc_ms: i64, off: Offset) -> u32 {
    ((utc_ms + off(utc_ms)).rem_euclid(DAY_MS) / HOUR_MS) as u32
}

pub const MONTH_NAMES: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September",
    "October", "November", "December",
];
pub const WEEKDAY_NAMES: [&str; 7] = [
    "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday",
];

/// "Tue 6 Oct" style label for a day number.
pub fn short_label(day: i64) -> String {
    let (_, m, d) = civil_from_days(day);
    format!(
        "{} {} {}",
        &WEEKDAY_NAMES[weekday_mon0(day) as usize][..3],
        d,
        &MONTH_NAMES[m as usize - 1][..3]
    )
}

#[cfg(test)]
pub mod testutil {
    /// A UTC-5 zone that springs forward to UTC-4 at `change_utc_ms`.
    pub fn dst_zone(change_utc_ms: i64) -> impl Fn(i64) -> i64 {
        move |utc| if utc >= change_utc_ms { -4 * 3_600_000 } else { -5 * 3_600_000 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(_: i64) -> i64 {
        0
    }

    #[test]
    fn civil_roundtrip() {
        let d = days_from_civil(2026, 10, 5);
        assert_eq!(civil_from_days(d), (2026, 10, 5));
        assert_eq!(format_ymd(d), "2026-10-05");
        assert_eq!(parse_ymd("2026-02-29"), None);
        assert_eq!(parse_ymd("2024-02-29"), Some(days_from_civil(2024, 2, 29)));
    }

    #[test]
    fn weekday_of_known_dates() {
        // 2026-10-05 is a Monday.
        assert_eq!(weekday_mon0(days_from_civil(2026, 10, 5)), 0);
        assert_eq!(weekday_mon0(days_from_civil(2026, 10, 11)), 6);
        assert_eq!(week_start_day(days_from_civil(2026, 10, 11)), days_from_civil(2026, 10, 5));
    }

    #[test]
    fn dst_day_is_23_hours() {
        // US 2026: clocks spring forward 2026-03-08 at 07:00 UTC.
        let change = days_from_civil(2026, 3, 8) * DAY_MS + 7 * HOUR_MS;
        let zone = dst_zone_for_test(change);
        let (s, e) = day_bounds(days_from_civil(2026, 3, 8), &zone);
        assert_eq!(e - s, 23 * HOUR_MS);
        let (s2, e2) = day_bounds(days_from_civil(2026, 3, 9), &zone);
        assert_eq!(e2 - s2, 24 * HOUR_MS);
        assert_eq!(s2, e);
    }

    fn dst_zone_for_test(change: i64) -> impl Fn(i64) -> i64 {
        testutil::dst_zone(change)
    }

    #[test]
    fn utc_zone_bounds() {
        let day = days_from_civil(2026, 1, 1);
        assert_eq!(day_bounds(day, &utc), (day * DAY_MS, (day + 1) * DAY_MS));
        assert_eq!(local_hour(day * DAY_MS + 5 * HOUR_MS, &utc), 5);
    }
}
