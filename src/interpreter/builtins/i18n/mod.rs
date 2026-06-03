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

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFunction, Value};

fn get_locale() -> String {
    helpers::get_locale()
}

fn set_locale(locale: String) {
    helpers::set_locale(&locale);
}

/// Convert a values hash into a flat `[(name, displayed_value)]` slice for
/// `helpers::interpolate`. Non-string keys are skipped.
fn values_to_strings(hash: &HashPairs) -> Vec<(String, String)> {
    hash.iter()
        .filter_map(|(k, v)| match k {
            HashKey::String(s) => Some((s.to_string(), format!("{}", v))),
            _ => None,
        })
        .collect()
}

/// Heuristic: does this hash look like a legacy flat translations table
/// (every string key contains a dot, e.g. `"en.greeting"`)? Used to
/// disambiguate the back-compat 3rd-arg path from interpolation values.
fn has_dotted_locale_keys(hash: &HashPairs) -> bool {
    if hash.is_empty() {
        return false;
    }
    hash.iter().all(|(k, _)| match k {
        HashKey::String(s) => s.contains('.'),
        _ => false,
    })
}

/// Look up a key in a legacy flat translations hash (`{"en.greeting": "Hi"}`).
/// Tries `<locale>.<key>` then `en.<key>`.
fn legacy_lookup(translations: &HashPairs, locale: &str, key: &str) -> Option<String> {
    let primary = format!("{}.{}", locale, key);
    if let Some(v) = find_string(translations, &primary) {
        return Some(v);
    }
    if locale != "en" {
        let fallback = format!("en.{}", key);
        return find_string(translations, &fallback);
    }
    None
}

fn legacy_lookup_plural(
    translations: &HashPairs,
    locale: &str,
    key: &str,
    n: i64,
) -> Option<String> {
    let suffix = if n == 0 {
        "_zero"
    } else if n == 1 {
        "_one"
    } else {
        "_other"
    };
    let primary = format!("{}.{}{}", locale, key, suffix);
    if let Some(v) = find_string(translations, &primary) {
        return Some(v);
    }
    if locale != "en" {
        let fallback = format!("en.{}{}", key, suffix);
        return find_string(translations, &fallback);
    }
    None
}

fn find_string(hash: &HashPairs, key: &str) -> Option<String> {
    hash.iter()
        .find_map(|(k, v)| match (k, v) {
            (HashKey::String(s), Value::String(out)) if **s == *key => Some(out.clone()),
            _ => None,
        })
        .map(|s| s.to_string())
}

