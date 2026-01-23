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
    time_ago_localized(timestamp, "en")
}

/// Convert a timestamp to a localized human-readable "time ago" string.
///
/// # Arguments
/// * `timestamp` - Unix timestamp to compare against current time
/// * `locale` - Locale code (e.g., "en", "fr", "de", "es", "it", "pt", "ja", "zh")
///
/// # Returns
/// Localized human-readable string like "il y a 5 minutes", "vor 2 Stunden"
pub fn time_ago_localized(timestamp: i64, locale: &str) -> String {
    let now = Utc::now().timestamp();
    let diff = now - timestamp;

    if diff < 0 {
        return get_time_ago_text(locale, "future", 0);
    }

    if diff < 60 {
        let secs = diff;
        return if secs == 1 {
            get_time_ago_text(locale, "second", 1)
        } else {
            get_time_ago_text(locale, "seconds", secs)
        };
    }

    if diff < 3600 {
        let mins = diff / 60;
        return if mins == 1 {
            get_time_ago_text(locale, "minute", 1)
        } else {
            get_time_ago_text(locale, "minutes", mins)
        };
    }

    if diff < 86400 {
        let hours = diff / 3600;
        return if hours == 1 {
            get_time_ago_text(locale, "hour", 1)
        } else {
            get_time_ago_text(locale, "hours", hours)
        };
    }

    if diff < 604800 {
        let days = diff / 86400;
        return if days == 1 {
            get_time_ago_text(locale, "day", 1)
        } else {
            get_time_ago_text(locale, "days", days)
        };
    }

    if diff < 2592000 {
        let weeks = diff / 604800;
        return if weeks == 1 {
            get_time_ago_text(locale, "week", 1)
        } else {
            get_time_ago_text(locale, "weeks", weeks)
        };
    }

    if diff < 31536000 {
        let months = diff / 2592000;
        return if months == 1 {
            get_time_ago_text(locale, "month", 1)
        } else {
            get_time_ago_text(locale, "months", months)
        };
    }

    let years = diff / 31536000;
    if years == 1 {
        get_time_ago_text(locale, "year", 1)
    } else {
        get_time_ago_text(locale, "years", years)
    }
}

