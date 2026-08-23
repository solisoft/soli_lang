//! Money built-in class for Soli.
//!
//! Currency-aware amounts over `rust_decimal`, stored as a plain hash so
//! the value round-trips through JSON/templates like any other data:
//!
//!   let m = Money.new("49.90", "EUR");      // Int/Float/Decimal/String in
//!   m["amount"];                            // Decimal "49.90"
//!   m["currency"];                          // "EUR"
//!
//! All operations are static and immutable — they return new money hashes
//! and never mutate their arguments. Currency mismatches are errors, not
//! silent coercions.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFunction, Value};

/// ISO-4217 minor-unit exponents for the currencies apps actually bill in.
/// Anything not listed defaults to 2 decimal places.
fn currency_exponent(code: &str) -> u32 {
    match code {
        "JPY" | "KRW" | "VND" | "CLP" | "ISK" => 0,
        "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
        _ => 2,
    }
}

fn currency_symbol(code: &str) -> Option<&'static str> {
    Some(match code {
        "USD" => "$",
        "EUR" => "€",
        "GBP" => "£",
        "JPY" => "¥",
        "CHF" => "CHF",
        "CAD" => "CA$",
        "AUD" => "A$",
        "NZD" => "NZ$",
        "CNY" => "¥",
        "INR" => "₹",
        "SEK" | "NOK" | "DKK" => "kr",
        "PLN" => "zł",
        "BRL" => "R$",
        "ZAR" => "R",
        _ => return None,
    })
}

fn amount_of(v: &Value, ctx: &str) -> Result<Decimal, String> {
    match v {
        Value::Int(n) => Ok(Decimal::from(*n)),
        Value::Float(f) => Decimal::try_from(*f)
            .map_err(|_| format!("{ctx}: cannot represent {f} exactly as a decimal")),
        Value::Decimal(d) => Ok(d.0),
        Value::String(s) => {
            let cleaned = s.trim().replace('_', "");
            if cleaned.contains(',') {
                return Err(format!(
                    "{ctx}: ambiguous \",\" in \"{s}\" — use \".\" for decimals and \"_\" for grouping"
                ));
            }
            cleaned
                .parse::<Decimal>()
                .map_err(|_| format!("{ctx}: cannot parse \"{s}\" as an amount"))
        }
        other => Err(format!(
            "{ctx}: amount expects Int/Float/Decimal/string, got {}",
            other.type_name()
        )),
    }
}

fn is_money_hash(v: &Value) -> bool {
    matches!(v, Value::Hash(h) if h.borrow().get(&HashKey::String("_money".into())) == Some(&Value::Bool(true)))
}

fn parts_of(v: &Value, ctx: &str) -> Result<(Decimal, String), String> {
    if !is_money_hash(v) {
        return Err(format!("{ctx}: expected a Money.new() hash"));
    }
    let Value::Hash(h) = v else { unreachable!() };
    let h = h.borrow();
    let amount = h
        .get(&HashKey::String("amount".into()))
        .ok_or_else(|| format!("{ctx}: money hash missing \"amount\""))?;
    let currency = h
        .get(&HashKey::String("currency".into()))
        .ok_or_else(|| format!("{ctx}: money hash missing \"currency\""))?;
    let Value::String(cur) = currency else {
        return Err(format!("{ctx}: money hash \"currency\" must be a string"));
    };
    // Normalize here too, not just in `Money.new`. Money is documented as a
    // plain hash that round-trips through JSON and the database, so a code can
    // arrive from a payload or a record without ever passing through the
    // constructor — and a lowercase "jpy" then took the wrong minor-unit
    // exponent (2 instead of 0) and missed its symbol.
    Ok((amount_of(amount, ctx)?, normalize_currency(cur, ctx)?))
}

