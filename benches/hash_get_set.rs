//! Microbench for `hash_get_value` / `hash_set_value`.
//!
//! Isolates the helpers (no interpreter/VM overhead) so we can see:
//! - linear scan on small maps vs hashed lookup on large ones
//! - overwrite without cloning the key vs the old `insert(HashKey::from_value)`

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use solilang::interpreter::value::{hash_get_value, hash_set_value, HashKey, HashPairs, Value};

fn populated(n: usize) -> HashPairs {
    let mut map = HashPairs::with_capacity_and_hasher(n, ahash::RandomState::default());
    for i in 0..n {
        map.insert(
            HashKey::String(format!("key_{i}").into()),
            Value::Int(i as i64),
        );
    }
    map
}

fn bench_get(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_get");
    for n in [4usize, 8, 16, 64, 256] {
        let map = populated(n);
        let hit = Value::String(format!("key_{}", n / 2).into());
        let miss = Value::String("missing".into());

        group.bench_with_input(BenchmarkId::new("hit", n), &n, |b, _| {
            b.iter(|| hash_get_value(black_box(&map), black_box(&hit)))
        });
        group.bench_with_input(BenchmarkId::new("miss", n), &n, |b, _| {
            b.iter(|| hash_get_value(black_box(&map), black_box(&miss)))
        });
    }
    group.finish();
}

fn bench_set(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_set");
    for n in [4usize, 8, 16, 64, 256] {
        let base = populated(n);
        let existing = Value::String(format!("key_{}", n / 2).into());
        let fresh = Value::String("brand_new".into());

        group.bench_with_input(BenchmarkId::new("overwrite", n), &n, |b, _| {
            b.iter_batched(
                || base.clone(),
                |mut map| {
                    hash_set_value(black_box(&mut map), black_box(&existing), Value::Int(99));
                    map
                },
                criterion::BatchSize::SmallInput,
            )
        });

        group.bench_with_input(
            BenchmarkId::new("overwrite_legacy_insert", n),
            &n,
            |b, _| {
                b.iter_batched(
                    || base.clone(),
                    |mut map| {
                        let key = HashKey::from_value(black_box(&existing)).unwrap();
                        map.insert(key, Value::Int(99));
                        map
                    },
                    criterion::BatchSize::SmallInput,
                )
            },
        );

        group.bench_with_input(BenchmarkId::new("insert_new", n), &n, |b, _| {
            b.iter_batched(
                || base.clone(),
                |mut map| {
                    hash_set_value(black_box(&mut map), black_box(&fresh), Value::Int(1));
                    map
                },
                criterion::BatchSize::SmallInput,
            )
        });
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// VM: compile once, execute each iteration (same engine as `soli serve`).
// ---------------------------------------------------------------------------

use solilang::lexer::Scanner;
use solilang::parser::Parser;
use solilang::vm::{CompiledModule, Compiler, Vm};

fn compile_vm(source: &str) -> CompiledModule {
    let tokens = Scanner::new(source).scan_tokens().expect("lexer");
    let program = Parser::new(tokens).parse().expect("parser");
    Compiler::compile(&program).expect("compile")
}

fn make_vm() -> Vm {
    let mut vm = Vm::new();
    vm.globals.insert(
        "str".to_string(),
        Value::NativeFunction(solilang::interpreter::value::NativeFunction::new(
            "str",
            Some(1),
            |args| {
                let resolved = args.iter().next().unwrap();
                Ok(Value::String(format!("{}", resolved).into()))
            },
        )),
    );
    vm
}

fn exec_vm(module: &CompiledModule) {
    let mut vm = make_vm();
    vm.execute(&module.main).expect("vm");
}

/// Build N string keys once, then GET them in a tight loop.
fn vm_get_source(n: usize, iters: usize, method: bool) -> String {
    let access = if method {
        "total = total + h.get(keys[k])"
    } else {
        "total = total + h[keys[k]]"
    };
    format!(
        r#"
let keys = [];
let h = {{}};
let i = 0;
while (i < {n}) {{
    let key = "k" + str(i);
    keys.push(key);
    h[key] = i;
    i = i + 1;
}}
let j = 0;
let total = 0;
while (j < {iters}) {{
    let k = j % {n};
    {access};
    j = j + 1;
}}
"#
    )
}

/// Build N string keys once, then OVERWRITE them in a tight loop.
fn vm_set_source(n: usize, iters: usize, method: bool) -> String {
    let access = if method {
        "h.set(keys[k], j)"
    } else {
        "h[keys[k]] = j"
    };
    format!(
        r#"
let keys = [];
let h = {{}};
let i = 0;
while (i < {n}) {{
    let key = "k" + str(i);
    keys.push(key);
    h[key] = i;
    i = i + 1;
}}
let j = 0;
while (j < {iters}) {{
    let k = j % {n};
    {access};
    j = j + 1;
}}
"#
    )
}

fn bench_vm_get_set(c: &mut Criterion) {
    const ITERS: usize = 20_000;
    let mut group = c.benchmark_group("hash_vm");
    group.sample_size(40);
    group.warm_up_time(std::time::Duration::from_secs(1));
    group.measurement_time(std::time::Duration::from_secs(3));

    for n in [4usize, 8, 16, 64, 256] {
        let get_idx = compile_vm(&vm_get_source(n, ITERS, false));
        group.bench_with_input(BenchmarkId::new("get_index", n), &n, |b, _| {
            b.iter(|| exec_vm(black_box(&get_idx)))
        });

        let get_m = compile_vm(&vm_get_source(n, ITERS, true));
        group.bench_with_input(BenchmarkId::new("get_method", n), &n, |b, _| {
            b.iter(|| exec_vm(black_box(&get_m)))
        });

        let set_idx = compile_vm(&vm_set_source(n, ITERS, false));
        group.bench_with_input(BenchmarkId::new("set_index", n), &n, |b, _| {
            b.iter(|| exec_vm(black_box(&set_idx)))
        });

        let set_m = compile_vm(&vm_set_source(n, ITERS, true));
        group.bench_with_input(BenchmarkId::new("set_method", n), &n, |b, _| {
            b.iter(|| exec_vm(black_box(&set_m)))
        });
    }

    group.finish();
}

criterion_group!(benches, bench_get, bench_set, bench_vm_get_set);
criterion_main!(benches);