/// Get localized "time ago" text for a given unit and count.
fn get_time_ago_text(locale: &str, unit: &str, count: i64) -> String {
    match locale {
        "fr" => match unit {
            "future" => "dans le futur".to_string(),
            "second" => "il y a 1 seconde".to_string(),
            "seconds" => format!("il y a {} secondes", count),
            "minute" => "il y a 1 minute".to_string(),
            "minutes" => format!("il y a {} minutes", count),
            "hour" => "il y a 1 heure".to_string(),
            "hours" => format!("il y a {} heures", count),
            "day" => "il y a 1 jour".to_string(),
            "days" => format!("il y a {} jours", count),
            "week" => "il y a 1 semaine".to_string(),
            "weeks" => format!("il y a {} semaines", count),
            "month" => "il y a 1 mois".to_string(),
            "months" => format!("il y a {} mois", count),
            "year" => "il y a 1 an".to_string(),
            "years" => format!("il y a {} ans", count),
            _ => format!("{} {}", count, unit),
        },
        "de" => match unit {
            "future" => "in der Zukunft".to_string(),
            "second" => "vor 1 Sekunde".to_string(),
            "seconds" => format!("vor {} Sekunden", count),
            "minute" => "vor 1 Minute".to_string(),
            "minutes" => format!("vor {} Minuten", count),
            "hour" => "vor 1 Stunde".to_string(),
            "hours" => format!("vor {} Stunden", count),
            "day" => "vor 1 Tag".to_string(),
            "days" => format!("vor {} Tagen", count),
            "week" => "vor 1 Woche".to_string(),
            "weeks" => format!("vor {} Wochen", count),
            "month" => "vor 1 Monat".to_string(),
            "months" => format!("vor {} Monaten", count),
            "year" => "vor 1 Jahr".to_string(),
            "years" => format!("vor {} Jahren", count),
            _ => format!("{} {}", count, unit),
        },
        "es" => match unit {
            "future" => "en el futuro".to_string(),
            "second" => "hace 1 segundo".to_string(),
            "seconds" => format!("hace {} segundos", count),
            "minute" => "hace 1 minuto".to_string(),
            "minutes" => format!("hace {} minutos", count),
            "hour" => "hace 1 hora".to_string(),
            "hours" => format!("hace {} horas", count),
            "day" => "hace 1 día".to_string(),
            "days" => format!("hace {} días", count),
            "week" => "hace 1 semana".to_string(),
            "weeks" => format!("hace {} semanas", count),
            "month" => "hace 1 mes".to_string(),
            "months" => format!("hace {} meses", count),
            "year" => "hace 1 año".to_string(),
            "years" => format!("hace {} años", count),
            _ => format!("{} {}", count, unit),
        },
        "it" => match unit {
            "future" => "nel futuro".to_string(),
            "second" => "1 secondo fa".to_string(),
            "seconds" => format!("{} secondi fa", count),
            "minute" => "1 minuto fa".to_string(),
            "minutes" => format!("{} minuti fa", count),
            "hour" => "1 ora fa".to_string(),
            "hours" => format!("{} ore fa", count),
            "day" => "1 giorno fa".to_string(),
            "days" => format!("{} giorni fa", count),
            "week" => "1 settimana fa".to_string(),
            "weeks" => format!("{} settimane fa", count),
            "month" => "1 mese fa".to_string(),
            "months" => format!("{} mesi fa", count),
            "year" => "1 anno fa".to_string(),
            "years" => format!("{} anni fa", count),
            _ => format!("{} {}", count, unit),
        },
        "pt" => match unit {
            "future" => "no futuro".to_string(),
            "second" => "há 1 segundo".to_string(),
            "seconds" => format!("há {} segundos", count),
            "minute" => "há 1 minuto".to_string(),
            "minutes" => format!("há {} minutos", count),
            "hour" => "há 1 hora".to_string(),
            "hours" => format!("há {} horas", count),
            "day" => "há 1 dia".to_string(),
            "days" => format!("há {} dias", count),
            "week" => "há 1 semana".to_string(),
            "weeks" => format!("há {} semanas", count),
            "month" => "há 1 mês".to_string(),
            "months" => format!("há {} meses", count),
            "year" => "há 1 ano".to_string(),
            "years" => format!("há {} anos", count),
            _ => format!("{} {}", count, unit),
        },
        "ja" => match unit {
            "future" => "未来".to_string(),
            "second" => "1秒前".to_string(),
            "seconds" => format!("{}秒前", count),
            "minute" => "1分前".to_string(),
            "minutes" => format!("{}分前", count),
            "hour" => "1時間前".to_string(),
            "hours" => format!("{}時間前", count),
            "day" => "1日前".to_string(),
            "days" => format!("{}日前", count),
            "week" => "1週間前".to_string(),
            "weeks" => format!("{}週間前", count),
            "month" => "1ヶ月前".to_string(),
            "months" => format!("{}ヶ月前", count),
            "year" => "1年前".to_string(),
            "years" => format!("{}年前", count),
            _ => format!("{} {}", count, unit),
        },
        "zh" => match unit {
            "future" => "未来".to_string(),
            "second" => "1秒前".to_string(),
            "seconds" => format!("{}秒前", count),
            "minute" => "1分钟前".to_string(),
            "minutes" => format!("{}分钟前", count),
            "hour" => "1小时前".to_string(),
            "hours" => format!("{}小时前", count),
            "day" => "1天前".to_string(),
            "days" => format!("{}天前", count),
            "week" => "1周前".to_string(),
            "weeks" => format!("{}周前", count),
            "month" => "1个月前".to_string(),
            "months" => format!("{}个月前", count),
            "year" => "1年前".to_string(),
            "years" => format!("{}年前", count),
            _ => format!("{} {}", count, unit),
        },
        // English (default)
        _ => match unit {
            "future" => "in the future".to_string(),
            "second" => "1 second ago".to_string(),
            "seconds" => format!("{} seconds ago", count),
            "minute" => "1 minute ago".to_string(),
            "minutes" => format!("{} minutes ago", count),
            "hour" => "1 hour ago".to_string(),
            "hours" => format!("{} hours ago", count),
            "day" => "1 day ago".to_string(),
            "days" => format!("{} days ago", count),
            "week" => "1 week ago".to_string(),
            "weeks" => format!("{} weeks ago", count),
            "month" => "1 month ago".to_string(),
            "months" => format!("{} months ago", count),
            "year" => "1 year ago".to_string(),
            "years" => format!("{} years ago", count),
            _ => format!("{} {}", count, unit),
        },
    }
}

/// Localize a date/time according to locale and format.
///
/// # Arguments
/// * `timestamp` - Unix timestamp in seconds
/// * `locale` - Locale code (e.g., "en", "fr", "de", "es")
/// * `format` - Format name: "short", "long", "full", "time", "datetime", or a strftime string
///
/// # Returns
/// Localized date string
pub fn localize_date(timestamp: i64, locale: &str, format: &str) -> String {
    let dt = match DateTime::from_timestamp(timestamp, 0) {
        Some(dt) => dt,
        None => return String::new(),
    };

    // Get month and day names for the locale
    let (months, days, date_format, time_format, datetime_format_str) = get_locale_data(locale);

    // Handle named formats
    let strftime_format = match format {
        "short" => date_format,
        "long" => "%B %d, %Y",
        "full" => "%A, %B %d, %Y",
        "time" => time_format,
        "datetime" => datetime_format_str,
        other => other, // Use as strftime format directly
    };

    // Format the date
    let formatted = dt.format(strftime_format).to_string();

    // Replace English month/day names with localized versions
    localize_names(&formatted, months, days, locale)
}

