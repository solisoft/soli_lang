use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};
use crate::span::Span;

mod datetime_inner {
    use crate::interpreter::value::Value;
    use crate::span::Span;
    use chrono::{Datelike, Timelike};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn weekday_name(wday: chrono::Weekday) -> String {
        match wday {
            chrono::Weekday::Mon => "monday",
            chrono::Weekday::Tue => "tuesday",
            chrono::Weekday::Wed => "wednesday",
            chrono::Weekday::Thu => "thursday",
            chrono::Weekday::Fri => "friday",
            chrono::Weekday::Sat => "saturday",
            chrono::Weekday::Sun => "sunday",
        }
        .to_string()
    }

    pub fn datetime_now_local(_args: Vec<Value>, _span: Span) -> Result<Value, String> {
        let now = chrono::Local::now();
        Ok(Value::Int(now.timestamp()))
    }

    pub fn datetime_now_utc(_args: Vec<Value>, _span: Span) -> Result<Value, String> {
        let now = chrono::Utc::now();
        Ok(Value::Int(now.timestamp()))
    }

    pub fn datetime_from_unix(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match args.get(0) {
            Some(Value::Int(ts)) => Ok(Value::Int(*ts)),
            Some(Value::Float(ts)) => Ok(Value::Int(*ts as i64)),
            _ => Err("datetime_from_unix requires an integer timestamp".to_string()),
        }
    }

    pub fn datetime_to_unix(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match args.get(0) {
            Some(Value::Int(ts)) => Ok(Value::Int(*ts)),
            Some(Value::Float(ts)) => Ok(Value::Int(*ts as i64)),
            _ => Err("datetime_to_unix requires an integer timestamp".to_string()),
        }
    }

    pub fn datetime_parse(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match args.get(0) {
            Some(Value::String(s)) => {
                let s = s.as_str();
                let datetime = if s.ends_with('Z') || s.contains("+") {
                    chrono::DateTime::parse_from_rfc3339(s)
                        .or_else(|_| chrono::DateTime::parse_from_rfc2822(s))
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
                            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").map(
                                |nd| {
                                    chrono::DateTime::from_naive_utc_and_offset(
                                        nd,
                                        chrono::FixedOffset::east_opt(0).unwrap(),
                                    )
                                },
                            )
                        })
                };

