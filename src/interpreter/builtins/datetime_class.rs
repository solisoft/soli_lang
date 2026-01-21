//! DateTime and Duration built-in classes for SoliLang.
//!
//! Provides native DateTime and Duration classes with comprehensive
//! date and time functionality.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use chrono::{Datelike, Local, TimeZone, Timelike};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, Instance, NativeFunction, Value};

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
        Ok(dt) => Ok(dt.timestamp()),
        Err(_) => Err(format!("Invalid datetime format: {}", s)),
    }
}

pub fn register_datetime_and_duration_classes(env: &mut Environment) {
    eprintln!("DEBUG: Registering DateTime and Duration classes");
    // This should print during interpreter initialization

    // Register datetime_now function
    env.define(
        "datetime_now".to_string(),
        Value::NativeFunction(NativeFunction::new("datetime_now", Some(0), |_args| {
            let now = Local::now();
            let mut inst = Instance::new(Rc::new(Class {
                name: "DateTime".to_string(),
                superclass: None,
                methods: HashMap::new(),
                static_methods: HashMap::new(),
                native_static_methods: HashMap::new(),
                native_methods: HashMap::new(),
                constructor: None,
            }));
            inst.set("timestamp".to_string(), Value::Int(now.timestamp()));
            Ok(Value::Instance(Rc::new(RefCell::new(inst))))
        })),
    );

    // Build DateTime instance methods
    let mut dt_native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    dt_native_methods.insert(
        "year".to_string(),
        Rc::new(NativeFunction::new("DateTime.year", Some(0), |args| {
            let this = match args.get(0) {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.year() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("timestamp").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(t, 0)
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::Int(local.year() as i64))
                }
                _ => Err("DateTime missing timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "month".to_string(),
        Rc::new(NativeFunction::new("DateTime.month", Some(0), |args| {
            let this = match args.get(0) {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.month() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("timestamp").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(t, 0)
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::Int(local.month() as i64))
                }
                _ => Err("DateTime missing timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "day".to_string(),
        Rc::new(NativeFunction::new("DateTime.day", Some(0), |args| {
            let this = match args.get(0) {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.day() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("timestamp").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(t, 0)
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::Int(local.day() as i64))
                }
                _ => Err("DateTime missing timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "hour".to_string(),
        Rc::new(NativeFunction::new("DateTime.hour", Some(0), |args| {
            let this = match args.get(0) {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.hour() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("timestamp").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(t, 0)
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::Int(local.hour() as i64))
                }
                _ => Err("DateTime missing timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "minute".to_string(),
        Rc::new(NativeFunction::new("DateTime.minute", Some(0), |args| {
            let this = match args.get(0) {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.minute() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("timestamp").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(t, 0)
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::Int(local.minute() as i64))
                }
                _ => Err("DateTime missing timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "second".to_string(),
        Rc::new(NativeFunction::new("DateTime.second", Some(0), |args| {
            let this = match args.get(0) {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.second() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("timestamp").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(t, 0)
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::Int(local.second() as i64))
                }
                _ => Err("DateTime missing timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "weekday".to_string(),
        Rc::new(NativeFunction::new("DateTime.weekday", Some(0), |args| {
            let this = match args.get(0) {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.weekday() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("timestamp").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(t, 0)
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::String(weekday_name(local.weekday())))
                }
                _ => Err("DateTime missing timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "to_unix".to_string(),
        Rc::new(NativeFunction::new("DateTime.to_unix", Some(0), |args| {
            let this = match args.get(0) {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.to_unix() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("timestamp").cloned();
            match ts {
                Some(Value::Int(t)) => Ok(Value::Int(t)),
                _ => Err("DateTime missing timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "to_iso".to_string(),
        Rc::new(NativeFunction::new("DateTime.to_iso", Some(0), |args| {
            let this = match args.get(0) {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.to_iso() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("timestamp").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(t, 0)
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                    Ok(Value::String(dt.to_rfc3339()))
                }
                _ => Err("DateTime missing timestamp".to_string()),
            }
        })),
    );

    dt_native_methods.insert(
        "to_string".to_string(),
        Rc::new(NativeFunction::new("DateTime.to_string", Some(0), |args| {
            let this = match args.get(0) {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("DateTime.to_string() called on non-DateTime".to_string()),
            };
            let ts = this.borrow().fields.get("timestamp").cloned();
            match ts {
                Some(Value::Int(t)) => {
                    let dt = chrono::DateTime::from_timestamp(t, 0)
                        .ok_or_else(|| "Invalid timestamp".to_string())?;
                    let local = dt.with_timezone(&Local);
                    Ok(Value::String(local.format("%Y-%m-%d %H:%M:%S").to_string()))
                }
                _ => Err("DateTime missing timestamp".to_string()),
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
                let this = match args.get(0) {
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
                let this = match args.get(0) {
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
                let this = match args.get(0) {
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
                let this = match args.get(0) {
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
            let this = match args.get(0) {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Duration.to_string() called on non-Duration".to_string()),
            };
            match this.borrow().fields.get("seconds").cloned() {
                Some(Value::Float(s)) => Ok(Value::String(format!("{}s", s))),
                Some(Value::Int(s)) => Ok(Value::String(format!("{}s", s))),
                _ => Err("Duration missing seconds".to_string()),
            }
        })),
    );

    // Clone for use in static methods
    let dt_methods_for_utc = dt_native_methods.clone();
    let dt_methods_for_parse = dt_native_methods.clone();
    let dur_methods_for_between = dur_native_methods.clone();
    let dur_methods_for_seconds = dur_native_methods.clone();
    let dur_methods_for_minutes = dur_native_methods.clone();
    let dur_methods_for_hours = dur_native_methods.clone();
    let dur_methods_for_days = dur_native_methods.clone();
    let dur_methods_for_weeks = dur_native_methods.clone();

    // Create DateTime static methods
    let mut dt_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    dt_static_methods.insert(
        "utc".to_string(),
        Rc::new(NativeFunction::new("DateTime.utc", Some(0), move |_args| {
            let now = chrono::Utc::now();
            let mut inst = Instance::new(Rc::new(Class {
                name: "DateTime".to_string(),
                superclass: None,
                methods: HashMap::new(),
                static_methods: HashMap::new(),
                native_static_methods: HashMap::new(),
                native_methods: dt_methods_for_utc.clone(),
                constructor: None,
            }));
            inst.set("timestamp".to_string(), Value::Int(now.timestamp()));
            Ok(Value::Instance(Rc::new(RefCell::new(inst))))
        })),
    );

    dt_static_methods.insert(
        "parse".to_string(),
        Rc::new(NativeFunction::new(
            "DateTime.parse",
            Some(1),
            move |args| {
                let s = match args.get(0) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("DateTime.parse() requires string".to_string()),
                };
                let timestamp = parse_datetime_string(&s)?;
                let mut inst = Instance::new(Rc::new(Class {
                    name: "DateTime".to_string(),
                    superclass: None,
                    methods: HashMap::new(),
                    static_methods: HashMap::new(),
                    native_static_methods: HashMap::new(),
                    native_methods: dt_methods_for_parse.clone(),
                    constructor: None,
                }));
                inst.set("timestamp".to_string(), Value::Int(timestamp));
                Ok(Value::Instance(Rc::new(RefCell::new(inst))))
            },
        )),
    );

    // Create DateTime class
    let date_time_class = Class {
        name: "DateTime".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: dt_static_methods,
        native_methods: dt_native_methods,
        constructor: None,
    };
    env.define(
        "DateTime".to_string(),
        Value::Class(Rc::new(date_time_class)),
    );

    // Create Duration static methods
    let mut dur_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    dur_static_methods.insert(
        "between".to_string(),
        Rc::new(NativeFunction::new(
            "Duration.between",
            Some(2),
            move |args| {
                let dt1 = match args.get(0) {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("Duration.between() requires DateTime".to_string()),
                };
                let dt2 = match args.get(1) {
                    Some(Value::Instance(inst)) => inst,
                    _ => return Err("Duration.between() requires DateTime".to_string()),
                };
                let ts1 = dt1.borrow().fields.get("timestamp").cloned();
                let ts2 = dt2.borrow().fields.get("timestamp").cloned();
                match (ts1, ts2) {
                    (Some(Value::Int(t1)), Some(Value::Int(t2))) => {
                        let mut dur = Instance::new(Rc::new(Class {
                            name: "Duration".to_string(),
                            superclass: None,
                            methods: HashMap::new(),
                            static_methods: HashMap::new(),
                            native_static_methods: HashMap::new(),
                            native_methods: dur_methods_for_between.clone(),
                            constructor: None,
                        }));
                        dur.set("seconds".to_string(), Value::Float((t2 - t1) as f64));
                        Ok(Value::Instance(Rc::new(RefCell::new(dur))))
                    }
                    _ => Err("DateTime missing timestamp".to_string()),
                }
            },
        )),
    );

    dur_static_methods.insert(
        "seconds".to_string(),
        Rc::new(NativeFunction::new(
            "Duration.seconds",
            Some(1),
            move |args| {
                let s = match args.get(0) {
                    Some(Value::Float(f)) => *f,
                    Some(Value::Int(i)) => *i as f64,
                    _ => return Err("Duration.seconds() requires number".to_string()),
                };
                let mut dur = Instance::new(Rc::new(Class {
                    name: "Duration".to_string(),
                    superclass: None,
                    methods: HashMap::new(),
                    static_methods: HashMap::new(),
                    native_static_methods: HashMap::new(),
                    native_methods: dur_methods_for_seconds.clone(),
                    constructor: None,
                }));
                dur.set("seconds".to_string(), Value::Float(s));
                Ok(Value::Instance(Rc::new(RefCell::new(dur))))
            },
        )),
    );

    dur_static_methods.insert(
        "minutes".to_string(),
        Rc::new(NativeFunction::new(
            "Duration.minutes",
            Some(1),
            move |args| {
                let m = match args.get(0) {
                    Some(Value::Float(f)) => *f,
                    Some(Value::Int(i)) => *i as f64,
                    _ => return Err("Duration.minutes() requires number".to_string()),
                };
                let mut dur = Instance::new(Rc::new(Class {
                    name: "Duration".to_string(),
                    superclass: None,
                    methods: HashMap::new(),
                    static_methods: HashMap::new(),
                    native_static_methods: HashMap::new(),
                    native_methods: dur_methods_for_minutes.clone(),
                    constructor: None,
                }));
                dur.set("seconds".to_string(), Value::Float(m * 60.0));
                Ok(Value::Instance(Rc::new(RefCell::new(dur))))
            },
        )),
    );

    dur_static_methods.insert(
        "hours".to_string(),
        Rc::new(NativeFunction::new(
            "Duration.hours",
            Some(1),
            move |args| {
                let h = match args.get(0) {
                    Some(Value::Float(f)) => *f,
                    Some(Value::Int(i)) => *i as f64,
                    _ => return Err("Duration.hours() requires number".to_string()),
                };
                let mut dur = Instance::new(Rc::new(Class {
                    name: "Duration".to_string(),
                    superclass: None,
                    methods: HashMap::new(),
                    static_methods: HashMap::new(),
                    native_static_methods: HashMap::new(),
                    native_methods: dur_methods_for_hours.clone(),
                    constructor: None,
                }));
                dur.set("seconds".to_string(), Value::Float(h * 3600.0));
                Ok(Value::Instance(Rc::new(RefCell::new(dur))))
            },
        )),
    );

    dur_static_methods.insert(
        "days".to_string(),
        Rc::new(NativeFunction::new("Duration.days", Some(1), move |args| {
            let d = match args.get(0) {
                Some(Value::Float(f)) => *f,
                Some(Value::Int(i)) => *i as f64,
                _ => return Err("Duration.days() requires number".to_string()),
            };
            let mut dur = Instance::new(Rc::new(Class {
                name: "Duration".to_string(),
                superclass: None,
                methods: HashMap::new(),
                static_methods: HashMap::new(),
                native_static_methods: HashMap::new(),
                native_methods: dur_methods_for_days.clone(),
                constructor: None,
            }));
            dur.set("seconds".to_string(), Value::Float(d * 86400.0));
            Ok(Value::Instance(Rc::new(RefCell::new(dur))))
        })),
    );

    dur_static_methods.insert(
        "weeks".to_string(),
        Rc::new(NativeFunction::new(
            "Duration.weeks",
            Some(1),
            move |args| {
                let w = match args.get(0) {
                    Some(Value::Float(f)) => *f,
                    Some(Value::Int(i)) => *i as f64,
                    _ => return Err("Duration.weeks() requires number".to_string()),
                };
                let mut dur = Instance::new(Rc::new(Class {
                    name: "Duration".to_string(),
                    superclass: None,
                    methods: HashMap::new(),
                    static_methods: HashMap::new(),
                    native_static_methods: HashMap::new(),
                    native_methods: dur_methods_for_weeks.clone(),
                    constructor: None,
                }));
                dur.set("seconds".to_string(), Value::Float(w * 86400.0 * 7.0));
                Ok(Value::Instance(Rc::new(RefCell::new(dur))))
            },
        )),
    );

    // Create Duration class
    let duration_class = Class {
        name: "Duration".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: dur_static_methods,
        native_methods: HashMap::new(),
        constructor: None,
    };
    env.define(
        "Duration".to_string(),
        Value::Class(Rc::new(duration_class)),
    );
}
