//! Differential test: tree-walking interpreter vs bytecode VM.
//!
//! Every program below runs cleanly on the tree-walker (the reference engine).
//! Each is executed through both engines and their observable outcome compared.
//! With Soli's optional-`let` enabled on the VM (`SOLI_VM_OPTIONAL_LET=1`), a
//! well-formed program must produce *identical* output on both engines.
//!
//! This harness exists to surface — and then guard against — a class of VM bugs
//! around local-variable assignment inside control-flow constructs. Cases known
//! to still diverge are listed in `KNOWN_DIVERGENT`; the test stays green while
//! tracking them, FAILS when a *new* divergence appears, and FAILS when a known
//! divergence is fixed (prompting you to remove it from the list and lock in the
//! fix). See memory `project_vm_local_assignment_bugs`.

use std::process::Command;

/// (name, source). Each source must run cleanly on the tree-walker and print
/// deterministic output.
const CASES: &[(&str, &str)] = &[
    // --- if / elsif / else with local assignment ---
    (
        "if_assign_outer_true",
        "fn f(c) { let x = 1\n  if c { x = 9 }\n  return x }\nprint(f(true))",
    ),
    (
        "if_assign_outer_false",
        "fn f(c) { let x = 1\n  if c { x = 9 }\n  return x }\nprint(f(false))",
    ),
    (
        "ifelse_assign",
        "fn f(c) { let x = 0\n  if c { x = 1 } else { x = 2 }\n  return x }\nprint(f(true))\nprint(f(false))",
    ),
    (
        "elsif_chain",
        "fn g(n) { let t = \"?\"\n  if n < 0 { t = \"neg\" } elsif n == 0 { t = \"zero\" } else { t = \"pos\" }\n  return t }\nprint(g(-1))\nprint(g(0))\nprint(g(5))",
    ),
    (
        "nested_if_assign",
        "fn f(a, b) { let r = 0\n  if a { if b { r = 3 } else { r = 2 } } else { r = 1 }\n  return r }\nprint(f(true, true))\nprint(f(true, false))\nprint(f(false, false))",
    ),
    // --- while ---
    (
        "while_accumulate",
        "fn f() { let s = 0\n  let i = 0\n  while i < 5 { s = s + i\n    i = i + 1 }\n  return s }\nprint(f())",
    ),
    (
        "while_truthy_local",
        "fn f() { let run = true\n  let n = 0\n  while run { n = n + 1\n    if n >= 3 { run = false } }\n  return n }\nprint(f())",
    ),
    (
        "while_assign_in_if",
        "fn f() { let total = 0\n  let i = 0\n  while i < 6 { if i % 2 == 0 { total = total + i }\n    i = i + 1 }\n  return total }\nprint(f())",
    ),
    // --- for loops ---
    ("for_value_sum", "let s = 0\nfor v in [1, 2, 3, 4] { s = s + v }\nprint(s)"),
    (
        "for_value_assign_in_if",
        "let c = 0\nfor v in [1, 2, 3, 4, 5] { if v > 2 { c = c + 1 } }\nprint(c)",
    ),
    ("for_range_sum", "let s = 0\nfor v in 1..5 { s = s + v }\nprint(s)"),
    // --- closures capturing loop variables / locals ---
    (
        "while_closure_capture",
        "let fns = []\nlet i = 0\nwhile i < 3 { let x = i * 10\n  fns.push(fn() { return x })\n  i = i + 1 }\nprint(fns[0]())\nprint(fns[1]())\nprint(fns[2]())",
    ),
    (
        "for_value_closure_capture",
        "let fns = []\nfor x in [10, 20, 30] { fns.push(fn() { return x }) }\nprint(fns[0]())\nprint(fns[1]())\nprint(fns[2]())",
    ),
    (
        // Access only the first two closures: both engines yield >= 2 elements
        // for `1..3`, so this isolates closure-capture from the range-bounds
        // divergence (see `for_range_sum`, tracked separately).
        "for_range_closure_capture",
        "let fns = []\nfor x in 1..3 { fns.push(fn() { return x * 10 }) }\nprint(fns[0]())\nprint(fns[1]())",
    ),
    (
        "nested_for_closures",
        "let fns = []\nfor a in [1, 2] { for b in [10, 20] { fns.push(fn() { return a + b }) } }\nprint(fns[0]())\nprint(fns[3]())",
    ),
    (
        "closure_counter",
        "fn make_counter() { let count = 0\n  return fn() { count = count + 1\n    return count } }\nlet c = make_counter()\nprint(c())\nprint(c())\nlet d = make_counter()\nprint(d())",
    ),
    (
        "nested_fn_capture",
        "fn outer() { let a = 10\n  fn inner() { return a + 1 }\n  return inner() }\nprint(outer())",
    ),
    (
        "closure_in_if",
        "fn make(c) { let fns = []\n  if c { fns.push(fn() { return 1 }) } else { fns.push(fn() { return 2 }) }\n  return fns[0]() }\nprint(make(true))\nprint(make(false))",
    ),
    // --- short-circuit ---
    (
        "and_short_circuit",
        "fn f(a, b) { return a && b }\nprint(f(true, 5))\nprint(f(false, 5))\nprint(f(0, 9))",
    ),
    (
        "or_short_circuit",
        "fn f(a, b) { return a || b }\nprint(f(0, 7))\nprint(f(3, 7))",
    ),
    // --- compound / postfix assignment ---
    (
        "compound_assign",
        "fn f() { let x = 10\n  x += 5\n  x -= 2\n  x *= 2\n  return x }\nprint(f())",
    ),
    (
        "or_assign_default",
        "fn f(v) { let x = v\n  x ||= 99\n  return x }\nprint(f(0))\nprint(f(7))",
    ),
    // --- recursion (reentrant locals) ---
    (
        "recursion_let",
        "fn fact(n) { let acc = 1\n  if n > 1 { acc = n * fact(n - 1) }\n  return acc }\nprint(fact(6))",
    ),
    (
        "fib_recursion",
        "fn fib(n) { if n < 2 { return n }\n  return fib(n - 1) + fib(n - 2) }\nprint(fib(12))",
    ),
    // --- match ---
    (
        "match_value",
        "fn classify(n) { return match n { 0 => \"zero\", 1 => \"one\", _ => \"many\" } }\nprint(classify(0))\nprint(classify(1))\nprint(classify(7))",
    ),
    // --- comprehensions ---
    (
        "list_comprehension",
        "let r = [x * 2 for x in [1, 2, 3, 4] if x > 1]\nprint(r)",
    ),
    (
        // Comprehension as a sub-expression (inside an array literal). Used to
        // silently corrupt a neighbouring array on the VM; now the VM refuses
        // to compile comprehensions, so it errors and falls back instead.
        "list_comprehension_nested",
        "let r = [[1, 2], [x for x in [3, 4]]]\nprint(r)",
    ),
    // --- iteration method chains with closures ---
    (
        "map_filter_chain",
        "let r = [1, 2, 3, 4, 5].map(fn(x) x * 2).filter(fn(x) x > 4)\nprint(r)",
    ),
    // --- optional-let: bare assignment (no `let`) ---
    ("bare_top_level", "x = 5\nx = x + 1\nprint(x)"),
    (
        "bare_fn_local",
        "fn go() { s = 0\n  s = s + 3\n  return s }\nprint(go())",
    ),
    (
        "bare_recursion",
        "fn fact(n) { acc = 1\n  if n > 1 { acc = n * fact(n - 1) }\n  return acc }\nprint(fact(5))",
    ),
    (
        "bare_assign_in_if",
        "fn f(c) { total = 0\n  if c { total = 5 }\n  return total }\nprint(f(true))\nprint(f(false))",
    ),
    (
        "in_fn_assign_global",
        "g = 1\nfn f() { g = 99 }\nf()\nprint(g)",
    ),
    // --- try / catch / finally ---
    (
        "try_finally_runs",
        "fn f() { let log = []\n  try { log.push(\"t\") } finally { log.push(\"f\") }\n  return log }\nprint(f())",
    ),
    (
        "try_catch_recovers",
        "fn f() { try { throw \"boom\" } catch e { return \"caught\" }\n  return \"no\" }\nprint(f())",
    ),
    // --- KNOWN-DIVERGENT (tracked VM bugs) ---
    (
        "for_index_sum",
        "let s = 0\nfor v, i in [10, 20, 30] { s = s + i }\nprint(s)",
    ),
    (
        "for_index_read",
        "for v, i in [9, 8, 7] { print(str(i) + \":\" + str(v)) }",
    ),
    (
        "try_catch_assign_outer",
        "fn f() { let x = 0\n  try { x = 1\n    throw \"e\" } catch e { x = 2 }\n  return x }\nprint(f())",
    ),
];