                match datetime {
                    Ok(dt) => Ok(Value::Int(dt.timestamp())),
                    Err(_) => Ok(Value::Null),
                }
            }
            _ => Err("datetime_parse requires a string".to_string()),
        }
    }

    pub fn datetime_parse_tz(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match (args.get(0), args.get(1)) {
            (Some(Value::String(s)), Some(Value::String(tz))) => {
                let s = s.as_str();
                let datetime = chrono::DateTime::parse_from_rfc3339(&format!("{}T{}", s, tz))
                    .or_else(|_| chrono::DateTime::parse_from_rfc2822(&format!("{} {}", s, tz)));

                match datetime {
                    Ok(dt) => Ok(Value::Int(dt.timestamp())),
                    Err(_) => Ok(Value::Null),
                }
            }
            _ => Err("datetime_parse_tz requires (string, string)".to_string()),
        }
    }

    pub fn datetime_format(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match (args.get(0), args.get(1)) {
            (Some(Value::Int(ts)), Some(Value::String(fmt))) => {
                let datetime = chrono::DateTime::from_timestamp(*ts, 0)
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                let local = datetime.with_timezone(&chrono::Local);
                let formatted = local.format(fmt.as_str()).to_string();
                Ok(Value::String(formatted))
            }
            _ => Err("datetime_format requires (timestamp: Int, format: String)".to_string()),
        }
    }

    pub fn datetime_components(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match args.get(0) {
            Some(Value::Int(ts)) => {
                let datetime = chrono::DateTime::from_timestamp(*ts, 0)
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                let local = datetime.with_timezone(&chrono::Local);

                let components: Vec<(Value, Value)> = vec![
                    (
                        Value::String("year".to_string()),
                        Value::Int(local.year() as i64),
                    ),
                    (
                        Value::String("month".to_string()),
                        Value::Int(local.month() as i64),
                    ),
                    (
                        Value::String("day".to_string()),
                        Value::Int(local.day() as i64),
                    ),
                    (
                        Value::String("hour".to_string()),
                        Value::Int(local.hour() as i64),
                    ),
                    (
                        Value::String("minute".to_string()),
                        Value::Int(local.minute() as i64),
                    ),
                    (
                        Value::String("second".to_string()),
                        Value::Int(local.second() as i64),
                    ),
                    (
                        Value::String("nanosecond".to_string()),
                        Value::Int(local.nanosecond() as i64),
                    ),
                    (
                        Value::String("weekday".to_string()),
                        Value::String(weekday_name(local.weekday())),
                    ),
                    (
                        Value::String("ordinal".to_string()),
                        Value::Int(local.ordinal() as i64),
                    ),
                    (Value::String("is_dst".to_string()), Value::Bool(false)),
                ];

                Ok(Value::Hash(Rc::new(RefCell::new(components))))
            }
            _ => Err("datetime_components requires a timestamp".to_string()),
        }
    }

    pub fn datetime_add(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match (args.get(0), args.get(1)) {
            (Some(Value::Int(ts)), Some(Value::Int(seconds))) => Ok(Value::Int(ts + seconds)),
            (Some(Value::Int(ts)), Some(Value::Float(seconds))) => {
                Ok(Value::Int(ts + (*seconds as i64)))
            }
            _ => Err("datetime_add requires (timestamp: Int, seconds: Int|Float)".to_string()),
        }
    }

    pub fn datetime_add_days(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match args.get(0) {
            Some(Value::Int(ts)) => Ok(Value::Int(ts + (24 * 60 * 60))),
            _ => Err("datetime_add_days requires a timestamp".to_string()),
        }
    }

    pub fn datetime_add_hours(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match args.get(0) {
            Some(Value::Int(ts)) => Ok(Value::Int(ts + (60 * 60))),
            _ => Err("datetime_add_hours requires a timestamp".to_string()),
        }
    }

    pub fn datetime_add_weeks(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match args.get(0) {
            Some(Value::Int(ts)) => Ok(Value::Int(ts + (7 * 24 * 60 * 60))),
            _ => Err("datetime_add_weeks requires a timestamp".to_string()),
        }
    }

    pub fn datetime_add_months(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match args.get(0) {
            Some(Value::Int(ts)) => {
                let datetime = chrono::DateTime::from_timestamp(*ts, 0)
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                let local = datetime.with_timezone(&chrono::Local);
                let new = local + chrono::Months::new(1);
                Ok(Value::Int(new.timestamp()))
            }
            _ => Err("datetime_add_months requires a timestamp".to_string()),
        }
    }

    pub fn datetime_add_years(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match args.get(0) {
            Some(Value::Int(ts)) => {
                let datetime = chrono::DateTime::from_timestamp(*ts, 0)
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                let local = datetime.with_timezone(&chrono::Local);
                let new = local + chrono::Months::new(12);
                Ok(Value::Int(new.timestamp()))
            }
            _ => Err("datetime_add_years requires a timestamp".to_string()),
        }
    }

    pub fn datetime_diff(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match (args.get(0), args.get(1)) {
            (Some(Value::Int(ts1)), Some(Value::Int(ts2))) => Ok(Value::Int(ts2 - ts1)),
            _ => Err("datetime_diff requires (timestamp1: Int, timestamp2: Int)".to_string()),
        }
    }

    pub fn datetime_sub(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match (args.get(0), args.get(1)) {
            (Some(Value::Int(ts)), Some(Value::Int(seconds))) => Ok(Value::Int(ts - seconds)),
            (Some(Value::Int(ts)), Some(Value::Float(seconds))) => {
                Ok(Value::Int(ts - (*seconds as i64)))
            }
            _ => Err("datetime_sub requires (timestamp: Int, seconds: Int|Float)".to_string()),
        }
    }

    pub fn datetime_is_before(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match (args.get(0), args.get(1)) {
            (Some(Value::Int(ts1)), Some(Value::Int(ts2))) => Ok(Value::Bool(ts1 < ts2)),
            _ => Err("datetime_is_before requires (timestamp1: Int, timestamp2: Int)".to_string()),
        }
    }

    pub fn datetime_is_after(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match (args.get(0), args.get(1)) {
            (Some(Value::Int(ts1)), Some(Value::Int(ts2))) => Ok(Value::Bool(ts1 > ts2)),
            _ => Err("datetime_is_after requires (timestamp1: Int, timestamp2: Int)".to_string()),
        }
    }

    pub fn datetime_is_same(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match (args.get(0), args.get(1)) {
            (Some(Value::Int(ts1)), Some(Value::Int(ts2))) => Ok(Value::Bool(ts1 == ts2)),
            _ => Err("datetime_is_same requires (timestamp1: Int, timestamp2: Int)".to_string()),
        }
    }

    pub fn datetime_to_iso(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match args.get(0) {
            Some(Value::Int(ts)) => {
                let datetime = chrono::DateTime::from_timestamp(*ts, 0)
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                Ok(Value::String(datetime.to_rfc3339()))
            }
            _ => Err("datetime_to_iso requires a timestamp".to_string()),
        }
    }

    pub fn datetime_weekday(args: Vec<Value>, _span: Span) -> Result<Value, String> {
        match args.get(0) {
            Some(Value::Int(ts)) => {
                let datetime = chrono::DateTime::from_timestamp(*ts, 0)
                    .ok_or_else(|| "Invalid timestamp".to_string())?;
                let local = datetime.with_timezone(&chrono::Local);
                Ok(Value::String(weekday_name(local.weekday())))
            }
            _ => Err("datetime_weekday requires a timestamp".to_string()),
        }
    }
}