/// Register the I18n class in the given environment.
pub fn register_i18n_class(env: &mut Environment) {
    let mut i18n_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // I18n.locale() - Get current locale
    i18n_static_methods.insert(
        "locale".to_string(),
        Rc::new(NativeFunction::new("I18n.locale", Some(0), |_args| {
            Ok(Value::String(get_locale().into()))
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
                    set_locale(locale.clone().to_string());
                    Ok(Value::String(locale.clone()))
                }
                other => Err(format!(
                    "I18n.set_locale expects a string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // I18n.translate(key, locale_or_values?, values?) - Translate a string.
    //
    // - 2nd arg String   → locale; current locale otherwise
    // - 2nd or 3rd Hash  → interpolation values (placeholders `{name}` in
    //                      translations are replaced by Display-stringified
    //                      values; unknown placeholders are left as-is)
    // - Legacy back-compat: a 3rd-arg Hash whose keys are all dotted (e.g.
    //   `"en.greeting"`) is consulted as a flat translations table when the
    //   auto-loaded `config/locales/*.yml` store yields no hit.
    i18n_static_methods.insert(
        "translate".to_string(),
        Rc::new(NativeFunction::new("I18n.translate", None, |args| {
            if args.is_empty() {
                return Err("I18n.translate expects a key string".to_string());
            }
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("I18n.translate expects a key string".to_string()),
            };

            let mut locale = get_locale();
            let mut values: Option<HashPairs> = None;
            let mut legacy: Option<HashPairs> = None;

            if args.len() > 1 {
                match &args[1] {
                    Value::String(s) => locale = s.clone().to_string(),
                    Value::Null => {}
                    Value::Hash(h) => values = Some(h.borrow().clone()),
                    other => {
                        return Err(format!(
                        "I18n.translate second arg must be a locale string or values hash, got {}",
                        other.type_name()
                    ))
                    }
                }
            }
            if args.len() > 2 {
                match &args[2] {
                    Value::Null => {}
                    Value::Hash(h) => {
                        let hp = h.borrow().clone();
                        if has_dotted_locale_keys(&hp) {
                            legacy = Some(hp);
                        } else {
                            values = Some(hp);
                        }
                    }
                    other => {
                        return Err(format!(
                            "I18n.translate third arg must be a hash, got {}",
                            other.type_name()
                        ))
                    }
                }
            }

            let raw = helpers::lookup_translation(&locale, &key)
                .or_else(|| {
                    legacy
                        .as_ref()
                        .and_then(|t| legacy_lookup(t, &locale, &key))
                })
                .unwrap_or_else(|| key.clone().to_string());

            let interp = values.as_ref().map(values_to_strings).unwrap_or_default();
            Ok(Value::String(helpers::interpolate(&raw, &interp).into()))
        })),
    );

    // I18n.plural(key, n, locale_or_values?, values?) - Pluralized translate.
    //
    // Resolves `<key>_zero` (n==0), `<key>_one` (n==1), or `<key>_other`
    // under the active locale tree. `count` is auto-injected into the
    // interpolation values if not explicitly provided, so messages can read
    // `"You have {count} items"`.
    i18n_static_methods.insert(
        "plural".to_string(),
        Rc::new(NativeFunction::new("I18n.plural", None, |args| {
            if args.len() < 2 {
                return Err("I18n.plural expects (key, n, ...)".to_string());
            }
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err("I18n.plural expects a key string".to_string()),
            };
            let n = match &args[1] {
                Value::Int(i) => *i,
                Value::Float(f) => *f as i64,
                _ => return Err("I18n.plural expects a number".to_string()),
            };

            let mut locale = get_locale();
            let mut values: Option<HashPairs> = None;
            let mut legacy: Option<HashPairs> = None;

            if args.len() > 2 {
                match &args[2] {
                    Value::String(s) => locale = s.clone().to_string(),
                    Value::Null => {}
                    Value::Hash(h) => values = Some(h.borrow().clone()),
                    other => {
                        return Err(format!(
                            "I18n.plural third arg must be a locale string or values hash, got {}",
                            other.type_name()
                        ))
                    }
                }
            }
            if args.len() > 3 {
                match &args[3] {
                    Value::Null => {}
                    Value::Hash(h) => {
                        let hp = h.borrow().clone();
                        if has_dotted_locale_keys(&hp) {
                            legacy = Some(hp);
                        } else {
                            values = Some(hp);
                        }
                    }
                    other => {
                        return Err(format!(
                            "I18n.plural fourth arg must be a hash, got {}",
                            other.type_name()
                        ))
                    }
                }
            }

            let raw = helpers::lookup_plural(&locale, &key, n)
                .or_else(|| {
                    legacy
                        .as_ref()
                        .and_then(|t| legacy_lookup_plural(t, &locale, &key, n))
                })
                .unwrap_or_else(|| key.clone().to_string());

            let mut interp = values.as_ref().map(values_to_strings).unwrap_or_default();
            if !interp.iter().any(|(k, _)| k == "count") {
                interp.push(("count".to_string(), n.to_string()));
            }
            Ok(Value::String(helpers::interpolate(&raw, &interp).into()))
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
                    Value::Null => get_locale().into(),
                    _ => {
                        return Err("I18n.format_number locale must be a string or null".to_string())
                    }
                }
            } else {
                get_locale().into()
            };

            let formatted = match locale.as_ref() {
                "fr" | "de" | "es" | "it" => {
                    // Use comma as decimal separator
                    format!("{}", n).replace('.', ",")
                }
                _ => {
                    // Use dot as decimal separator (default for "en" and others)
                    format!("{}", n)
                }
            };
            Ok(Value::String(formatted.into()))
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
                    Value::Null => get_locale().into(),
                    _ => {
                        return Err(
                            "I18n.format_currency locale must be a string or null".to_string()
                        )
                    }
                }
            } else {
                get_locale().into()
            };

            let symbol = match currency.as_ref() {
                "USD" => "$",
                "EUR" => "€",
                "GBP" => "£",
                "JPY" => "¥",
                _ => &currency,
            };

            let (decimal_sep, thousands_sep, symbol_after) = match locale.as_ref() {
                "fr" | "de" | "es" | "it" => (",", ".", true),
                _ => (".", ",", false), // default for "en" and others
            };

            // Round to total cents first so the integer and fractional parts
            // can't desync. Computing them independently caused a carry bug:
            // amount = 9.995 produced int_part=9 and frac_part=100 (rounded
            // up) → "9,100 €" instead of "10,00 €".
            let cents = (amount * 100.0).round() as i64;
            let int_part = cents / 100;
            let frac_part = (cents % 100).abs();
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

            let number = if frac_part > 0 {
                format!("{}{}{:02}", formatted_int, decimal_sep, frac_part)
            } else {
                formatted_int
            };
            let result = if symbol_after {
                format!("{} {}", number, symbol)
            } else {
                format!("{}{}", symbol, number)
            };
            Ok(Value::String(result.into()))
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
                    Value::Null => get_locale().into(),
                    _ => return Err("I18n.format_date locale must be a string or null".to_string()),
                }
            } else {
                get_locale().into()
            };

            let dt = match chrono::DateTime::from_timestamp(ts, 0) {
                Some(d) => d,
                None => return Err("Invalid timestamp".to_string()),
            };
            let local = dt.with_timezone(&chrono::Local);

            use chrono::Datelike;
            let formatted = match locale.as_ref() {
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
            Ok(Value::String(formatted.into()))
        })),
    );

    // Create the I18n class
    let i18n_class = Class {
        name: "I18n".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: i18n_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    env.define("I18n".to_string(), Value::Class(Rc::new(i18n_class)));
}
