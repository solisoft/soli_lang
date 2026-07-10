pub mod helpers;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

pub fn register_datetime_builtins(env: &mut Environment) {
    env.define(
        "datetime_now".to_string(),
        Value::NativeFunction(NativeFunction::new("datetime_now", Some(0), |_args| {
            Ok(Value::Int(helpers::datetime_now()))
        })),
    );

    env.define(
        "freeze_time".to_string(),
        Value::NativeFunction(NativeFunction::new("freeze_time", Some(1), |args| {
            let timestamp = match &args[0] {
                Value::Int(n) => *n,
                Value::String(s) => helpers::datetime_parse(s)
                    .ok_or_else(|| format!("freeze_time(): invalid date string {:?}", s))?,
                other => {
                    return Err(format!(
                        "freeze_time() expects timestamp (int) or date string, got {}",
                        other.type_name()
                    ));
                }
            };
            helpers::freeze_datetime(timestamp);
            Ok(Value::Int(timestamp))
        })),
    );

    env.define(
        "travel_to".to_string(),
        Value::NativeFunction(NativeFunction::new("travel_to", Some(1), |args| {
            let timestamp = match &args[0] {
                Value::Int(n) => *n,
                Value::String(s) => helpers::datetime_parse(s)
                    .ok_or_else(|| format!("travel_to(): invalid date string {:?}", s))?,
                other => {
                    return Err(format!(
                        "travel_to() expects timestamp (int) or date string, got {}",
                        other.type_name()
                    ));
                }
            };
            helpers::freeze_datetime(timestamp);
            Ok(Value::Int(timestamp))
        })),
    );

    env.define(
        "unfreeze_time".to_string(),
        Value::NativeFunction(NativeFunction::new("unfreeze_time", Some(0), |_args| {
            helpers::unfreeze_datetime();
            Ok(Value::Null)
        })),
    );

    env.define(
        "time_ago".to_string(),
        Value::NativeFunction(NativeFunction::new("time_ago", Some(1), |args| {
            use super::i18n::helpers as i18n_helpers;
            let timestamp = match &args[0] {
                Value::Int(n) => *n,
                Value::String(s) => helpers::datetime_parse(s).unwrap_or(0),
                other => {
                    return Err(format!(
                        "time_ago() expects timestamp (int) or date string, got {}",
                        other.type_name()
                    ))
                }
            };
            let locale = i18n_helpers::get_locale();
            Ok(Value::String(
                helpers::time_ago_localized(timestamp, &locale).into(),
            ))
        })),
    );

    env.define(
        "set_locale".to_string(),
        Value::NativeFunction(NativeFunction::new("set_locale", Some(1), |args| {
            use super::i18n::helpers as i18n_helpers;
            let locale = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "set_locale() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            i18n_helpers::set_locale(&locale);
            Ok(Value::String(locale))
        })),
    );

    env.define(
        "locale".to_string(),
        Value::NativeFunction(NativeFunction::new("locale", Some(0), |_args| {
            use super::i18n::helpers as i18n_helpers;
            Ok(Value::String(i18n_helpers::get_locale().into()))
        })),
    );
}
