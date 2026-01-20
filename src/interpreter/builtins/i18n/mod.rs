//! i18n (internationalization) built-in functions for Soli.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Register i18n built-in functions in the given environment.
pub fn register_i18n_builtins(env: &mut Environment) {
    // __i18n_locale() - Get current locale
    env.define(
        "__i18n_locale".to_string(),
        Value::NativeFunction(NativeFunction::new("__i18n_locale", Some(0), |_args| {
            let locale = env.get_locale().unwrap_or_else(|| "en".to_string());
            Ok(Value::String(locale))
        })),
    );

    // __i18n_set_locale(locale) - Set current locale
    env.define(
        "__i18n_set_locale".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__i18n_set_locale",
            Some(1),
            |args| match &args[0] {
                Value::String(locale) => {
                    env.set_locale(locale.clone());
                    Ok(Value::String(locale.clone()))
                }
                _ => Err("__i18n_set_locale expects a string".to_string()),
            },
        )),
    );

    // __i18n_translate(key, locale?, translations?) - Translate a string
    env.define(
        "__i18n_translate".to_string(),
        Value::NativeFunction(NativeFunction::new("__i18n_translate", None, |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("__i18n_translate expects a key string".to_string()),
            };

            let locale = if args.len() > 1 {
                match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => return Err("__i18n_translate locale must be a string".to_string()),
                }
            } else {
                env.get_locale().unwrap_or_else(|| "en".to_string())
            };

            let translations = if args.len() > 2 {
                match &args[2] {
                    Value::Hash(h) => h.borrow().clone(),
                    _ => return Err("__i18n_translate translations must be a Hash".to_string()),
                }
            } else {
                Vec::new()
            };

            // Simple translation lookup
            let locale_key = format!("{}.{}", locale, key);
            if let Some(trans) = translations.iter().find(|(k, _)| {
                if let Value::String(s) = k {
                    s == &locale_key
                } else {
                    false
                }
            }) {
                Ok(trans.1.clone())
            } else if let Some(trans) = translations.iter().find(|(k, _)| {
                if let Value::String(s) = k {
                    s == &format!("en.{}", key)
                } else {
                    false
                }
            }) {
                Ok(trans.1.clone())
            } else {
                // Fallback to key
                Ok(Value::String(key))
            }
        })),
    );

    // __i18n_plural(key, n, locale?, translations?) - Get plural form
    env.define(
        "__i18n_plural".to_string(),
        Value::NativeFunction(NativeFunction::new("__i18n_plural", None, |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("__i18n_plural expects a key string".to_string()),
            };

            let n = match &args[1] {
                Value::Int(i) => *i,
                Value::Float(f) => *f as i64,
                _ => return Err("__i18n_plural expects a number".to_string()),
            };

            let locale = if args.len() > 2 {
                match &args[2] {
                    Value::String(s) => s.clone(),
                    _ => return Err("__i18n_plural locale must be a string".to_string()),
                }
            } else {
                env.get_locale().unwrap_or_else(|| "en".to_string())
            };

            let translations = if args.len() > 3 {
                match &args[3] {
                    Value::Hash(h) => h.borrow().clone(),
                    _ => return Err("__i18n_plural translations must be a Hash".to_string()),
                }
            } else {
                Vec::new()
            };

            // Simple pluralization: use _one suffix for singular, _other for plural
            let plural_key = if n == 1 {
                format!("{}.{}_one", locale, key)
            } else {
                format!("{}.{}_other", locale, key)
            };

            if let Some(trans) = translations.iter().find(|(k, _)| {
                if let Value::String(s) = k {
                    s == &plural_key
                } else {
                    false
                }
            }) {
                Ok(trans.1.clone())
            } else {
                // Fallback to key
                Ok(Value::String(key))
            }
        })),
    );

    // __i18n_format_number(n, locale?) - Format number with locale
    env.define(
        "__i18n_format_number".to_string(),
        Value::NativeFunction(NativeFunction::new("__i18n_format_number", None, |args| {
            let n = match &args[0] {
                Value::Int(i) => *i as f64,
                Value::Float(f) => *f,
                _ => return Err("__i18n_format_number expects a number".to_string()),
            };

            let locale = if args.len() > 1 {
                match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => return Err("__i18n_format_number locale must be a string".to_string()),
                }
            } else {
                env.get_locale().unwrap_or_else(|| "en".to_string())
            };

            let formatted = match locale.as_str() {
                "fr" | "de" | "es" | "it" => {
                    // Use comma as decimal separator
                    format!("{}", n).replace('.', ",")
                }
                "en" | _ => {
                    // Use dot as decimal separator
                    format!("{}", n)
                }
            };
            Ok(Value::String(formatted))
        })),
    );

    // __i18n_format_currency(amount, currency, locale?) - Format currency
    env.define(
        "__i18n_format_currency".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__i18n_format_currency",
            None,
            |args| {
                let amount = match &args[0] {
                    Value::Int(i) => *i as f64,
                    Value::Float(f) => *f,
                    _ => return Err("__i18n_format_currency expects a number".to_string()),
                };

                let currency = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => return Err("__i18n_format_currency expects a currency code".to_string()),
                };

                let locale = if args.len() > 2 {
                    match &args[2] {
                        Value::String(s) => s.clone(),
                        _ => {
                            return Err("__i18n_format_currency locale must be a string".to_string())
                        }
                    }
                } else {
                    env.get_locale().unwrap_or_else(|| "en".to_string())
                };

                let symbol = match currency.as_str() {
                    "USD" => "$",
                    "EUR" => "€",
                    "GBP" => "£",
                    "JPY" => "¥",
                    _ => &currency,
                };

                let (decimal_sep, thousands_sep) = match locale.as_str() {
                    "fr" | "de" | "es" | "it" => (",", "."),
                    "en" | _ => (".", ","),
                };

                let int_part = amount as i64;
                let frac_part = ((amount - int_part as f64) * 100.0) as i64;
                let int_str = int_part.to_string();
                let formatted_int: String = int_str
                    .chars()
                    .rev()
                    .collect::<Vec<_>>()
                    .chunks(3)
                    .map(|chunk| chunk.iter().collect::<String>())
                    .collect::<Vec<_>>()
                    .join(thousands_sep)
                    .chars()
                    .rev()
                    .collect();

                let result = if frac_part > 0 {
                    format!("{}{}{:02}", symbol, formatted_int, frac_part)
                } else {
                    format!("{}{}", symbol, formatted_int)
                };
                Ok(Value::String(result))
            },
        )),
    );

    // __i18n_format_date(ts, locale?) - Format date with locale
    env.define(
        "__i18n_format_date".to_string(),
        Value::NativeFunction(NativeFunction::new("__i18n_format_date", None, |args| {
            let ts = match &args[0] {
                Value::Int(n) => *n,
                _ => return Err("__i18n_format_date requires a timestamp".to_string()),
            };

            let locale = if args.len() > 1 {
                match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => return Err("__i18n_format_date locale must be a string".to_string()),
                }
            } else {
                env.get_locale().unwrap_or_else(|| "en".to_string())
            };

            let dt = match chrono::DateTime::from_timestamp(ts, 0) {
                Some(d) => d,
                None => return Err("Invalid timestamp".to_string()),
            };
            let local = dt.with_timezone(&chrono::Local);

            let formatted = match locale.as_str() {
                "fr" => format!(
                    "{:02}/{:02}/{:04}",
                    local.day(),
                    local.month(),
                    local.year()
                ),
                "en" => format!(
                    "{:02}/{:02}/{:04}",
                    local.month(),
                    local.day(),
                    local.year()
                ),
                "de" => format!(
                    "{:02}.{:02}.{:04}",
                    local.day(),
                    local.month(),
                    local.year()
                ),
                _ => format!(
                    "{:04}-{:02}-{:02}",
                    local.year(),
                    local.month(),
                    local.day()
                ),
            };
            Ok(Value::String(formatted))
        })),
    );
}