/// Cases that currently diverge because of an unfixed VM bug. Keep this list in
/// sync with reality: when a fix lands, the corresponding case starts matching
/// and the test will tell you to remove it from here.
const KNOWN_DIVERGENT: &[&str] = &[
    // #9 — comprehensions are unsupported on the VM: it now refuses to compile
    //      them (a correct impl needs compile-time stack-depth tracking), so
    //      handlers using them fall back to the interpreter. Both a top-level
    //      and a nested (sub-expression) comprehension stay divergent because
    //      the VM errors where the interpreter succeeds — but neither is
    //      silently wrong anymore.
    "list_comprehension",
    "list_comprehension_nested",
    // Fixed and locked in by this harness:
    //   #5  for-with-index (ForIter index)   — compiler now maintains the counter
    //   #6  assignment inside catch          — TryBegin catch_ip off-by-one
    //   #7  range bounds (a..b exclusive)     — VM range ops now exclusive
    //   #8  `||=` panic (let-from-local)      — removed unsafe GetLocal2 fusion
    //   #10 return inside catch               — TryBegin catch_ip off-by-one
];

/// Run `source` through the soli binary; `vm` selects the bytecode VM with
/// optional-`let` enabled. Returns the observable outcome: stdout on success,
/// or a sentinel on any non-success (error/panic) so error *text* differences
/// don't count as behavioral divergence.
fn run(source: &str, idx: usize, vm: bool) -> String {
    let mut path = std::env::temp_dir();
    path.push(format!("soli_diff_{}_{}.sl", std::process::id(), idx));
    std::fs::write(&path, source).expect("write temp source");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_soli"));
    cmd.arg(&path);
    if vm {
        cmd.arg("--vm").env("SOLI_VM_OPTIONAL_LET", "1");
    }
    let output = cmd.output().expect("run soli");
    let _ = std::fs::remove_file(&path);

    if output.status.success() {
        String::from_utf8_lossy(&output.stdout).into_owned()
    } else {
        "<non-success>".to_string()
    }
}