pub fn register_datetime_builtins(env: &mut Environment) {
    // __datetime_now_local() - Get current local time as timestamp
    env.define(
        "__datetime_now_local".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__datetime_now_local",
            Some(0),
            |args| datetime_inner::datetime_now_local(args, Span::default()),
        )),
    );

    // __datetime_now_utc() - Get current UTC time as timestamp
    env.define(
        "__datetime_now_utc".to_string(),
        Value::NativeFunction(NativeFunction::new("__datetime_now_utc", Some(0), |args| {
            datetime_inner::datetime_now_utc(args, Span::default())
        })),
    );

    // __datetime_from_unix(timestamp) - Create datetime from Unix timestamp
    env.define(
        "__datetime_from_unix".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__datetime_from_unix",
            Some(1),
            |args| datetime_inner::datetime_from_unix(args, Span::default()),
        )),
    );

    // __datetime_to_unix(timestamp) - Get Unix timestamp from datetime
    env.define(
        "__datetime_to_unix".to_string(),
        Value::NativeFunction(NativeFunction::new("__datetime_to_unix", Some(1), |args| {
            datetime_inner::datetime_to_unix(args, Span::default())
        })),
    );

    // __datetime_parse(string) - Parse ISO string to timestamp
    env.define(
        "__datetime_parse".to_string(),
        Value::NativeFunction(NativeFunction::new("__datetime_parse", Some(1), |args| {
            datetime_inner::datetime_parse(args, Span::default())
        })),
    );

    // __datetime_parse_tz(string, timezone) - Parse with timezone
    env.define(
        "__datetime_parse_tz".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__datetime_parse_tz",
            Some(2),
            |args| datetime_inner::datetime_parse_tz(args, Span::default()),
        )),
    );

    // __datetime_format(timestamp, format) - Format datetime to string
    env.define(
        "__datetime_format".to_string(),
        Value::NativeFunction(NativeFunction::new("__datetime_format", Some(2), |args| {
            datetime_inner::datetime_format(args, Span::default())
        })),
    );

    // __datetime_components(timestamp) - Get datetime components as Hash
    env.define(
        "__datetime_components".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__datetime_components",
            Some(1),
            |args| datetime_inner::datetime_components(args, Span::default()),
        )),
    );

    // __datetime_add(timestamp, seconds) - Add seconds to datetime
    env.define(
        "__datetime_add".to_string(),
        Value::NativeFunction(NativeFunction::new("__datetime_add", Some(2), |args| {
            datetime_inner::datetime_add(args, Span::default())
        })),
    );

    // __datetime_add_days(timestamp) - Add one day to datetime
    env.define(
        "__datetime_add_days".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__datetime_add_days",
            Some(1),
            |args| datetime_inner::datetime_add_days(args, Span::default()),
        )),
    );

    // __datetime_add_hours(timestamp) - Add one hour to datetime
    env.define(
        "__datetime_add_hours".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__datetime_add_hours",
            Some(1),
            |args| datetime_inner::datetime_add_hours(args, Span::default()),
        )),
    );

    // __datetime_add_weeks(timestamp) - Add one week to datetime
    env.define(
        "__datetime_add_weeks".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__datetime_add_weeks",
            Some(1),
            |args| datetime_inner::datetime_add_weeks(args, Span::default()),
        )),
    );

    // __datetime_add_months(timestamp) - Add one month to datetime
    env.define(
        "__datetime_add_months".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__datetime_add_months",
            Some(1),
            |args| datetime_inner::datetime_add_months(args, Span::default()),
        )),
    );

    // __datetime_add_years(timestamp) - Add one year to datetime
    env.define(
        "__datetime_add_years".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__datetime_add_years",
            Some(1),
            |args| datetime_inner::datetime_add_years(args, Span::default()),
        )),
    );

    // __datetime_sub(timestamp, seconds) - Subtract seconds from datetime
    env.define(
        "__datetime_sub".to_string(),
        Value::NativeFunction(NativeFunction::new("__datetime_sub", Some(2), |args| {
            datetime_inner::datetime_sub(args, Span::default())
        })),
    );

    // __datetime_diff(timestamp1, timestamp2) - Get difference in seconds
    env.define(
        "__datetime_diff".to_string(),
        Value::NativeFunction(NativeFunction::new("__datetime_diff", Some(2), |args| {
            datetime_inner::datetime_diff(args, Span::default())
        })),
    );

    // __datetime_is_before(timestamp1, timestamp2) - Check if timestamp1 < timestamp2
    env.define(
        "__datetime_is_before".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__datetime_is_before",
            Some(2),
            |args| datetime_inner::datetime_is_before(args, Span::default()),
        )),
    );

    // __datetime_is_after(timestamp1, timestamp2) - Check if timestamp1 > timestamp2
    env.define(
        "__datetime_is_after".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__datetime_is_after",
            Some(2),
            |args| datetime_inner::datetime_is_after(args, Span::default()),
        )),
    );

    // __datetime_is_same(timestamp1, timestamp2) - Check if timestamp1 == timestamp2
    env.define(
        "__datetime_is_same".to_string(),
        Value::NativeFunction(NativeFunction::new("__datetime_is_same", Some(2), |args| {
            datetime_inner::datetime_is_same(args, Span::default())
        })),
    );

    // __datetime_to_iso(timestamp) - Convert to ISO 8601 string
    env.define(
        "__datetime_to_iso".to_string(),
        Value::NativeFunction(NativeFunction::new("__datetime_to_iso", Some(1), |args| {
            datetime_inner::datetime_to_iso(args, Span::default())
        })),
    );

    // __datetime_weekday(timestamp) - Get weekday name
    env.define(
        "__datetime_weekday".to_string(),
        Value::NativeFunction(NativeFunction::new("__datetime_weekday", Some(1), |args| {
            datetime_inner::datetime_weekday(args, Span::default())
        })),
    );
}
