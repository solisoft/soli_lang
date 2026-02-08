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
    vm.execute(&module.main).expect("vm runtime error");
}

fn load_program(name: &str) -> String {
    let path = format!("benches/programs/{}.sl", name);
    fs::read_to_string(&path).unwrap_or_else(|_| panic!("failed to read {}", path))
}

fn fibonacci_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("fib_recursive_comparison");
    let source = load_program("fib_recursive");

    group.bench_function("treewalk", |b| {
        b.iter(|| run_treewalk(black_box(&source)))
    });
    group.bench_function("vm", |b| {
        b.iter(|| run_vm(black_box(&source)))
    });

    group.finish();
}

fn fib_iterative_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("fib_iterative_comparison");
    let source = load_program("fib_iterative");

    group.bench_function("treewalk", |b| {
        b.iter(|| run_treewalk(black_box(&source)))
    });
    group.bench_function("vm", |b| {
        b.iter(|| run_vm(black_box(&source)))
    });

    group.finish();
}

fn loop_comparison(c: &mut Criterion) {
    let mut group = c.benchmark_group("loop_sum_comparison");
    let source = load_program("loop_sum");

    group.bench_function("treewalk", |b| {
        b.iter(|| run_treewalk(black_box(&source)))
    });
    group.bench_function("vm", |b| {
        b.iter(|| run_vm(black_box(&source)))
    });

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

criterion_group!(
    benches,
    fibonacci_comparison,
    fib_iterative_comparison,
    loop_comparison,
    fib_scaling_comparison,
    compilation_overhead,
);

criterion_main!(benches);
