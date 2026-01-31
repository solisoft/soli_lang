//! i18n (internationalization) built-in class for Soli.
//!
//! Provides the I18n class with static methods for internationalization:
//! - locale management
//! - string translation
//! - pluralization
//! - number, currency, and date formatting

pub mod helpers;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use indexmap::IndexMap;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, NativeFunction, Value};

fn get_locale() -> String {
    helpers::get_locale()
}

fn set_locale(locale: String) {
    helpers::set_locale(&locale);
}

/// Register the I18n class in the given environment.
pub fn register_i18n_class(env: &mut Environment) {
    let mut i18n_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // I18n.locale() - Get current locale
    i18n_static_methods.insert(
        "locale".to_string(),
        Rc::new(NativeFunction::new("I18n.locale", Some(0), |_args| {
            Ok(Value::String(get_locale()))
        })),
    );

    // I18n.set_locale(locale) - Set current locale
    i18n_static_methods.insert(
        "set_locale".to_string(),
        Rc::new(NativeFunction::new(
            "I18n.set_locale",
            Some(1),
            |args| match &args[0] {
                Value::String(locale) => {
                    set_locale(locale.clone());
                    Ok(Value::String(locale.clone()))
                }
                other => Err(format!(
                    "I18n.set_locale expects a string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // I18n.translate(key, locale?, translations?) - Translate a string
    i18n_static_methods.insert(
        "translate".to_string(),
        Rc::new(NativeFunction::new("I18n.translate", None, |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("I18n.translate expects a key string".to_string()),
            };

            let locale = if args.len() > 1 {
                match &args[1] {
                    Value::String(s) => s.clone(),
                    Value::Null => get_locale(),
                    _ => return Err("I18n.translate locale must be a string or null".to_string()),
                }
            } else {
                get_locale()
            };

            let translations: IndexMap<HashKey, Value> = if args.len() > 2 {
                match &args[2] {
                    Value::Hash(h) => h.borrow().clone(),
                    _ => return Err("I18n.translate translations must be a Hash".to_string()),
                }
            } else {
                IndexMap::new()
            };

            // Simple translation lookup
            let locale_key = format!("{}.{}", locale, key);
            if let Some(trans) = translations.iter().find(|(k, _)| {
                if let HashKey::String(s) = k {
                    s == &locale_key
                } else {
                    false
                }
            }) {
                Ok(trans.1.clone())
            } else if let Some(trans) = translations.iter().find(|(k, _)| {
                if let HashKey::String(s) = k {
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

    // I18n.plural(key, n, locale?, translations?) - Get plural form
    i18n_static_methods.insert(
        "plural".to_string(),
        Rc::new(NativeFunction::new("I18n.plural", None, |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("I18n.plural expects a key string".to_string()),
            };

            let n = match &args[1] {
                Value::Int(i) => *i,
                Value::Float(f) => *f as i64,
                _ => return Err("I18n.plural expects a number".to_string()),
            };

            let locale = if args.len() > 2 {
                match &args[2] {
                    Value::String(s) => s.clone(),
                    Value::Null => get_locale(),
                    _ => return Err("I18n.plural locale must be a string or null".to_string()),
                }
            } else {
                get_locale()
            };

            let translations: IndexMap<HashKey, Value> = if args.len() > 3 {
                match &args[3] {
                    Value::Hash(h) => h.borrow().clone(),
                    _ => return Err("I18n.plural translations must be a Hash".to_string()),
                }
            } else {
                IndexMap::new()
            };

            // Simple pluralization: use _zero for 0, _one for 1, _other for plural
            let plural_key = if n == 0 {
                format!("{}.{}_zero", locale, key)
            } else if n == 1 {
                format!("{}.{}_one", locale, key)
            } else {
                format!("{}.{}_other", locale, key)
            };

            if let Some(trans) = translations.iter().find(|(k, _)| {
                if let HashKey::String(s) = k {
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

    // I18n.format_number(n, locale?) - Format number with locale
    i18n_static_methods.insert(
        "format_number".to_string(),
        Rc::new(NativeFunction::new("I18n.format_number", None, |args| {
            let n = match &args[0] {
                Value::Int(i) => *i as f64,
                Value::Float(f) => *f,
                _ => return Err("I18n.format_number expects a number".to_string()),
            };

            let locale = if args.len() > 1 {
                match &args[1] {
                    Value::String(s) => s.clone(),
                    Value::Null => get_locale(),
                    _ => {
                        return Err("I18n.format_number locale must be a string or null".to_string())
                    }
                }
            } else {
                get_locale()
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

    // I18n.format_currency(amount, currency, locale?) - Format currency
    i18n_static_methods.insert(
        "format_currency".to_string(),
        Rc::new(NativeFunction::new("I18n.format_currency", None, |args| {
            let amount = match &args[0] {
                Value::Int(i) => *i as f64,
                Value::Float(f) => *f,
                _ => return Err("I18n.format_currency expects a number".to_string()),
            };

            let currency = match &args[1] {
                Value::String(s) => s.clone(),
                _ => return Err("I18n.format_currency expects a currency code".to_string()),
            };

            let locale = if args.len() > 2 {
                match &args[2] {
                    Value::String(s) => s.clone(),
                    Value::Null => get_locale(),
                    _ => {
                        return Err(
                            "I18n.format_currency locale must be a string or null".to_string()
                        )
                    }
                }
            } else {
                get_locale()
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
            let frac_part = ((amount - int_part as f64) * 100.0).round() as i64;
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
                format!("{}{}{}{:02}", symbol, formatted_int, decimal_sep, frac_part)
            } else {
                format!("{}{}", symbol, formatted_int)
            };
            Ok(Value::String(result))
        })),
    );

    // I18n.format_date(ts, locale?) - Format date with locale
    i18n_static_methods.insert(
        "format_date".to_string(),
        Rc::new(NativeFunction::new("I18n.format_date", None, |args| {
            let ts = match &args[0] {
                Value::Int(n) => *n,
                _ => return Err("I18n.format_date requires a timestamp".to_string()),
            };

            let locale = if args.len() > 1 {
                match &args[1] {
                    Value::String(s) => s.clone(),
                    Value::Null => get_locale(),
                    _ => return Err("I18n.format_date locale must be a string or null".to_string()),
                }
            } else {
                get_locale()
            };

            let dt = match chrono::DateTime::from_timestamp(ts, 0) {
                Some(d) => d,
                None => return Err("Invalid timestamp".to_string()),
            };
            let local = dt.with_timezone(&chrono::Local);

            use chrono::Datelike;
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

    // Create the I18n class
    let i18n_class = Class {
        name: "I18n".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: i18n_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
    };

    env.define("I18n".to_string(), Value::Class(Rc::new(i18n_class)));
}
