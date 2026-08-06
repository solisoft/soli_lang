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
        b.iter(|| Value::String(black_box(&already_upper).to_uppercase().into()))
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

criterion_group!(benches, bench_join, bench_string_identity);
criterion_main!(benches);
