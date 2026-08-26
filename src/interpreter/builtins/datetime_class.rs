//! DateTime and Duration built-in classes for SoliLang.
//!
//! Provides native DateTime and Duration classes with comprehensive
//! date and time functionality.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use chrono::{Datelike, Duration, NaiveDate, Timelike};

use super::datetime::local_zone;

use super::i18n::helpers::{get_locale as i18n_get_locale, interpolate, lookup_translation};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Instance, NativeFunction, Value};

/// DateTime's instance-method table, keyed by method name.
type DateTimeMethodMap = HashMap<String, Rc<NativeFunction>>;

thread_local! {
    /// Complete instance classes for DateTime/Duration values, filled at
    /// the end of `register_datetime_and_duration_classes`. Methods that
    /// construct result instances read these at call time, so chained
    /// results always carry the full method map — snapshot clones of the
    /// half-built map used to drop later-registered methods (e.g.
    /// `dt.add_days(3).format(...)` failed because `format` was missing
    /// from `add_days`'s captured map). Sharing one `Rc<Class>` also
    /// avoids rebuilding a Class per returned instance.
    static DATETIME_INSTANCE_CLASS: RefCell<Option<Rc<Class>>> = const { RefCell::new(None) };
    /// DateTime's instance methods, reachable without a receiver object.
    /// A `DateTime` is a `Value::DateTime(nanos, use_utc)` rather than an `Instance`, so
    /// dispatch cannot go through a class's `native_methods` — both engines
    /// look methods up here instead. One map, so the two engines cannot drift.
    static DATETIME_METHODS: RefCell<Option<Rc<DateTimeMethodMap>>> = const { RefCell::new(None) };
    static DURATION_INSTANCE_CLASS: RefCell<Option<Rc<Class>>> = const { RefCell::new(None) };
}

/// Look up one of DateTime's instance methods for a native receiver.
pub fn datetime_method(name: &str) -> Option<Rc<NativeFunction>> {
    DATETIME_METHODS.with(|m| m.borrow().as_ref().and_then(|map| map.get(name).cloned()))
}

/// Every method name DateTime answers to — used by the type checker and by
/// `respond_to?`-style checks so they agree with dispatch by construction.
pub fn datetime_method_names() -> Vec<String> {
    DATETIME_METHODS.with(|m| {
        m.borrow()
            .as_ref()
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default()
    })
}

fn duration_instance_class() -> Result<Rc<Class>, String> {
    DURATION_INSTANCE_CLASS
        .with(|c| c.borrow().clone())
        .ok_or_else(|| "Duration class not registered on this thread".to_string())
}

fn weekday_name(wday: chrono::Weekday) -> String {
    match wday {
        chrono::Weekday::Mon => "Monday",
        chrono::Weekday::Tue => "Tuesday",
        chrono::Weekday::Wed => "Wednesday",
        chrono::Weekday::Thu => "Thursday",
        chrono::Weekday::Fri => "Friday",
        chrono::Weekday::Sat => "Saturday",
        chrono::Weekday::Sun => "Sunday",
    }
    .to_string()
}

fn parse_datetime_string(s: &str) -> Result<i64, String> {
    let s = s.trim();
    let datetime = if s.ends_with('Z') || s.contains("+") {
        chrono::DateTime::parse_from_rfc3339(s).or_else(|_| chrono::DateTime::parse_from_rfc2822(s))
    } else {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
            .map(|nd| {
                chrono::DateTime::from_naive_utc_and_offset(
                    nd,
                    chrono::FixedOffset::east_opt(0).unwrap(),
                )
            })
            .or_else(|_| {
                chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map(|d| {
                    chrono::DateTime::from_naive_utc_and_offset(
                        d.and_hms_opt(0, 0, 0).unwrap(),
                        chrono::FixedOffset::east_opt(0).unwrap(),
                    )
                })
            })
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").map(|nd| {
                    chrono::DateTime::from_naive_utc_and_offset(
                        nd,
                        chrono::FixedOffset::east_opt(0).unwrap(),
                    )
                })
            })
    };

    match datetime {
        Ok(dt) => match dt.timestamp_nanos_opt() {
            Some(nanos) => Ok(nanos),
            None => Ok(dt.timestamp() * 1_000_000_000),
        },
        Err(_) => Err(format!("Invalid datetime format: {}", s)),
    }
}

/// Unpack a DateTime receiver: `(nanos, use_utc)`.
fn recv_dt(args: &[Value], method: &str) -> Result<(i64, bool), String> {
    match args.first() {
        Some(Value::DateTime(t, u)) => Ok((*t, *u)),
        _ => Err(format!("{method}() called on non-DateTime")),
    }
}

