//! Date helpers for PDF document metadata.
//!
//! krilla's `Metadata::creation_date` takes a `DateTime` built from
//! year/month/day/hour/minute/second/utc-offset components. This module
//! converts from the most common sources:
//!   - ISO date / datetime strings (`"2025-11-04"` or
//!     `"2025-11-04T08:30:00Z"`) commonly found in Markdoc frontmatter.
//!   - The current system time, for build-time stamping.

use krilla::metadata::DateTime;
use time::format_description::well_known::Iso8601;
use time::{Date, OffsetDateTime, PrimitiveDateTime};

/// Convert a `time::OffsetDateTime` into krilla's `DateTime` builder
/// chain.
fn from_offset_dt(dt: OffsetDateTime) -> DateTime {
    let mut out = DateTime::new(dt.year() as u16)
        .month(dt.month() as u8)
        .day(dt.day())
        .hour(dt.hour())
        .minute(dt.minute())
        .second(dt.second());
    let offset_h = dt.offset().whole_hours();
    let offset_m = (dt.offset().whole_minutes() % 60).unsigned_abs() as u8;
    out = out.utc_offset_hour(offset_h);
    if offset_m != 0 {
        out = out.utc_offset_minute(offset_m);
    }
    out
}

fn from_primitive_dt(dt: PrimitiveDateTime) -> DateTime {
    DateTime::new(dt.year() as u16)
        .month(dt.month() as u8)
        .day(dt.day())
        .hour(dt.hour())
        .minute(dt.minute())
        .second(dt.second())
        .utc_offset_hour(0)
}

fn from_date(d: Date) -> DateTime {
    DateTime::new(d.year() as u16)
        .month(d.month() as u8)
        .day(d.day())
}

/// Parse a permissive ISO date / datetime string into a krilla
/// `DateTime`. Accepts:
///   - `YYYY-MM-DD`
///   - `YYYY-MM-DDTHH:MM:SS`
///   - `YYYY-MM-DDTHH:MM:SSZ` / with offset
pub fn parse_iso(s: &str) -> Option<DateTime> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(dt) = OffsetDateTime::parse(trimmed, &Iso8601::DEFAULT) {
        return Some(from_offset_dt(dt));
    }
    if let Ok(dt) = PrimitiveDateTime::parse(trimmed, &Iso8601::DEFAULT) {
        return Some(from_primitive_dt(dt));
    }
    if let Ok(d) = Date::parse(trimmed, &Iso8601::DEFAULT) {
        return Some(from_date(d));
    }
    None
}

/// Current system time as a krilla `DateTime`. Falls back to UTC if
/// the local timezone offset can't be determined (e.g. in containerised
/// environments without `/etc/localtime`).
pub fn now() -> DateTime {
    match OffsetDateTime::now_local() {
        Ok(dt) => from_offset_dt(dt),
        Err(_) => from_offset_dt(OffsetDateTime::now_utc()),
    }
}

/// Today as `YYYY-MM-DD` (UTC fallback). Used by the renderer to
/// drive the `{date}` header/footer template variable when the caller
/// hasn't supplied an explicit date string.
pub fn today_yyyy_mm_dd() -> String {
    let dt = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    format!("{:04}-{:02}-{:02}", dt.year(), dt.month() as u8, dt.day())
}

/// Re-format a `YYYY-MM-DD…` ISO string into a clean `YYYY-MM-DD`
/// (drops any time/offset component). Returns the input unchanged
/// when parsing fails, so the templater always has *something* to
/// substitute.
pub fn iso_to_date_only(s: &str) -> String {
    let trimmed = s.trim();
    if let Ok(dt) = OffsetDateTime::parse(trimmed, &Iso8601::DEFAULT) {
        return format!("{:04}-{:02}-{:02}", dt.year(), dt.month() as u8, dt.day());
    }
    if let Ok(dt) = PrimitiveDateTime::parse(trimmed, &Iso8601::DEFAULT) {
        return format!("{:04}-{:02}-{:02}", dt.year(), dt.month() as u8, dt.day());
    }
    if let Ok(d) = Date::parse(trimmed, &Iso8601::DEFAULT) {
        return format!("{:04}-{:02}-{:02}", d.year(), d.month() as u8, d.day());
    }
    trimmed.to_string()
}

/// Parse the four-digit year out of an ISO date string (`2024-07-30…`).
/// Returns `None` when the leading token isn't a plausible year.
pub fn year_of(s: &str) -> Option<i32> {
    let digits: String = s
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let year: i32 = digits.parse().ok()?;
    (1000..=9999).contains(&year).then_some(year)
}

/// The current calendar year (local time, UTC fallback).
pub fn current_year() -> i32 {
    let dt = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
    dt.year()
}

/// Format a copyright year span. A single year when the first-release
/// year is absent or equals `current`, otherwise `first–current` joined
/// with an en dash. Examples: `"2026"`, `"2024–2026"`.
///
/// Pre-computing this in the caller keeps the renderer free of date
/// conditionals — the result is dropped into a template variable.
pub fn copyright_year_span(first: Option<i32>, current: i32) -> String {
    match first {
        Some(first) if first < current => format!("{first}\u{2013}{current}"),
        _ => current.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_date_only() {
        assert!(parse_iso("2025-11-04").is_some());
    }

    #[test]
    fn parses_datetime_with_offset() {
        assert!(parse_iso("2025-11-04T08:30:00+01:00").is_some());
    }

    #[test]
    fn parses_datetime_zulu() {
        assert!(parse_iso("2025-11-04T08:30:00Z").is_some());
    }

    #[test]
    fn returns_none_for_empty() {
        assert!(parse_iso("").is_none());
        assert!(parse_iso("   ").is_none());
    }

    #[test]
    fn year_of_extracts_leading_year() {
        assert_eq!(year_of("2024-07-30"), Some(2024));
        assert_eq!(year_of("  2026 "), Some(2026));
        assert_eq!(year_of("2025-01-01T08:30:00Z"), Some(2025));
        assert_eq!(year_of("not-a-date"), None);
        assert_eq!(year_of("99"), None); // too few digits to be a year
        assert_eq!(year_of(""), None);
    }

    #[test]
    fn copyright_span_collapses_equal_years() {
        // first == current → single year.
        assert_eq!(copyright_year_span(Some(2026), 2026), "2026");
        // missing first → single (current) year.
        assert_eq!(copyright_year_span(None, 2026), "2026");
        // first before current → en-dashed range.
        assert_eq!(copyright_year_span(Some(2024), 2026), "2024\u{2013}2026");
        // first after current (clock skew / bad data) → just current.
        assert_eq!(copyright_year_span(Some(2030), 2026), "2026");
    }

    #[test]
    fn returns_none_for_garbage() {
        assert!(parse_iso("not a date").is_none());
        assert!(parse_iso("2025-13-99").is_none());
    }

    #[test]
    fn now_succeeds() {
        let _ = now();
    }
}
