//! Pure datetime helper functions for use in templates.
//!
//! These functions work with primitive types (i64, &str, String) and can be
//! called from both the interpreter and template contexts.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};

/// Get current Unix timestamp (UTC).
pub fn datetime_now() -> i64 {
    Utc::now().timestamp()
}

/// Format a Unix timestamp using strftime format string.
///
/// # Arguments
/// * `timestamp` - Unix timestamp in seconds
/// * `format` - strftime format string (e.g., "%Y-%m-%d %H:%M:%S")
///
/// # Returns
/// Formatted date string, or empty string if timestamp is invalid.
pub fn datetime_format(timestamp: i64, format: &str) -> String {
    match DateTime::from_timestamp(timestamp, 0) {
        Some(dt) => dt.format(format).to_string(),
        None => String::new(),
    }
}

/// Parse a date string to Unix timestamp.
///
/// Supports multiple formats:
/// - RFC 3339 (e.g., "2024-01-15T10:30:00Z")
/// - RFC 2822
/// - ISO date with time (e.g., "2024-01-15T10:30:00" or "2024-01-15 10:30:00")
/// - ISO date only (e.g., "2024-01-15")
///
/// # Returns
/// Some(timestamp) if parsing succeeds, None otherwise.
pub fn datetime_parse(s: &str) -> Option<i64> {
    // Try RFC 3339 first (most common for APIs)
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp());
    }

    // Try RFC 2822
    if let Ok(dt) = DateTime::parse_from_rfc2822(s) {
        return Some(dt.timestamp());
    }

    // Try ISO datetime without timezone
    if let Ok(nd) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(nd.and_utc().timestamp());
    }

    // Try datetime with space separator
    if let Ok(nd) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(nd.and_utc().timestamp());
    }

    // Try date only
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        if let Some(dt) = d.and_hms_opt(0, 0, 0) {
            return Some(dt.and_utc().timestamp());
        }
    }

    None
}

/// Add days to a Unix timestamp.
///
/// # Arguments
/// * `timestamp` - Unix timestamp in seconds
/// * `days` - Number of days to add (can be negative)
///
/// # Returns
/// New timestamp with days added.
pub fn datetime_add_days(timestamp: i64, days: i64) -> i64 {
    timestamp + (days * 24 * 60 * 60)
}

/// Add hours to a Unix timestamp.
///
/// # Arguments
/// * `timestamp` - Unix timestamp in seconds
/// * `hours` - Number of hours to add (can be negative)
///
/// # Returns
/// New timestamp with hours added.
pub fn datetime_add_hours(timestamp: i64, hours: i64) -> i64 {
    timestamp + (hours * 60 * 60)
}

/// Get the difference between two timestamps in seconds.
///
/// # Arguments
/// * `t1` - First timestamp
/// * `t2` - Second timestamp
///
/// # Returns
/// t1 - t2 (difference in seconds)
pub fn datetime_diff(t1: i64, t2: i64) -> i64 {
    t1 - t2
}

/// Convert a timestamp to a human-readable "time ago" string.
///
/// # Arguments
/// * `timestamp` - Unix timestamp to compare against current time
///
/// # Returns
/// Human-readable string like "5 minutes ago", "2 hours ago", "3 days ago"
pub fn time_ago(timestamp: i64) -> String {
    let now = Utc::now().timestamp();
    let diff = now - timestamp;

    if diff < 0 {
        return "in the future".to_string();
    }

    if diff < 60 {
        let secs = diff;
        return if secs == 1 {
            "1 second ago".to_string()
        } else {
            format!("{} seconds ago", secs)
        };
    }

    if diff < 3600 {
        let mins = diff / 60;
        return if mins == 1 {
            "1 minute ago".to_string()
        } else {
            format!("{} minutes ago", mins)
        };
    }

    if diff < 86400 {
        let hours = diff / 3600;
        return if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{} hours ago", hours)
        };
    }

    if diff < 604800 {
        let days = diff / 86400;
        return if days == 1 {
            "1 day ago".to_string()
        } else {
            format!("{} days ago", days)
        };
    }

    if diff < 2592000 {
        let weeks = diff / 604800;
        return if weeks == 1 {
            "1 week ago".to_string()
        } else {
            format!("{} weeks ago", weeks)
        };
    }

    if diff < 31536000 {
        let months = diff / 2592000;
        return if months == 1 {
            "1 month ago".to_string()
        } else {
            format!("{} months ago", months)
        };
    }

    let years = diff / 31536000;
    if years == 1 {
        "1 year ago".to_string()
    } else {
        format!("{} years ago", years)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_datetime_now() {
        let now = datetime_now();
        assert!(now > 0);
    }

    #[test]
    fn test_datetime_format() {
        // 2024-01-15 00:00:00 UTC
        let ts = 1705276800;
        assert_eq!(datetime_format(ts, "%Y-%m-%d"), "2024-01-15");
        assert_eq!(datetime_format(ts, "%Y"), "2024");
    }

    #[test]
    fn test_datetime_parse() {
        // Test ISO date
        let ts = datetime_parse("2024-01-15");
        assert!(ts.is_some());

        // Test ISO datetime
        let ts = datetime_parse("2024-01-15T10:30:00");
        assert!(ts.is_some());

        // Test RFC 3339
        let ts = datetime_parse("2024-01-15T10:30:00Z");
        assert!(ts.is_some());

        // Test invalid
        let ts = datetime_parse("not a date");
        assert!(ts.is_none());
    }

    #[test]
    fn test_datetime_add_days() {
        let ts = 1705276800; // 2024-01-15 00:00:00 UTC
        let new_ts = datetime_add_days(ts, 1);
        assert_eq!(new_ts, ts + 86400);

        let new_ts = datetime_add_days(ts, -1);
        assert_eq!(new_ts, ts - 86400);
    }

    #[test]
    fn test_datetime_add_hours() {
        let ts = 1705276800;
        let new_ts = datetime_add_hours(ts, 2);
        assert_eq!(new_ts, ts + 7200);
    }

    #[test]
    fn test_datetime_diff() {
        assert_eq!(datetime_diff(100, 50), 50);
        assert_eq!(datetime_diff(50, 100), -50);
    }

    #[test]
    fn test_time_ago() {
        let now = datetime_now();

        // 30 seconds ago
        assert!(time_ago(now - 30).contains("seconds ago"));

        // 5 minutes ago
        assert!(time_ago(now - 300).contains("minutes ago"));

        // 2 hours ago
        assert!(time_ago(now - 7200).contains("hours ago"));

        // 3 days ago
        assert!(time_ago(now - 259200).contains("days ago"));
    }
}
