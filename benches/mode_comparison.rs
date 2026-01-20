//! Benchmark comparing execution modes: Tree-walk, Bytecode VM, and JIT.
//!
//! Run with: cargo bench --features jit --bench mode_comparison

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use solilang::bytecode::{Compiler, VM};
use solilang::interpreter::Interpreter;
use solilang::lexer::Scanner;
use solilang::parser::Parser;

#[cfg(feature = "jit")]
use solilang::jit::JitVM;

/// Parse source code into AST.
fn parse(source: &str) -> solilang::ast::Program {
    let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
    Parser::new(tokens).parse().expect("parser error")
}

/// Run with tree-walk interpreter.
fn run_tree_walk(source: &str) {
    let program = parse(source);
    let mut interpreter = Interpreter::new();
    interpreter.interpret(&program).expect("runtime error");
}

/// Run with bytecode VM.
fn run_bytecode(source: &str) {
    let program = parse(source);
    let mut compiler = Compiler::new();
    let function = compiler.compile(&program).expect("compile error");
    let mut vm = VM::new();
    vm.run(function).expect("runtime error");
}

/// Run with JIT VM.
#[cfg(feature = "jit")]
fn run_jit(source: &str) {
    let program = parse(source);
    let mut compiler = Compiler::new();
    let function = compiler.compile(&program).expect("compile error");
    let mut vm = JitVM::new();
    vm.run(function).expect("runtime error");
}

/// Recursive fibonacci - tests function call overhead.
fn fibonacci_recursive_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("fib_recursive");

    let source = r#"
fn fib(n: Int) -> Int {
    if (n <= 1) { return n; }
    return fib(n - 1) + fib(n - 2);
}
let result = fib(20);
"#;

    group.bench_function("tree_walk", |b| b.iter(|| run_tree_walk(black_box(source))));

    group.bench_function("bytecode", |b| b.iter(|| run_bytecode(black_box(source))));

    #[cfg(feature = "jit")]
    group.bench_function("jit", |b| b.iter(|| run_jit(black_box(source))));

    group.finish();
}

/// Iterative fibonacci - tests loop performance.
fn fibonacci_iterative_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("fib_iterative");

    let source = r#"
fn fib_iter(n: Int) -> Int {
    if (n <= 1) { return n; }
    let a = 0;
    let b = 1;
    let i = 2;
    while (i <= n) {
        let temp = a + b;
        a = b;
        b = temp;
        i = i + 1;
    }
    return b;
}
let result = fib_iter(40);
"#;

    group.bench_function("tree_walk", |b| b.iter(|| run_tree_walk(black_box(source))));

    group.bench_function("bytecode", |b| b.iter(|| run_bytecode(black_box(source))));

    #[cfg(feature = "jit")]
    group.bench_function("jit", |b| b.iter(|| run_jit(black_box(source))));

    group.finish();
}

/// Sum to N - tests tight loop performance.
fn sum_to_n_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("sum_to_n");

    let source = r#"
fn sum_to(n: Int) -> Int {
    let total = 0;
    let i = 1;
    while (i <= n) {
        total = total + i;
        i = i + 1;
    }
    return total;
}
let result = sum_to(10000);
"#;

    group.bench_function("tree_walk", |b| b.iter(|| run_tree_walk(black_box(source))));

    group.bench_function("bytecode", |b| b.iter(|| run_bytecode(black_box(source))));

    #[cfg(feature = "jit")]
    group.bench_function("jit", |b| b.iter(|| run_jit(black_box(source))));

    group.finish();
}

/// Arithmetic intensive - tests numeric operations.
fn arithmetic_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("arithmetic");

    let source = r#"
fn compute(n: Int) -> Int {
    let result = 0;
    let i = 0;
    while (i < n) {
        result = result + i * 2 - i / 2 + i % 3;
        i = i + 1;
    }
    return result;
}
let result = compute(5000);
"#;

    group.bench_function("tree_walk", |b| b.iter(|| run_tree_walk(black_box(source))));

    group.bench_function("bytecode", |b| b.iter(|| run_bytecode(black_box(source))));

    #[cfg(feature = "jit")]
    group.bench_function("jit", |b| b.iter(|| run_jit(black_box(source))));

    group.finish();
}

/// Nested loops - tests loop overhead.
fn nested_loops_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("nested_loops");

    let source = r#"
fn nested(n: Int) -> Int {
    let sum = 0;
    let i = 0;
    while (i < n) {
        let j = 0;
        while (j < n) {
            sum = sum + 1;
            j = j + 1;
        }
        i = i + 1;
    }
    return sum;
}
let result = nested(100);
"#;

    group.bench_function("tree_walk", |b| b.iter(|| run_tree_walk(black_box(source))));

    group.bench_function("bytecode", |b| b.iter(|| run_bytecode(black_box(source))));

    #[cfg(feature = "jit")]
    group.bench_function("jit", |b| b.iter(|| run_jit(black_box(source))));

    group.finish();
}

/// Function calls - tests call overhead.
fn function_calls_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("function_calls");

    let source = r#"
fn add(a: Int, b: Int) -> Int {
    return a + b;
}

fn multiply(a: Int, b: Int) -> Int {
    return a * b;
}

fn compute(n: Int) -> Int {
    let result = 0;
    let i = 0;
    while (i < n) {
        result = add(result, multiply(i, 2));
        i = i + 1;
    }
    return result;
}
let result = compute(1000);
"#;

    group.bench_function("tree_walk", |b| b.iter(|| run_tree_walk(black_box(source))));

    group.bench_function("bytecode", |b| b.iter(|| run_bytecode(black_box(source))));

    #[cfg(feature = "jit")]
    group.bench_function("jit", |b| b.iter(|| run_jit(black_box(source))));

    group.finish();
}

/// Comparison across different fib(N) values.
fn fib_scaling_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("fib_scaling");
    group.sample_size(10); // Reduce sample size for slower benchmarks

    for n in [10, 15, 20].iter() {
        let source = format!(
            r#"
fn fib(n: Int) -> Int {{
    if (n <= 1) {{ return n; }}
    return fib(n - 1) + fib(n - 2);
}}
let result = fib({});
"#,
            n
        );

        group.bench_with_input(BenchmarkId::new("tree_walk", n), &source, |b, src| {
            b.iter(|| run_tree_walk(black_box(src)))
        });

        group.bench_with_input(BenchmarkId::new("bytecode", n), &source, |b, src| {
            b.iter(|| run_bytecode(black_box(src)))
        });

        #[cfg(feature = "jit")]
        group.bench_with_input(BenchmarkId::new("jit", n), &source, |b, src| {
            b.iter(|| run_jit(black_box(src)))
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    fibonacci_recursive_comparison,
    fibonacci_iterative_comparison,
    sum_to_n_comparison,
    arithmetic_comparison,
    nested_loops_comparison,
    function_calls_comparison,
    fib_scaling_comparison,
);

criterion_main!(benches);