/// Get locale-specific data (month names, day names, formats)
fn get_locale_data(locale: &str) -> (&'static [&'static str], &'static [&'static str], &'static str, &'static str, &'static str) {
    match locale {
        "fr" => (
            &["janvier", "février", "mars", "avril", "mai", "juin",
              "juillet", "août", "septembre", "octobre", "novembre", "décembre"],
            &["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"],
            "%d/%m/%Y",
            "%H:%M",
            "%d/%m/%Y %H:%M",
        ),
        "de" => (
            &["Januar", "Februar", "März", "April", "Mai", "Juni",
              "Juli", "August", "September", "Oktober", "November", "Dezember"],
            &["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"],
            "%d.%m.%Y",
            "%H:%M",
            "%d.%m.%Y %H:%M",
        ),
        "es" => (
            &["enero", "febrero", "marzo", "abril", "mayo", "junio",
              "julio", "agosto", "septiembre", "octubre", "noviembre", "diciembre"],
            &["lunes", "martes", "miércoles", "jueves", "viernes", "sábado", "domingo"],
            "%d/%m/%Y",
            "%H:%M",
            "%d/%m/%Y %H:%M",
        ),
        "it" => (
            &["gennaio", "febbraio", "marzo", "aprile", "maggio", "giugno",
              "luglio", "agosto", "settembre", "ottobre", "novembre", "dicembre"],
            &["lunedì", "martedì", "mercoledì", "giovedì", "venerdì", "sabato", "domenica"],
            "%d/%m/%Y",
            "%H:%M",
            "%d/%m/%Y %H:%M",
        ),
        "pt" => (
            &["janeiro", "fevereiro", "março", "abril", "maio", "junho",
              "julho", "agosto", "setembro", "outubro", "novembro", "dezembro"],
            &["segunda-feira", "terça-feira", "quarta-feira", "quinta-feira", "sexta-feira", "sábado", "domingo"],
            "%d/%m/%Y",
            "%H:%M",
            "%d/%m/%Y %H:%M",
        ),
        _ => (
            // English (default)
            &["January", "February", "March", "April", "May", "June",
              "July", "August", "September", "October", "November", "December"],
            &["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
            "%m/%d/%Y",
            "%I:%M %p",
            "%m/%d/%Y %I:%M %p",
        ),
    }
}

/// English month names (full) for replacement
const EN_MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December"
];

/// English month names (abbreviated) for replacement
const EN_MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"
];

/// English day names (full) for replacement
const EN_DAYS: [&str; 7] = [
    "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"
];

/// English day names (abbreviated) for replacement
const EN_DAYS_SHORT: [&str; 7] = [
    "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"
];

