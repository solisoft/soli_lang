use criterion::{black_box, criterion_group, criterion_main, Criterion};
use solilang::interpreter::Interpreter;
use solilang::lexer::Scanner;
use solilang::parser::Parser;

fn run_program(source: &str) {
    let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
    let mut parser = Parser::new(tokens);
    let program = parser.parse().expect("parser error");

    let mut interpreter = Interpreter::new();
    interpreter.interpret(&program).expect("runtime error");
}

fn array_read_heavy(c: &mut Criterion) {
    let source = r#"
fn build_array(n: Int) -> Int[] {
    let arr: Int[] = [];
    let i = 0;
    while (i < n) {
        arr.push(i);
        i = i + 1;
    }
    return arr;
}

let arr = build_array(1024);
let i = 0;
let total = 0;
while (i < 2000) {
    total = total + arr.len();
    total = total + arr.get(512);
    total = total + arr.first();
    total = total + arr.last();
    let rendered = arr.join(",");
    total = total + rendered.len();
    i = i + 1;
}
"#;

    c.bench_function("array_read_heavy", |b| {
        b.iter(|| run_program(black_box(source)))
    });
}

fn hash_read_heavy(c: &mut Criterion) {
    let source = r#"
fn build_hash(n: Int) -> Hash {
    let h = {};
    let i = 0;
    while (i < n) {
        h.set("key_" + str(i), i);
        i = i + 1;
    }
    return h;
}

let h = build_hash(1024);
let i = 0;
let total = 0;
while (i < 2000) {
    total = total + h.get("key_512");
    if (h.has_key("key_256")) {
        total = total + 1;
    }
    total = total + h.keys().len();
    total = total + h.values().len();
    total = total + h.entries().len();
    i = i + 1;
}
"#;

    c.bench_function("hash_read_heavy", |b| {
        b.iter(|| run_program(black_box(source)))
    });
}

fn string_method_heavy(c: &mut Criterion) {
    let source = r#"
let s = "alpha,beta,gamma,delta\nepsilon,zeta,eta,theta\niota,kappa,lambda,mu";
let i = 0;
let total = 0;
while (i < 3000) {
    total = total + s.split(",").len();
    total = total + s.lines().len();
    total = total + s.chars().len();
    total = total + s.partition("gamma").len();
    total = total + s.rpartition("theta").len();
    if (s.contains("lambda")) {
        total = total + 1;
    }
    i = i + 1;
}
"#;

    c.bench_function("string_method_heavy", |b| {
        b.iter(|| run_program(black_box(source)))
    });
}

criterion_group!(
    benches,
    array_read_heavy,
    hash_read_heavy,
    string_method_heavy,
);
criterion_main!(benches);
