//! DateTime and Duration built-in classes for SoliLang.
//!
//! Provides native DateTime and Duration classes with comprehensive
//! date and time functionality.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};

use super::i18n::helpers::{get_locale as i18n_get_locale, interpolate, lookup_translation};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Instance, NativeFunction, Value};

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
    static DURATION_INSTANCE_CLASS: RefCell<Option<Rc<Class>>> = const { RefCell::new(None) };
}

fn datetime_instance_class() -> Result<Rc<Class>, String> {
    DATETIME_INSTANCE_CLASS
        .with(|c| c.borrow().clone())
        .ok_or_else(|| "DateTime class not registered on this thread".to_string())
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

    dt_native_methods.insert(
        "year".to_string(),
        Rc::new(NativeFunction::new("DateTime.year", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.year() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("_ts").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(
                        t / 1_000_000_000,
                        (t % 1_000_000_000) as u32,
                    )
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::Int(local.year() as i64))
                }
                _ => Err("DateTime missing internal timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "month".to_string(),
        Rc::new(NativeFunction::new("DateTime.month", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.month() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("_ts").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(
                        t / 1_000_000_000,
                        (t % 1_000_000_000) as u32,
                    )
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::Int(local.month() as i64))
                }
                _ => Err("DateTime missing internal timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "day".to_string(),
        Rc::new(NativeFunction::new("DateTime.day", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.day() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("_ts").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(
                        t / 1_000_000_000,
                        (t % 1_000_000_000) as u32,
                    )
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::Int(local.day() as i64))
                }
                _ => Err("DateTime missing internal timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "hour".to_string(),
        Rc::new(NativeFunction::new("DateTime.hour", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.hour() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("_ts").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(
                        t / 1_000_000_000,
                        (t % 1_000_000_000) as u32,
                    )
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                    Ok(Value::Int(dt.hour() as i64)) // Return UTC hour
                }
                _ => Err("DateTime missing internal timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "minute".to_string(),
        Rc::new(NativeFunction::new("DateTime.minute", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.minute() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("_ts").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(
                        t / 1_000_000_000,
                        (t % 1_000_000_000) as u32,
                    )
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                    Ok(Value::Int(dt.minute() as i64)) // Return UTC minute
                }
                _ => Err("DateTime missing internal timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "second".to_string(),
        Rc::new(NativeFunction::new("DateTime.second", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.second() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("_ts").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(
                        t / 1_000_000_000,
                        (t % 1_000_000_000) as u32,
                    )
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::Int(local.second() as i64))
                }
                _ => Err("DateTime missing internal timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "millisecond".to_string(),
        Rc::new(NativeFunction::new(
            "DateTime.millisecond",
            Some(0),
            |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("DateTime.millisecond() called on non-DateTime".to_string()),
                };
                let ts = this.borrow().fields.get("_ts").cloned();
                match ts {
                    Some(Value::Int(t)) => {
                        let dt = chrono::DateTime::from_timestamp(
                            t / 1_000_000_000,
                            (t % 1_000_000_000) as u32,
                        )
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                        let local = dt.with_timezone(&Local);
                        Ok(Value::Int(local.timestamp_subsec_millis() as i64))
                    }
                    _ => Err("DateTime missing internal timestamp".to_string()),
                }
            },
        )),
    );

    dt_native_methods.insert(
        "weekday".to_string(),
        Rc::new(NativeFunction::new("DateTime.weekday", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.weekday() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("_ts").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(
                        t / 1_000_000_000,
                        (t % 1_000_000_000) as u32,
                    )
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::String(weekday_name(local.weekday()).into()))
                }
                _ => Err("DateTime missing internal timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "to_unix".to_string(),
        Rc::new(NativeFunction::new("DateTime.to_unix", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.to_unix() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("_ts").cloned();
            match ts {
                Some(Value::Int(t)) => Ok(Value::Int(t / 1_000_000_000)), // Convert to seconds
                _ => Err("DateTime missing internal timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "to_iso".to_string(),
        Rc::new(NativeFunction::new("DateTime.to_iso", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.to_iso() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("_ts").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(
                        t / 1_000_000_000,
                        (t % 1_000_000_000) as u32,
                    )
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                    Ok(Value::String(dt.to_rfc3339().into()))
                }
                _ => Err("DateTime missing internal timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "to_string".to_string(),
        Rc::new(NativeFunction::new("DateTime.to_string", Some(0), |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.to_string() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("_ts").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(
                        t / 1_000_000_000,
                        (t % 1_000_000_000) as u32,
                    )
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::String(
                        local.format("%Y-%m-%d %H:%M:%S").to_string().into(),
                    ))
                }
                _ => Err("DateTime missing internal timestamp".to_string()),
            }
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

    // Clone for use in instance methods that create new DateTime instances

    // Add instance methods that create new DateTime instances
    dt_native_methods.insert(
        "add_days".to_string(),
        Rc::new(NativeFunction::new("DateTime.add_days", Some(1), {
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("DateTime.add_days() called on non-DateTime".to_string()),
                };
                let days = match args.get(1) {
                    Some(Value::Int(d)) => *d,
                    Some(Value::Float(d)) => *d as i64,
                    _ => return Err("DateTime.add_days() requires number".to_string()),
                };
                let ts = this.borrow().fields.get("_ts").cloned();
                match ts {
                    Some(Value::Int(t)) => {
                        let new_ts = t + days * 86400 * 1_000_000_000;
                        let mut inst = Instance::new(datetime_instance_class()?);
                        inst.set("_ts".to_string(), Value::Int(new_ts));
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("DateTime missing internal timestamp".to_string()),
                }
            }
        })),
    );

    dt_native_methods.insert(
        "add_hours".to_string(),
        Rc::new(NativeFunction::new("DateTime.add_hours", Some(1), {
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("DateTime.add_hours() called on non-DateTime".to_string()),
                };
                let hours = match args.get(1) {
                    Some(Value::Int(h)) => *h,
                    Some(Value::Float(h)) => *h as i64,
                    _ => return Err("DateTime.add_hours() requires number".to_string()),
                };
                let ts = this.borrow().fields.get("_ts").cloned();
                match ts {
                    Some(Value::Int(t)) => {
                        let new_ts = t + hours * 3600 * 1_000_000_000;
                        let mut inst = Instance::new(datetime_instance_class()?);
                        inst.set("_ts".to_string(), Value::Int(new_ts));
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("DateTime missing internal timestamp".to_string()),
                }
            }
        })),
    );

    dt_native_methods.insert(
        "add_minutes".to_string(),
        Rc::new(NativeFunction::new("DateTime.add_minutes", Some(1), {
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("DateTime.add_minutes() called on non-DateTime".to_string()),
                };
                let minutes = match args.get(1) {
                    Some(Value::Int(m)) => *m,
                    Some(Value::Float(m)) => *m as i64,
                    _ => return Err("DateTime.add_minutes() requires number".to_string()),
                };
                let ts = this.borrow().fields.get("_ts").cloned();
                match ts {
                    Some(Value::Int(t)) => {
                        let new_ts = t + minutes * 60 * 1_000_000_000;
                        let mut inst = Instance::new(datetime_instance_class()?);
                        inst.set("_ts".to_string(), Value::Int(new_ts));
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("DateTime missing internal timestamp".to_string()),
                }
            }
        })),
    );

    dt_native_methods.insert(
        "subtract_days".to_string(),
        Rc::new(NativeFunction::new("DateTime.subtract_days", Some(1), {
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("DateTime.subtract_days() called on non-DateTime".to_string()),
                };
                let days = match args.get(1) {
                    Some(Value::Int(d)) => *d,
                    Some(Value::Float(d)) => *d as i64,
                    _ => return Err("DateTime.subtract_days() requires number".to_string()),
                };
                let ts = this.borrow().fields.get("_ts").cloned();
                match ts {
                    Some(Value::Int(t)) => {
                        let new_ts = t - days * 86400 * 1_000_000_000;
                        let mut inst = Instance::new(datetime_instance_class()?);
                        inst.set("_ts".to_string(), Value::Int(new_ts));
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("DateTime missing internal timestamp".to_string()),
                }
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
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("DateTime.format() called on non-DateTime".to_string()),
                };
                let fmt = match args.get(1) {
                    Some(Value::String(f)) => f.clone(),
                    _ => return Err("DateTime.format() requires format string".to_string()),
                };
                let locale = match args.get(2) {
                    Some(Value::String(l)) => Some(l.clone()),
                    Some(_) => return Err("DateTime.format() locale must be a string".to_string()),
                    None => None,
                };
                let ts = this.borrow().fields.get("_ts").cloned();
                match ts {
                    Some(Value::Int(t)) => {
                        let dt = chrono::DateTime::from_timestamp(
                            t / 1_000_000_000,
                            (t % 1_000_000_000) as u32,
                        )
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                        let local = dt.with_timezone(&Local);
                        let formatted = local.format(&fmt).to_string();
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
                    _ => Err("DateTime missing internal timestamp".to_string()),
                }
            }
        })),
    );

    // Clone for boundary instance methods

    dt_native_methods.insert(
        "beginning_of_minute".to_string(),
        Rc::new(NativeFunction::new(
            "DateTime.beginning_of_minute",
            Some(0),
            {
                move |args| {
                    let this = match args.first() {
                        Some(Value::Instance(inst)) => inst,
                        _ => {
                            return Err(
                                "DateTime.beginning_of_minute() called on non-DateTime".to_string()
                            )
                        }
                    };
                    let ts = this.borrow().fields.get("_ts").cloned();
                    match ts {
                        Some(Value::Int(t)) => {
                            let dt = chrono::DateTime::from_timestamp(
                                t / 1_000_000_000,
                                (t % 1_000_000_000) as u32,
                            )
                            .ok_or_else(|| "Invalid timestamp".to_string())?;
                            let local = dt.with_timezone(&Local);
                            let boundary = local
                                .with_second(0)
                                .and_then(|d| d.with_nanosecond(0))
                                .ok_or_else(|| {
                                    "Failed to compute beginning_of_minute".to_string()
                                })?;
                            let new_ts = boundary.timestamp_nanos_opt().unwrap_or(0);
                            let mut inst = Instance::new(datetime_instance_class()?);
                            inst.set("_ts".to_string(), Value::Int(new_ts));
                            Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                        }
                        _ => Err("DateTime missing internal timestamp".to_string()),
                    }
                }
            },
        )),
    );

    dt_native_methods.insert(
        "end_of_minute".to_string(),
        Rc::new(NativeFunction::new("DateTime.end_of_minute", Some(0), {
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("DateTime.end_of_minute() called on non-DateTime".to_string()),
                };
                let ts = this.borrow().fields.get("_ts").cloned();
                match ts {
                    Some(Value::Int(t)) => {
                        let dt = chrono::DateTime::from_timestamp(
                            t / 1_000_000_000,
                            (t % 1_000_000_000) as u32,
                        )
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                        let local = dt.with_timezone(&Local);
                        let boundary = local
                            .with_second(59)
                            .and_then(|d| d.with_nanosecond(999_000_000))
                            .ok_or_else(|| "Failed to compute end_of_minute".to_string())?;
                        let new_ts = boundary.timestamp_nanos_opt().unwrap_or(0);
                        let mut inst = Instance::new(datetime_instance_class()?);
                        inst.set("_ts".to_string(), Value::Int(new_ts));
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("DateTime missing internal timestamp".to_string()),
                }
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
                    let this = match args.first() {
                        Some(Value::Instance(inst)) => inst,
                        _ => {
                            return Err(
                                "DateTime.beginning_of_hour() called on non-DateTime".to_string()
                            )
                        }
                    };
                    let ts = this.borrow().fields.get("_ts").cloned();
                    match ts {
                        Some(Value::Int(t)) => {
                            let dt = chrono::DateTime::from_timestamp(
                                t / 1_000_000_000,
                                (t % 1_000_000_000) as u32,
                            )
                            .ok_or_else(|| "Invalid timestamp".to_string())?;
                            let local = dt.with_timezone(&Local);
                            let boundary = local
                                .with_minute(0)
                                .and_then(|d| d.with_second(0))
                                .and_then(|d| d.with_nanosecond(0))
                                .ok_or_else(|| "Failed to compute beginning_of_hour".to_string())?;
                            let new_ts = boundary.timestamp_nanos_opt().unwrap_or(0);
                            let mut inst = Instance::new(datetime_instance_class()?);
                            inst.set("_ts".to_string(), Value::Int(new_ts));
                            Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                        }
                        _ => Err("DateTime missing internal timestamp".to_string()),
                    }
                }
            },
        )),
    );

    dt_native_methods.insert(
        "end_of_hour".to_string(),
        Rc::new(NativeFunction::new("DateTime.end_of_hour", Some(0), {
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("DateTime.end_of_hour() called on non-DateTime".to_string()),
                };
                let ts = this.borrow().fields.get("_ts").cloned();
                match ts {
                    Some(Value::Int(t)) => {
                        let dt = chrono::DateTime::from_timestamp(
                            t / 1_000_000_000,
                            (t % 1_000_000_000) as u32,
                        )
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                        let local = dt.with_timezone(&Local);
                        let boundary = local
                            .with_minute(59)
                            .and_then(|d| d.with_second(59))
                            .and_then(|d| d.with_nanosecond(999_000_000))
                            .ok_or_else(|| "Failed to compute end_of_hour".to_string())?;
                        let new_ts = boundary.timestamp_nanos_opt().unwrap_or(0);
                        let mut inst = Instance::new(datetime_instance_class()?);
                        inst.set("_ts".to_string(), Value::Int(new_ts));
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("DateTime missing internal timestamp".to_string()),
                }
            }
        })),
    );

    dt_native_methods.insert(
        "beginning_of_day".to_string(),
        Rc::new(NativeFunction::new("DateTime.beginning_of_day", Some(0), {
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => {
                        return Err("DateTime.beginning_of_day() called on non-DateTime".to_string())
                    }
                };
                let ts = this.borrow().fields.get("_ts").cloned();
                match ts {
                    Some(Value::Int(t)) => {
                        let dt = chrono::DateTime::from_timestamp(
                            t / 1_000_000_000,
                            (t % 1_000_000_000) as u32,
                        )
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                        let local = dt.with_timezone(&Local);
                        let boundary = local
                            .with_hour(0)
                            .and_then(|d| d.with_minute(0))
                            .and_then(|d| d.with_second(0))
                            .and_then(|d| d.with_nanosecond(0))
                            .ok_or_else(|| "Failed to compute beginning_of_day".to_string())?;
                        let new_ts = boundary.timestamp_nanos_opt().unwrap_or(0);
                        let mut inst = Instance::new(datetime_instance_class()?);
                        inst.set("_ts".to_string(), Value::Int(new_ts));
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("DateTime missing internal timestamp".to_string()),
                }
            }
        })),
    );

    dt_native_methods.insert(
        "end_of_day".to_string(),
        Rc::new(NativeFunction::new("DateTime.end_of_day", Some(0), {
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("DateTime.end_of_day() called on non-DateTime".to_string()),
                };
                let ts = this.borrow().fields.get("_ts").cloned();
                match ts {
                    Some(Value::Int(t)) => {
                        let dt = chrono::DateTime::from_timestamp(
                            t / 1_000_000_000,
                            (t % 1_000_000_000) as u32,
                        )
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                        let local = dt.with_timezone(&Local);
                        let boundary = local
                            .with_hour(23)
                            .and_then(|d| d.with_minute(59))
                            .and_then(|d| d.with_second(59))
                            .and_then(|d| d.with_nanosecond(999_000_000))
                            .ok_or_else(|| "Failed to compute end_of_day".to_string())?;
                        let new_ts = boundary.timestamp_nanos_opt().unwrap_or(0);
                        let mut inst = Instance::new(datetime_instance_class()?);
                        inst.set("_ts".to_string(), Value::Int(new_ts));
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("DateTime missing internal timestamp".to_string()),
                }
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
                    let this = match args.first() {
                        Some(Value::Instance(inst)) => inst,
                        _ => {
                            return Err(
                                "DateTime.beginning_of_month() called on non-DateTime".to_string()
                            )
                        }
                    };
                    let ts = this.borrow().fields.get("_ts").cloned();
                    match ts {
                        Some(Value::Int(t)) => {
                            let dt = chrono::DateTime::from_timestamp(
                                t / 1_000_000_000,
                                (t % 1_000_000_000) as u32,
                            )
                            .ok_or_else(|| "Invalid timestamp".to_string())?;
                            let local = dt.with_timezone(&Local);
                            let naive = NaiveDate::from_ymd_opt(local.year(), local.month(), 1)
                                .ok_or_else(|| "Failed to compute beginning_of_month".to_string())?
                                .and_hms_nano_opt(0, 0, 0, 0)
                                .ok_or_else(|| {
                                    "Failed to compute beginning_of_month".to_string()
                                })?;
                            let boundary = Local.from_local_datetime(&naive).unwrap();
                            let new_ts = boundary.timestamp_nanos_opt().unwrap_or(0);
                            let mut inst = Instance::new(datetime_instance_class()?);
                            inst.set("_ts".to_string(), Value::Int(new_ts));
                            Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                        }
                        _ => Err("DateTime missing internal timestamp".to_string()),
                    }
                }
            },
        )),
    );

    dt_native_methods.insert(
        "end_of_month".to_string(),
        Rc::new(NativeFunction::new("DateTime.end_of_month", Some(0), {
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("DateTime.end_of_month() called on non-DateTime".to_string()),
                };
                let ts = this.borrow().fields.get("_ts").cloned();
                match ts {
                    Some(Value::Int(t)) => {
                        let dt = chrono::DateTime::from_timestamp(
                            t / 1_000_000_000,
                            (t % 1_000_000_000) as u32,
                        )
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                        let local = dt.with_timezone(&Local);
                        let (next_year, next_month) = if local.month() == 12 {
                            (local.year() + 1, 1)
                        } else {
                            (local.year(), local.month() + 1)
                        };
                        let last_day = NaiveDate::from_ymd_opt(next_year, next_month, 1)
                            .ok_or_else(|| "Failed to compute end_of_month".to_string())?
                            - Duration::days(1);
                        let naive = last_day
                            .and_hms_nano_opt(23, 59, 59, 999_000_000)
                            .ok_or_else(|| "Failed to compute end_of_month".to_string())?;
                        let boundary = Local.from_local_datetime(&naive).unwrap();
                        let new_ts = boundary.timestamp_nanos_opt().unwrap_or(0);
                        let mut inst = Instance::new(datetime_instance_class()?);
                        inst.set("_ts".to_string(), Value::Int(new_ts));
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("DateTime missing internal timestamp".to_string()),
                }
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
                    let this = match args.first() {
                        Some(Value::Instance(inst)) => inst,
                        _ => {
                            return Err(
                                "DateTime.beginning_of_year() called on non-DateTime".to_string()
                            )
                        }
                    };
                    let ts = this.borrow().fields.get("_ts").cloned();
                    match ts {
                        Some(Value::Int(t)) => {
                            let dt = chrono::DateTime::from_timestamp(
                                t / 1_000_000_000,
                                (t % 1_000_000_000) as u32,
                            )
                            .ok_or_else(|| "Invalid timestamp".to_string())?;
                            let local = dt.with_timezone(&Local);
                            let naive = NaiveDate::from_ymd_opt(local.year(), 1, 1)
                                .ok_or_else(|| "Failed to compute beginning_of_year".to_string())?
                                .and_hms_nano_opt(0, 0, 0, 0)
                                .ok_or_else(|| "Failed to compute beginning_of_year".to_string())?;
                            let boundary = Local.from_local_datetime(&naive).unwrap();
                            let new_ts = boundary.timestamp_nanos_opt().unwrap_or(0);
                            let mut inst = Instance::new(datetime_instance_class()?);
                            inst.set("_ts".to_string(), Value::Int(new_ts));
                            Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                        }
                        _ => Err("DateTime missing internal timestamp".to_string()),
                    }
                }
            },
        )),
    );

    dt_native_methods.insert(
        "end_of_year".to_string(),
        Rc::new(NativeFunction::new("DateTime.end_of_year", Some(0), {
            move |args| {
                let this = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("DateTime.end_of_year() called on non-DateTime".to_string()),
                };
                let ts = this.borrow().fields.get("_ts").cloned();
                match ts {
                    Some(Value::Int(t)) => {
                        let dt = chrono::DateTime::from_timestamp(
                            t / 1_000_000_000,
                            (t % 1_000_000_000) as u32,
                        )
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                        let local = dt.with_timezone(&Local);
                        let first_of_next = NaiveDate::from_ymd_opt(local.year() + 1, 1, 1)
                            .ok_or_else(|| "Failed to compute end_of_year".to_string())?;
                        let last_day = first_of_next - Duration::days(1);
                        let naive = last_day
                            .and_hms_nano_opt(23, 59, 59, 999_000_000)
                            .ok_or_else(|| "Failed to compute end_of_year".to_string())?;
                        let boundary = Local.from_local_datetime(&naive).unwrap();
                        let new_ts = boundary.timestamp_nanos_opt().unwrap_or(0);
                        let mut inst = Instance::new(datetime_instance_class()?);
                        inst.set("_ts".to_string(), Value::Int(new_ts));
                        Ok(Value::Instance(Rc::new(RefCell::new(inst))))
                    }
                    _ => Err("DateTime missing internal timestamp".to_string()),
                }
            }
        })),
    );

    // Clone for use in static methods

    // Create DateTime static methods
    let mut dt_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // now() - Create DateTime with current local time
    dt_static_methods.insert(
        "now".to_string(),
        Rc::new(NativeFunction::new("DateTime.now", Some(0), move |_args| {
            let now = Local::now();
            let mut inst = Instance::new(datetime_instance_class()?);
            inst.set(
                "_ts".to_string(),
                Value::Int(now.timestamp() * 1_000_000_000),
            );
            Ok(Value::Instance(Rc::new(RefCell::new(inst))))
        })),
    );

    dt_static_methods.insert(
        "utc".to_string(),
        Rc::new(NativeFunction::new("DateTime.utc", Some(0), move |_args| {
            let now = chrono::Utc::now();
            let mut inst = Instance::new(datetime_instance_class()?);
            inst.set(
                "_ts".to_string(),
                Value::Int(now.timestamp_nanos_opt().unwrap_or(0)),
            );
            Ok(Value::Instance(Rc::new(RefCell::new(inst))))
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
                let mut inst = Instance::new(datetime_instance_class()?);
                inst.set("_ts".to_string(), Value::Int(timestamp));
                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
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
            move |_args| {
                let mut inst = Instance::new(datetime_instance_class()?);
                inst.set("_ts".to_string(), Value::Int(0));
                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
            },
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
            let mut inst = Instance::new(datetime_instance_class()?);
            inst.set("_ts".to_string(), Value::Int(ts_nanos));
            Ok(Value::Instance(Rc::new(RefCell::new(inst))))
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
    env.define("DateTime".to_string(), Value::Class(date_time_class));

    // Create Duration static methods
    let mut dur_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    dur_static_methods.insert(
        "between".to_string(),
        Rc::new(NativeFunction::new(
            "Duration.between",
            Some(2),
            move |args| {
                let dt1 = match args.first() {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("Duration.between() requires DateTime".to_string()),
                };
                let dt2 = match args.get(1) {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("Duration.between() requires DateTime".to_string()),
                };
                let ts1 = dt1.borrow().fields.get("_ts").cloned();
                let ts2 = dt2.borrow().fields.get("_ts").cloned();
                match (ts1, ts2) {
                    (Some(Value::Int(t1)), Some(Value::Int(t2))) => {
                        let mut dur = Instance::new(duration_instance_class()?);
                        // `_ts` is in nanoseconds; Duration stores seconds.
                        dur.set(
                            "seconds".to_string(),
                            Value::Float((t2 - t1) as f64 / 1_000_000_000.0),
                        );
                        Ok(Value::Instance(Rc::new(RefCell::new(dur))))
                    }
                    _ => Err("DateTime missing internal timestamp".to_string()),
                }
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
                dur.set("seconds".to_string(), Value::Float(s));
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
                dur.set("seconds".to_string(), Value::Float(m * 60.0));
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
                dur.set("seconds".to_string(), Value::Float(h * 3600.0));
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
                dur.set("seconds".to_string(), Value::Float(d * 86400.0));
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
                dur.set("seconds".to_string(), Value::Float(w * 86400.0 * 7.0));
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
