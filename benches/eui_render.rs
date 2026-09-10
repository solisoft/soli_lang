//! The EUI encoder on a keyed list: what one render costs when nothing,
//! one row, or everything changed. The regression net for `serve::eui`.

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use serde_json::{json, Value};
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