/// Wall-clock components in the DateTime's selected view (local or UTC).
fn wall_clock(nanos: i64, use_utc: bool) -> chrono::DateTime<chrono::FixedOffset> {
    if use_utc {
        chrono::DateTime::from_timestamp_nanos(nanos).fixed_offset()
    } else {
        local_zone::local_from_nanos(nanos)
    }
}

/// Human-readable **magnitude** of a duration (e.g. `"1 hour 1 minute"`).
/// Uses `seconds.abs()` so negative intervals from `Duration.between` still
/// name the length. Never appends `" ago"` — that belongs to `time_ago`.
fn humanize_duration(seconds: f64, locale: &str) -> String {
    let total_secs = seconds.abs();
    let (primary_unit, primary_count, secondary_count) = if total_secs < 60.0 {
        ("seconds", total_secs as i64, 0)
    } else if total_secs < 3600.0 {
        let minutes = (total_secs / 60.0).floor();
        let remaining_secs = (total_secs % 60.0).floor();
        (
            "minutes",
            minutes as i64,
            if remaining_secs > 0.0 {
                remaining_secs as i64
            } else {
                0
            },
        )
    } else if total_secs < 86400.0 {
        let hours = (total_secs / 3600.0).floor();
        let remaining_mins = ((total_secs % 3600.0) / 60.0).floor();
        (
            "hours",
            hours as i64,
            if remaining_mins > 0.0 {
                remaining_mins as i64
            } else {
                0
            },
        )
    } else {
        let days = (total_secs / 86400.0).floor();
        let remaining_hrs = ((total_secs % 86400.0) / 3600.0).floor();
        (
            "days",
            days as i64,
            if remaining_hrs > 0.0 {
                remaining_hrs as i64
            } else {
                0
            },
        )
    };
    let key = format!("duration.{}", primary_unit);
    let primary_translated = lookup_translation(locale, &key).unwrap_or_else(|| {
        let count_str = primary_count.to_string();
        match primary_unit {
            "seconds" => format!(
                "{} second{}",
                count_str,
                if primary_count == 1 { "" } else { "s" }
            ),
            "minutes" => format!(
                "{} minute{}",
                count_str,
                if primary_count == 1 { "" } else { "s" }
            ),
            "hours" => format!(
                "{} hour{}",
                count_str,
                if primary_count == 1 { "" } else { "s" }
            ),
            "days" => format!(
                "{} day{}",
                count_str,
                if primary_count == 1 { "" } else { "s" }
            ),
            _ => format!("{} {}", count_str, primary_unit),
        }
    });
    let primary_formatted = interpolate(
        &primary_translated,
        &[("count".to_string(), primary_count.to_string())],
    );
    if secondary_count > 0 {
        let sec_key = if primary_unit == "minutes" {
            "seconds"
        } else if primary_unit == "hours" {
            "minutes"
        } else {
            "hours"
        };
        let secondary_translated = lookup_translation(locale, sec_key).unwrap_or_else(|| {
            let count_str = secondary_count.to_string();
            match sec_key {
                "seconds" => format!(
                    "{} second{}",
                    count_str,
                    if secondary_count == 1 { "" } else { "s" }
                ),
                "minutes" => format!(
                    "{} minute{}",
                    count_str,
                    if secondary_count == 1 { "" } else { "s" }
                ),
                "hours" => format!(
                    "{} hour{}",
                    count_str,
                    if secondary_count == 1 { "" } else { "s" }
                ),
                _ => format!("{} {}", count_str, sec_key),
            }
        });
        let secondary_formatted = interpolate(
            &secondary_translated,
            &[("count".to_string(), secondary_count.to_string())],
        );
        format!("{} {}", primary_formatted, secondary_formatted)
    } else {
        primary_formatted
    }
}