#[test]
fn tree_walker_and_vm_agree() {
    let known: std::collections::HashSet<&str> = KNOWN_DIVERGENT.iter().copied().collect();

    let mut new_divergences: Vec<String> = Vec::new();
    let mut fixed: Vec<&str> = Vec::new();

    for (idx, (name, source)) in CASES.iter().enumerate() {
        let tw = run(source, idx, false);
        let vm = run(source, idx, true);
        let diverges = tw != vm;
        let is_known = known.contains(name);

        match (diverges, is_known) {
            (true, false) => new_divergences.push(format!(
                "  [NEW DIVERGENCE] {name}\n    tree-walker: {tw:?}\n    vm:          {vm:?}"
            )),
            (false, true) => fixed.push(name),
            _ => {}
        }
    }

    let mut msg = String::new();
    if !new_divergences.is_empty() {
        msg.push_str(&format!(
            "{} program(s) produce different results on the tree-walker vs the VM \
             (a VM correctness bug — see memory project_vm_local_assignment_bugs):\n{}\n",
            new_divergences.len(),
            new_divergences.join("\n")
        ));
    }
    if !fixed.is_empty() {
        msg.push_str(&format!(
            "{} known-divergent case(s) now AGREE — remove them from KNOWN_DIVERGENT \
             to lock in the fix: {:?}\n",
            fixed.len(),
            fixed
        ));
    }
    assert!(msg.is_empty(), "\n{msg}");
}
