//! Microbenchmarks for array join and string identity/case methods.
//!
//! Run: `cargo bench --bench array_string_paths`

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use solilang::interpreter::executor::calls::array_ops::join_values;
use solilang::interpreter::executor::calls::string_methods::{
    downcase_string, identity_string, reuse_or_slice, upcase_string,
};
use solilang::interpreter::value::{SoliStr, Value};

fn sample_string_array(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| Value::String(format!("user-{i}@example.com").into()))
        .collect()
}

fn sample_int_array(n: usize) -> Vec<Value> {
    (0..n).map(|i| Value::Int(i as i64)).collect()
}

/// Legacy join: intermediate Vec of Display strings + join.
fn legacy_join(items: &[Value], delim: &str) -> String {
    let parts: Vec<String> = items.iter().map(|v| format!("{}", v)).collect();
    parts.join(delim)
}

fn bench_join(c: &mut Criterion) {
    let strings = sample_string_array(500);
    let ints = sample_int_array(500);
    let mut group = c.benchmark_group("array_join");
    group.sample_size(80);

    group.bench_function("join_values_500_strings", |b| {
        b.iter(|| join_values(black_box(&strings), ","))
    });
    group.bench_function("legacy_format_join_500_strings", |b| {
        b.iter(|| legacy_join(black_box(&strings), ","))
    });
    group.bench_function("join_values_500_ints", |b| {
        b.iter(|| join_values(black_box(&ints), ","))
    });
    group.bench_function("legacy_format_join_500_ints", |b| {
        b.iter(|| legacy_join(black_box(&ints), ","))
    });

    group.finish();
}

fn bench_string_identity(c: &mut Criterion) {
    let already_upper: SoliStr = "HELLO_WORLD_ALREADY_UPPER_CASE_STRING_XYZ".into();
    let mixed: SoliStr = "Hello_World_Mixed_Case_String_Needs_Work".into();
    let needs_trim: SoliStr = "  padded value with spaces  ".into();
    let clean: SoliStr = "no-padding-needed-here".into();

    let mut group = c.benchmark_group("string_methods");
    group.sample_size(100);

    group.bench_function("upcase_noop_ascii", |b| {
        b.iter(|| upcase_string(black_box(&already_upper)))
    });
    group.bench_function("upcase_needs_work", |b| {
        b.iter(|| upcase_string(black_box(&mixed)))
    });
    group.bench_function("upcase_legacy_to_uppercase", |b| {
        b.iter(|| Value::String(black_box(&already_upper).to_uppercase()))
    });

    group.bench_function("downcase_noop", |b| {
        b.iter(|| {
            downcase_string(black_box(&SoliStr::from(
                already_upper.to_ascii_lowercase().as_str(),
            )))
        })
    });

    group.bench_function("trim_noop_reuse", |b| {
        b.iter(|| {
            let s = black_box(&clean);
            reuse_or_slice(s, s.trim())
        })
    });
    group.bench_function("trim_needs_work", |b| {
        b.iter(|| {
            let s = black_box(&needs_trim);
            reuse_or_slice(s, s.trim())
        })
    });
    group.bench_function("trim_legacy_always_alloc", |b| {
        b.iter(|| Value::String(black_box(&clean).trim().to_string().into()))
    });

    group.bench_function("to_s_identity_clone", |b| {
        b.iter(|| identity_string(black_box(&clean)))
    });
    group.bench_function("to_s_legacy_to_string", |b| {
        b.iter(|| Value::String(black_box(&clean).to_string().into()))
    });

    group.finish();
}

fn sample_hash_pairs(n: usize) -> Vec<(solilang::interpreter::value::HashKey, Value)> {
    use solilang::interpreter::value::HashKey;
    (0..n)
        .map(|i| {
            (
                HashKey::String(format!("key-{i}").into()),
                Value::String(format!("value-{i}").into()),
            )
        })
        .collect()
}

fn legacy_hash_to_string(entries: &[(solilang::interpreter::value::HashKey, Value)]) -> String {
    let parts: Vec<String> = entries
        .iter()
        .map(|(k, v)| format!("{} => {}", k.to_value(), v))
        .collect();
    format!("{{{}}}", parts.join(", "))
}

fn bench_hash(c: &mut Criterion) {
    use solilang::interpreter::executor::calls::array_ops::{
        find_in_entries, hash_pairs_to_string,
    };
    use solilang::interpreter::value::HashKey;

    let entries = sample_hash_pairs(200);
    let needle = HashKey::String("key-150".into());
    let missing = HashKey::String("missing-key".into());

    let mut group = c.benchmark_group("hash_methods");
    group.sample_size(80);

    group.bench_function("to_string_200", |b| {
        b.iter(|| {
            hash_pairs_to_string(
                black_box(&entries).iter().map(|(k, v)| (k, v)),
                entries.len(),
            )
        })
    });
    group.bench_function("to_string_legacy_format_200", |b| {
        b.iter(|| legacy_hash_to_string(black_box(&entries)))
    });

    group.bench_function("find_in_entries_hit", |b| {
        b.iter(|| find_in_entries(black_box(&entries), black_box(&needle)))
    });
    group.bench_function("find_in_entries_miss", |b| {
        b.iter(|| find_in_entries(black_box(&entries), black_box(&missing)))
    });
    group.bench_function("legacy_rebuild_map_get", |b| {
        b.iter(|| {
            let map: solilang::interpreter::value::HashPairs =
                black_box(&entries).iter().cloned().collect();
            map.get(black_box(&needle)).cloned()
        })
    });

    group.finish();
}

fn sample_json_docs(n: usize) -> Vec<serde_json::Value> {
    (0..n)
        .map(|i| {
            serde_json::json!({
                "_key": format!("key{i}"),
                "name": format!("User {i}"),
                "email": format!("user{i}@example.com"),
                "active": true,
                "score": 42,
                "note": "hello world string payload"
            })
        })
        .collect()
}

fn bench_json_doc_convert(c: &mut Criterion) {
    use solilang::interpreter::builtins::model::crud::{json_to_value, json_to_value_owned};

    let docs = sample_json_docs(100);
    let mut group = c.benchmark_group("model_json_convert");
    group.sample_size(80);

    group.bench_function("ref_clone_100_docs", |b| {
        b.iter(|| {
            let values: Vec<_> = black_box(&docs).iter().map(json_to_value).collect();
            values
        })
    });
    group.bench_function("owned_move_100_docs", |b| {
        b.iter_batched(
            || docs.clone(),
            |owned| {
                let values: Vec<_> = owned.into_iter().map(json_to_value_owned).collect();
                values
            },
            criterion::BatchSize::SmallInput,
        )
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_join,
    bench_string_identity,
    bench_hash,
    bench_json_doc_convert
);
criterion_main!(benches);
