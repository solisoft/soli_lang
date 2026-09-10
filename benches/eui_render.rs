//! The EUI encoder on a keyed list: what one render costs when nothing,
//! one row, or everything changed. The regression net for `serve::eui`.

use std::cell::RefCell;
use std::rc::Rc;

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use serde_json::{json, Value};
use solilang::interpreter::value::{HashKey, HashPairs, Value as SoliValue};
use solilang::serve::eui::tree::Encoder;

const ROWS: usize = 10_000;

/// A list of `n` keyed rows, each a box with a text and a click handler —
/// the shape of a feed, a table, an inbox.
fn list(n: usize, changed: Option<usize>) -> Value {
    let rows: Vec<Value> = (0..n)
        .map(|i| {
            let text = if changed == Some(i) {
                format!("row {i} (edited)")
            } else {
                format!("row {i}")
            };
            json!({
                "k": "box",
                "key": format!("r{i}"),
                "s": {"display": "row", "gap": 2},
                "p": {"id": i},
                "on": {"click": "pick"},
                "c": [{"k": "text", "t": text}]
            })
        })
        .collect();
    json!({"k": "scroll", "c": [{"k": "box", "c": rows}]})
}

fn reversed(n: usize) -> Value {
    let mut v = list(n, None);
    if let Some(rows) = v["c"][0]["c"].as_array_mut() {
        rows.reverse();
    }
    v
}

fn sv(x: &str) -> SoliValue {
    SoliValue::String(x.into())
}

fn sh(pairs: Vec<(&str, SoliValue)>) -> SoliValue {
    let mut map = HashPairs::default();
    for (k, v) in pairs {
        map.insert(HashKey::String(k.into()), v);
    }
    SoliValue::Hash(Rc::new(RefCell::new(map)))
}

fn sl(items: Vec<SoliValue>) -> SoliValue {
    SoliValue::Array(Rc::new(RefCell::new(items)))
}

/// The same list as interpreter values — what the server actually renders
/// from. Every call builds fresh objects, as a view that rebuilds its rows
/// each render does.
fn value_list(n: usize) -> SoliValue {
    let rows: Vec<SoliValue> = (0..n)
        .map(|i| {
            sh(vec![
                ("k", sv("box")),
                ("key", sv(&format!("r{i}"))),
                (
                    "s",
                    sh(vec![("display", sv("row")), ("gap", SoliValue::Int(2))]),
                ),
                ("p", sh(vec![("id", SoliValue::Int(i as i64))])),
                ("on", sh(vec![("click", sv("pick"))])),
                (
                    "c",
                    sl(vec![sh(vec![
                        ("k", sv("text")),
                        ("t", sv(&format!("row {i}"))),
                    ])]),
                ),
            ])
        })
        .collect();
    sh(vec![
        ("k", sv("scroll")),
        ("c", sl(vec![sh(vec![("k", sv("box")), ("c", sl(rows))])])),
    ])
}

/// An encoder that has already sent the base list once, as values.
fn warm_values() -> Encoder {
    let mut enc = Encoder::default();
    enc.render_value("bench", &value_list(ROWS), false)
        .expect("base render");
    enc
}

/// An encoder that has already sent the base list once.
fn warm() -> Encoder {
    let mut enc = Encoder::default();
    enc.render(&list(ROWS, None), false).expect("base render");
    enc
}

fn bench(c: &mut Criterion) {
    let base = list(ROWS, None);
    let one_changed = list(ROWS, Some(ROWS / 2));
    let rev = reversed(ROWS);
    let empty = json!({"k": "scroll", "c": [{"k": "box", "c": []}]});

    let mut group = c.benchmark_group("eui_render_10k");
    group.sample_size(20);

    group.bench_function("mount", |b| {
        b.iter_batched(
            Encoder::default,
            |mut enc| black_box(enc.render(&base, false).unwrap()),
            BatchSize::LargeInput,
        )
    });
    group.bench_function("unchanged", |b| {
        b.iter_batched(
            warm,
            |mut enc| black_box(enc.render(&base, false).unwrap()),
            BatchSize::LargeInput,
        )
    });
    group.bench_function("one_row_changed", |b| {
        b.iter_batched(
            warm,
            |mut enc| black_box(enc.render(&one_changed, false).unwrap()),
            BatchSize::LargeInput,
        )
    });
    group.bench_function("reversed", |b| {
        b.iter_batched(
            warm,
            |mut enc| black_box(enc.render(&rev, false).unwrap()),
            BatchSize::LargeInput,
        )
    });
    group.bench_function("cleared", |b| {
        b.iter_batched(
            warm,
            |mut enc| black_box(enc.render(&empty, false).unwrap()),
            BatchSize::LargeInput,
        )
    });
    // The value path: fresh row objects each render (every row converted,
    // styles by fingerprint), and the same row objects (every row kept).
    group.bench_function("values_rebuilt", |b| {
        b.iter_batched(
            || (warm_values(), value_list(ROWS)),
            |(mut enc, tree)| black_box(enc.render_value("bench", &tree, false).unwrap()),
            BatchSize::LargeInput,
        )
    });
    group.bench_function("values_kept", |b| {
        let tree = value_list(ROWS);
        b.iter_batched(
            || {
                let mut enc = Encoder::default();
                enc.render_value("bench", &tree, false).unwrap();
                enc
            },
            |mut enc| black_box(enc.render_value("bench", &tree, false).unwrap()),
            BatchSize::LargeInput,
        )
    });
    group.bench_function("resync", |b| {
        b.iter_batched(
            warm,
            |mut enc| black_box(enc.render(&base, true).unwrap()),
            BatchSize::LargeInput,
        )
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
