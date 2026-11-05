//! The handler budget (`interpreter::deadline`) on both engines, for the
//! work that never reaches a backward jump: a block a native iterator calls
//! per element, call-only recursion, and a comprehension's loop.
//!
//! Each test runs under a budget that has already run out, so the first
//! clock read (every few thousand checks) reports it; a loop that is not
//! checked runs to completion instead and the test sees `Ok`.

use web_time::Duration;

use crate::error::RuntimeError;
use crate::interpreter::deadline;
use crate::interpreter::value::Value;
use crate::interpreter::Interpreter;
use crate::lexer::Scanner;
use crate::parser::Parser;
use crate::vm::compiler::Compiler;
use crate::vm::Vm;

fn on_vm(source: &str) -> Result<Value, RuntimeError> {
    let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
    let program = Parser::new(tokens).parse().expect("parser error");
    let module = Compiler::compile(&program).expect("compile error");
    Vm::new().execute(&module.main)
}

fn on_tree_walker(source: &str) -> Result<(), RuntimeError> {
    let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
    let program = Parser::new(tokens).parse().expect("parser error");
    Interpreter::new().interpret(&program)
}

fn assert_timed_out<T: std::fmt::Debug>(
    engine: &str,
    source: &str,
    result: Result<T, RuntimeError>,
) {
    match result {
        Err(err) => assert!(
            err.to_string().contains("handler budget"),
            "{engine}: `{source}` failed, but not on the budget: {err}"
        ),
        Ok(value) => panic!("{engine}: `{source}` ran past the budget: {value:?}"),
    }
}

/// An already spent budget: expiry is reported at the first clock read.
fn spent_budget() -> deadline::DeadlineGuard {
    let guard = deadline::enter(Duration::from_nanos(1));
    std::thread::sleep(Duration::from_millis(2));
    guard
}

/// `n.times { … }`, `upto`, `downto` (the native driver) and `each` (the
/// callback loop) stop at the budget on the VM. They checked it nowhere:
/// `n.times { |i| total = i }` with a request-sized `n` pinned a worker.
#[test]
fn vm_block_iterators_stop_at_the_budget() {
    let cases = [
        "total = 0\n200000.times { |i| total = i }\ntotal",
        "total = 0\n0.upto(200000) { |i| total = i }\ntotal",
        "total = 0\n200000.downto(0) { |i| total = i }\ntotal",
        "total = 0\n(0..200000).each { |i| total = i }\ntotal",
        "items = (0..200000)\ntotal = 0\nitems.each do |i|\n  total = i\nend\ntotal",
    ];
    for source in cases {
        let _budget = spent_budget();
        assert_timed_out("vm", source, on_vm(source));
    }
}

/// Recursion with no loop in it stops at the budget on the VM: only
/// `Op::Loop` checked it.
#[test]
fn vm_call_only_recursion_stops_at_the_budget() {
    let source = "def fib(n)\n  return n if n < 2\n\n  fib(n - 1) + fib(n - 2)\nend\nfib(30)";
    let _budget = spent_budget();
    assert_timed_out("vm", source, on_vm(source));
}

/// A rescued timeout does not buy another round: expiry is sticky, so the
/// next check raises again.
#[test]
fn vm_a_rescued_timeout_raises_again() {
    let source = "total = 0\nfirst = (200000.times { |i| total = i }) rescue \"stopped\"\n\
                  200000.times { |i| total = i }\ntotal";
    let _budget = spent_budget();
    assert_timed_out("vm", source, on_vm(source));
}

/// The tree-walker's comprehensions run no statement per element, so the
/// check `execute` makes never ran inside them.
#[test]
fn tree_walker_comprehensions_stop_at_the_budget() {
    let cases = [
        "kept = [i for i in (0..200000) if i < 0]",
        "squares = {i: i * i for i in (0..200000) if i < 0}",
    ];
    for source in cases {
        let _budget = spent_budget();
        assert_timed_out("tree-walker", source, on_tree_walker(source));
    }
}