/// Get abbreviated month names for a locale
fn get_months_short(locale: &str) -> &'static [&'static str] {
    match locale {
        "fr" => &["janv.", "févr.", "mars", "avr.", "mai", "juin",
                  "juil.", "août", "sept.", "oct.", "nov.", "déc."],
        "de" => &["Jan.", "Feb.", "März", "Apr.", "Mai", "Juni",
                  "Juli", "Aug.", "Sept.", "Okt.", "Nov.", "Dez."],
        "es" => &["ene.", "feb.", "mar.", "abr.", "may.", "jun.",
                  "jul.", "ago.", "sept.", "oct.", "nov.", "dic."],
        "it" => &["gen.", "feb.", "mar.", "apr.", "mag.", "giu.",
                  "lug.", "ago.", "set.", "ott.", "nov.", "dic."],
        "pt" => &["jan.", "fev.", "mar.", "abr.", "mai.", "jun.",
                  "jul.", "ago.", "set.", "out.", "nov.", "dez."],
        _ => &["Jan", "Feb", "Mar", "Apr", "May", "Jun",
               "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
    }
}

/// Get abbreviated day names for a locale
fn get_days_short(locale: &str) -> &'static [&'static str] {
    match locale {
        "fr" => &["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."],
        "de" => &["Mo.", "Di.", "Mi.", "Do.", "Fr.", "Sa.", "So."],
        "es" => &["lun.", "mar.", "mié.", "jue.", "vie.", "sáb.", "dom."],
        "it" => &["lun.", "mar.", "mer.", "gio.", "ven.", "sab.", "dom."],
        "pt" => &["seg.", "ter.", "qua.", "qui.", "sex.", "sáb.", "dom."],
        _ => &["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    }
}

/// Check if a word match is standalone (not part of a larger word)
fn is_standalone_word(s: &str, start: usize, word_len: usize) -> bool {
    let before_ok = start == 0 || !s[..start].chars().last().map(|c| c.is_alphabetic()).unwrap_or(false);
    let after_ok = start + word_len >= s.len() || !s[start + word_len..].chars().next().map(|c| c.is_alphabetic()).unwrap_or(false);
    before_ok && after_ok
}

/// Replace a word only if it appears as a standalone word
fn replace_standalone(s: &str, from: &str, to: &str) -> String {
    let mut result = String::new();
    let mut remaining = s;

    while let Some(pos) = remaining.find(from) {
        let abs_pos = s.len() - remaining.len() + pos;
        if is_standalone_word(s, abs_pos, from.len()) {
            result.push_str(&remaining[..pos]);
            result.push_str(to);
            remaining = &remaining[pos + from.len()..];
        } else {
            result.push_str(&remaining[..pos + from.len()]);
            remaining = &remaining[pos + from.len()..];
        }
    }
    result.push_str(remaining);
    result
}

/// Replace English month/day names with localized versions
fn localize_names(formatted: &str, months: &[&str], days: &[&str], locale: &str) -> String {
    let mut result = formatted.to_string();

    // Replace full month names first (they are longer, less chance of partial match)
    for (i, en_month) in EN_MONTHS.iter().enumerate() {
        if result.contains(en_month) {
            result = result.replace(en_month, months[i]);
        }
    }

    // Replace abbreviated month names (only standalone matches)
    let months_short = get_months_short(locale);
    for (i, en_month) in EN_MONTHS_SHORT.iter().enumerate() {
        if result.contains(en_month) {
            result = replace_standalone(&result, en_month, months_short[i]);
        }
    }

    // Replace full day names
    for (i, en_day) in EN_DAYS.iter().enumerate() {
        if result.contains(en_day) {
            result = result.replace(en_day, days[i]);
        }
    }

    // Replace abbreviated day names (only standalone matches)
    let days_short = get_days_short(locale);
    for (i, en_day) in EN_DAYS_SHORT.iter().enumerate() {
        if result.contains(en_day) {
            result = replace_standalone(&result, en_day, days_short[i]);
        }
    }

    result
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

    #[test]
    fn test_localize_date_utf8() {
        // February 15, 2024 10:30:00 UTC
        let ts = 1708000200;

        // French - should contain "février" with accent
        let fr_long = localize_date(ts, "fr", "long");
        assert!(fr_long.contains("février"), "French long format should contain 'février', got: {}", fr_long);

        // French short format
        let fr_short = localize_date(ts, "fr", "short");
        assert!(fr_short.contains("/"), "French short should use / separator, got: {}", fr_short);

        // German - should contain "Februar"
        let de_long = localize_date(ts, "de", "long");
        assert!(de_long.contains("Februar"), "German should contain 'Februar', got: {}", de_long);

        // Spanish Wednesday test (miércoles has accent)
        // March 6, 2024 is a Wednesday
        let wed_ts = 1709726400;
        let es_full = localize_date(wed_ts, "es", "full");
        assert!(es_full.contains("miércoles"), "Spanish full format should contain 'miércoles', got: {}", es_full);

        // Italian with accented day names
        let it_full = localize_date(wed_ts, "it", "full");
        assert!(it_full.contains("mercoledì"), "Italian full format should contain 'mercoledì', got: {}", it_full);

        // Custom format with UTF-8 literal
        let custom = localize_date(ts, "fr", "Créé le %d/%m/%Y à %H:%M");
        assert!(custom.contains("à"), "Custom format should preserve 'à', got: {}", custom);
        assert!(custom.contains("Créé"), "Custom format should preserve 'Créé', got: {}", custom);

        // Test abbreviated month names (%b) with French locale
        let fr_abbrev = localize_date(ts, "fr", "%d %b à %Hh");
        assert!(fr_abbrev.contains("févr."), "French abbreviated should contain 'févr.', got: {}", fr_abbrev);
        assert!(fr_abbrev.contains("à"), "French abbreviated should preserve 'à', got: {}", fr_abbrev);

        // Test abbreviated day names (%a) with French locale
        let fr_day_abbrev = localize_date(wed_ts, "fr", "%a %d %b");
        assert!(fr_day_abbrev.contains("mer."), "French abbreviated day should contain 'mer.', got: {}", fr_day_abbrev);
    }
}
