//! Benchmarks comparing tree-walking interpreter vs bytecode VM.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use solilang::interpreter::Interpreter;
use solilang::lexer::Scanner;
use solilang::parser::Parser;
use solilang::vm::{Compiler, Vm};
use std::fs;

/// Parse source into an AST.
fn parse(source: &str) -> solilang::ast::Program {
    let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
    Parser::new(tokens).parse().expect("parser error")
}

/// Run via tree-walking interpreter.
fn run_treewalk(source: &str) {
    let program = parse(source);
    let mut interpreter = Interpreter::new();
    interpreter.interpret(&program).expect("runtime error");
}

/// Run via bytecode VM (compile + execute).
fn run_vm(source: &str) {
    let program = parse(source);
    let module = Compiler::compile(&program).expect("compile error");
    let mut vm = Vm::new();
    // Register print as a native function so programs that call it don't fail
    vm.globals.insert(
        "print".to_string(),
        solilang::interpreter::value::Value::NativeFunction(
            solilang::interpreter::value::NativeFunction::new("print", None, |_args| {
                Ok(solilang::interpreter::value::Value::Null)
            }),
        ),
    );
    vm.globals.insert(
        "puts".to_string(),
        solilang::interpreter::value::Value::NativeFunction(
            solilang::interpreter::value::NativeFunction::new("puts", None, |_args| {
                Ok(solilang::interpreter::value::Value::Null)
            }),
        ),
    );
    vm.globals.insert(
        "clock".to_string(),
        solilang::interpreter::value::Value::NativeFunction(
            solilang::interpreter::value::NativeFunction::new("clock", Some(0), |_args| {
                use std::time::{SystemTime, UNIX_EPOCH};
                let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                Ok(solilang::interpreter::value::Value::Float(
                    duration.as_secs_f64(),
                ))
            }),
        ),
    );
    vm.globals.insert(
        "str".to_string(),
        solilang::interpreter::value::Value::NativeFunction(
            solilang::interpreter::value::NativeFunction::new("str", Some(1), |args| {
                let resolved = args.into_iter().next().unwrap();
                Ok(solilang::interpreter::value::Value::String(format!(
                    "{}",
                    resolved
                )))
            }),
        ),
    );
    vm.globals.insert(
        "len".to_string(),
        solilang::interpreter::value::Value::NativeFunction(
            solilang::interpreter::value::NativeFunction::new("len", Some(1), |args| {
                let resolved = args.into_iter().next().unwrap();
                Ok(solilang::interpreter::value::Value::Int(
                    resolved.display_len() as i64,
                ))
            }),
        ),
    );
    vm.execute(&module.main).expect("vm runtime error");
}

fn load_program(name: &str) -> String {
    let path = format!("benches/programs/{}.sl", name);
    fs::read_to_string(&path).unwrap_or_else(|_| panic!("failed to read {}", path))
}

fn fibonacci_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("fib_recursive_comparison");
    let source = load_program("fib_recursive");

    group.bench_function("treewalk", |b| b.iter(|| run_treewalk(black_box(&source))));
    group.bench_function("vm", |b| b.iter(|| run_vm(black_box(&source))));

    group.finish();
}

fn fib_iterative_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("fib_iterative_comparison");
    let source = load_program("fib_iterative");

    group.bench_function("treewalk", |b| b.iter(|| run_treewalk(black_box(&source))));
    group.bench_function("vm", |b| b.iter(|| run_vm(black_box(&source))));

    group.finish();
}

fn loop_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("loop_sum_comparison");
    let source = load_program("loop_sum");

    group.bench_function("treewalk", |b| b.iter(|| run_treewalk(black_box(&source))));
    group.bench_function("vm", |b| b.iter(|| run_vm(black_box(&source))));

    group.finish();
}

fn fib_scaling_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("fib_scaling_comparison");

    for n in [10, 15, 20].iter() {
        let source = format!(
            r#"
fn fib(n: Int) -> Int {{
    if (n <= 1) {{
        return n;
    }}
    return fib(n - 1) + fib(n - 2);
}}
let result = fib({});
"#,
            n
        );

        group.bench_with_input(BenchmarkId::new("treewalk", n), &source, |b, src| {
            b.iter(|| run_treewalk(black_box(src)))
        });
        group.bench_with_input(BenchmarkId::new("vm", n), &source, |b, src| {
            b.iter(|| run_vm(black_box(src)))
        });
    }

    group.finish();
}

/// Benchmark compilation time alone (not execution).
fn compilation_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("compilation_overhead");

    let source = load_program("fib_recursive");
    let program = parse(&source);

    group.bench_function("compile_fib", |b| {
        b.iter(|| Compiler::compile(black_box(&program)).unwrap())
    });

    let source = load_program("loop_sum");
    let program = parse(&source);

    group.bench_function("compile_loop", |b| {
        b.iter(|| Compiler::compile(black_box(&program)).unwrap())
    });

    group.finish();
}

fn json_ops_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_ops_comparison");
    let source = load_program("json_ops");

    group.bench_function("treewalk", |b| b.iter(|| run_treewalk(black_box(&source))));
    group.bench_function("vm", |b| b.iter(|| run_vm(black_box(&source))));

    group.finish();
}

fn json_ops_large_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_ops_large_comparison");
    let source = load_program("json_ops_large");

    group.bench_function("treewalk", |b| b.iter(|| run_treewalk(black_box(&source))));
    group.bench_function("vm", |b| b.iter(|| run_vm(black_box(&source))));

    group.finish();
}

fn array_ops_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("array_ops_comparison");
    let source = load_program("array_ops");

    group.bench_function("treewalk", |b| b.iter(|| run_treewalk(black_box(&source))));
    group.bench_function("vm", |b| b.iter(|| run_vm(black_box(&source))));

    group.finish();
}

fn string_ops_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("string_ops_comparison");
    let source = load_program("string_ops");

    group.bench_function("treewalk", |b| b.iter(|| run_treewalk(black_box(&source))));
    group.bench_function("vm", |b| b.iter(|| run_vm(black_box(&source))));

    group.finish();
}

fn hash_ops_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("hash_ops_comparison");
    let source = load_program("hash_ops");

    group.bench_function("treewalk", |b| b.iter(|| run_treewalk(black_box(&source))));
    group.bench_function("vm", |b| b.iter(|| run_vm(black_box(&source))));

    group.finish();
}

fn class_ops_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("class_ops_comparison");
    let source = load_program("class_ops");

    group.bench_function("treewalk", |b| b.iter(|| run_treewalk(black_box(&source))));
    group.bench_function("vm", |b| b.iter(|| run_vm(black_box(&source))));

    group.finish();
}

criterion_group!(
    benches,
    fibonacci_comparison,
    fib_iterative_comparison,
    loop_comparison,
    fib_scaling_comparison,
    compilation_overhead,
    json_ops_comparison,
    json_ops_large_comparison,
    array_ops_comparison,
    string_ops_comparison,
    hash_ops_comparison,
    class_ops_comparison,
);

criterion_main!(benches);
