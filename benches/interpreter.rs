//! Interpreter benchmarks for Solilang.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use solilang::interpreter::Interpreter;
use solilang::lexer::Scanner;
use solilang::parser::Parser;
use solilang::types::TypeChecker;
use std::fs;

/// Run a Solilang program from source code.
fn run_program(source: &str) {
    let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
    let mut parser = Parser::new(tokens);
    let program = parser.parse().expect("parser error");

    let mut checker = TypeChecker::new();
    checker.check(&program).expect("type error");

    let mut interpreter = Interpreter::new();
    interpreter.interpret(&program).expect("runtime error");
}

/// Load and run a benchmark program file.
fn run_benchmark_file(name: &str) {
    let path = format!("benches/programs/{}.sl", name);
    let source = fs::read_to_string(&path).expect(&format!("failed to read {}", path));
    run_program(&source);
}

fn fibonacci_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("fibonacci");

    group.bench_function("recursive_fib20", |b| {
        b.iter(|| run_benchmark_file(black_box("fib_recursive")))
    });

    group.bench_function("iterative_fib30", |b| {
        b.iter(|| run_benchmark_file(black_box("fib_iterative")))
    });

    group.finish();
}

fn loop_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("loops");

    group.bench_function("sum_10000", |b| {
        b.iter(|| run_benchmark_file(black_box("loop_sum")))
    });

    group.finish();
}

fn collection_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("collections");

    group.bench_function("array_ops_1000", |b| {
        b.iter(|| run_benchmark_file(black_box("array_ops")))
    });

    group.bench_function("hash_ops_500", |b| {
        b.iter(|| run_benchmark_file(black_box("hash_ops")))
    });

    group.finish();
}

fn oop_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("oop");

    group.bench_function("class_method_1000", |b| {
        b.iter(|| run_benchmark_file(black_box("class_ops")))
    });

    group.bench_function("deep_inheritance_1000", |b| {
        b.iter(|| run_benchmark_file(black_box("inheritance_deep")))
    });

    group.finish();
}

fn pipeline_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("pipeline");

    group.bench_function("pipeline_1000", |b| {
        b.iter(|| run_benchmark_file(black_box("pipeline_ops")))
    });

    group.finish();
}

fn string_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("strings");

    group.bench_function("string_concat_500", |b| {
        b.iter(|| run_benchmark_file(black_box("string_ops")))
    });

    group.finish();
}

/// Benchmark parsing only (no execution).
fn parsing_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("parsing");

    let source = fs::read_to_string("benches/programs/class_ops.sl").unwrap();

    group.bench_function("parse_class_program", |b| {
        b.iter(|| {
            let tokens = Scanner::new(black_box(&source)).scan_tokens().unwrap();
            let mut parser = Parser::new(tokens);
            parser.parse().unwrap()
        })
    });

    group.finish();
}

/// Benchmark type checking only.
fn typecheck_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("typecheck");

    let source = fs::read_to_string("benches/programs/class_ops.sl").unwrap();
    let tokens = Scanner::new(&source).scan_tokens().unwrap();
    let mut parser = Parser::new(tokens);
    let program = parser.parse().unwrap();

    group.bench_function("typecheck_class_program", |b| {
        b.iter(|| {
            let mut checker = TypeChecker::new();
            checker.check(black_box(&program)).unwrap()
        })
    });

    group.finish();
}

/// Parameterized fibonacci benchmark for different N values.
fn fibonacci_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("fib_scaling");

    for n in [10, 15, 20, 25].iter() {
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

        group.bench_with_input(BenchmarkId::new("recursive", n), &source, |b, src| {
            b.iter(|| run_program(black_box(src)))
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    fibonacci_benchmarks,
    loop_benchmarks,
    collection_benchmarks,
    oop_benchmarks,
    pipeline_benchmarks,
    string_benchmarks,
    parsing_benchmarks,
    typecheck_benchmarks,
    fibonacci_scaling,
);

criterion_main!(benches);