pub fn register_datetime_and_duration_classes(env: &mut Environment) {
    // Build DateTime instance methods
    let mut dt_native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Component accessors all use the same wall-clock view (local by default;
    // UTC after `.utc()`). Previously hour/minute were hard-coded to UTC while
    // year/month/day/second/format used local — composing parts could yield a
    // moment that never existed.
    dt_native_methods.insert(
        "year".to_string(),
        Rc::new(NativeFunction::new("DateTime.year", Some(0), |args| {
            let (t, use_utc) = recv_dt(args, "DateTime.year")?;
            Ok(Value::Int(wall_clock(t, use_utc).year() as i64))
        })),
    );

    dt_native_methods.insert(
        "month".to_string(),
        Rc::new(NativeFunction::new("DateTime.month", Some(0), |args| {
            let (t, use_utc) = recv_dt(args, "DateTime.month")?;
            Ok(Value::Int(wall_clock(t, use_utc).month() as i64))
        })),
    );

    dt_native_methods.insert(
        "day".to_string(),
        Rc::new(NativeFunction::new("DateTime.day", Some(0), |args| {
            let (t, use_utc) = recv_dt(args, "DateTime.day")?;
            Ok(Value::Int(wall_clock(t, use_utc).day() as i64))
        })),
    );

    dt_native_methods.insert(
        "hour".to_string(),
        Rc::new(NativeFunction::new("DateTime.hour", Some(0), |args| {
            let (t, use_utc) = recv_dt(args, "DateTime.hour")?;
            Ok(Value::Int(wall_clock(t, use_utc).hour() as i64))
        })),
    );

    dt_native_methods.insert(
        "minute".to_string(),
        Rc::new(NativeFunction::new("DateTime.minute", Some(0), |args| {
            let (t, use_utc) = recv_dt(args, "DateTime.minute")?;
            Ok(Value::Int(wall_clock(t, use_utc).minute() as i64))
        })),
    );

    dt_native_methods.insert(
        "second".to_string(),
        Rc::new(NativeFunction::new("DateTime.second", Some(0), |args| {
            let (t, use_utc) = recv_dt(args, "DateTime.second")?;
            Ok(Value::Int(wall_clock(t, use_utc).second() as i64))
        })),
    );

    dt_native_methods.insert(
        "millisecond".to_string(),
        Rc::new(NativeFunction::new(
            "DateTime.millisecond",
            Some(0),
            |args| {
                let (t, use_utc) = recv_dt(args, "DateTime.millisecond")?;
                Ok(Value::Int(
                    wall_clock(t, use_utc).timestamp_subsec_millis() as i64
                ))
            },
        )),
    );

    dt_native_methods.insert(
        "weekday".to_string(),
        Rc::new(NativeFunction::new("DateTime.weekday", Some(0), |args| {
            let (t, use_utc) = recv_dt(args, "DateTime.weekday")?;
            Ok(Value::String(
                weekday_name(wall_clock(t, use_utc).weekday()).into(),
            ))
        })),
    );

    dt_native_methods.insert(
        "to_unix".to_string(),
        Rc::new(NativeFunction::new("DateTime.to_unix", Some(0), |args| {
            let (t, _) = recv_dt(args, "DateTime.to_unix")?;
            Ok(Value::Int(t / 1_000_000_000)) // Convert to seconds
        })),
    );

    dt_native_methods.insert(
        "to_iso".to_string(),
        Rc::new(NativeFunction::new("DateTime.to_iso", Some(0), |args| {
            let (t, _) = recv_dt(args, "DateTime.to_iso")?;
            let dt = chrono::DateTime::from_timestamp_nanos(t);
            Ok(Value::String(dt.to_rfc3339().into()))
        })),
    );

    dt_native_methods.insert(
        "to_string".to_string(),
        Rc::new(NativeFunction::new("DateTime.to_string", Some(0), |args| {
            let (t, use_utc) = recv_dt(args, "DateTime.to_string")?;
            Ok(Value::String(
                wall_clock(t, use_utc)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
                    .into(),
            ))
        })),
    );

    // View toggles — same instant, different component zone.
    dt_native_methods.insert(
        "utc".to_string(),
        Rc::new(NativeFunction::new("DateTime.utc", Some(0), |args| {
            let (t, _) = recv_dt(args, "DateTime.utc")?;
            Ok(Value::DateTime(t, true))
        })),
    );

    dt_native_methods.insert(
        "local".to_string(),
        Rc::new(NativeFunction::new("DateTime.local", Some(0), |args| {
            let (t, _) = recv_dt(args, "DateTime.local")?;
            Ok(Value::DateTime(t, false))
        })),
    );

    // Build Duration instance methods
    let mut dur_native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    dur_native_methods.insert(
        "total_seconds".to_string(),
        Rc::new(NativeFunction::new(
            "Duration.total_seconds",
            Some(0),
            |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("Duration.total_seconds() called on non-Duration".to_string()),
                };
                match this.borrow().fields.get("seconds").cloned() {
                    Some(Value::Float(s)) => Ok(Value::Float(s)),
                    Some(Value::Int(s)) => Ok(Value::Float(s as f64)),
                    _ => Err("Duration missing seconds".to_string()),
                }
            },
        )),
    );

    dur_native_methods.insert(
        "total_minutes".to_string(),
        Rc::new(NativeFunction::new(
            "Duration.total_minutes",
            Some(0),
            |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("Duration.total_minutes() called on non-Duration".to_string()),
                };
                match this.borrow().fields.get("seconds").cloned() {
                    Some(Value::Float(s)) => Ok(Value::Float(s / 60.0)),
                    Some(Value::Int(s)) => Ok(Value::Float(s as f64 / 60.0)),
                    _ => Err("Duration missing seconds".to_string()),
                }
            },
        )),
    );

    dur_native_methods.insert(
        "total_hours".to_string(),
        Rc::new(NativeFunction::new(
            "Duration.total_hours",
            Some(0),
            |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("Duration.total_hours() called on non-Duration".to_string()),
                };
                match this.borrow().fields.get("seconds").cloned() {
                    Some(Value::Float(s)) => Ok(Value::Float(s / 3600.0)),
                    Some(Value::Int(s)) => Ok(Value::Float(s as f64 / 3600.0)),
                    _ => Err("Duration missing seconds".to_string()),
                }
            },
        )),
    );

    dur_native_methods.insert(
        "total_days".to_string(),
        Rc::new(NativeFunction::new(
            "Duration.total_days",
            Some(0),
            |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("Duration.total_days() called on non-Duration".to_string()),
                };
                match this.borrow().fields.get("seconds").cloned() {
                    Some(Value::Float(s)) => Ok(Value::Float(s / 86400.0)),
                    Some(Value::Int(s)) => Ok(Value::Float(s as f64 / 86400.0)),
                    _ => Err("Duration missing seconds".to_string()),
                }
            },
        )),
    );

    dur_native_methods.insert(
        "to_string".to_string(),
        Rc::new(NativeFunction::new("Duration.to_string", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Duration.to_string() called on non-Duration".to_string()),
            };
            match this.borrow().fields.get("seconds").cloned() {
                Some(Value::Float(s)) => Ok(Value::String(format!("{}s", s).into())),
                Some(Value::Int(s)) => Ok(Value::String(format!("{}s", s).into())),
                _ => Err("Duration missing seconds".to_string()),
            }
        })),
    );

    dur_native_methods.insert(
        String::from("humanize"),
        Rc::new(NativeFunction::new(
            "Duration.humanize",
            None,
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("Duration.humanize() called on non-Duration".to_string()),
                };
                let locale = if args.len() > 1 {
                    match &args[1] {
                        Value::String(s) => s.clone(),
                        Value::Null => i18n_get_locale().into(),
                        _ => return Err("Duration.humanize() locale must be a string".to_string()),
                    }
                } else {
                    i18n_get_locale().into()
                };
                let seconds = match this.borrow().fields.get("seconds").cloned() {
                    Some(Value::Float(s)) => s,
                    Some(Value::Int(s)) => s as f64,
                    _ => return Err("Duration missing seconds".to_string()),
                };
                Ok(Value::String(humanize_duration(seconds, &locale).into()))
            },
        )),
    );

    // Instance methods that create new DateTime instances preserve the view flag.
    dt_native_methods.insert(
        "add_days".to_string(),
        Rc::new(NativeFunction::new("DateTime.add_days", Some(1), {
            move |args| {
                let (t, use_utc) = recv_dt(args, "DateTime.add_days")?;
                let days = match args.get(1) {
                    Some(Value::Int(d)) => *d,
                    Some(Value::Float(d)) => *d as i64,
                    _ => return Err("DateTime.add_days() requires number".to_string()),
                };
                Ok(Value::DateTime(t + days * 86400 * 1_000_000_000, use_utc))
            }
        })),
    );

    dt_native_methods.insert(
        "add_hours".to_string(),
        Rc::new(NativeFunction::new("DateTime.add_hours", Some(1), {
            move |args| {
                let (t, use_utc) = recv_dt(args, "DateTime.add_hours")?;
                let hours = match args.get(1) {
                    Some(Value::Int(h)) => *h,
                    Some(Value::Float(h)) => *h as i64,
                    _ => return Err("DateTime.add_hours() requires number".to_string()),
                };
                Ok(Value::DateTime(t + hours * 3600 * 1_000_000_000, use_utc))
            }
        })),
    );

    dt_native_methods.insert(
        "add_minutes".to_string(),
        Rc::new(NativeFunction::new("DateTime.add_minutes", Some(1), {
            move |args| {
                let (t, use_utc) = recv_dt(args, "DateTime.add_minutes")?;
                let minutes = match args.get(1) {
                    Some(Value::Int(m)) => *m,
                    Some(Value::Float(m)) => *m as i64,
                    _ => return Err("DateTime.add_minutes() requires number".to_string()),
                };
                Ok(Value::DateTime(t + minutes * 60 * 1_000_000_000, use_utc))
            }
        })),
    );

    dt_native_methods.insert(
        "subtract_days".to_string(),
        Rc::new(NativeFunction::new("DateTime.subtract_days", Some(1), {
            move |args| {
                let (t, use_utc) = recv_dt(args, "DateTime.subtract_days")?;
                let days = match args.get(1) {
                    Some(Value::Int(d)) => *d,
                    Some(Value::Float(d)) => *d as i64,
                    _ => return Err("DateTime.subtract_days() requires number".to_string()),
                };
                Ok(Value::DateTime(t - days * 86400 * 1_000_000_000, use_utc))
            }
        })),
    );

    dt_native_methods.insert(
        "format".to_string(),
        Rc::new(NativeFunction::new("DateTime.format", None, {
            move |args| {
                // args: [this, format_string] or [this, format_string, locale]
                if args.len() < 2 || args.len() > 3 {
                    return Err(format!(
                        "DateTime.format() expects 1-2 arguments, got {}",
                        args.len() - 1
                    ));
                }
                let (t, use_utc) = recv_dt(args, "DateTime.format")?;
                let fmt = match args.get(1) {
                    Some(Value::String(f)) => f.clone(),
                    _ => return Err("DateTime.format() requires format string".to_string()),
                };
                let locale = match args.get(2) {
                    Some(Value::String(l)) => Some(l.clone()),
                    Some(_) => return Err("DateTime.format() locale must be a string".to_string()),
                    None => None,
                };
                let wall = wall_clock(t, use_utc);
                let formatted = wall.format(&fmt).to_string();
                match locale {
                    Some(ref loc) if **loc != *"en" => {
                        use super::datetime::helpers::{get_locale_data, localize_names};
                        let (months, days, _, _, _) = get_locale_data(loc);
                        Ok(Value::String(
                            localize_names(&formatted, months, days, loc).into(),
                        ))
                    }
                    _ => Ok(Value::String(formatted.into())),
                }
            }
        })),
    );

    // Boundary helpers operate in the DateTime's selected view and preserve it.
    fn boundary_from_naive(naive: chrono::NaiveDateTime, use_utc: bool) -> Result<Value, String> {
        let boundary = if use_utc {
            use chrono::TimeZone;
            chrono::Utc.from_utc_datetime(&naive).fixed_offset()
        } else {
            local_zone::resolve_local(&naive)
        };
        let new_ts = boundary.timestamp_nanos_opt().unwrap_or(0);
        Ok(Value::DateTime(new_ts, use_utc))
    }

    dt_native_methods.insert(
        "beginning_of_minute".to_string(),
        Rc::new(NativeFunction::new(
            "DateTime.beginning_of_minute",
            Some(0),
            {
                move |args| {
                    let (t, use_utc) = recv_dt(args, "DateTime.beginning_of_minute")?;
                    let wall = wall_clock(t, use_utc);
                    let naive = wall
                        .naive_local()
                        .with_second(0)
                        .and_then(|d| d.with_nanosecond(0))
                        .ok_or_else(|| "Failed to compute beginning_of_minute".to_string())?;
                    boundary_from_naive(naive, use_utc)
                }
            },
        )),
    );

    dt_native_methods.insert(
        "end_of_minute".to_string(),
        Rc::new(NativeFunction::new("DateTime.end_of_minute", Some(0), {
            move |args| {
                let (t, use_utc) = recv_dt(args, "DateTime.end_of_minute")?;
                let wall = wall_clock(t, use_utc);
                let naive = wall
                    .naive_local()
                    .with_second(59)
                    .and_then(|d| d.with_nanosecond(999_000_000))
                    .ok_or_else(|| "Failed to compute end_of_minute".to_string())?;
                boundary_from_naive(naive, use_utc)
            }
        })),
    );

    dt_native_methods.insert(
        "beginning_of_hour".to_string(),
        Rc::new(NativeFunction::new(
            "DateTime.beginning_of_hour",
            Some(0),
            {
                move |args| {
                    let (t, use_utc) = recv_dt(args, "DateTime.beginning_of_hour")?;
                    let wall = wall_clock(t, use_utc);
                    let naive = wall
                        .naive_local()
                        .with_minute(0)
                        .and_then(|d| d.with_second(0))
                        .and_then(|d| d.with_nanosecond(0))
                        .ok_or_else(|| "Failed to compute beginning_of_hour".to_string())?;
                    boundary_from_naive(naive, use_utc)
                }
            },
        )),
    );

    dt_native_methods.insert(
        "end_of_hour".to_string(),
        Rc::new(NativeFunction::new("DateTime.end_of_hour", Some(0), {
            move |args| {
                let (t, use_utc) = recv_dt(args, "DateTime.end_of_hour")?;
                let wall = wall_clock(t, use_utc);
                let naive = wall
                    .naive_local()
                    .with_minute(59)
                    .and_then(|d| d.with_second(59))
                    .and_then(|d| d.with_nanosecond(999_000_000))
                    .ok_or_else(|| "Failed to compute end_of_hour".to_string())?;
                boundary_from_naive(naive, use_utc)
            }
        })),
    );

    dt_native_methods.insert(
        "beginning_of_day".to_string(),
        Rc::new(NativeFunction::new("DateTime.beginning_of_day", Some(0), {
            move |args| {
                let (t, use_utc) = recv_dt(args, "DateTime.beginning_of_day")?;
                let wall = wall_clock(t, use_utc);
                let naive = wall
                    .naive_local()
                    .with_hour(0)
                    .and_then(|d| d.with_minute(0))
                    .and_then(|d| d.with_second(0))
                    .and_then(|d| d.with_nanosecond(0))
                    .ok_or_else(|| "Failed to compute beginning_of_day".to_string())?;
                boundary_from_naive(naive, use_utc)
            }
        })),
    );

    dt_native_methods.insert(
        "end_of_day".to_string(),
        Rc::new(NativeFunction::new("DateTime.end_of_day", Some(0), {
            move |args| {
                let (t, use_utc) = recv_dt(args, "DateTime.end_of_day")?;
                let wall = wall_clock(t, use_utc);
                let naive = wall
                    .naive_local()
                    .with_hour(23)
                    .and_then(|d| d.with_minute(59))
                    .and_then(|d| d.with_second(59))
                    .and_then(|d| d.with_nanosecond(999_000_000))
                    .ok_or_else(|| "Failed to compute end_of_day".to_string())?;
                boundary_from_naive(naive, use_utc)
            }
        })),
    );

    dt_native_methods.insert(
        "beginning_of_month".to_string(),
        Rc::new(NativeFunction::new(
            "DateTime.beginning_of_month",
            Some(0),
            {
                move |args| {
                    let (t, use_utc) = recv_dt(args, "DateTime.beginning_of_month")?;
                    let wall = wall_clock(t, use_utc);
                    let naive = NaiveDate::from_ymd_opt(wall.year(), wall.month(), 1)
                        .ok_or_else(|| "Failed to compute beginning_of_month".to_string())?
                        .and_hms_nano_opt(0, 0, 0, 0)
                        .ok_or_else(|| "Failed to compute beginning_of_month".to_string())?;
                    boundary_from_naive(naive, use_utc)
                }
            },
        )),
    );

    dt_native_methods.insert(
        "end_of_month".to_string(),
        Rc::new(NativeFunction::new("DateTime.end_of_month", Some(0), {
            move |args| {
                let (t, use_utc) = recv_dt(args, "DateTime.end_of_month")?;
                let wall = wall_clock(t, use_utc);
                let (next_year, next_month) = if wall.month() == 12 {
                    (wall.year() + 1, 1)
                } else {
                    (wall.year(), wall.month() + 1)
                };
                let last_day = NaiveDate::from_ymd_opt(next_year, next_month, 1)
                    .ok_or_else(|| "Failed to compute end_of_month".to_string())?
                    - Duration::days(1);
                let naive = last_day
                    .and_hms_nano_opt(23, 59, 59, 999_000_000)
                    .ok_or_else(|| "Failed to compute end_of_month".to_string())?;
                boundary_from_naive(naive, use_utc)
            }
        })),
    );

    dt_native_methods.insert(
        "beginning_of_year".to_string(),
        Rc::new(NativeFunction::new(
            "DateTime.beginning_of_year",
            Some(0),
            {
                move |args| {
                    let (t, use_utc) = recv_dt(args, "DateTime.beginning_of_year")?;
                    let wall = wall_clock(t, use_utc);
                    let naive = NaiveDate::from_ymd_opt(wall.year(), 1, 1)
                        .ok_or_else(|| "Failed to compute beginning_of_year".to_string())?
                        .and_hms_nano_opt(0, 0, 0, 0)
                        .ok_or_else(|| "Failed to compute beginning_of_year".to_string())?;
                    boundary_from_naive(naive, use_utc)
                }
            },
        )),
    );

    dt_native_methods.insert(
        "end_of_year".to_string(),
        Rc::new(NativeFunction::new("DateTime.end_of_year", Some(0), {
            move |args| {
                let (t, use_utc) = recv_dt(args, "DateTime.end_of_year")?;
                let wall = wall_clock(t, use_utc);
                let first_of_next = NaiveDate::from_ymd_opt(wall.year() + 1, 1, 1)
                    .ok_or_else(|| "Failed to compute end_of_year".to_string())?;
                let last_day = first_of_next - Duration::days(1);
                let naive = last_day
                    .and_hms_nano_opt(23, 59, 59, 999_000_000)
                    .ok_or_else(|| "Failed to compute end_of_year".to_string())?;
                boundary_from_naive(naive, use_utc)
            }
        })),
    );

    // Create DateTime static methods
    let mut dt_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // now() - Create DateTime with current local-view instant
    dt_static_methods.insert(
        "now".to_string(),
        Rc::new(NativeFunction::new("DateTime.now", Some(0), move |_args| {
            let now = local_zone::now_local();
            Ok(Value::DateTime(now.timestamp() * 1_000_000_000, false))
        })),
    );

    // utc() static - current instant with UTC component view
    dt_static_methods.insert(
        "utc".to_string(),
        Rc::new(NativeFunction::new("DateTime.utc", Some(0), move |_args| {
            let now = chrono::Utc::now();
            Ok(Value::DateTime(
                now.timestamp_nanos_opt().unwrap_or(0),
                true,
            ))
        })),
    );

    dt_static_methods.insert(
        "parse".to_string(),
        Rc::new(NativeFunction::new(
            "DateTime.parse",
            Some(1),
            move |args| {
                let s = match args.first() {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("DateTime.parse() requires string".to_string()),
                };
                let timestamp = parse_datetime_string(&s)?;
                Ok(Value::DateTime(timestamp, false))
            },
        )),
    );

    // microtime() - Returns current time in microseconds as float (static method)
    use std::time::{SystemTime, UNIX_EPOCH};
    dt_static_methods.insert(
        "microtime".to_string(),
        Rc::new(NativeFunction::new(
            "DateTime.microtime",
            Some(0),
            |_args| {
                let duration = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| e.to_string())?;
                let micros =
                    duration.as_secs() as f64 * 1_000_000.0 + duration.subsec_micros() as f64;
                Ok(Value::Float(micros))
            },
        )),
    );

    // epoch() - Create DateTime at Unix epoch (1970-01-01 00:00:00 UTC)
    dt_static_methods.insert(
        "epoch".to_string(),
        Rc::new(NativeFunction::new(
            "DateTime.epoch",
            Some(0),
            move |_args| Ok(Value::DateTime(0, false)),
        )),
    );

    // from_unix(timestamp) - Create DateTime from Unix timestamp (seconds)
    dt_static_methods.insert(
        "from_unix".to_string(),
        Rc::new(NativeFunction::new("DateTime.from_unix", Some(1), move |args| {
            let ts = match args.first() {
                Some(Value::Int(t)) => *t,
                Some(Value::Float(t)) => *t as i64,
                _ => return Err("DateTime.from_unix() requires number".to_string()),
            };
            // Use checked multiplication to avoid overflow
            let ts_nanos = ts.checked_mul(1_000_000_000)
                .ok_or_else(|| "DateTime.from_unix(): timestamp overflow (value too large, expected seconds not milliseconds)".to_string())?;
            Ok(Value::DateTime(ts_nanos, false))
        })),
    );

    // Create DateTime class
    let date_time_class = Rc::new(Class {
        name: "DateTime".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: dt_static_methods,
        native_methods: dt_native_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    });
    // Every DateTime instance — including results of chained calls like
    // `dt.add_days(3)` — shares this complete class via the thread-local.
    DATETIME_INSTANCE_CLASS.with(|c| {
        *c.borrow_mut() = Some(date_time_class.clone());
    });
    DATETIME_METHODS.with(|m| {
        *m.borrow_mut() = Some(Rc::new(date_time_class.native_methods.clone()));
    });
    env.define("DateTime".to_string(), Value::Class(date_time_class));

    // Create Duration static methods
    let mut dur_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    dur_static_methods.insert(
        "between".to_string(),
        Rc::new(NativeFunction::new(
            "Duration.between",
            Some(2),
            move |args| {
                let (t1, _) = match args.first() {
                    Some(Value::DateTime(t, u)) => (*t, *u),
                    _ => return Err("Duration.between() requires DateTime".to_string()),
                };
                let (t2, _) = match args.get(1) {
                    Some(Value::DateTime(t, u)) => (*t, *u),
                    _ => return Err("Duration.between() requires DateTime".to_string()),
                };
                let mut dur = Instance::new(duration_instance_class()?);
                // `_ts` is in nanoseconds; Duration stores seconds.
                dur.set("seconds", Value::Float((t2 - t1) as f64 / 1_000_000_000.0));
                Ok(Value::Instance(Rc::new(RefCell::new(dur))))
            },
        )),
    );

    // of_seconds(n) - Create Duration from seconds
    dur_static_methods.insert(
        "of_seconds".to_string(),
        Rc::new(NativeFunction::new("Duration.of_seconds", Some(1), {
            move |args| {
                let s = match args.first() {
                    Some(Value::Float(f)) => *f,
                    Some(Value::Int(i)) => *i as f64,
                    _ => return Err("Duration.of_seconds() requires number".to_string()),
                };
                let mut dur = Instance::new(duration_instance_class()?);
                dur.set("seconds", Value::Float(s));
                Ok(Value::Instance(Rc::new(RefCell::new(dur))))
            }
        })),
    );

    // of_minutes(n) - Create Duration from minutes
    dur_static_methods.insert(
        "of_minutes".to_string(),
        Rc::new(NativeFunction::new("Duration.of_minutes", Some(1), {
            move |args| {
                let m = match args.first() {
                    Some(Value::Float(f)) => *f,
                    Some(Value::Int(i)) => *i as f64,
                    _ => return Err("Duration.of_minutes() requires number".to_string()),
                };
                let mut dur = Instance::new(duration_instance_class()?);
                dur.set("seconds", Value::Float(m * 60.0));
                Ok(Value::Instance(Rc::new(RefCell::new(dur))))
            }
        })),
    );

    // of_hours(n) - Create Duration from hours
    dur_static_methods.insert(
        "of_hours".to_string(),
        Rc::new(NativeFunction::new("Duration.of_hours", Some(1), {
            move |args| {
                let h = match args.first() {
                    Some(Value::Float(f)) => *f,
                    Some(Value::Int(i)) => *i as f64,
                    _ => return Err("Duration.of_hours() requires number".to_string()),
                };
                let mut dur = Instance::new(duration_instance_class()?);
                dur.set("seconds", Value::Float(h * 3600.0));
                Ok(Value::Instance(Rc::new(RefCell::new(dur))))
            }
        })),
    );

    // of_days(n) - Create Duration from days
    dur_static_methods.insert(
        "of_days".to_string(),
        Rc::new(NativeFunction::new("Duration.of_days", Some(1), {
            move |args| {
                let d = match args.first() {
                    Some(Value::Float(f)) => *f,
                    Some(Value::Int(i)) => *i as f64,
                    _ => return Err("Duration.of_days() requires number".to_string()),
                };
                let mut dur = Instance::new(duration_instance_class()?);
                dur.set("seconds", Value::Float(d * 86400.0));
                Ok(Value::Instance(Rc::new(RefCell::new(dur))))
            }
        })),
    );

    // of_weeks(n) - Create Duration from weeks
    dur_static_methods.insert(
        "of_weeks".to_string(),
        Rc::new(NativeFunction::new("Duration.of_weeks", Some(1), {
            move |args| {
                let w = match args.first() {
                    Some(Value::Float(f)) => *f,
                    Some(Value::Int(i)) => *i as f64,
                    _ => return Err("Duration.of_weeks() requires number".to_string()),
                };
                let mut dur = Instance::new(duration_instance_class()?);
                dur.set("seconds", Value::Float(w * 86400.0 * 7.0));
                Ok(Value::Instance(Rc::new(RefCell::new(dur))))
            }
        })),
    );

    // Aliases: seconds, minutes, hours, days, weeks
    dur_static_methods.insert(
        "seconds".to_string(),
        dur_static_methods.get("of_seconds").unwrap().clone(),
    );
    dur_static_methods.insert(
        "minutes".to_string(),
        dur_static_methods.get("of_minutes").unwrap().clone(),
    );
    dur_static_methods.insert(
        "hours".to_string(),
        dur_static_methods.get("of_hours").unwrap().clone(),
    );
    dur_static_methods.insert(
        "days".to_string(),
        dur_static_methods.get("of_days").unwrap().clone(),
    );
    dur_static_methods.insert(
        "weeks".to_string(),
        dur_static_methods.get("of_weeks").unwrap().clone(),
    );

    // Create Duration class
    let duration_class = Rc::new(Class {
        name: "Duration".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: dur_static_methods,
        native_methods: dur_native_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    });
    // Every Duration instance shares this complete class via the
    // thread-local — same scheme as DateTime above.
    DURATION_INSTANCE_CLASS.with(|c| {
        *c.borrow_mut() = Some(duration_class.clone());
    });
    env.define("Duration".to_string(), Value::Class(duration_class));
}