fn make_money(amount: Decimal, currency: &str) -> Value {
    let exp = currency_exponent(currency);
    // Quantize to the currency's minor units.
    //
    // The exponent alone only reached `DecimalValue`'s precision slot, which
    // `Display` ignores — so `Money.mul(19.99 EUR, 0.2)` stored 3.998 while
    // `Money.format` rendered "4.00 €", and `amount` / `add` / `compare` kept
    // using the sub-cent residue. Rounding here means every money value is
    // exactly what it displays as, so no chain of operations can drift away
    // from it and no separate `Money.round` is needed.
    //
    // Half-away-from-zero, the ordinary money convention, matching
    // `float_methods`' rounding rather than `Decimal`'s banker's default.
    let amount =
        amount.round_dp_with_strategy(exp, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    let mut h = HashPairs::default();
    h.insert(HashKey::String("_money".into()), Value::Bool(true));
    h.insert(
        HashKey::String("amount".into()),
        Value::Decimal(crate::interpreter::value::DecimalValue(amount, exp)),
    );
    h.insert(
        HashKey::String("currency".into()),
        Value::String(currency.into()),
    );
    Value::Hash(Rc::new(RefCell::new(h)))
}

/// Uppercase a currency code and require exactly three ASCII letters.
///
/// The old check was `s.len() == 3` on *bytes*, so "€" (three UTF-8 bytes) was
/// accepted as a currency, and lowercase passed through unchanged — which made
/// `Money.add(Money.new(5, "eur"), Money.new(5, "EUR"))` a currency mismatch
/// and `currency_exponent("jpy")` return 2 instead of 0.
fn normalize_currency(code: &str, ctx: &str) -> Result<String, String> {
    let trimmed = code.trim();
    if trimmed.len() == 3 && trimmed.chars().all(|c| c.is_ascii_alphabetic()) {
        return Ok(trimmed.to_ascii_uppercase());
    }
    Err(format!(
        "{ctx}: currency expects a 3-letter ISO code (e.g. \"EUR\"), got \"{code}\""
    ))
}

fn same_currency(a: &str, b: &str, ctx: &str) -> Result<(), String> {
    if a != b {
        return Err(format!("{ctx}: currency mismatch ({a} vs {b})"));
    }
    Ok(())
}

pub fn register_money_class(env: &mut Environment) {
    let mut m: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Money.new(amount, currency) -> money hash
    m.insert(
        "new".to_string(),
        Rc::new(NativeFunction::new("Money.new", Some(2), |args| {
            let amount = amount_of(&args[0], "Money.new()")?;
            let currency = match &args[1] {
                Value::String(s) => normalize_currency(s, "Money.new()")?,
                other => {
                    return Err(format!(
                        "Money.new(): currency expects a string, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(make_money(amount, &currency))
        })),
    );

    // Money.add(a, b) / Money.sub(a, b)
    for (fname, op) in [("add", 0), ("sub", 1)] {
        let full = format!("Money.{fname}");
        let err_ctx = full.clone();
        m.insert(
            fname.to_string(),
            Rc::new(NativeFunction::new(&full, Some(2), move |args| {
                let (a, ca) = parts_of(&args[0], &err_ctx)?;
                let (b, cb) = parts_of(&args[1], &err_ctx)?;
                same_currency(&ca, &cb, &err_ctx)?;
                Ok(make_money(if op == 0 { a + b } else { a - b }, &ca))
            })),
        );
    }

    // Money.mul(m, factor_int) — scaling by a non-money scalar only.
    m.insert(
        "mul".to_string(),
        Rc::new(NativeFunction::new("Money.mul", Some(2), |args| {
            let (a, c) = parts_of(&args[0], "Money.mul()")?;
            let f = match &args[1] {
                Value::Int(n) => Decimal::from(*n),
                Value::Float(x) => Decimal::try_from(*x).map_err(|_| {
                    "Money.mul(): factor cannot be represented as a decimal".to_string()
                })?,
                other => {
                    return Err(format!(
                        "Money.mul() factor expects Int/Float, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(make_money(a * f, &c))
        })),
    );

    // Money.compare(a, b) -> -1 | 0 | 1
    m.insert(
        "compare".to_string(),
        Rc::new(NativeFunction::new("Money.compare", Some(2), |args| {
            let (a, ca) = parts_of(&args[0], "Money.compare()")?;
            let (b, cb) = parts_of(&args[1], "Money.compare()")?;
            same_currency(&ca, &cb, "Money.compare()")?;
            Ok(Value::Int(match a.cmp(&b) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            }))
        })),
    );

    // Money.allocate(m, [50, 25, 25]) -> array of money hashes that sum to
    // the original with no lost cents (largest-remainder method).
    m.insert(
        "allocate".to_string(),
        Rc::new(NativeFunction::new("Money.allocate", Some(2), |args| {
            let (total, c) = parts_of(&args[0], "Money.allocate()")?;
            let ratios = match &args[1] {
                Value::Array(a) => a.borrow().clone(),
                other => {
                    return Err(format!(
                        "Money.allocate() expects an array of ratios, got {}",
                        other.type_name()
                    ))
                }
            };
            if ratios.is_empty() {
                return Err("Money.allocate(): ratio array is empty".to_string());
            }
            let mut weights: Vec<i64> = Vec::with_capacity(ratios.len());
            for r in &ratios {
                match r {
                    Value::Int(n) if *n >= 0 => weights.push(*n),
                    _ => return Err("Money.allocate() ratios expect non-negative Ints".to_string()),
                }
            }
            let sum: i64 = weights.iter().sum();
            if sum <= 0 {
                return Err("Money.allocate(): ratio sum must be positive".to_string());
            }
            // Work in the currency's minor units so allocation is exact.
            let exp = currency_exponent(&c);
            let scale = Decimal::from(10u64.pow(exp));
            let total_minor = (total * scale).round();
            let mut out: Vec<Value> = Vec::with_capacity(weights.len());
            let mut distributed: i64 = 0;
            // Keep the remainder as a Decimal. `rem` is a fraction in [0, 1),
            // so the old `rem.to_i64()` truncated every one of them to 0: the
            // sort key was constant and the index tie-break handed the leftover
            // units to the lowest indices. `allocate(100 EUR, [1, 2])` returned
            // 33.34 + 66.66 instead of 33.33 + 66.67, and a zero-ratio
            // participant could be handed a unit.
            let mut exact: Vec<(i64, Decimal)> = Vec::with_capacity(weights.len());
            for w in &weights {
                let share = total_minor * Decimal::from(*w) / Decimal::from(sum);
                let floor = share.floor();
                let rem = share - floor;
                exact.push((floor.to_i64().unwrap_or_default(), rem));
                distributed += floor.to_i64().unwrap_or_default();
            }
            // Hand the remaining minor units to the largest remainders.
            let leftover = total_minor - Decimal::from(distributed);
            let mut order: Vec<usize> = (0..exact.len()).collect();
            order.sort_by(|a, b| exact[*b].1.cmp(&exact[*a].1).then(a.cmp(b)));
            let extra: std::collections::HashSet<usize> = order
                .into_iter()
                .take(leftover.to_i64().unwrap_or_default() as usize)
                .collect();
            for (i, (base, _)) in exact.iter().enumerate() {
                let units = base + if extra.contains(&i) { 1 } else { 0 };
                out.push(make_money(Decimal::from(units) / scale, &c));
            }
            Ok(Value::Array(Rc::new(RefCell::new(out))))
        })),
    );

    // Money.format(m, opts?) -> localized string.
    // opts: {"symbol": true|false} (default true), {"locale": "de"|"en"|"fr"}
    // choosing decimal separator and thousands grouping; defaults to "en".
    m.insert(
        "format".to_string(),
        // Arity 1 or 2 (optional opts hash); validated below.
        Rc::new(NativeFunction::new("Money.format", None, |args| {
            if args.is_empty() || args.len() > 2 {
                return Err("Money.format() expects (money, opts?)".to_string());
            }
            let (amount, c) = parts_of(&args[0], "Money.format()")?;
            let (mut want_symbol, locale): (bool, String) = match args.get(1) {
                None | Some(Value::Null) => (true, "en".to_string()),
                Some(Value::Hash(h)) => {
                    let h = h.borrow();
                    let sym = !matches!(
                        h.get(&HashKey::String("symbol".into())),
                        Some(Value::Bool(false))
                    );
                    let loc = match h.get(&HashKey::String("locale".into())) {
                        Some(Value::String(l)) => l.as_str().to_string(),
                        _ => "en".to_string(),
                    };
                    (sym, loc)
                }
                Some(other) => {
                    return Err(format!(
                        "Money.format() opts expects a hash, got {}",
                        other.type_name()
                    ))
                }
            };
            let symbol = currency_symbol(&c);
            if symbol.is_none() {
                want_symbol = false;
            }
            let exp = currency_exponent(&c);
            let comma_decimal = matches!(locale.as_str(), "de" | "fr" | "nl");
            let neg = amount.is_sign_negative();
            let scaled = amount.abs() * Decimal::from(10u64.pow(exp));
            let units = scaled.round().to_u64().unwrap_or_default();
            let divisor = 10u64.pow(exp);
            let major = units / divisor;
            let minor = units % divisor;
            let major_str = {
                let digits = major.to_string();
                let group = if comma_decimal { '.' } else { ',' };
                let mut grouped = String::new();
                let bytes = digits.as_bytes();
                for (i, ch) in bytes.iter().enumerate() {
                    if i > 0 && (bytes.len() - i).is_multiple_of(3) {
                        grouped.push(group);
                    }
                    grouped.push(*ch as char);
                }
                grouped
            };
            // Same locale set as the grouping char above: a locale that groups
            // with '.' must not also use '.' as the decimal separator, or
            // `Money.format(1234.5 EUR, {"locale": "fr"})` renders the
            // ambiguous "1.234.50 €".
            let decimal_sep = if comma_decimal { ',' } else { '.' };
            let minor_str = if exp == 0 {
                String::new()
            } else {
                format!("{}{:0width$}", decimal_sep, minor, width = exp as usize)
            };
            let body = format!("{major_str}{minor_str}");
            let sign = if neg { "-" } else { "" };
            let out = match (want_symbol, symbol) {
                (true, Some(sym)) => {
                    if locale == "fr" {
                        format!("{sign}{body} {sym}")
                    } else if c == "EUR" {
                        format!("{sign}{body} €")
                    } else {
                        format!("{sign}{sym}{body}")
                    }
                }
                _ => format!("{sign}{body} {c}"),
            };
            Ok(Value::String(out.into()))
        })),
    );

    let class = Class {
        name: "Money".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: m,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define("Money".to_string(), Value::Class(Rc::new(class)));
}

// ---------- tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    fn money(amount: &str, currency: &str) -> Value {
        make_money(amount.parse().unwrap(), currency)
    }

    #[test]
    fn new_accepts_scalar_shapes_and_normalizes() {
        assert!(matches!(
            amount_of(&Value::Int(5), "t"),
            Ok(d) if d == Decimal::from(5)
        ));
        // "." is the decimal separator; "," is rejected as ambiguous;
        // "_" is accepted as a grouping separator.
        assert_eq!(
            amount_of(&Value::String("1_234.56".into()), "t").unwrap(),
            Decimal::from_str_exact("1234.56").unwrap()
        );
        assert!(amount_of(&Value::String("49,90".into()), "t").is_err());
    }

    #[test]
    fn add_requires_same_currency() {
        let eur = money("10.00", "EUR");
        let usd = money("10.00", "USD");
        let r = parts_of(&eur, "t").and_then(|(a, ca)| {
            let (b, cb) = parts_of(&usd, "t")?;
            same_currency(&ca, &cb, "t")?;
            Ok(make_money(a + b, &ca))
        });
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("currency mismatch"));
    }

    #[test]
    fn allocate_splits_without_losing_cents() {
        // Classic case: 100.00 split three equal ways → 33.34 / 33.33 / 33.33.
        let total = money("100.00", "EUR");
        let (t, c) = parts_of(&total, "t").unwrap();
        let exp = currency_exponent(&c);
        let scale = Decimal::from(10u64.pow(exp));
        let total_minor = (t * scale).round();
        let weights = vec![1i64, 1, 1];
        let sum: i64 = weights.iter().sum();
        let mut floors = Vec::new();
        let mut rems = Vec::new();
        let mut distributed = 0i64;
        for w in &weights {
            let share = total_minor * Decimal::from(*w) / Decimal::from(sum);
            let fl = share.floor();
            // Remainder in cents (0..100 scale) so it is comparable as ints.
            rems.push(
                ((share - fl) * Decimal::from(100))
                    .round()
                    .to_i64()
                    .unwrap_or_default(),
            );
            distributed += fl.to_i64().unwrap_or_default();
            floors.push(fl);
        }
        let leftover = total_minor - Decimal::from(distributed);
        assert_eq!(leftover, Decimal::ONE);
        // Largest-remainder: exactly `leftover` shares get one extra cent,
        // highest remainder first; all shares then sum back to the total.
        let leftover_units = leftover.to_i64().unwrap_or_default();
        let mut order: Vec<usize> = (0..rems.len()).collect();
        order.sort_by(|a, b| rems[*b].cmp(&rems[*a]));
        let extras: std::collections::HashSet<usize> =
            order.into_iter().take(leftover_units as usize).collect();
        let allocated_sum: Decimal = floors
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let extra = if extras.contains(&i) {
                    Decimal::ONE
                } else {
                    Decimal::ZERO
                };
                (*f + extra) / scale
            })
            .sum();
        assert_eq!(allocated_sum, t);
        assert_eq!(leftover_units, 1);
    }

    #[test]
    fn formats_de_and_en_locales() {
        let m = money("1234567.89", "EUR");
        // Direct formatting math, mirroring the native body:
        let (amt, cur) = parts_of(&m, "t").unwrap();
        assert_eq!(cur, "EUR");
        let units = (amt.abs() * Decimal::from(100)).round().to_u64().unwrap();
        assert_eq!(units, 123_456_789);
        let major = units / 100;
        assert_eq!(major, 1_234_567);
        let minor = units % 100;
        assert_eq!(minor, 89);
    }

    #[test]
    fn zero_decimal_currencies_have_no_minor_part() {
        assert_eq!(currency_exponent("JPY"), 0);
        assert_eq!(currency_exponent("KWD"), 3);
        assert_eq!(currency_exponent("EUR"), 2);
        assert_eq!(currency_symbol("JPY"), Some("¥"));
        assert_eq!(currency_symbol("XXX"), None);
    }
}
