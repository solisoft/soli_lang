//! Microbenchmarks for JSON parse/stringify paths.
//!
//! Run: `cargo bench --bench json_paths`
//!
//! Includes a **legacy** `json_to_value` (Decimal auto-promote on every string)
//! so one run can compare pre/post behaviour without stashing source.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rust_decimal::Decimal;
use solilang::interpreter::value::{
    json_to_value, parse_json, stringify_to_string, value_to_json, HashKey, HashPairs, Value,
};
use std::cell::RefCell;
use std::rc::Rc;

fn sample_json() -> String {
    // String-heavy payload: many non-numeric strings exercise the old
    // Decimal-try-parse tax in json_to_value.
    let mut users = String::from("[");
    for i in 0..100 {
        if i > 0 {
            users.push(',');
        }
        users.push_str(&format!(
            r#"{{"id":{i},"name":"User {i}","email":"user{i}@example.com","status":"active","note":"hello world {i}","sku":"SKU-{i:04}"}}"#
        ));
    }
    users.push(']');
    format!(
        r#"{{"users":{users},"count":100,"meta":{{"version":"1.0","source":"bench","tag":"prod"}}}}"#
    )
}

fn sample_value(json: &str) -> Value {
    parse_json(json).expect("fixture parses")
}

/// Pre-fix conversion: try `Decimal::parse` on every JSON string.
fn legacy_json_to_value(json: serde_json::Value) -> Result<Value, String> {
    match json {
        serde_json::Value::Null => Ok(Value::Null),
        serde_json::Value::Bool(b) => Ok(Value::Bool(b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Int(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Float(f))
            } else {
                Err("Invalid JSON number".to_string())
            }
        }
        serde_json::Value::String(s) => {
            if let Ok(d) = s.parse::<Decimal>() {
                let precision = s.split('.').nth(1).map(|p| p.len() as u32).unwrap_or(0);
                Ok(Value::Decimal(solilang::interpreter::value::DecimalValue(
                    d, precision,
                )))
            } else {
                Ok(Value::String(s.into()))
            }
        }
        serde_json::Value::Array(arr) => {
            let mut items = Vec::with_capacity(arr.len());
            for v in arr {
                items.push(legacy_json_to_value(v)?);
            }
            Ok(Value::Array(Rc::new(RefCell::new(items))))
        }
        serde_json::Value::Object(obj) => {
            let mut map = HashPairs::default();
            for (k, v) in obj {
                map.insert(HashKey::String(k.into()), legacy_json_to_value(v)?);
            }
            Ok(Value::Hash(Rc::new(RefCell::new(map))))
        }
    }
}

fn bench_parse_paths(c: &mut Criterion) {
    let json = sample_json();
    let mut group = c.benchmark_group("json_parse");
    group.sample_size(80);

    group.bench_function("parse_json_direct", |b| {
        b.iter(|| parse_json(black_box(&json)).unwrap())
    });

    group.bench_function("serde_then_json_to_value_current", |b| {
        b.iter(|| {
            let v: serde_json::Value = serde_json::from_str(black_box(&json)).unwrap();
            json_to_value(v).unwrap()
        })
    });

    group.bench_function("serde_then_json_to_value_legacy_decimal", |b| {
        b.iter(|| {
            let v: serde_json::Value = serde_json::from_str(black_box(&json)).unwrap();
            legacy_json_to_value(v).unwrap()
        })
    });

    group.finish();
}

fn bench_stringify_paths(c: &mut Criterion) {
    let json = sample_json();
    let value = sample_value(&json);
    let mut group = c.benchmark_group("json_stringify");
    group.sample_size(80);

    group.bench_function("stringify_to_string_sonic", |b| {
        b.iter(|| stringify_to_string(black_box(&value)).unwrap())
    });

    group.bench_function("value_to_json_then_to_string", |b| {
        b.iter(|| {
            let j = value_to_json(black_box(&value)).unwrap();
            j.to_string()
        })
    });

    group.finish();
}

fn bench_string_heavy_json_to_value(c: &mut Criterion) {
    // Pure strings (no numbers) — maximizes Decimal auto-parse attempts on the
    // legacy path and isolates that cost from number parsing.
    let mut arr = String::from("[");
    for i in 0..500 {
        if i > 0 {
            arr.push(',');
        }
        arr.push_str(&format!(r#""field value number {i} not a decimal""#));
    }
    arr.push(']');
    let tree: serde_json::Value = serde_json::from_str(&arr).unwrap();

    let mut group = c.benchmark_group("json_to_value_strings");
    group.sample_size(80);

    group.bench_function("current_500_strings", |b| {
        b.iter(|| json_to_value(black_box(tree.clone())).unwrap())
    });

    group.bench_function("legacy_decimal_500_strings", |b| {
        b.iter(|| legacy_json_to_value(black_box(tree.clone())).unwrap())
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_parse_paths,
    bench_stringify_paths,
    bench_string_heavy_json_to_value
);
criterion_main!(benches);
