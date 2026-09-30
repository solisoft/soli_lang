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
    (
        // Mutating the iterated array inside the body: both engines iterate
        // LIVE (bounds-checked indexing, appended items are visited) — pinned
        // when the tree-walker dropped its upfront snapshot clone.
        "for_array_mutation_live",
        "let a = [1, 2, 3]\nfor x in a { if x < 3 { a.push(x + 10) } }\nprint(a)",
    ),
    (
        // Recursive flatten (+ explicit depth) must agree across engines — the
        // VM used to flatten only one level and reject a depth argument. Pins
        // the shared `array_ops` implementation.
        "array_flatten_recursive",
        "print([[1, [2]], 3].flatten())\nprint([1, [2, [3, [4]]]].flatten(1))",
    ),
    (
        "array_uniq_compact",
        "print([1, 2, 2, 3, 1].uniq())\nprint([1, null, 2, null].compact())",
    ),
    (
        // Range with a negative/empty span must not iterate.
        "for_range_empty",
        "let s = 0\nfor v in 5..5 { s = s + 1 }\nfor v in 5..2 { s = s + 1 }\nprint(s)",
    ),
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
        "list_comprehension_range",
        "let r = [x * x for x in 1..5]\nprint(r)",
    ),
    (
        "list_comprehension_empty",
        "let r = [x for x in [1, 2, 3] if x > 99]\nprint(r)",
    ),
    (
        "list_comprehension_return",
        "fn f() { return [x for x in [1, 2, 3]] }\nprint(f())",
    ),
    (
        "hash_comprehension",
        "let h = {x: x * 10 for x in [1, 2, 3]}\nprint(h)",
    ),
    (
        // Two comprehensions in one program — catches post-loop height desync.
        "two_comprehensions",
        "let a = [x for x in [1, 2]]\nlet b = [y * 10 for y in [3, 4]]\nprint(a)\nprint(b)",
    ),
    (
        // Closure capturing the loop variable — each must capture its own value.
        "comprehension_closure_capture",
        "let fns = [fn() { return x } for x in [1, 2, 3]]\nprint(fns[0]())\nprint(fns[2]())",
    ),
    (
        // Comprehension inside a for-loop body (clean per-iteration).
        "comprehension_in_loop",
        "let all = []\nfor n in [1, 2] { let r = [x for x in [n, n]]\n  all.push(r) }\nprint(all)",
    ),
    (
        // Comprehension as a sub-expression (inside an array literal). Compiled
        // as a zero-arg lambda so the result array lives at a real local slot.
        "list_comprehension_nested",
        "let r = [[1, 2], [x for x in [3, 4]]]\nprint(r)",
    ),
    (
        // Comprehension as a call argument — wrapped in a clean-frame lambda.
        "comprehension_call_arg",
        "fn total(a) { let t = 0\n  for v in a { t = t + v }\n  return t }\nprint(total([x * 2 for x in [1, 2, 3]]))",
    ),
    (
        // Backticks used to refuse compiled mode. They lower to System.shell;
        // `.stdout` auto-resolves the Future on both engines.
        "backtick_echo",
        "print(`printf hello`.stdout)",
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
    // --- additional coverage (regression guards) ---
    (
        "match_literal",
        "fn f(n) { return match n { 0 => \"z\", 1 => \"o\", _ => \"m\" } }\nprint(f(0))\nprint(f(1))\nprint(f(9))",
    ),
    (
        "nested_try",
        "fn f() { let r = 0\n  try { try { throw \"a\" } catch e { r = 1\n      throw \"b\" } } catch e { r = r + 10 }\n  return r }\nprint(f())",
    ),
    (
        "return_in_finally_block",
        "fn f() { try { return \"t\" } finally { } }\nprint(f())",
    ),
    (
        "throw_across_call",
        "fn boom() { throw \"x\" }\nfn f() { try { boom() } catch e { return \"caught\" } }\nprint(f())",
    ),
    (
        "closure_captures_param",
        "fn adder(n) { return fn(x) { return x + n } }\nlet add5 = adder(5)\nprint(add5(10))",
    ),
    (
        "ternary",
        "fn f(n) { return n > 0 ? \"pos\" : \"neg\" }\nprint(f(5))\nprint(f(-1))",
    ),
    (
        "nullish_coalescing",
        "fn f(v) { return v ?? \"default\" }\nprint(f(null))\nprint(f(\"x\"))",
    ),
    (
        "compound_index_assign",
        "let a = [10, 20, 30]\na[1] = a[1] + 5\nprint(a)",
    ),
    (
        "hash_compound_assign",
        "let h = {\"n\": 1}\nh[\"n\"] = h[\"n\"] + 10\nprint(h)",
    ),
    (
        "early_return_in_loop",
        "fn first_even(a) { for v in a { if v % 2 == 0 { return v } }\n  return -1 }\nprint(first_even([1, 3, 4, 7]))",
    ),
    (
        "deep_recursion",
        "fn sum_to(n) { if n == 0 { return 0 }\n  return n + sum_to(n - 1) }\nprint(sum_to(100))",
    ),
    (
        "array_method_chain",
        "let r = [1, 2, 3, 4, 5].filter(fn(x) x > 2).map(fn(x) x * 10)\nprint(r)",
    ),
    (
        "pipeline",
        "fn double(x) { return x * 2 }\nprint(5 |> double())",
    ),
    (
        "string_interpolation",
        "let name = \"world\"\nprint(\"hello \\(name)\")",
    ),
    // --- KNOWN-DIVERGENT (tracked VM gaps) ---
    (
        "match_var_binding",
        "fn f(n) { return match n { x => x + 1 } }\nprint(f(5))",
    ),
    (
        "match_array_pattern",
        "fn f(a) { return match a { [x, y] => x + y, _ => 0 } }\nprint(f([3, 4]))",
    ),
    (
        "match_hash_pattern",
        "fn f(h) { return match h { {name: n} => n, _ => \"?\" } }\nprint(f({\"name\": \"bob\"}))",
    ),
    (
        "instance_method_call",
        "class C { x: Int\n  new(x) { this.x = x }\n  fn get() { return this.x } }\nlet c = C(42)\nprint(c.get())",
    ),
    (
        // super() ctor chaining + super.method() through a 3-level
        // hierarchy — resolves against the *defining* class's superclass
        // (frame class context), not the instance's class.
        "super_init_and_method",
        "class A { t: String\n  new(t) { this.t = t }\n  fn d() { return \"A(\" + this.t + \")\" } }\nclass B extends A { new(t) { super(t + \"b\") }\n  fn d() { return \"B[\" + super.d() + \"]\" } }\nclass C extends B { new(t) { super(t + \"c\") }\n  fn d() { return \"C{\" + super.d() + \"}\" } }\nprint(C(\"x\").d())",
    ),
    (
        // Field initializers apply in both instantiation forms.
        "field_initializer_defaults",
        "class S { theme: String = \"dark\"\n  size: Int = 25 }\nlet s = S()\nprint(s.theme)\nprint(s.size)",
    ),
    (
        // Native-method errors route through the active rescue handler in
        // the VM (run() wraps run_dispatch and re-throws via
        // throw_exception) — they used to abort the whole program.
        "rescue_native_error",
        "print(\"zz\".hex rescue \"caught\")",
    ),
    (
        // …and through try/catch, binding the error text like the
        // tree-walker.
        "try_catch_native_error",
        "try { print(\"zz\".hex) } catch (e) { print(\"caught\") }",
    ),
    // --- enums (VM-native: declaration, construction, methods, equality) ---
    // Enum-variant *match patterns* and named construction deliberately bail to
    // the tree-walker (binding-pattern / named-arg VM gaps), so they aren't
    // exercised here; positional construction + method dispatch + `==` are
    // VM-native and must match the tree-walker exactly.
    (
        "enum_unit_variant_access",
        "enum Color { Red, Green, Blue }\nprint(Color.Green.variant())\nprint(Color.Red.variant())",
    ),
    (
        "enum_unit_equality",
        "enum Color { Red, Green, Blue }\nprint(Color.Red == Color.Red)\nprint(Color.Red == Color.Blue)",
    ),
    (
        "enum_payload_positional_and_structural_eq",
        "enum Box { Empty, Full(x: Int) }\nprint(Box.Full(7).variant())\nprint(Box.Full(7) == Box.Full(7))\nprint(Box.Full(7) == Box.Full(8))\nprint(Box.Empty == Box.Full(1))",
    ),
    (
        "enum_method_with_if",
        "enum Color { Red, Green, Blue\n  fn hex() { if this == Color.Red { return \"#f00\" }\n    if this == Color.Green { return \"#0f0\" }\n    return \"#00f\" } }\nprint(Color.Red.hex())\nprint(Color.Green.hex())\nprint(Color.Blue.hex())",
    ),
    (
        // DB/JSON serialization shape + `parse` reconstruction must match.
        "enum_serialization_and_parse",
        "enum C { Red, Blue, Tag(name: String) }\nprint(json_stringify({\"c\": C.Red, \"t\": C.Tag(\"x\")}))\nprint(C.parse(\"Red\") == C.Red)\nprint(C.parse({\"variant\": \"Tag\", \"name\": \"y\"}) == C.Tag(\"y\"))",
    ),
    // --- functions stored in hash entries (dispatch tables) ---
    // The VM used to reject `handlers.on_create(rec)` outright: an unknown hash
    // method raised NoSuchProperty without ever checking whether the key held a
    // callable. The tree-walker called it, so the pattern passed every test and
    // failed only in production, where the VM runs.
    (
        "hash_entry_function_called",
        "let h = {\"cb\": fn(x) { return x * 2 }}\nprint(h.cb(21))",
    ),
    (
        "hash_entry_function_zero_args",
        "let h = {\"f\": fn() { return 7 }}\nprint(h.f())",
    ),
    (
        "hash_entry_function_two_args",
        "let h = {\"j\": fn(a, b) { return a + b }}\nprint(h.j(2, 3))",
    ),
    (
        "hash_entry_function_via_local",
        "let h = {\"f\": fn() { return 7 }}\nlet g = h.f\nprint(g())",
    ),
    (
        "hash_builtin_method_still_wins",
        "let h = {\"a\": 1, \"b\": 2}\nprint(h.keys())",
    ),
    (
        "hash_typo_raises_in_both_engines",
        "let h = {\"a\": 1}\nlet caught = false\ntry { h.lenght() } catch e { caught = true }\nprint(caught)",
    ),
    (
        "hash_universal_members_still_resolve",
        "let h = {\"a\": 1}\nprint(h.present?())\nprint(h.blank?())\nprint(h.class())",
    ),
    (
        "hash_shift_still_resolves",
        "let h = {\"a\": 1}\nprint(h.shift())",
    ),
    (
        // `delete` and `shift` keep the remaining keys in insertion order. The
        // VM swapped the last key into the hole (both the literal-key opcode
        // and the dynamic-key call), and both engines did it on `shift`.
        "hash_delete_shift_keep_order",
        "let h = {\"a\": 1, \"b\": 2, \"c\": 3, \"d\": 4}\nh.delete(\"a\")\nprint(h.keys())\nlet k = \"b\"\nh.delete(k)\nprint(h.keys())\nlet s = {:x => 1, :y => 2, :z => 3}\ns.delete(:x)\nprint(s.keys())\nlet g = {\"a\": 1, \"b\": 2, \"c\": 3}\nprint(g.shift())\nprint(g.keys())",
    ),
    (
        // `except` keeps the survivors' order, ignores absent and unhashable
        // keys, and leaves the receiver alone.
        "hash_except_order_and_misses",
        "let h = {\"a\": 1, \"b\": 2, \"c\": 3, 1: \"one\"}\nprint(h.except([\"c\", \"a\", \"zz\", 1.5, 1]))\nprint(h.except([]))\nprint(h.keys())\nlet e = h.except([\"b\"])\ne.set(\"n\", 9)\nprint(e.keys())\nprint(h.length())",
    ),
    (
        // Borrowed-tier methods answered straight off the VM stack, including
        // errors and the fall-through to a callable stored in the hash.
        "hash_borrowed_methods",
        "let h = {\"a\": 1, \"b\": {\"c\": [10, 20]}}\nprint(h.dig(\"b\", \"c\", -1))\nprint(h.slice([\"b\", \"a\", \"q\"]).keys())\nprint(h.values_at(\"a\", \"q\"))\nprint(h.merge({\"a\": 5, \"z\": 0}))\nprint(h.to_h() == h)\nprint(h.fetch(\"q\", 7))\nprint(h.fetch(\"q\") rescue \"missing\")\nlet t = {\"hi\": fn(n) n * 2}\nprint(t.hi(21))",
    ),
    (
        // Templates (`Kernel` in `vm_callback_loop.rs`) evaluate pure callback bodies without a frame; every hand-off — overflow, floats, mixed types, strings, null, unhashable keys, a side-effecting body — must give what the frame path gives.
        "callback_templates_match_the_frame_path",
        "fn main() {\n  let big = 9223372036854775807\n  let a = [1, 2, 3, 4]\n  print(a.map(fn(x) x * 2))\n  print(a.map(fn(x) x))\n  print(a.filter(fn(x) x > 2))\n  print(a.filter(fn(x) x <= 2))\n  print([1.5, 2.5].map(fn(x) x * 2.0))\n  print([1, 2.5, 3].map(fn(x) x + 1))\n  print([1, \"s\", 3].filter(fn(x) x == 1))\n  try { print([1, big].map(fn(x) x * 2)) } catch e { print(\"overflow\") }\n  try { print([1, big].map(fn(x) x + 1)) } catch e { print(\"overflow2\") }\n  print([big, 1].map(fn(x) x - 1))\n  let h = {\"a\": 1, \"b\": 2.5, \"c\": 3}\n  print(h.map(fn(k, v) [k, v + 1]))\n  print({\"a\": 1, \"b\": 2}.map(fn(k, v) [v, k]))\n  print(h.filter(fn(k, v) v > 1))\n  print(h.any?(fn(k, v) v > 2))\n  print(h.all?(fn(k, v) v > 0))\n  print(h.transform_values(fn(v) v * 10))\n  print(h.transform_keys(fn(k) k))\n  print(h.each_value(fn(v) v) == h)\n  let n = {\"x\": null, \"y\": 1}\n  print(n.filter(fn(k, v) v))\n  try { print(n.transform_values(fn(v) v + 1)) } catch e { print(\"null+1\") }\n  try { print({\"a\": 1.5}.map(fn(k, v) [v, k])) } catch e { print(\"unhashable\") }\n  let ints = {1: 10, 2: 20}\n  print(ints.transform_keys(fn(k) k * 2))\n  print(ints.map(fn(k, v) [k, v * 2]))\n  let big12 = {}\n  for i in 0..12 { big12[\"k#{i}\"] = i }\n  print(big12.filter(fn(k, v) v >= 9))\n  print(big12.transform_values(fn(v) v - 1))\n  print(big12.any?(fn(k, v) v > 10))\n  let s = [3, 1, 2]\n  let calls = 0\n  print(s.map(fn(x) { calls = calls + 1\n    x * 2 }))\n  print(calls)\n  print([].map(fn(x) x * 2))\n  print({}.filter(fn(k, v) v > 1))\n}\nmain()",
    ),
    (
        // The accumulator template (`total = total + v` in a loop callback) writes a captured variable: open and closed cells, ints, floats, a mixed and a string sum (frame path), overflow midway.
        "callback_accumulator_template",
        "fn make_counter() {\n  let n = 0\n  let add = fn(x) { n = n + x }\n  return [add, fn() { n }]\n}\nfn main() {\n  let row = {\"a\": 1, \"b\": 2, \"c\": 3}\n  let c = 0\n  row.each(fn(k, v) { c = c + 1 })\n  print(c)\n  let total = 0\n  row.each(fn(k, v) { total = total + v })\n  print(total)\n  let ft = 0.5\n  [1.5, 2.25].each(fn(x) { ft = ft + x })\n  print(ft)\n  let mix = 1\n  [1.5, 2].each(fn(x) { mix = mix + x })\n  print(mix)\n  let s = \"\"\n  [\"a\", \"b\"].each(fn(x) { s = s + x })\n  print(s)\n  let big = 9223372036854775800\n  try { [5, 5].each(fn(x) { big = big + x }) } catch e { print(\"overflow\") }\n  print(big)\n  let pair = make_counter()\n  [1, 2, 3].each(pair[0])\n  print(pair[1]())\n  let r = [10, 20].map(fn(x) { total = total + x })\n  print(r)\n  print(total)\n  let ks = 0\n  {1: \"x\", 2: \"y\"}.each(fn(k, v) { ks = ks + k })\n  print(ks)\n  let seen = 0\n  print({\"a\": 1, \"b\": 5}.any?(fn(k, v) { seen = seen + v }))\n  print(seen)\n}\nmain()",
    ),
    (
        // `map` callbacks returning pairs: pooled arrays, a pair the callback keeps, nested and unhashable pairs.
        "callback_map_pairs_and_pool",
        "fn main() {\n  let h = {\"a\": 1, \"b\": 2, \"c\": 3}\n  print(h.map(fn(k, v) [k, v * 2]))\n  let saved = []\n  print(h.map(fn(k, v) { let p = [k + k, v]\n    saved.push(p)\n    p }))\n  print(saved)\n  let lits = []\n  for i in 0..3 { lits.push([i, i]) }\n  print(lits)\n  print(h.map(fn(k, v) [k, [v, v]]))\n  try { h.map(fn(k, v) [1.5, v]) } catch e { print(\"err\") }\n  print([1, 2].map(fn(x) [x, x]))\n}\nmain()",
    ),
    (
        // `each |pair|` refills the previous pair in place only when the callback kept no reference to it.
        "callback_pair_argument_reuse",
        "fn main() {\n  let h = {\"a\": 1, \"b\": 2, \"c\": 3}\n  let kept = []\n  h.each(fn(p) { kept.push(p) })\n  print(kept)\n  let last = null\n  h.each(fn(p) { last = p })\n  print(last)\n  let firsts = []\n  h.each(fn(p) { firsts.push(p[0]) })\n  print(firsts)\n  h.each(fn(p) { p.push(9) })\n  let seen = []\n  h.each(fn(p) { seen.push(p.length()) })\n  print(seen)\n}\nmain()",
    ),
    (
        // `GetLocal, Constant, <cmp>` fuses into `<Cmp>LocalConst`: int, float
        // and mixed operands, strings, all four comparators, in expressions,
        // `if`, `while` and a predicate callback.
        "compare_local_with_constant",
        "fn main() {\n  let i = 5\n  let f = 2.5\n  let s = \"abc\"\n  let n = null\n  print([i < 10, i <= 5, i > 4, i >= 6])\n  print([f < 3, f <= 2.5, f > 2.0, f >= 2.6])\n  print([i < 5.5, i > 4.9, f < 3, f > 2])\n  print([\"b\" < \"c\", s > \"abb\", s >= \"abc\", s <= \"a\"])\n  let c = 0\n  let k = 0\n  while k < 10 {\n    if k > 3 { c = c + k }\n    if k >= 8 { c = c + 100 }\n    k = k + 1\n  }\n  print(c)\n  print([1, 5, 12, 40].filter(fn(x) x > 10))\n  print({\"a\": 1, \"b\": 20}.any?(fn(k, v) v >= 20))\n}\nmain()",
    ),
    (
        // Iterator methods with a compiled callback run their loop inside the
        // VM's dispatch loop (`vm_callback_loop.rs`). Every kind, the
        // `(k, v)` / `[k, v]` argument shapes, short-circuiting (the call
        // count proves it), results and errors, empty receivers, mutation
        // during iteration, a throw out of a callback, nesting and chaining.
        "callback_loops_match_the_native_drivers",
        "fn main() {\n  let h = {\"a\": 1, \"b\": 2, \"c\": 3}\n  let seen = []\n  let r = h.each(fn(k, v) { seen.push(\"#{k}=#{v}\") })\n  print(seen)\n  print(r == h)\n  let pairs = []\n  h.each(fn(p) { pairs.push(p) })\n  print(pairs)\n  let vals = []\n  print(h.each_value(fn(v) { vals.push(v * 10) }) == h)\n  print(vals)\n  let ks = []\n  h.each_key(fn(k) { ks.push(k) })\n  print(ks)\n  print(h.map(fn(k, v) [k + \"!\", v * 2]))\n  print(h.map(fn(k, v) { if v == 2 { return \"skip\" }\n    [k, v] }))\n  let failed = false\n  try { h.map(fn(k, v) [[1], v]) } catch e { failed = true }\n  print(failed)\n  print(h.filter(fn(k, v) v > 1))\n  print(h.select(fn(k, v) v != 2))\n  print(h.reject(fn(k, v) v > 1))\n  let calls = 0\n  print(h.any?(fn(k, v) { calls = calls + 1\n    v >= 2 }))\n  print(calls)\n  calls = 0\n  print(h.all?(fn(k, v) { calls = calls + 1\n    v < 2 }))\n  print(calls)\n  print(h.any?(fn(k, v) v > 99))\n  print(h.all?(fn(k, v) v > 0))\n  print(h.transform_values(fn(v) v * 100))\n  print(h.transform_keys(fn(k) k + k))\n  print({}.map(fn(k, v) [k, v]))\n  print({}.any?(fn(k, v) true))\n  print({}.all?(fn(k, v) false))\n  let a = [1, 2, 3, 4]\n  print(a.map(fn(x) x * x))\n  print(a.filter(fn(x) x % 2 == 0))\n  let acc = []\n  print(a.each(fn(x) { acc.push(x) }) == a)\n  print(acc)\n  print([].map(fn(x) x))\n  let grow = [1, 2]\n  grow.each(fn(x) { if x < 5 { grow.push(x + 10) } })\n  print(grow)\n  try { a.map(fn(x) { if x == 3 { throw {\"code\": 404} }\n    x }) } catch e { print(e) }\n  print(a.map(fn(x) a.map(fn(y) x * y).length()))\n  print(h.map(fn(k, v) [k, v]).length())\n  print(a.map(fn(x) x * 2).filter(fn(x) x > 4).map(fn(x) x + 1))\n}\nmain()",
    ),
    (
        // Hash literals: distinct keys build directly (`HashWithKeys`); a
        // repeated key keeps the first key's position with the last value;
        // past eight entries the hash moves to its indexed form, same order.
        "hash_literal_keys_small_and_large",
        "let d = {\"a\": 1, \"b\": 2, \"a\": 3}\nprint(d)\nprint(d.keys())\nlet big = {\"k1\": 1, \"k2\": 2, \"k3\": 3, \"k4\": 4, \"k5\": 5, \"k6\": 6, \"k7\": 7, \"k8\": 8, \"k9\": 9, \"k10\": 10}\nprint(big.keys())\nbig.delete(\"k2\")\nprint(big[\"k9\"])\nprint(big.length())\nlet grow = {}\nfor i in 0..12 { grow[\"g#{i}\"] = i }\ngrow.delete(\"g0\")\nprint(grow.keys())\nprint({\"x\": 1, \"y\": 2} == {\"y\": 2, \"x\": 1})",
    ),
    (
        // `a && <comparison>` inside an `if`: the `&&` short-circuit jumps
        // onto the `if`'s `JumpIfFalse`, which the peephole must then leave
        // unfused. An off-by-one in its jump-target guard fused it anyway, and
        // the short-circuit landed inside the then-branch.
        "and_with_comparison_rhs_in_if",
        "fn f(cond) {\n  let t = 0\n  for k in 0..10 { if cond(k) { t = t + k } }\n  return t\n}\nlet t1 = 0\nfor k in 0..10 { if k > 4 && k != 7 { t1 = t1 + k } }\nprint(t1)\nlet t2 = 0\nfor k in 0..10 { if k > 4 && k < 7 { t2 = t2 + k } }\nprint(t2)\nlet t3 = 0\nfor k in 0..10 { if k != 7 && k > 4 { t3 = t3 + k } }\nprint(t3)\nlet t4 = 0\nfor k in 0..10 { if k < 3 || k >= 8 { t4 = t4 + k } }\nprint(t4)\nlet t5 = 0\nlet k = 0\nwhile k < 10 { if k > 1 && k <= 5 { t5 = t5 + k }\n  k = k + 1 }\nprint(t5)",
    ),
    (
        // The VM's fast dispatch tier hands an op back to the general arm
        // whenever it cannot finish it inline. Each line here takes one of
        // those hand-offs: an overflow (the error must still carry the line),
        // mixed int/float arithmetic and comparison, string `+`, float locals
        // in the fused local ops, and string equality against a constant.
        "fast_tier_hands_off_to_the_general_arm",
        "fn f() {\n  let big = 9223372036854775807\n  try { print(big + 1) } catch e { print(\"overflow\") }\n  let s = \"\"\n  let i = 0\n  while i < 3 { s = s + \"ab\"\n    i = i + 1 }\n  print(s)\n  let x = 1.5\n  let y = 2\n  print(x + y)\n  print(y * x - 1)\n  print(y < x)\n  print(x <= 1.5)\n  print(x + x)\n  x += 1\n  print(x)\n  let name = \"soli\"\n  print(name == \"soli\")\n  print(name != \"ruby\")\n  let n = null\n  print(n == null)\n  let t = 0\n  for k in 0..10 { if k > 4 && k != 7 { t = t + k } }\n  print(t)\n}\nf()",
    ),
    (
        // `hash.key` dot access: a hit, and a miss falling back to a method.
        "hash_dot_access",
        "let h = {\"name\": \"x\", \"n\": null}\nprint(h.name)\nprint(h.n)\nprint(h.length)",
    ),
    (
        "inspect_quotes_nested_strings",
        "print([1, \"a\"].inspect())\nprint({\"k\": \"v\"}.inspect())\nprint([[1, \"a\"], {\"k\": \"v\"}].inspect())",
    ),
    (
        "hash_to_s_matches_to_string",
        "let h = {\"a\": 1, \"b\": 2}\nprint(h.to_s())\nprint(h.to_string())",
    ),
    (
        "pop_on_empty_array_is_null_like_shift",
        "let a = []\nprint(a.pop())\nprint(a.shift())\nprint(a.first())\nlet b = [1, 2]\nprint(b.pop())",
    ),
    (
        "sort_by_propagates_an_error_from_the_key",
        "let caught = false\ntry { [1, 2, 3].sort_by(fn(x) x.no_such_method()) } catch e { caught = true }\nprint(caught)",
    ),
    (
        "push_returns_the_array_so_it_chains",
        "print([1].push(2).push(3))",
    ),
    (
        "hash_size_matches_len",
        "let h = {\"a\": 1, \"b\": 2}\nprint(h.size())\nprint(h.len())",
    ),
    (
        "array_get_out_of_range_is_null",
        "print([10, 20].get(99))\nprint([10, 20].get(1))\nprint([10, 20].get(0 - 1))",
    ),
    (
        "comparison_errors_name_operands_in_source_order",
        "try { print([1] > 1) } catch e { print(e) }\ntry { print(1 > [1]) } catch e { print(e) }",
    ),
    (
        "substring_counts_characters_not_bytes",
        "print(\"日本語\".substring(0, 1))\nprint(\"é\".substring(0, 1))\nprint(\"abc\".substring(0, 2))",
    ),
    (
        "sum_min_max_handle_floats",
        "print([1.5, 2.5].sum())\nprint([1, 2.5].sum())\nprint([1.5].min())\nprint([1.5, 0.5].max())",
    ),
    (
        "fetch_raises_without_a_default_but_get_does_not",
        "let h = {\"a\": 1}\nprint(h.get(\"z\"))\nprint(h.fetch(\"z\", 9))\nlet caught = false\ntry { h.fetch(\"z\") } catch e { caught = true }\nprint(caught)",
    ),
    (
        "padding_rejects_an_absurd_width_instead_of_aborting",
        "let caught = false\ntry { \"x\".ljust(9223372036854775807) } catch e { caught = true }\nprint(caught)\nprint(\"x\".ljust(5))",
    ),
    (
        "truncate_is_not_capped_like_padding_is",
        "print(\"abcdef\".truncate(9223372036854775807))\nprint(\"abcdef\".truncate(3))",
    ),
    (
        "reduce_without_an_initial_value_seeds_from_the_first_element",
        "print([1, 2, 3].reduce(fn(a, b) a + b))\nprint([5].reduce(fn(a, b) a + b))\nprint([1, 2, 3].reduce(fn(a, b) a + b, 100))",
    ),
    (
        "a_non_function_where_a_callback_belongs_names_the_method",
        "let caught = false\ntry { [1, 2, 3].all?(null) } catch e { caught = true }\nprint(caught)",
    ),
    (
        "min_and_max_compare_strings_not_just_numbers",
        "print([\"a\", \"b\", \"c\"].max())\nprint([\"c\", \"b\", \"a\"].min())\nprint([1, 2].max())",
    ),
    (
        "return_from_inside_a_loop_does_not_corrupt_the_callers_loop",
        "fn inner(text) {\n  for line in text.split(\"|\") {\n    if len(line) > 2 { return line }\n  }\n  return \"none\"\n}\nfn run() {\n  let seen = []\n  for info in [{\"k\": \"one\"}, {\"k\": \"two\"}, {\"k\": \"three\"}] {\n    seen.push(info[\"k\"])\n    inner(\"aaa|bbb\")\n  }\n  return seen\n}\nprint(run())",
    ),
    (
        "nested_iteration_across_a_call_keeps_both_loops_intact",
        "fn count_up(n) {\n  let t = 0\n  for i in [1, 2, 3] {\n    if i == n { return t }\n    t = t + i\n  }\n  return t\n}\nlet out = []\nfor x in [\"a\", \"b\"] {\n  out.push(x)\n  count_up(2)\n}\nprint(out)",
    ),
    (
        "throw_from_inside_a_loop_does_not_corrupt_the_callers_loop",
        "fn inner(xs) {\n  for x in xs {\n    if x == \"b\" { throw \"boom\" }\n  }\n  return \"ok\"\n}\nlet seen = []\nfor info in [{\"k\": \"one\"}, {\"k\": \"two\"}, {\"k\": \"three\"}] {\n  seen.push(info[\"k\"])\n  try { inner([\"a\", \"b\", \"c\"]) } catch e { }\n}\nprint(seen)",
    ),
    (
        "throw_from_a_loop_caught_in_the_same_function",
        "let seen = []\nfor a in [\"x\", \"y\", \"z\"] {\n  seen.push(a)\n  try { for b in [1, 2, 3] { throw \"inner\" } } catch e { }\n}\nprint(seen)",
    ),
    // `next` compiles natively now (see the `next_*` cases above). This one
    // predates that and stays as the original regression: before any of this,
    // `next` compiled fine and was silently *ignored*, so the loop body ran for
    // every element.
    (
        "next_is_refused_rather_than_ignored",
        "let kept = []\nfor i in [1, 2, 3, 4] {\n  if i == 2 { next }\n  kept.push(i)\n}\nprint(kept)",
    ),
    // --- a thrown value must survive the call boundary it is thrown across ---
    // The interpreter used to stringify the payload when it left the function,
    // so `catch` bound the *rendering* and any structured access on it failed.
    (
        "throw_hash_across_call_keeps_hash",
        "def p() { throw {\"code\": 42} }\ntry { p() } catch e { print(e[\"code\"]) }",
    ),
    (
        "throw_int_across_call_keeps_int",
        "def p() { throw 42 }\ntry { p() } catch e { print(e + 1) }",
    ),
    (
        "throw_array_across_call_keeps_array",
        "def p() { throw [1, 2, 3] }\ntry { p() } catch e { print(e[1]) }",
    ),
    (
        "throw_across_two_call_frames",
        "def i() { throw {\"code\": 7} }\ndef o() { i() }\ntry { o() } catch e { print(e[\"code\"]) }",
    ),
    (
        "throw_rethrown_from_catch_keeps_value",
        "def p() { throw {\"a\": 1} }\ndef q() { try { p() } catch e { throw e } }\ntry { q() } catch e { print(e[\"a\"]) }",
    ),
    (
        "throw_from_loop_in_fn_keeps_value",
        "def p() { for i in [1, 2] { throw {\"n\": i} } }\ntry { p() } catch e { print(e[\"n\"]) }",
    ),
    (
        "throw_from_method_keeps_value",
        "class C { def m() { throw {\"m\": 3} } }\nlet c = C()\ntry { c.m() } catch e { print(e[\"m\"]) }",
    ),
    (
        "throw_from_lambda_keeps_value",
        "let f = fn() { throw {\"z\": 5} }\ntry { f() } catch e { print(e[\"z\"]) }",
    ),
    // --- nested sub-patterns compile ---
    // The last structural gap. A nested pattern is tested only after its
    // container extracted and bound the value it lives in, so an inner failure
    // unwinds with outer bindings already on the stack — which is why each fail
    // jump now carries its own live count instead of one number per arm.
    (
        "match_nested_hash_in_hash",
        "let data = {\"user\": {\"name\": \"Bob\"}}\nprint(match data { {user: {name: n}} => \"nested: \" + n, _ => \"no match\" })",
    ),
    (
        "match_nested_miss_falls_through",
        "let data = {\"user\": {\"other\": 1}}\nprint(match data { {user: {name: n}} => \"nested\", _ => \"no match\" })",
    ),
    (
        "match_literal_inside_array",
        "fn f(v) { return match v { [1, x] => \"one:\" + str(x), _ => \"no\" } }\nprint([f([1, 9]), f([2, 9])])",
    ),
    (
        "match_hash_inside_array",
        "print(match [{\"k\": 5}] { [{k: v}] => v, _ => 0 })",
    ),
    (
        "match_nested_with_guard",
        "let d = {\"u\": {\"age\": 30}}\nprint(match d { {u: {age: a}} if a > 18 => \"adult\", {u: {age: a}} => \"minor\", _ => \"?\" })",
    ),
    // --- enum-variant patterns compile ---
    // Class name and `__variant` tag are both checked before any payload is
    // bound. Payload field *names* come from the class's `__enum_variants`
    // metadata, which is only known once the instance is in hand — hence the
    // `EnumPayload` opcode rather than a compile-time property read.
    (
        "match_enum_variant_with_and_without_payload",
        "enum Status { Active, Pending(reason: String) }\nfn f(s) { return match s { Status.Active => \"Live\", Status.Pending(r) => \"Waiting: \" + r, _ => \"?\" } }\nprint([f(Status.Active), f(Status.Pending(\"later\"))])",
    ),
    (
        "match_enum_variant_two_payload_fields",
        "enum Shape { Circle(radius: Float), Rect(w: Float, h: Float) }\nfn area(s) { return match s { Shape.Circle(r) => 3.0 * r * r, Shape.Rect(w, h) => w * h, _ => 0.0 } }\nprint([area(Shape.Circle(2.0)), area(Shape.Rect(3.0, 4.0))])",
    ),
    (
        "match_enum_wrong_variant_falls_through",
        "enum Status { Active, Pending(reason: String) }\nfn f(s) { return match s { Status.Active => \"live\", _ => \"other\" } }\nprint(f(Status.Pending(\"x\")))",
    ),
    (
        "match_enum_non_enum_value_falls_through",
        "enum Status { Active }\nfn f(s) { return match s { Status.Active => \"live\", _ => \"other\" } }\nprint(f(42))",
    ),
    (
        "match_enum_variant_with_guard",
        "enum Shape { Circle(radius: Float) }\nfn f(s) { return match s { Shape.Circle(r) if r > 5.0 => \"big\", Shape.Circle(r) => \"small\", _ => \"?\" } }\nprint([f(Shape.Circle(9.0)), f(Shape.Circle(1.0))])",
    ),
    // --- hash patterns with named fields compile ---
    // Every key test runs before any binding is pushed, same provable shape as
    // the array form. A missing key falls through; extra keys are fine.
    (
        "match_hash_two_fields",
        "fn f(v) { return match v { {name: n, age: a} => n + \":\" + str(a), _ => \"other\" } }\nprint([f({\"name\": \"ada\", \"age\": 36}), f({\"name\": \"x\"}), f(7)])",
    ),
    (
        "match_hash_extra_keys_are_fine",
        "fn f(v) { return match v { {name: n} => \"got:\" + n, _ => \"no\" } }\nprint(f({\"name\": \"ada\", \"extra\": 1}))",
    ),
    (
        "match_hash_missing_key_falls_through",
        "fn f(v) { return match v { {name: n} => \"got\", _ => \"missing\" } }\nprint(f({\"other\": 1}))",
    ),
    (
        "match_hash_with_guard",
        "fn f(v) { return match v { {age: a} if a >= 18 => \"adult\", {age: a} => \"minor\", _ => \"no\" } }\nprint([f({\"age\": 30}), f({\"age\": 5}), f(1)])",
    ),
    (
        "match_mixed_literal_array_hash_arms",
        "fn f(v) { return match v { 1 => \"one\", [a, b] => \"arr\", {k: x} => \"hash:\" + str(x), _ => \"?\" } }\nprint([f(1), f([1, 2]), f({\"k\": 9}), f(\"z\")])",
    ),
    (
        "match_hash_with_rest",
        "fn f(v) { return match v { {name: n, ...rest} => n + \"/\" + str(rest), _ => \"no\" } }\nprint(f({\"name\": \"a\", \"b\": 1, \"c\": 2}))",
    ),
    (
        "match_hash_rest_is_empty_when_nothing_is_left",
        "fn f(v) { return match v { {name: n, ...rest} => str(rest), _ => \"no\" } }\nprint(f({\"name\": \"a\"}))",
    ),
    (
        "match_hash_rest_still_requires_the_named_keys",
        "fn f(v) { return match v { {name: n, ...rest} => \"has\", _ => \"no\" } }\nprint(f({\"other\": 1}))",
    ),
    // --- fixed-length array patterns compile ---
    // Every test runs before any binding is pushed, so a failing arm never has
    // to unwind a half-built set of bindings. `...rest` and non-binding
    // sub-patterns still defer — `array_rest_pattern_falls_back` pins that.
    (
        "match_array_two_elements",
        "fn f(v) { return match v { [a, b] => \"two:\" + str(a) + str(b), _ => \"other\" } }\nprint([f([1, 2]), f([1, 2, 3]), f(\"x\")])",
    ),
    (
        "match_array_wildcard_element",
        "fn f(v) { return match v { [_, b] => \"second:\" + str(b), _ => \"no\" } }\nprint([f([9, 8]), f([1])])",
    ),
    (
        "match_array_with_guard",
        "fn f(v) { return match v { [a, b] if a > b => \"desc\", [a, b] => \"asc\", _ => \"no\" } }\nprint([f([5, 1]), f([1, 5]), f([1])])",
    ),
    (
        "match_array_empty",
        "fn f(v) { return match v { [] => \"empty\", _ => \"not\" } }\nprint([f([]), f([1])])",
    ),
    (
        "match_array_head_and_rest",
        "fn f(v) { return match v { [first, ...rest] => \"first:\" + str(first) + \" rest:\" + str(rest), _ => \"no\" } }\nprint([f([1, 2, 3]), f([9]), f([])])",
    ),
    (
        "match_array_two_named_then_rest",
        "fn f(v) { return match v { [a, b, ...r] => str(a) + str(b) + \"/\" + str(r), _ => \"no\" } }\nprint([f([1, 2, 3, 4]), f([1, 2]), f([1])])",
    ),
    (
        "match_array_rest_with_guard",
        "fn f(v) { return match v { [a, ...r] if r.length > 1 => \"long\", [a, ...r] => \"short\", _ => \"no\" } }\nprint([f([1, 2, 3]), f([1]), f([])])",
    ),
    // --- typed patterns (`Int: n`) compile, and a non-exhaustive match raises ---
    (
        "match_typed_primitive_arms",
        "fn f(v) { return match v { Int: n => \"int:\" + str(n), String: s => \"str:\" + s, _ => \"other\" } }\nprint([f(3), f(\"hi\"), f(2.5)])",
    ),
    (
        "match_typed_with_guard",
        "fn f(v) { return match v { Int: n if n > 10 => \"big\", Int: n => \"small\", _ => \"no\" } }\nprint([f(50), f(2), f(\"x\")])",
    ),
    (
        "match_typed_bool_and_void",
        "fn f(v) { return match v { Bool: b => \"bool:\" + str(b), Void: x => \"null\", _ => \"?\" } }\nprint([f(true), f(null), f(1)])",
    ),
    (
        "match_typed_then_variable_arm",
        "fn f(v) { return match v { Int: n => \"int\", other => \"rest\" } }\nprint([f(1), f(\"x\")])",
    ),
    // A match that falls through every arm raised in the interpreter and
    // silently produced null in the VM. Both raise now.
    (
        "non_exhaustive_match_raises",
        "fn f(v) { return match v { 1 => \"one\" } }\nprint(f(9))",
    ),
    // --- binding match patterns compile at a clean stack position ---
    // The subject now lives in a real local slot, so `x => …` can bind it and
    // the arm collapses [subject, binding, result] to [result] via SetLocal.
    // Mid-expression binding arms wrap in a lambda so the subject has a slot.
    // `binding_pattern_as_call_argument` pins that both engines agree.
    (
        "match_binding_with_guard",
        "fn f(n) { return match n { x if x < 0 => \"neg\", 0 => \"zero\", x => \"pos\" + str(x) } }\nprint([f(-2), f(0), f(5)])",
    ),
    (
        "match_binding_used_twice_in_body",
        "fn f(n) { return match n { x => x + x } }\nprint(f(4))",
    ),
    (
        "match_binding_on_a_string_subject",
        "fn f(s) { return match s { \"hi\" => \"greeting\", other => \"got:\" + other } }\nprint([f(\"hi\"), f(\"zz\")])",
    ),
    (
        "match_binding_nested",
        "fn f(a, b) { return match a { x => match b { y => x * 10 + y } } }\nprint(f(3, 4))",
    ),
    (
        "match_literal_as_call_argument_still_compiles",
        "let out = []\nfor i in [1, 2, 3] { out.push(match i { 1 => \"a\", _ => \"b\" }) }\nprint(out)",
    ),
    (
        "binding_pattern_as_call_argument",
        "let out = []\nfor i in [1, 2, 3] { out.push(match i { 1 => \"a\", n => \"n\" + str(n) }) }\nprint(out)",
    ),
    // --- slot numbering around a `match` ---
    // The subject sits on the value stack while arm bodies compile but is NOT
    // registered in the compiler's locals, so a local declared in an arm could
    // plausibly be numbered one slot low. It is not — these pin that, because
    // compiling binding patterns means registering the subject as a local and
    // renumbering, and that work must preserve this rather than repair it.
    // See tasks/todo/vm-compile-binding-match-patterns.md.
    (
        "match_arm_reads_an_outer_local",
        "fn f() { let a = 5\n  let r = match 1 { 1 => a, _ => 0 }\n  return r }\nprint(f())",
    ),
    (
        "local_declared_after_a_match",
        "fn f() { let r = match 1 { 1 => 10, _ => 0 }\n  let b = 7\n  return r + b }\nprint(f())",
    ),
    (
        "match_inside_a_loop_with_locals",
        "fn f() { let total = 0\n  for i in [1, 2, 3] { let m = match i { 1 => 100, 2 => 200, _ => 300 }\n    total = total + m }\n  return total }\nprint(f())",
    ),
    (
        "two_matches_then_a_local",
        "fn f() { let a = match 1 { 1 => 1, _ => 0 }\n  let b = match 2 { 2 => 2, _ => 0 }\n  let c = 3\n  return a + b + c }\nprint(f())",
    ),
    (
        "match_arm_body_declares_a_local",
        "fn f() { let a = 9\n  let r = match 1 { 1 => { let inner = a * 2\n      inner }, _ => 0 }\n  return r }\nprint(f())",
    ),
    // --- re-`let` of a local in the same scope ---
    // The tree-walker allows it (`define_or_update` writes the existing
    // binding); the VM refused to compile, demoting the handler for code that
    // runs fine. Two of this repo's own spec files do it.
    (
        "re_let_same_scope",
        "fn f() { let x = 1\n  let x = 2\n  return x }\nprint(f())",
    ),
    (
        "re_let_changes_type",
        "fn f() { let x = 1\n  let x = \"two\"\n  return x }\nprint(f())",
    ),
    (
        "re_let_three_times",
        "fn f() { let x = 1\n  let x = 2\n  let x = 3\n  return x }\nprint(f())",
    ),
    (
        "re_let_seen_by_an_earlier_closure",
        "fn f() { let x = 1\n  let g = fn() { return x }\n  let x = 9\n  return g() }\nprint(f())",
    ),
    (
        "re_let_in_a_loop_body",
        "let out = []\nfor i in [1, 2] { let v = i\n  let v = i * 10\n  out.push(v) }\nprint(out)",
    ),
    (
        "shadowing_in_a_nested_block_is_unchanged",
        "fn f() { let x = 1\n  if true { let x = 2 }\n  return x }\nprint(f())",
    ),
    // --- safe navigation compiles natively now ---
    // It was refused (and before that `unimplemented!()`, which panicked and,
    // with panic="abort" in release, took the server with it). The subtle part
    // is that a null receiver must NOT evaluate the arguments.
    (
        "safe_nav_null_property",
        "let a = null\nprint(a&.name)",
    ),
    (
        "safe_nav_present_property",
        "let a = {\"name\": \"x\"}\nprint(a&.name)",
    ),
    (
        "safe_nav_null_method_call",
        "let a = null\nprint(a&.upcase())",
    ),
    (
        "safe_nav_present_method_call",
        "let a = \"hi\"\nprint(a&.upcase())",
    ),
    (
        "safe_nav_null_skips_arguments",
        "let a = null\nlet seen = []\nprint(a&.foo(seen.push(1)))\nprint(seen.length)",
    ),
    (
        "safe_nav_chained_through_null",
        "let a = {\"b\": null}\nprint(a&.b&.c)",
    ),
    (
        "safe_nav_chained_present",
        "let a = {\"b\": {\"c\": \"deep\"}}\nprint(a&.b&.c)",
    ),
    (
        "safe_nav_with_nullish_fallback",
        "let a = null\nlet r = (a&.name) ?? \"fallback\"\nprint(r)",
    ),
    (
        "safe_nav_in_a_loop",
        "let rows = [{\"n\": \"a\"}, null, {\"n\": \"c\"}]\nlet out = []\nfor r in rows { out.push(r&.n) }\nprint(out)",
    ),
    (
        "break_in_a_lambda_is_absorbed_at_the_function_boundary",
        "let seen = []\nfor n in [1, 2] { [10, 20, 30].each(fn(x) { break\n    seen.push(x) })\n  seen.push(n) }\nprint(seen)",
    ),
    (
        "next_in_a_lambda_is_absorbed_at_the_function_boundary",
        "let seen = []\nfor n in [1, 2] { [10, 20].each(fn(x) { next\n    seen.push(x) })\n  seen.push(n) }\nprint(seen)",
    ),
    // --- `next` compiles natively now ---
    // It was refused, so any loop using it demoted. Unlike `break` the iterator
    // stays (the same loop takes its next element), but the body's own locals
    // still have to come off or each skipped iteration would leave its locals
    // behind and the stack would grow for the life of the loop.
    (
        "next_in_for",
        "let kept = []\nfor i in [1, 2, 3, 4] { if i == 2 { next }\n  kept.push(i) }\nprint(kept)",
    ),
    (
        "next_with_parens",
        "let kept = []\nfor i in [1, 2, 3, 4] { if i == 2 { next() }\n  kept.push(i) }\nprint(kept)",
    ),
    (
        "next_in_while",
        "let kept = []\nlet n = 0\nwhile n < 5 { n = n + 1\n  if n == 3 { next }\n  kept.push(n) }\nprint(kept)",
    ),
    (
        "next_in_range_for",
        "let kept = []\nfor i in 1..6 { if i % 2 == 0 { next }\n  kept.push(i) }\nprint(kept)",
    ),
    (
        "next_with_body_locals",
        "let kept = []\nfor i in [1, 2, 3] { let d = i * 2\n  if i == 2 { next }\n  kept.push(d) }\nprint(kept)",
    ),
    (
        "next_with_index_variable",
        "let kept = []\nfor v, idx in [9, 8, 7] { if idx == 1 { next }\n  kept.push(idx) }\nprint(kept)",
    ),
    (
        "next_in_nested_loops",
        "let out = []\nfor a in [1, 2] { for b in [1, 2, 3] { if b == 2 { next }\n    out.push(a * 10 + b) } }\nprint(out)",
    ),
    (
        "next_and_break_together",
        "let out = []\nfor i in [1, 2, 3, 4, 5] { if i == 2 { next }\n  if i == 4 { break }\n  out.push(i) }\nprint(out)",
    ),
    (
        "next_with_closure_capture",
        "let fns = []\nfor i in [1, 2, 3] { if i == 2 { next }\n  fns.push(fn() { return i }) }\nprint([fns[0](), fns[1]()])",
    ),
    (
        "user_global_named_next_is_a_variable",
        "let next = 42\nprint(next + 1)",
    ),
    (
        "next_inside_try_falls_back",
        "let kept = []\nfor i in [1, 2, 3] { try { if i == 2 { next }\n    kept.push(i) } catch e { } }\nprint(kept)",
    ),
    // --- `break` compiles natively now ---
    // It used to be refused outright, so every one of these demoted. The hard
    // parts are what the jump site has to unwind: body locals (closing the
    // upvalue when a closure captured one, so per-iteration bindings stay
    // distinct) and the `for` iterator, which `ForIter` only pops when the
    // sequence runs out.
    (
        "break_in_for",
        "let out = []\nfor i in [1, 2, 3, 4, 5] { if i == 3 { break }\n  out.push(i) }\nprint(out)",
    ),
    (
        "break_in_while",
        "let n = 0\nwhile n < 100 { n = n + 1\n  if n == 4 { break } }\nprint(n)",
    ),
    (
        "break_in_range_for",
        "let out = []\nfor i in 1..10 { if i > 3 { break }\n  out.push(i) }\nprint(out)",
    ),
    (
        "break_inner_loop_only",
        "let out = []\nfor a in [1, 2] { for b in [10, 20, 30] { if b == 20 { break }\n    out.push(a * 100 + b) } }\nprint(out)",
    ),
    (
        "break_does_not_leak_the_iterator",
        "let first = []\nfor i in [1, 2, 3] { if i == 2 { break }\n  first.push(i) }\nlet second = []\nfor j in [7, 8, 9] { second.push(j) }\nprint([first, second])",
    ),
    (
        "break_with_body_locals",
        "let out = []\nfor i in [1, 2, 3] { let doubled = i * 2\n  let label = \"x\"\n  if i == 2 { break }\n  out.push(doubled) }\nprint(out)",
    ),
    (
        "break_with_closure_capture",
        "let fns = []\nfor i in [1, 2, 3] { fns.push(fn() { return i })\n  if i == 2 { break } }\nprint([fns[0](), fns[1]()])",
    ),
    (
        "break_with_index_variable",
        "let out = []\nfor v, idx in [9, 8, 7] { if idx == 2 { break }\n  out.push(idx) }\nprint(out)",
    ),
    // Still refused: jumping out would strand the handler that `try` pushed.
    (
        "break_inside_try",
        "let out = []\nfor i in [1, 2, 3] { try { if i == 2 { break }\n    out.push(i) } catch e { } }\nprint(out)",
    ),
    (
        "next_inside_try",
        "let kept = []\nfor i in [1, 2, 3, 4] { try { if i == 2 { next }\n    kept.push(i) } catch e { } }\nprint(kept)",
    ),
    (
        "break_inside_try_finally_runs_the_block",
        "let log = []\nfor i in [1, 2, 3] { try { if i == 2 { break }\n    log.push(i) } finally { log.push(\"f\") } }\nprint(log)",
    ),
    (
        "next_inside_try_finally_runs_the_block",
        "let log = []\nfor i in [1, 2, 3] { try { if i == 2 { next }\n    log.push(i) } finally { log.push(\"f\") } }\nprint(log)",
    ),
    (
        "break_out_of_nested_trys_runs_both_finallys",
        "let log = []\nfor i in [1, 2] { try { try { if i == 1 { break }\n      log.push(i) } finally { log.push(\"in\") } } finally { log.push(\"out\") } }\nprint(log)",
    ),
    (
        "break_inside_try_does_not_strand_the_handler",
        "let out = []\nfor i in [1, 2, 3] { try { if i == 2 { break }\n    out.push(i) } catch e { } }\ntry { throw \"after\" } catch e { out.push(\"caught\") }\nprint(out)",
    ),
    (
        "break_inside_try_does_not_leak_the_iterator",
        "let a = []\nfor i in [1, 2, 3] { try { if i == 2 { break }\n    a.push(i) } catch e { } }\nlet b = []\nfor j in [7, 8] { b.push(j) }\nprint([a, b])",
    ),
    // --- uncaught throws must stop the program, in both engines ---
    // The tree-walker's top-level loop discarded the control-flow result, so a
    // `throw` with no enclosing `try` evaporated and the NEXT STATEMENT RAN.
    (
        "top_level_throw_stops_the_program",
        "print(\"before\")\nthrow \"boom\"\nprint(\"AFTER-MUST-NOT-PRINT\")",
    ),
    // A `return` from inside a `try` skips the `TryEnd` that pops the handler,
    // so the VM kept a handler whose catch_ip pointed into a dead frame. The
    // next throw with no newer handler above it matched that corpse and the
    // exception vanished.
    (
        "return_from_try_does_not_leak_its_handler",
        "def leaky() -> String {\n  try { return \"early\" } catch e { return \"W\" }\n}\ndef boom() -> String { throw \"kaboom\" }\nprint(leaky())\nprint(boom())",
    ),
    (
        "leaked_handler_does_not_shadow_an_outer_catch",
        "def leaky() -> String {\n  try { return \"early\" } catch e { return \"W\" }\n}\ndef boom() -> String { throw \"kaboom\" }\nprint(leaky())\ntry { boom() } catch e { print(\"outer: \" + str(e)) }",
    ),
    (
        "repeated_handler_leaks_then_throw",
        "def leaky() -> String {\n  try { return \"e\" } catch e { return \"W\" }\n}\nlet i = 0\nwhile i < 5 { leaky()\n  i = i + 1 }\nprint(\"done\")\nthrow \"final\"",
    ),
    // --- ActiveRecord-style chainables on a materialized array ---
    // `has_many` accessors hand back a plain array, so a Rails-habit chain
    // lands on one. The tree-walker accepted these; the VM raised "Cannot
    // access property", so the chain passed `soli test` and 500'd in
    // production.
    (
        "array_order_by_field",
        "print([{\"n\": 2}, {\"n\": 1}].order(\"n\"))",
    ),
    (
        "array_order_desc_then_all",
        "print([{\"n\": 1}, {\"n\": 2}].order(\"n\", \"desc\").all())",
    ),
    (
        "array_order_missing_field_is_deterministic",
        "print([{\"n\": 2}, {\"x\": 9}, {\"n\": 1}].order(\"n\"))",
    ),
    (
        "array_all_and_includes_passthrough",
        "print([1, 2].all())\nprint([1, 2].includes())",
    ),
    // --- the peephole must remap `TryBegin`'s two targets ---
    // Fusing an instruction inside a `try` shifts every later offset. The
    // catch target was not remapped, so it landed one instruction into the
    // catch body. Depending on what fused, that skipped a local's initialiser
    // (garbage locals), skipped the handler entirely (the exception escaped an
    // enclosing `catch`), or ran off the end of the chunk and panicked.
    // Each case below puts a different fusable shape inside the try.
    (
        "peephole_try_local_prop",
        "def thrower() -> String { throw \"boom\" }\ndef cb() -> String {\n  try {\n    let h = {\"n\": 1}\n    let v = h.n\n    let r = thrower()\n    return \"try\"\n  } catch e {\n    let marker = \"GOOD\"\n    return \"catch:\" + marker\n  }\n}\nprint(cb())",
    ),
    (
        "peephole_try_incr_local",
        "def thrower() -> String { throw \"boom\" }\ndef cb() -> String {\n  try {\n    let i = 0\n    i = i + 1\n    let r = thrower()\n    return \"try\"\n  } catch e {\n    return \"CAUGHT\"\n  }\n}\nprint(cb())",
    ),
    (
        "peephole_try_mul_local_const",
        "def thrower() -> String { throw \"boom\" }\ndef cb() -> String {\n  try {\n    let a = 3\n    let b = a * 2\n    let r = thrower()\n    return \"try\"\n  } catch e {\n    let marker = \"GOOD\"\n    return \"catch:\" + marker\n  }\n}\nprint(cb())",
    ),
    (
        "peephole_try_nested_prop",
        "def thrower() -> String { throw \"boom\" }\ndef cb() -> String {\n  try {\n    let h = {\"a\": {\"b\": 2}}\n    let v = h.a.b\n    let r = thrower()\n    return \"try\"\n  } catch e {\n    let marker = \"GOOD\"\n    return \"catch:\" + marker\n  }\n}\nprint(cb())",
    ),
    (
        "catch_local_after_try_return_with_interpolated_call",
        "def thrower() -> Array {\n  throw \"boom\"\n}\ndef cb() -> String {\n  try {\n    let ok = thrower()\n    return \"try:#{ok.length}\"\n  } catch e {\n    let kind = \"CATCH-LOCAL\"\n    return \"catch:#{kind}\"\n  }\n}\nprint(cb())",
    ),
    // --- a throw inside a native callback keeps its value ---
    // Both engines destroyed it, differently: the interpreter replaced it with
    // a generic "Exception in array method", the VM with the rendering of the
    // value. `sort_by` swallowed it outright and returned the list unsorted.
    (
        "throw_in_map_keeps_value",
        "try { [1, 2, 3].map(fn(x) { throw {\"code\": 404} }) } catch e { print(e[\"code\"]) }",
    ),
    (
        "throw_in_filter_keeps_value",
        "try { [1, 2, 3].filter(fn(x) { throw {\"code\": 404} }) } catch e { print(e[\"code\"]) }",
    ),
    (
        "throw_in_reduce_keeps_value",
        "try { [1, 2, 3].reduce(fn(a, x) { throw {\"code\": 404} }, 0) } catch e { print(e[\"code\"]) }",
    ),
    (
        "throw_in_each_keeps_value",
        "try { [1, 2, 3].each(fn(x) { throw {\"code\": 404} }) } catch e { print(e[\"code\"]) }",
    ),
    (
        "throw_in_sort_by_is_not_swallowed",
        "let out = \"none\"\ntry { [3, 1, 2].sort_by(fn(x) { throw {\"code\": 404} }) } catch e { out = str(e[\"code\"]) }\nprint(out)",
    ),
    (
        "throw_in_hash_each_keeps_value",
        "try { {\"a\": 1}.each(fn(k, v) { throw {\"code\": 404} }) } catch e { print(e[\"code\"]) }",
    ),
    (
        "throw_in_int_times_keeps_value",
        "try { 3.times(fn(i) { throw {\"code\": 404} }) } catch e { print(e[\"code\"]) }",
    ),
    (
        "throw_class_instance_in_map_keeps_instance",
        "class MyErr { message: String\n  new(m: String) { this.message = m } }\ntry { [1].map(fn(x) { throw MyErr(\"in-map\") }) } catch e { print(e.message) }",
    ),
    // --- `finally` runs on every exit path ---
    // Both engines run these natively: the compiler inlines the `finally`
    // block on every edge that leaves the `try`. They pin the edges that were
    // wrong when it was emitted only after the catch clauses — cleanup skipped
    // on `return`, and a pending exception discarded when no catch matched.
    (
        "finally_runs_when_try_returns",
        "def p() {\n  try { return \"ret\" } finally { print(\"CLEANUP\") }\n}\nprint(p())",
    ),
    (
        "finally_does_not_swallow_exception",
        "def p() {\n  try { throw \"BOOM\" } finally { print(\"CLEANUP\") }\n  return \"SWALLOWED\"\n}\ntry { print(p()) } catch e { print(\"propagated\") }",
    ),
    (
        "finally_runs_when_catch_returns",
        "def p() {\n  try { throw \"x\" } catch e { return \"from-catch\" } finally { print(\"CLEANUP\") }\n}\nprint(p())",
    ),
    (
        "finally_return_replaces_pending_throw",
        "def p() {\n  try { throw \"T\" } finally { return \"from-finally\" }\n}\nprint(p())",
    ),
    (
        "finally_throw_replaces_pending_throw",
        "def p() {\n  try { throw \"T\" } finally { throw \"TF\" }\n}\ntry { p() } catch e { print(e) }",
    ),
    (
        "finally_cleanup_no_leak_on_early_return",
        "let handles = []\ndef p() {\n  handles.push(\"h\")\n  try { return \"early\" } finally { handles.pop() }\n}\nprint(p())\nprint(handles.length)",
    ),
    // --- top-level `return` ends the program ---
    //
    // The VM has always stopped; the tree-walker ignored it and ran on into
    // the code the guard was written to skip, so a script that guarded its own
    // work behaved one way under `soli run` and another under `soli serve`.
    (
        "toplevel_return_stops_the_program",
        "print(\"a\")\nif true { return }\nprint(\"unreachable\")",
    ),
    (
        "toplevel_return_inside_unless_guard",
        "let ready = false\nunless ready\n  print(\"not ready\")\n  return\nend\nprint(\"unreachable\")",
    ),
    (
        "toplevel_return_after_output",
        "print(1)\nprint(2)\nreturn\nprint(3)",
    ),
    // A `return` inside a function is a function return, not a program one —
    // pinned so the fix above cannot be over-applied.
    (
        "return_in_fn_is_not_a_program_return",
        "def f() { return 1 }\nprint(f())\nprint(\"still running\")",
    ),
    // String interpolation: every shape the fast paths special-case — a
    // lone string part (shared, not copied), a lone non-string part, results
    // on both sides of the 15-byte inline limit, floats on and off the ryu
    // path, a part that itself interpolates (re-entrant build buffer), and
    // non-scalar parts.
    (
        "interpolation_shapes",
        "let s = \"Alice\"\nlet long = \"Alice-the-quite-long-name-of-a-user\"\nprint(\"#{s}\")\nprint(\"#{42}\")\nprint(\"#{s}#{s}\")\nprint(\"Hi #{s}!\")\nprint(\"Dear #{long}, you have #{3} new messages\")\nlet f = 0.1 + 0.2\nprint(\"#{f}|#{1.0}|#{2.5}|#{-0.0}|#{19.99}|#{1.0 / 3.0}|#{100000000000000000.0}\")\nprint(\"#{true}-#{false}-#{null}-#{:sym}\")\nprint(\"a=#{[1, 2.5, \"x\"]} h=#{{\"k\": 1}}\")\ndef greet(n) { return \"hi #{n} and #{n.length}\" }\nprint(\"outer #{greet(s)} / #{greet(long)} end\")\nprint(\"#{s}\".length)",
    ),
    // A zero-parameter static method read without parentheses is called, as a
    // zero-parameter instance method is (`Assistant.history_messages`). The VM
    // returned the method itself, so arithmetic on it failed.
    (
        "static_zero_arg_method_read_bare",
        "class Limits\n  static def history\n    20\n  end\n  static def keep(n)\n    n * 2\n  end\nend\nprint(Limits.history + 1)\nprint(Limits.keep(4))\nlet kept = Limits.history\nprint(kept)",
    ),
    // `sort` with a comparator block. The VM's `sort` took no argument and
    // raised "expected 0, got 1"; int/float mixes use the numeric order too.
    (
        "sort_with_comparator",
        "names = [\"bb\", \"a\", \"ccc\"]\nlongest_first = names.sort do |a, b|\n  b.length() - a.length()\nend\nprint(longest_first)\nprint(names.sort(fn(a, b) a.length() - b.length()))\nprint([3, 1.5, 2].sort())\nprint([1, 2, 3].sort(fn(a, b) \"x\"))",
    ),
    // A function's value is its last statement's, and a trailing `if` /
    // `unless` yields its taken branch's. The VM returned null unless the last
    // statement was a plain expression — an action ending in
    // `if post … redirect("/") end` rendered its form instead of redirecting.
    (
        "implicit_return_through_trailing_if",
        "class Signup\n  def go(m)\n    @seen = true\n    if m == \"POST\"\n      done = 1\n      \"redirected\"\n    end\n  end\n  def pick(m)\n    if m == \"POST\"\n      \"yes\"\n    else\n      \"no\"\n    end\n  end\n  def nested(a, b)\n    unless a\n      \"not a\"\n    else\n      if b\n        \"a and b\"\n      end\n    end\n  end\nend\ndef top(m)\n  if m\n    \"top-yes\"\n  end\nend\ns = new Signup()\nprint(s.go(\"POST\"))\nprint(s.go(\"GET\"))\nprint(s.pick(\"GET\"))\nprint(s.nested(false, true))\nprint(s.nested(true, true))\nprint(s.nested(true, false))\nprint(top(true))\nprint(top(false))",
    ),
    // A missing key read with a dot is null on the tree-walker; the VM answered
    // a method object, which is truthy — `if rec._errors` was true on a hash
    // without `_errors`, and a successful `Model.update(id, attrs)` took the
    // error branch (a GRC app, 2.6.2). Builtin hash methods still read as
    // methods on both engines, with or without parentheses.
    (
        "hash_missing_key_dot_is_null",
        "h = {\"title\": \"x\", \"b\": 2}\nprint(h._errors)\nif h._errors\n  print(\"error branch\")\nelse\n  print(\"ok branch\")\nend\nprint(h.missing)\nprint(h.title)\nprint(h.keys)\nprint(h.length())\nprint(h.to_json())\nprint(h.dig(\"title\"))",
    ),
    // A model class reached through a variable — the generic
    // `tenant_add(klass, attrs)` shape: `klass.new(attrs)` raised "Cannot
    // access property 'new'" on the VM (2.6.2).
    (
        "model_new_through_a_variable",
        "class Thing < Model\nend\ndef build(klass, attrs)\n  rec = klass.new(attrs)\n  return rec\nend\nprint(build(Thing, {\"title\": \"hello\"}).title)\nprint(Thing.new({\"title\": \"x\"}).title)",
    ),
    // Model scopes from a module's `included` block, read bare, with empty
    // parens and with an argument. The VM had no scope lookup on a model class
    // (2.6.3). `to_query` builds the query without a database.
    (
        "model_scope_from_an_included_module",
        "module Scoped\n  included do\n    scope(\"owned\", fn() { this.where({\"owner\": \"o1\"}) })\n    scope(\"of_kind\", fn(kind) { this.where({\"kind\": kind}) })\n  end\nend\nclass Gadget < Model\n  include Scoped\nend\nprint(Gadget.owned.to_query)\nprint(Gadget.owned().to_query)\nprint(Gadget.of_kind(\"lamp\").to_query)",
    ),
    // The universal members on a hash, read bare. The VM's hash branch went
    // straight to the method list and the key, so `h.present?` was `null` and
    // a guard written `if bounce.present? return bounce end` let a
    // non-manager through (2.6.4).
    (
        "hash_universal_members_read_bare",
        "def probe(h)\n  return [h.present?, h.blank?, h.nil?, h.class, h.empty?, h.present?()]\nend\nprint(probe({\"status\": 302}))\nprint(probe({}))\nbounce = {\"status\": 302}\nif bounce.present?\n  print(\"bounced\")\nend",
    ),
    // `defined` / `const_get` on a loaded class, from inside a method. They
    // read the tree-walker's `CURRENT_ENV`, which the VM never set: an app
    // testing `defined("ShopChannel")` took its "absent" branch (2.6.4).
    (
        "defined_and_const_get_see_a_class",
        "class Foo\n  static def label -> String\n    return \"foo\"\n  end\nend\nclass Bar\n  static def probe -> String\n    return str(defined(\"Foo\")) + \" \" + str(defined(\"Nope\"))\n  end\n  static def fetch -> String\n    return const_get(\"Foo\").label()\n  end\nend\nprint(Bar.probe())\nprint(Bar.fetch())\nprint(defined(\"Foo\"))",
    ),
    // A closure that shrinks the array it is iterating. The tree-walker used
    // to snapshot the elements first (so `each` emptied the array) while the
    // VM stopped at the live length. Both now follow the VM. The tail also
    // checks string append, which both engines must print as "abc".
    (
        "array_mutation_during_iteration",
        "a = [1, 2, 3]\na.each(fn(x) { a.pop() })\nprint(a)\nb = [1, 2, 3]\nprint(b.map(fn(x) { b.pop(); x }))\nprint(b)\nc = [1, 2, 3]\nprint(c.reduce(fn(acc, x) { c.pop(); acc + x }, 0))\nbuf = \"\"\nbuf = buf + \"a\"\nbuf = buf + \"b\"\nbuf << \"c\"\nprint(buf)",
    ),
    // `<<` on a variable a closure captured. The VM sent an upvalue to the
    // generic ArrayPush ("can only push to arrays") while the tree-walker
    // appended.
    (
        "string_shovel_on_a_captured_variable",
        "def run()\n  buf = \"a\"\n  add = fn() { buf << \"x\" }\n  add()\n  add()\n  return buf\nend\nprint(run())",
    ),
    // The variable is read before the right-hand side, as for any binary
    // operator — so a right-hand side that rebinds it does not change which
    // value is appended to. Both engines, for `<<`, `+` and `+=`.
    (
        "append_reads_the_variable_before_the_right_hand_side",
        "def run()\n  items = [1]\n  swap = fn() { items = [9]; 2 }\n  items << swap()\n  s = \"ab\"\n  f = fn() { s = \"zz\"; \"!\" }\n  s = s + f()\n  t = \"cd\"\n  g = fn() { t = \"yy\"; \"?\" }\n  t << g()\n  u = \"ef\"\n  h = fn() { u = \"ww\"; \".\" }\n  u += h()\n  return [items, s, t, u]\nend\nprint(run())",
    ),
    // `name = name + <expr>` and `name += <expr>` with a right-hand side the
    // peepholes do not cover: one fused op on the VM, for every type `+` takes.
    (
        "add_assign_with_a_computed_right_hand_side",
        "def run()\n  total = 0\n  total = total + [1, 2].length\n  total += 3 * 2\n  f = 1.5\n  f = f + [1].length\n  arr = [1]\n  arr = arr + [[2].first]\n  label = \"n=\"\n  label = label + str(total)\n  label += \"#{f}\"\n  up = \"u\"\n  bump = fn() { up = up + \"#{1 + 1}\"; up += str(3) }\n  bump()\n  alias = label\n  label += \"!\"\n  return [total, f, arr, label, alias, up]\nend\nprint(run())\ng = \"g\"\ng = g + \"#{1}\"\ng += str(2)\nprint(g)",
    ),
    // --- private methods and bare calls on self ---
    (
        "private_calls_on_self",
        "class Invoice\n  total: Int\n\n  new(total: Int)\n    @total = total\n  end\n\n  def via_at\n    @_tax(2)\n  end\n\n  def via_bare\n    _tax(2)\n  end\n\n  def via_bare_zero\n    _label\n  end\n\n  def via_bare_zero_parens\n    _label()\n  end\n\n  def via_block\n    [1, 2].map { |n| _tax(n) }\n  end\n\n  def other(invoice)\n    invoice._tax(2) rescue \"refused\"\n  end\n\n  private\n\n  def _tax(rate)\n    @total * rate\n  end\n\n  def _label\n    \"invoice #{@total}\"\n  end\nend\ninv = new Invoice(10)\nprint(inv.via_at)\nprint(inv.via_bare)\nprint(inv.via_bare_zero)\nprint(inv.via_bare_zero_parens)\nprint(inv.via_block)",
    ),
    (
        "private_refused_from_outside",
        "class Invoice\n  total: Int\n\n  new(total: Int)\n    @total = total\n  end\n\n  def via_at\n    @_tax(2)\n  end\n\n  def via_bare\n    _tax(2)\n  end\n\n  def via_bare_zero\n    _label\n  end\n\n  def via_bare_zero_parens\n    _label()\n  end\n\n  def via_block\n    [1, 2].map { |n| _tax(n) }\n  end\n\n  def other(invoice)\n    invoice._tax(2) rescue \"refused\"\n  end\n\n  private\n\n  def _tax(rate)\n    @total * rate\n  end\n\n  def _label\n    \"invoice #{@total}\"\n  end\nend\ninv = new Invoice(10)\nprint(inv._tax(2) rescue \"refused\")\nprint(inv._label rescue \"refused\")\nprint(inv.other(new Invoice(3)))",
    ),
    (
        "private_modifier_covers_one_method",
        "class A\n  private def secret\n    1\n  end\n\n  def open\n    2\n  end\nend\na = new A()\nprint(a.open)\nprint(a.secret rescue \"refused\")",
    ),
    (
        "private_bare_call_from_subclass",
        "class Base\n  private\n\n  def _double(n)\n    n * 2\n  end\nend\n\nclass Child < Base\n  def run\n    _double(21)\n  end\nend\nprint(new Child().run)",
    ),
    (
        "implicit_self_local_and_global_win",
        "def shared\n  \"global\"\nend\n\nclass C\n  def run\n    label = \"local\"\n    [label, shared()]\n  end\n\n  def label\n    \"method\"\n  end\n\n  def shared\n    \"method\"\n  end\nend\nprint(new C().run)",
    ),
];
/// Cases that currently diverge because of an unfixed VM bug. Keep this list in
/// sync with reality: when a fix lands, the corresponding case starts matching
/// and the test will tell you to remove it from here.
const KNOWN_DIVERGENT: &[&str] = &[
    // Empty: sub-expression comprehensions and binding `match` as a call
    // argument now compile via a clean-frame lambda (`Compiler::wrap_in_lambda`).
    // Binding patterns at a statement position already compiled — see
    // `match_binding_*`.
    //
    // Fixed and locked in by this harness:
    //   #5  for-with-index (ForIter index)   — compiler now maintains the counter
    //   #6  assignment inside catch          — TryBegin catch_ip off-by-one
    //   #7  range bounds (a..b exclusive)     — VM range ops now exclusive
    //   #8  `||=` panic (let-from-local)      — removed unsafe GetLocal2 fusion
    //   #10 return inside catch               — TryBegin catch_ip off-by-one
    //   #14 instance_method_call              — VmClosure methods stored on
    //        Class.vm_methods; ctor ("init") + methods dispatch with the
    //        receiver in the callee slot as `this`
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
