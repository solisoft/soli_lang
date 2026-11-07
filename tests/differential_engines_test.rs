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
        // `print`/`puts`/`println` resolve a Future (here a command
        // substitution) before printing it, as the tree-walker's builtins do;
        // the VM's `Op::Print` printed `<pending future>`.
        "print_resolves_a_future",
        "r = `printf hi`\nprint(r)\nputs(r, 1)\nprintln(`printf yo`)",
    ),
    (
        // The VM's `count` only took a `Function`, so a block (a VmClosure)
        // fell through to the equality count and gave 0; the tree-walker's
        // reject/none?/one?/count dropped a `throw` raised in the block.
        "array_block_predicates_count_and_propagate_throw",
        "xs = [1, 2, 3]\nprint(xs.count { |x| x > 1 })\nprint(xs.reject { |x| x > 1 })\nprint(xs.none? { |x| x > 5 })\nprint(xs.one? { |x| x == 2 })\ntry { xs.reject { |x| throw \"reject\" } } catch e { print(e) }\ntry { xs.none? { |x| throw \"none\" } } catch e { print(e) }\ntry { xs.one? { |x| throw \"one\" } } catch e { print(e) }\ntry { xs.count { |x| throw \"count\" } } catch e { print(e) }",
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
    (
        // A range in parentheses is counted through, as a bare one is, by a
        // `for` loop and by comprehensions: the same elements, index, closure
        // captures, `next` and `break`. A range used as a value is an array.
        "parenthesized_range_iterates_like_a_bare_one",
        r##"def total(n)
  sum = 0
  for i in (0..n)
    sum += i
  end
  sum
end

print(total(5), total(0), total(-3))
for i in ((2..5))
  print(i)
end
for value, index in (10..13)
  print(value, index)
end
fns = []
for i in (0..3)
  fns.push(fn() { i * 10 })
end
print(fns.map { |f| f() })
for i in (0..20)
  next if i % 3 != 0
  break if i > 10

  print(i)
end
low = 3
high = 7
print([i * i for i in (low..high)], [i for i in ((0..10)) if i % 4 == 0])
print({(i): i + 1 for i in (1..4)}, [i for i in (5..1)], {(i): i for i in (2..2)})
print([[i, i * 2] for i in (0..2)], [j for j in (high..high + 2)])
numbers = (1..4)
print(numbers, numbers.length, (1..4).each { |x| x }, (0..3).map { |x| x * 2 })"##,
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
    (
        // A self-assignment leaves every local as it was. The VM fused
        // `GetLocal(s), SetLocal(s)` into `SwapSetLocal`, which popped a value
        // the pair never pushed: two locals were swapped, and reading the last
        // one indexed past the stack (a panic).
        "self_assignment_keeps_every_local",
        r##"def keep
  a = 1
  b = 2
  a = a
  [a, b]
end

def keep_last
  a = 1
  b = 2
  a = a
  [b, a]
end

def keep_param(p, q)
  p = p
  [p, q]
end

def value_of
  x = 5
  y = (x = x)
  [x, y]
end

def in_loop(n)
  x = 0
  i = 0
  while i < n
    i += 1
    x = i
    x = x
  end
  x
end

def relet
  let a = 1
  let b = 2
  let a = a
  [a, b]
end
print(keep(), keep_last(), keep_param(3, 4), value_of(), in_loop(3), relet())"##,
    ),
    (
        // An assignment statement's `SetLocal, Pop` runs as one `SetLocalPop`
        // on the VM, which moves the value into the slot instead of copying
        // it there and dropping the copy. Each store shape: a heap value
        // shared with another local, `a = a op b`, `+=`, a re-`let`, a value a
        // branch merges into (`||`, `&&`, `??`, a ternary, a `match`,
        // `rescue`, `||=`), a chained store, a local a closure reads, and a
        // store whose `Pop` a short-circuit jumps to, which keeps the `Pop`.
        "assignment_statements_move_the_value_into_the_slot",
        r##"def boom
  throw "boom"
end

def stores(a, b, c, v)
  s = "heap"
  t = s
  t += "!"
  arr = [1]
  same = arr
  same.push(2)
  h = {"k": 1}
  g = h
  g["j"] = 2
  k = 7
  z = 2
  k = k - z
  k = k * 3
  k = k / 5
  k += 2
  w = k - z
  let r = 1
  let r = r + 1
  x = a || b
  y = a && b
  q = a ?? 3
  tern = c ? "yes" : "no"
  m = match v {
    1 => "one",
    n => "n=#{n}"
  }
  res = boom() rescue "rescued"
  n0 = 0
  n0 ||= 5
  p = q = 4
  [s, t, arr, same, h, g, k, w, r, x, y, q, tern, m, res, n0, p]
end
print(stores(nil, 2, true, 1))
print(stores(4, 2, false, 7))

def short(ok)
  x = 0
  ok && (x = 1)
  ok || (x = x + 10)
  ok ? (x = x * 2) : (x = x - 1)
  let y = 5
  [x, y]
end
print(short(false), short(true))

def captured
  x = 1
  read = fn() { x }
  x = 2
  read()
end
print(captured())

def loop_store(n)
  last = nil
  i = 0
  while i < n
    i += 1
    last = i * 10
  end
  last
end
print(loop_store(3))"##,
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
        // `h["k"] = v` / `h.set("k", v)` leaves `v` on the stack, so a
        // comprehension or binding `match` later in the expression must take
        // the clean-frame lambda. The VM counted the constant-key set opcodes
        // at -1, saw a clean stack, and built the result in `v`'s slot.
        "index_assignment_before_a_comprehension_or_match",
        r##"h = {}

def run
  g = {}
  xs = [1, 2]
  a = [g["k"] = 9, [x * 10 for x in xs]]
  b = [g.set("j", 8), [x + 1 for x in xs if x > 1]]
  c = [h["k"] = 7, {x: x * 2 for x in xs}]
  d = [g["m"] = 1, match xs { [p, q] => p + q, _ => 0 }]
  e = [h.set("n", 6), [x for x in xs]]
  [a, b, c, d, e, g, h]
end
print(run())
r = [h["t"] = 5, [x * 2 for x in [3, 4]]]
print(r)"##,
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
    // A bare local whose first mention is its own top-level assignment is
    // declared there, as `let` is, instead of hoisted to function entry. The
    // later reads and writes — in branches, loops, blocks, closures — must
    // reach the same binding.
    (
        "bare_locals_declared_in_place_captured_by_a_block",
        "def summarize(items)\n  count = items.length\n  total = 0\n  items.each do |item|\n    total += item\n  end\n  average = total / count\n  label = \"avg\"\n  \"#{label}=#{average} n=#{count}\"\nend\nprint(summarize([2, 4, 6]))",
    ),
    (
        "bare_locals_declared_in_place_reassigned_in_branches_and_loops",
        "def classify(n)\n  label = \"small\"\n  if n > 10\n    label = \"big\"\n  end\n  i = 0\n  while i < n\n    i += 1\n  end\n  steps = i\n  \"#{label}:#{steps}\"\nend\nprint(classify(3))\nprint(classify(12))",
    ),
    (
        "bare_locals_declared_in_place_in_methods_constructors_and_blocks",
        "class Invoice\n  total: Int\n  new(amount)\n    tax = amount / 10\n    @total = amount + tax\n  end\n  def summary(rate)\n    base = @total\n    extra = base * rate\n    discounted = base + extra - 1\n    \"#{base}/#{extra}/#{discounted}\"\n  end\nend\ninv = new Invoice(100)\nprint(inv.summary(2))\nresult = [1, 2, 3].map do |x|\n  doubled = x * 2\n  doubled + 1\nend\nprint(result)",
    ),
    (
        "bare_locals_declared_in_place_tail_chained_and_closed_over",
        "def tail(v)\n  y = v * 3\nend\nprint(tail(4))\ndef shadow\n  sq = fn(x) { x * x }\n  x = 3\n  sq(x)\nend\nprint(shadow())\ndef counter\n  count = 0\n  bump = fn() { count = count + 1 }\n  bump()\n  bump()\n  count\nend\nprint(counter())\ndef chained(v)\n  a = b = v + 1\n  a + b\nend\nprint(chained(1))",
    ),
    // --- try / catch / finally ---
    // --- value objects and domain errors ---
    // Construction, freeze, value equality, invariants, `with`, `to_h`, and
    // the error each refusal raises — the same text on both engines.
    (
        "value_object_equality_freeze_invariants",
        r##"class Amount < ValueObject
  cents: Int
  currency: String = "EUR"
  invariant("cents must not be negative") { @cents >= 0 }
end
a = new Amount(500, "EUR")
b = new Amount(500)
c = new Amount(cents: 700, currency: "USD")
print(a == b)
print(a == c)
print([a, c].includes?(new Amount(700, "USD")))
print([a, b, c].uniq.length)
print(a.to_h)
try
  a.cents = 1
catch e
  print(e)
end
try
  new Amount(-5)
catch InvariantViolation e
  print(e.message)
  print(e.details["class"])
end
try
  new Amount(1, "EUR", 3)
catch e
  print(e)
end
print(a)
print(a.to_s)
print("#{c}")"##,
    ),
    (
        "value_object_with_and_constructor",
        r##"class Amount < ValueObject
  cents: Int
  currency: String = "EUR"
  invariant(:cents, "cents must not be negative") { @cents >= 0 }

  def plus(other)
    Amount.new(@cents + other.cents, @currency)
  end
end
class Email < ValueObject
  address: String
  new(address: String)
    @address = address.trim.downcase
  end
  invariant("must contain @") { @address.includes?("@") }
end
a = Amount.new(500)
b = a.with(cents: 600)
print(a.cents, b.cents, b.currency)
c = a.with({"currency": "USD"})
print(c.to_h)
print(a.plus(b) == Amount.new(1100, "EUR"))
e = Email.new("  Ada@Example.COM ")
print(e.address)
print(e.with(address: "BOB@x.io").address)
try
  a.with(cents: -1)
catch InvariantViolation err
  print(err.message, err.details["field"])
end
try
  a.with(cent: 3)
catch err
  print(err)
end
try
  Email.new("nope")
catch DomainError err
  print(err.message)
end"##,
    ),
    (
        // A method of a top-level class reads its nested classes unqualified,
        // in both spellings, at any depth (the tree-walker raised "Undefined
        // variable 'Inner'"); `super` still reaches the parent.
        "nested_class_read_unqualified_in_its_outer_class",
        r##"class Shape
  def kind
    "shape"
  end
end
class Outer < Shape
  outer_value: Int

  new(value: Int)
    @outer_value = value
  end

  def create_inner(x: Int)
    Inner.new(this, x)
  end

  def create_old(x: Int)
    new Inner(this, x)
  end

  def parent_kind
    super.kind() + "!"
  end

  class Inner
    outer: Any
    inner_value: Int

    new(outer, value: Int)
      @outer = outer
      @inner_value = value
    end

    def combined -> Int
      @outer.outer_value + @inner_value
    end

    def deeper
      Deep.new(@inner_value)
    end

    class Deep
      v: Int
      new(v)
        @v = v
      end
    end
  end
end
outer = Outer.new(10)
print(outer.create_inner(20).combined())
print(outer.create_old(1).combined())
print(outer.parent_kind())
print(outer.create_inner(3).deeper().v)
print(Inner.new(1, 2)) rescue print("Inner is not global")"##,
    ),
    (
        // A call to a method an instance does not have reaches its
        // `method_missing` (inherited too); reflection reads see the VM's
        // compiled methods. The VM raised "Cannot access property" on all of
        // them.
        "instance_method_missing_and_reflection",
        r##"class Base
  def inherited
    0
  end

  def method_missing(name, args)
    "#{name}/#{args.length}"
  end
end
class Plain < Base
  x: Int
  new
    @x = 5
  end
  def real
    1
  end
end
class Two
  def method_missing(name, a, b)
    "#{name}:#{a}:#{b}"
  end
end
p = Plain.new()
print(p.anything(1, 2, 3), p.nothing(), p.real())
print(Two.new().foo(1), Two.new().foo(1, 2, 3))
print(p.respond_to?("real"), p.respond_to?("inherited"), p.respond_to?("nope"))
print(p.inspect(), p.methods().sort(), p.instance_variables())
print(p.instance_variable_get("x"))
p.instance_variable_set("x", 7)
print(p.x)"##,
    ),
    (
        // `...xs` in an array literal appends xs's elements (the VM used to
        // push the array itself: `[0, ...a]` was `[0, [1, 2]]`).
        "array_literal_spread",
        r##"a = [1, 2]
b = [3]
print([0, ...a])
print([...a, ...b, 4])
print([...[]], [...a, ...a])
c = [...a]
c.push(9)
print(a, c)
def wrap(xs)
  [:start, ...xs, :end]
end
print(wrap([1]))
print([1, 2].map { |x| [...a, x] })
print([...nil] rescue "refused")"##,
    ),
    (
        // `Klass.new(...)` is `new Klass(...)` on a class with a constructor,
        // whatever the arguments — none, one hash, named only. A class
        // without one keeps the attribute form; a module is not instantiated.
        "klass_new_runs_the_constructor",
        r##"class InsufficientFunds < DomainError
end
try
  throw InsufficientFunds.new("balance too low", {"needed": 30})
catch DomainError e
  print(e.message, e.details["needed"])
end
class Policy
  user: Any
  record: Any
  new(context)
    @user = "ctor:" + str(context)
  end
end
p = Policy.new({"user": "ada", "record": 1})
print(p.user, p.record)
q = Policy.new(user: "bob")
print(q.user)
r = Policy.new("ctx")
print(r.user)
class Amount < ValueObject
  cents: Int
  currency: String = "EUR"
end
print(Amount.new({"cents": 5}).to_h)
print(Amount.new(cents: 7).to_h)
print(Amount.new(9, "USD").to_h)
print(Amount.new({"cents": 5}) == Amount.new(5))
class Counter
  count: Int
  new
    @count = 10
  end
end
print(Counter.new().count)
class Point
  x: Int
  y: Int
  new(x = 0, y = 0)
    @x = x
    @y = y
  end
end
print(Point.new().x, Point.new(y: 4).y, Point.new(y: 4).x)
class Child < Point
end
print(Child.new(y: 6).y, Child.new().x)
class Bag
  a: Any
end
print(Bag.new({"a": 1}).a, Bag.new(a: 2).a, type(Bag.new()))
module Greetable
end
print(Greetable.new() rescue "module")"##,
    ),
    (
        // Outside a transaction `after_commit` runs its block at once, in
        // all three spellings; a domain event is a value object.
        "after_commit_outside_a_transaction_and_domain_events",
        r##"log = []
after_commit { log.push("brace") }
after_commit do
  log.push("do")
end
after_commit(fn() { log.push("fn") })
print(log)
class OrderPaid < DomainEvent
  number: String
end
print(OrderPaid.new("A") == OrderPaid.new(number: "A"))
print(OrderPaid.new("A"))
try
  publish("not an event")
catch e
  print("refused")
end"##,
    ),
    (
        // A class nested in a module: qualified name, `::` on both engines
        // (the VM used to resolve it to the module), a reopened module.
        "nested_classes_and_reopened_modules",
        r##"module Commerce
  class Product
    name: String
    price: Float
    new(name: String, price: Float)
      @name = name
      @price = price
    end
    def label -> String
      "#{@name}: #{@price}"
    end
  end
end
module Commerce
  class Cart
    def size
      0
    end
  end
  def self.version
    2
  end
end
print(Commerce::Product)
print(new Commerce::Product("Book", 12.5).label)
print(Commerce::Product.new("Mug", 3.0).label)
print(Commerce::Cart.new().size, Commerce.version)
print(Commerce::Product.new("Pen", 1.0).class)
print(Commerce.Product.new("Dot", 2.0).label)"##,
    ),
    (
        // Inside a module its classes are read unqualified — in a static
        // method, an instance method, a block, a module function.
        "unqualified_names_resolve_in_their_module",
        r##"module Billing
  class Invoice
    total: Int
    new(total: Int)
      @total = total
    end
  end

  class PayInvoice
    static def call(total: Int)
      Invoice.new(total)
    end

    def inside_block
      [1, 2].map { |t| Invoice.new(t).total }
    end
  end

  def self.make
    Invoice.new(9)
  end
end

print(Billing::PayInvoice.call(5).total)
print(Billing::PayInvoice.new().inside_block)
print(Billing.make.total)
print(Billing::PayInvoice.call(5).class)"##,
    ),
    (
        // A declared return type accepts a subclass, and a class of the
        // module named by its short name.
        "return_types_accept_subclasses_and_short_names",
        r##"class A
end
class B < A
end
def make -> A
  B.new()
end
print(make().class)
module M
  class C
  end
  class F
    static def build -> C
      C.new()
    end
  end
end
print(M::F.build.class)"##,
    ),
    (
        // Every kind of declared return type admits what the tree-walker
        // admits, in any case, and refuses what it refuses. The VM checks
        // against the annotation lowered once at compile time; it refused
        // its own closures for `-> Function` and `-> (Int) -> Int`.
        "return_types_admit_every_kind",
        r##"class Pt
end
class Pt3 < Pt
end
def ri -> Int
  1
end
def rf -> float
  1.5
end
def rd -> decimal
  1.5D
end
def rs -> STRING
  "s"
end
def rb -> Bool
  true
end
def ra -> Array
  [1]
end
def rh -> Hash
  {"a": 1}
end
def rv -> Void
  nil
end
def rn -> Int?
  nil
end
def rany -> Any
  "x"
end
def rp -> Pt
  Pt3.new()
end
def rfn -> Function
  return fn(x) { x * 2 }
end
def rft -> (Int) -> Int
  return fn(x) { x + 1 }
end
def rint_arr -> Int[]
  [1, 2]
end
print([ri(), rf(), rd(), rs(), rb(), ra(), rh(), rv(), rn(), rany(), rp().class])
print(rfn()(4))
print(rft()(4))
print(rint_arr())
def bad_hash -> Hash
  [1]
end
def bad_class -> Pt
  "nope"
end
def bad_void -> Void
  3
end
def bad_nullable -> Pt?
  4
end
print(bad_hash() rescue "refused hash")
print(bad_class() rescue "refused class")
print(bad_void() rescue "refused void")
print(bad_nullable() rescue "refused nullable")"##,
    ),
    (
        // A value object keys a hash by value.
        "value_objects_as_hash_keys",
        r##"class Amount < ValueObject
  cents: Int
  currency: String = "EUR"
end
h = {}
h[Amount.new(500)] = "five"
h[Amount.new(700, "USD")] = "seven"
print(h[Amount.new(500, "EUR")])
print(h.has_key(Amount.new(700, "USD")), h.has_key(Amount.new(1)))
print(h.keys[0], h.keys[0] == Amount.new(500))
h[Amount.new(500)] = "replaced"
print(h.length, h[Amount.new(500)])
tally = {}
[Amount.new(1), Amount.new(2), Amount.new(1)].each do |a|
  tally[a] = (tally[a] ?? 0) + 1
end
print(tally[Amount.new(1)], tally.length)
print(h)"##,
    ),
    (
        // A private class of a module: reachable inside it (unqualified or
        // qualified), refused outside with ::, and with a dot.
        "private_classes_stay_in_their_module",
        r##"module Billing
  private class Ledger
    static def balance -> Int
      42
    end
  end

  class Report
    static def total -> Int
      Ledger.balance + Billing::Ledger.balance
    end
  end

  def self.peek -> Int
    Ledger.balance
  end
end

print(Billing::Report.total)
print(Billing.peek)
try
  Billing::Ledger.balance
catch e
  print(e)
end
try
  Billing.Ledger.balance
catch e
  print(e)
end"##,
    ),
    // A typed last clause that matches runs its body and continues: the VM
    // used to fall into the no-match `Rethrow` after the body.
    (
        "typed_catch_lone",
        "try\n  throw new ValueError()\ncatch ValueError e\n  print(\"caught\")\nend\nprint(\"after\")",
    ),
    (
        "typed_catch_last_of_two_matches",
        "class A < Error\nend\nclass B < Error\nend\ntry\n  throw new B()\ncatch A e\n  print(\"a\")\ncatch B e\n  print(\"b\")\nend\nprint(\"after\")",
    ),
    (
        "typed_catch_with_finally",
        "fn f() { let log = []\n  try { throw new ValueError() } catch ValueError e { log.push(\"c\") } finally { log.push(\"f\") }\n  return log }\nprint(f())",
    ),
    (
        "typed_catch_no_match_rethrows",
        "try\n  try\n    throw new KeyError()\n  catch ValueError e\n    print(\"wrong\")\n  end\ncatch KeyError e\n  print(\"outer\")\nend",
    ),
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
        // `h.key = v` and constant-key `h["key"] = v` writes: overwrites keep
        // their position, new keys (short, and longer than an inline
        // `SoliStr`) append, on a global, a local, a block's upvalue and a
        // nested hash, across the promotion past eight keys. The VM's hash
        // dot write moved into the fast tier, keyed by the pool's `SoliStr`.
        "hash_constant_key_writes",
        r##"h = {"a": 1}
h.a = 2
h.b = 3
h.a_key_longer_than_fifteen = 4
h["c"] = 5
h["a"] = 6
h["another_key_longer_than_15"] = 7
print(h)
def build(n)
  out = {}
  out.first = n
  out["second"] = n + 1
  out.first = out.first + 10
  [1, 2].each { |k| out.from_block_key_long_name = k }
  i = 0
  while i < 10
    out["k#{i}"] = i
    i = i + 1
  end
  out.after_promotion = true
  out.first = 0
  out
end
print(build(5))
o = {"inner": {"x": 1}}
o.inner.y = 2
o["inner"]["z"] = 3
o["inner"]["x"] = 9
print(o)
"##,
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
        // `super(...)` reaching a constructor K2 inherits from K1: it runs as
        // K1's, so its own `super(...)` goes on to K0 rather than back to K1.
        // The served form of the same program is in APP_CASES.
        "super_into_an_inherited_constructor",
        r##"class K0
  k0: Int
  new(a)
    @k0 = a
  end
end
class K1 < K0
  k1: Int
  new(a)
    super(a + 1)
    @k1 = a
  end
end
class K2 < K1
end
class K3 < K2
  k3: Int
  new(a)
    super(a * 2)
    @k3 = a
  end
end
k = K3.new(3)
print(k.k0, k.k1, k.k3, K2.new(10).k0)"##,
    ),
    (
        // The documented custom-error shape: a constructor passing its
        // message and details up to `Error`'s, thrown, caught and rescued.
        "error_subclass_constructor_passes_its_message_up",
        r##"class AppError < Error
end
class Overdrawn < AppError
  amount: Int
  new(amount: Int)
    super("overdrawn by #{amount}", {"amount": amount})
    @amount = amount
  end
end
try
  throw Overdrawn.new(7)
catch err
  print(err.class, err.message, err.details["amount"], err.amount)
end
e = Overdrawn.new(9) rescue "rescued"
print(e.class)"##,
    ),
    (
        // `super` in a block inside a method. The tree-walker resolves it
        // from the class the block was written in; the VM reads the class off
        // the running frame, and a block's frame has none ("super used
        // outside of a subclass method"). Tracked in KNOWN_DIVERGENT.
        "super_in_a_block_of_a_method",
        r##"class A
  def m(x)
    x * 2
  end
end
class B < A
  def m(x)
    [x].map { |y| super.m(y) }
  end
end
print(B.new().m(3))"##,
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
    (
        // `TryBegin`/`TryEnd`/`RescueJump` run in the VM's fast tier: the
        // handler it pushes must unwind the stack, a `for` iterator and a
        // callee frame exactly as the general arm's did, on iterations that
        // throw and on those that do not.
        "try_rescue_in_loops_fast_tier",
        r##"def risky(i)
  throw "odd #{i}" if i % 2 == 1
  i * 10
end

def run_loops
  total = 0
  log = []
  i = 0
  while i < 6
    try
      total += risky(i)
    catch e
      log.push(e)
    finally
      total += 1
    end
    i += 1
  end
  arr = [1, 2, 3]
  for x in [0, 1, 5, 2]
    total += arr[x] rescue 100
    for y in [1, 2]
      total += risky(y) rescue y
    end
  end
  print(total)
  print(log)
  try
    total = 0
  catch e
    total = -1
  end
  total
end
print(run_loops())"##,
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
    (
        // A condition ending in `!=` branches the way `a != b` evaluates.
        // The VM fuses `NotEqual, JumpIfFalse` into `TestNotEqualJump`, whose
        // general arm compared two instances by pointer: two equal enum values
        // built apart (a payload variant, or one rebuilt by `parse` as an
        // `enum_field` read is) took the "different" branch. `unless`, `&&`,
        // a block's value and `a != b` itself are not fused and were right.
        "enum_not_equal_in_a_condition",
        r##"enum Shape
  Circle(radius: Int)
  Square(side: Int)
end

enum Status
  Active
  Inactive
end

def guard(a, b)
  return "guard" if a != b

  "-"
end

def loop_runs(a, b)
  rounds = 0
  while a != b
    rounds += 1
    break
  end
  rounds
end

def conditions(a, b)
  out = []
  if a != b
    out.push("if")
  else
    out.push("-")
  end
  if a == Shape.Square(9)
    out.push("square")
  elsif a != b
    out.push("elsif")
  else
    out.push("-")
  end
  out.push(a != b ? "ternary" : "-")
  label = "-"
  label = "postfix" if a != b
  out.push(label)
  out.push(guard(a, b))
  out.push(loop_runs(a, b))
  arm = match a {
    x if x != b => "match-guard",
    _ => "-"
  }
  out.push(arm)
  out.push([x for x in [a] if x != b].length)
  out.push([a].filter { |x| x != b }.length)
  unless a != b
    out.push("unless")
  end
  out.push(a != b && true)
  out.push(a != b)
  out
end

print(conditions(Shape.Circle(1), Shape.Circle(1)))
print(conditions(Shape.Circle(1), Shape.Circle(2)))
print(conditions(Shape.Circle(1), Shape.Square(1)))
print(conditions(Status.parse("Active"), Status.Active))
print(conditions(Status.parse("Inactive"), Status.Active))
print(conditions(Shape.parse({"variant": "Circle", "radius": 1}), Shape.Circle(1)))
# One shared unit-variant value: identity already answered this one.
print(conditions(Status.Active, Status.Active))"##,
    ),
    // A static field holding plain data (`Conf.per_page`, an enum's unit
    // variant) is read in the VM's fast tier; a static method, a nested class
    // and a write followed by a read keep the answer they had.
    (
        "static_field_reads",
        r##"enum Level { Low, High }
class Conf
  static per_page: Int = 20
  static rate: Float = 1.5
  static label: String = "conf"
  static tags: Array = ["a", "b"]
  static opts: Hash = {"k": 1}
  static nothing: Any = nil
  static level: Any = Level.High
  static def doubled
    @per_page * 2
  end
end
module Shop
  class Item
    static def kind
      "item"
    end
  end
end
def read_from_local
  klass = Conf
  total = 0
  i = 0
  while i < 3
    total += klass.per_page
    i += 1
  end
  [total, klass.tags.length, klass.label, klass.level == Level.High]
end
print(Conf.per_page, Conf.rate, Conf.label, Conf.tags, Conf.opts, Conf.nothing)
print(Conf.level == Level.High, Conf.level.variant())
print(read_from_local())
print(Conf.doubled)
Conf.per_page = 30
print(Conf.per_page, Conf.doubled)
print(Shop.Item.kind, Shop::Item.kind)"##,
    ),
    // `==` answers a unit variant compared with itself by identity; a payload
    // variant compared with itself still goes field by field, so a NaN in the
    // payload keeps it unequal to itself, as for two equal copies.
    (
        "enum_equality_identity",
        r##"enum Box { Empty, Full(x: Float) }
class Plain
end
nan = "NaN".to_f
empty = Box.Empty
print(empty == empty, empty == Box.Empty, empty != Box.Empty)
full = Box.Full(nan)
print(full == full, full != full)
one = Box.Full(1.0)
print(one == one, one == Box.Full(1.0), one == Box.Full(2.0), one == empty)
plain = Plain.new()
print(plain == plain, plain == Plain.new())"##,
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
        // The fused local ops (`IncrLocal`, `DecrLocal`, `AddLocalsInPlace`,
        // `AddNestedIndex`) raise where an Int leaves the range, and leave the
        // variable as it was. Their general arms wrapped instead — `i += 1` on
        // the largest Int gave the smallest — while the tree-walker raised.
        "int_overflow_in_fused_local_ops_raises",
        r##"const INT_MAX = 9223372036854775807
const INT_MIN = -9223372036854775807 - 1

# `x = x + …` is located at the `=` by the tree-walker and at the `+` by the
# VM, so those three shapes compare the message alone.
def without_location(error)
  error.split(" at ")[0]
end

def incr
  i = INT_MAX - 1
  i += 1
  try
    i += 1
  catch e
    print(e)
  end
  try
    i++
  catch e
    print(e)
  end
  try
    i = i + 1
  catch e
    print(without_location(e))
  end
  i
end

def decr
  i = INT_MIN + 1
  i -= 1
  try
    i -= 1
  catch e
    print(e)
  end
  try
    i--
  catch e
    print(e)
  end
  try
    i = i - 1
  catch e
    print(e)
  end
  i
end

def accumulate(step)
  total = INT_MAX - step
  total += step
  try
    total += step
  catch e
    print(e)
  end
  try
    total = total + step
  catch e
    print(without_location(e))
  end
  total
end

def keyed_sum
  h = {"a": INT_MAX - 1, "b": 1}
  names = ["a", "b", "b"]
  acc = 0
  k = 0
  while k < 2
    acc = acc + h[names[k]]
    k += 1
  end
  try
    acc += h[names[k]]
  catch e
    print(e)
  end
  try
    acc = acc + h[names[k]]
  catch e
    print(without_location(e))
  end
  acc
end

print(incr(), decr(), accumulate(1), keyed_sum())"##,
    ),
    (
        // `k += c`, `k -= c` and `k *= c` with an Int or Float constant other
        // than `+= 1`/`-= 1` are one in-place op on the VM
        // (`AddLocalConstInPlace` and its siblings). Its fast arm takes two
        // Ints or two Floats; an overflow, a mix, a string and `nil` go to the
        // general arm, which must raise or convert as the tree-walker does,
        // leaving the variable as it was on an error.
        "self_arithmetic_with_a_constant_in_place",
        r##"const INT_MAX = 9223372036854775807
const INT_MIN = -9223372036854775807 - 1

def ints
  k = 10
  k += 2
  k -= 5
  k *= 3
  k = k + 4
  k = k - 6
  k = k * 2
  k += -3
  k *= 0
  k += 7
  k
end

def floats
  x = 1.5
  x += 0.25
  x -= 0.5
  x *= 4.0
  y = 2
  y += 0.5
  z = 3.0
  z *= 2
  z -= 1
  [x, y, z]
end

def strings
  s = "ab"
  s *= 3
  t = "n"
  t += 2
  [s, t]
end

def overflow
  i = INT_MAX - 2
  i += 2
  try
    i += 2
  catch e
    print(e)
  end
  j = INT_MIN + 3
  j -= 3
  try
    j -= 3
  catch e
    print(e)
  end
  m = INT_MAX / 2 + 1
  try
    m *= 2
  catch e
    print(e)
  end
  [i, j, m]
end

def bad_type
  h = nil
  try
    h += 2
  catch e
    print("caught")
  end
  try
    h *= 2
  catch e
    print("caught")
  end
  h
end

def loop_sum(n)
  total = 0
  i = 0
  while i < n
    total += 10
    i += 1
  end
  total
end

print(ints(), floats(), strings(), overflow(), bad_type(), loop_sum(5))"##,
    ),
    (
        // Decimal with a Decimal, an Int or a Float on either side, for every
        // arithmetic and ordering operator, through the fused local ops, an
        // upvalue `+=`, globals and block callbacks. The VM had no Decimal arm
        // at all: each of these raised "Cannot add decimal and decimal" and
        // the like, and a zero Decimal divisor named the wrong error.
        "decimal_arithmetic_and_comparison",
        r##"def money(a, b, n, f)
  print(a + b, a - b, a * b, a / b, a % b)
  print(a + n, n + a, a - n, n - a, a * n, n * a, a / n, n / a)
  print(a + f, f + a, a - f, f - a, a * f, f * a)
  print(a + 1, a - 1, a * 3, a / 2, 100 - a, 3 * a)
  print(type(a * n), type(n - a), type(a + f), type(a * b), type(a / n))
  print(a > b, a < b, a >= b, a <= b, b < a, b <= a, a < a, a <= a)
  print(a > n, a < 20, n <= a, a >= f, f < a, a > 12.5, a < 12)
  if a > b
    print("ordered")
  end
  total = 0.00D
  [a, b].each do |x|
    total += x
  end
  total -= 1
  total *= 2
  print(total)
  step = a
  step = step + 1
  step = step - 1
  step += b
  print(step)
  try
    print(a / 0)
  catch error
    print(error)
  end
  try
    print(a / 0.00D)
  catch error
    print(error)
  end
  try
    print(7 / 0.00D)
  catch error
    print(error)
  end
  try
    print(a % 0.00D)
  catch error
    print(error)
  end
end
money(12.50D, 1.25D, 2, 1.5)
print(0.10D + 0.20D, 19.99D * 3, 10.00D / 3, 19.99D - 20.00D, 1.25D * 2)
price = 19.99D
qty = 3
subtotal = price * qty
shipping = 4.50D
print(subtotal, subtotal + shipping, subtotal > shipping, shipping <= 4.5, -price + 1)
big = 79228162514264337593543950.335D
try
  print(big * 10000)
catch error
  print(error)
end
try
  print(big + big * 1000)
catch error
  print(error)
end
prices = [3.00D, 1.5D, 2.25D]
print(prices.filter { |p| p > 2 }, prices.map { |p| p * 2 }, prices.reduce(fn(acc, p) acc + p, 0.00D))
print(prices.sort { |x, y| x < y ? -1 : 1 })"##,
    ),
    (
        // `Array - Array` keeps what is not `==` to an element of the right
        // side (instances by identity). The VM raised "Cannot subtract array
        // from array".
        "array_difference",
        r##"class Member
  name: String

  new(name: String)
    @name = name
  end
end

def without(items, removed)
  items - removed
end
print([1, 2, 3] - [2])
print([1, 2, 1, 3, 1] - [1], [1, 2] - [], [] - [1], [1, 2] - [3])
print(["apple", "banana", "cherry"] - ["banana"])
print([1, 2.0, 3] - [2], [[1], [2]] - [[1]], [{"a": 1}, {"a": 2}] - [{"a": 1}])
print([1.50D, 2.00D, nil] - [2.00D, nil])
ann = new Member("Ann")
bo = new Member("Bo")
print(without([ann, bo], [new Member("Ann")]).length, without([ann, bo], [ann]).length)
tags = ["a", "b", "c"]
tags = tags - ["b"]
tags -= ["c"]
print(tags, without(tags, tags))
same = [1, 2]
print(same - same, same)"##,
    ),
    (
        // A Money amount is a Decimal, so money code met the same gap.
        "money_amount_arithmetic",
        r##"line = Money.new(19.99, "EUR")
amount = line["amount"]
print(amount * 3, amount + 0.01D, amount > 10, amount - 20 < 0)
def line_total(money, qty)
  money["amount"] * qty
end
print(line_total(line, 4), Money.format(Money.new(line_total(line, 2), "EUR")))"##,
    ),
    (
        // A Decimal literal is parsed once, when the VM compiles it, and its
        // load is a copy in the fast tier. Its value and its precision (the
        // digits written after the point, which `==` compares) read as on
        // the tree-walker wherever a literal stands: an array, `%n[...]`, a
        // loop body run many times, a fused `local < literal`, a default
        // argument, a hash, a block and a match arm.
        "decimal_literals_keep_their_value_and_precision",
        r##"prices = [19.99D, 1.50D, 1.0D, 0.10D, 1_000.25D, 7.000D, -2.50D]
print(prices)
print(%n[1.5D 2D -2.50D 3.250D 1_000.5D])
long = 1.12345678901234567890123456789012D
print(long, long == 1.1234567890123456789012345679D)
print(1.25D + 0.25D == 1.5D, 1.0D == 1.00D, 19.99D == 19.99D)
def total(count)
  sum = 0.00D
  i = 0
  while i < count
    sum += 19.99D
    i += 1
  end
  sum
end
print(total(3))
def is_cheap(price)
  price < 20.00D
end
print(is_cheap(19.99D), is_cheap(25.00D))
def tax(amount, rate = 0.20D)
  amount * rate
end
print(tax(100.00D), tax(10.00D, 0.5D))
x = 2.50D
print(x >= 2.5D, x < 3.0D, x > 2.49D, x <= 2.50D)
row = {"price": 4.50D, "qty": 2}
print(row["price"] * row["qty"])
print(["a", "b"].map { |name| "#{name}:#{0.5D}" })
label = match 1 {
  1 => 9.99D,
  _ => 0.00D
}
print(label)"##,
    ),
    (
        // A zero divisor raises only between numbers, and a Float remainder by
        // zero is NaN, as the native kernels compute it. The VM checked the
        // divisor first: `1.5 % 0` raised, and Decimal, String and nil left
        // operands said "Division by zero" instead of their type error.
        "zero_divisor_follows_the_left_operand",
        r##"def remainder(a, b)
  a % b
end

def quotient(a, b)
  a / b
end

def attempt(label, thunk)
  try
    print(label, thunk())
  catch error
    print(label, error)
  end
end
attempt("float % 0", fn() { remainder(1.5, 0) })
attempt("float % 0.0", fn() { remainder(1.5, 0.0) })
attempt("int % 0.0", fn() { remainder(3, 0.0) })
attempt("decimal % 0", fn() { remainder(2.50D, 0) })
attempt("string / 0", fn() { quotient("s", 0) })
attempt("nil % 0", fn() { remainder(nil, 0) })
attempt("float / 0", fn() { quotient(1.5, 0) })
attempt("decimal / 0", fn() { quotient(2.50D, 0) })
attempt("decimal / float", fn() { quotient(2.50D, 0.5) })"##,
    ),
    (
        // Two instants 10 ns apart. The tree-walker compared DateTimes as f64
        // nanoseconds, which round to 256 ns here, so `early < late` was false
        // and `early >= late` true; the VM compared the i64. Both engines now
        // share one comparator.
        "datetime_ordering_is_exact",
        r##"early = DateTime.parse("2026-01-01T00:00:00.000000100Z")
late = DateTime.parse("2026-01-01T00:00:00.000000110Z")
print(early == late, early < late, early <= late, early > late, early >= late)
print(late > early, late >= early, late < early, late <= early)
def before(a, b)
  a < b
end
print(before(early, late), before(late, early))"##,
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
        "truncate_counts_characters_and_cuts_on_a_boundary",
        "print(\"ééééé\".truncate(3, \"\"))\nprint(\"ééééé\".truncate(5))\nprint(\"héllo world\".truncate(8))\nprint(\"日本語テキスト\".truncate(4, \"…\"))\nprint(\"😀😀😀\".truncate(2, \"\"))\nprint(\"héllo\".truncate(2))",
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
    // `MatchArrayLen` tests "is an Array" and the length in one op, on the
    // subject's slot or on a parked nested element; `ArrayRestLocal` builds
    // `...rest` natively. Pinned: non-array subjects of every kind fall
    // through, a nested element that is not an array fails only its arm, and
    // the rest is a copy the subject does not share.
    (
        "match_array_shape_on_non_array_subjects",
        "fn f(v) { return match v { [] => \"empty\", [a] => \"one\", [a, ...r] => \"many\", _ => \"not\" } }\nprint([f([]), f([1]), f([1, 2]), f(\"ab\"), f({\"a\": 1}), f(nil), f(0), f(1..3)])",
    ),
    (
        "match_array_nested_shape",
        "fn f(v) { return match v { [[x, y], z] => x * y + z, [[x, ...r], z] => r.length, [a, b] => a + b, _ => -1 } }\nprint([f([[2, 3], 4]), f([[1, 2, 3], 0]), f([1, 2]), f([[1], 2]), f([1])])",
    ),
    (
        "match_array_rest_is_a_copy",
        "xs = [1, 2, 3]\nr = match xs { [h, ...t] => t, _ => [] }\nr.push(9)\nprint([xs, r])",
    ),
    (
        "match_enum_payload_skips_wildcards_by_position",
        "enum Shape { Circle(radius: Int), Rect(w: Int, h: Int), Box(a: Int, b: Int, c: Int) }\nfn f(s) { return match s { Shape.Rect(_, h) => \"h=\" + str(h), Shape.Box(_, _, c) => \"c=\" + str(c), Shape.Circle(r) => \"r=\" + str(r), _ => \"?\" } }\nprint([f(Shape.Rect(3, 4)), f(Shape.Box(1, 2, 3)), f(Shape.Circle(5)), f(7)])",
    ),
    (
        "match_typed_primitives_and_typed_catch",
        "class Base { }\nclass Child < Base { }\nclass AppError < Error { }\nclass NotFound < AppError { }\nfn f(v) { return match v { Float: x => \"float\", String: s => \"string:\" + s, Bool: b => \"bool\", Void: z => \"nil\", Int: n => \"int\", _ => \"?\" } }\nprint([f(1.5), f(\"s\"), f(false), f(nil), f(1), f([1]), f(new Child())])\nfn g(e) { try { throw e } catch AppError err { return \"app\" } catch err { return \"other\" } }\nprint([g(new NotFound(\"x\")), g(new AppError(\"y\")), g(new ValueError(\"z\")), g(\"plain\")])",
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
    // `sort` without a comparator: homogeneous Int / String / non-NaN Float
    // arrays take a std-sort fast path, everything else the tolerant merge
    // sort (now over indices). Both engines share the helper; this pins that
    // they agree on every shape, -0.0 vs 0.0 keeping input order, mixes,
    // and a longer shuffled array that exercises many merge levels.
    (
        "sort_default_order_shapes",
        "print([5, -3, 9, 0, 9, 2].sort())\nprint([\"pear\", \"Apple\", \"apple\", \"\", \"z\"].sort())\nprint([0.0, -0.0, 1.5, -2.25, -0.0, 0.0].sort())\nprint([3, 1.5, 2, -0.5].sort())\nprint([5, \"a\", 3, nil, 1].sort())\nprint([{\"k\": 2}, 1, {\"k\": 1}].sort())\nxs = (0..300).map { |k| (k * 7919) % 300 }\nsorted = xs.sort\nprint(sorted.length, sorted[0], sorted[299], sorted == (0..300).map { |k| k })\nprint(xs[0], xs[1])\nwords = (0..120).map { |k| \"w#{(k * 37) % 120}\" }\nprint(words.sort()[0], words.sort()[1], words.sort()[119])\nprint([].sort, [1].sort)",
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
    // A key named like a hash method reads as the key, on both engines: the
    // VM's key fast paths answered `row.size` with the field while the
    // tree-walker answered the method, so a spec of `row.size` disagreed with
    // production. The VM's own slow read (`x.name ||=`, or any read once a
    // Hash user method exists) answered the method too, and `row.size ||= 0`
    // wrote the entry count back over the field. Without the key a bare read
    // is still the method, and parentheses always call it.
    (
        "hash_key_named_like_a_method_reads_the_key",
        r##"row = {"size": 2048, "key": "abc", "values": [1, 2], "select": "x", "length": 99, "class": "btn", "map": nil}
print(row.size, row.key, row.values, row.select, row.length, row.class, row.map)
def read(r)
  [r.size, r.key, r.values, r.length, "#{r.size}"]
end
print(read(row))
print(row&.size, {"rows": row}.rows.size)
plain = {"a": 1, "b": 2}
print(plain.size, plain.keys, plain.class, plain.length, plain.select)
print(row.size(), row.key("abc"), row.values().length, row.class())
row.size ||= 0
row.length += 1
row.map ??= "set"
print(row.size, row.length, row.map)
counts = {"size": nil}
counts.size ||= 7
print(counts)
print([{"size": 3}, {"size": 4}].map(&:size))"##,
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
    // --- field visibility ---
    (
        "field_visibility",
        "class Vault\n  private secret: String\n  protected level: Int\n\n  new(secret: String, level: Int)\n    @secret = secret\n    @level = level\n  end\n\n  def reveal\n    @secret\n  end\n\n  def level_of(other)\n    other.level\n  end\nend\n\ndef read_secret(x)\n  x.secret rescue \"refused\"\nend\n\ndef read_level(x)\n  x.level rescue \"refused\"\nend\n\ndef write_secret(x)\n  try\n    x.secret = \"lead\"\n    return \"written\"\n  catch error\n    return \"refused\"\n  end\nend\nv = new Vault(\"gold\", 3)\nprint(v.reveal)\nprint(v.level_of(new Vault(\"x\", 7)))\nprint(read_secret(v))\nprint(read_level(v))\nprint(write_secret(v))\nprint(v.reveal)",
    ),
    // --- protected methods ---
    (
        "protected_reachable_in_class_and_subclass",
        "class Account\n  cents: Int\n\n  new(cents: Int)\n    @cents = cents\n  end\n\n  def richer_than?(other)\n    balance > other.balance\n  end\n\n  protected\n\n  def balance\n    @cents\n  end\nend\n\nclass Savings < Account\n  def compare(other)\n    other.balance\n  end\nend\n\nclass Stranger\n  def peek(account)\n    account.balance\n  end\nend\nprint(new Account(10).richer_than?(new Account(5)))\nprint(new Savings(7).compare(new Account(5)))",
    ),
    (
        "protected_refused_outside",
        "class Account\n  cents: Int\n\n  new(cents: Int)\n    @cents = cents\n  end\n\n  def richer_than?(other)\n    balance > other.balance\n  end\n\n  protected\n\n  def balance\n    @cents\n  end\nend\n\nclass Savings < Account\n  def compare(other)\n    other.balance\n  end\nend\n\nclass Stranger\n  def peek(account)\n    account.balance\n  end\nend\ndef balance_of(x)\n  x.balance rescue \"refused\"\nend\na = new Account(10)\nprint(balance_of(a))\nprint(new Stranger().peek(a) rescue \"refused\")",
    ),
    // --- private methods and bare calls on self ---
    (
        "private_calls_on_self",
        "class Invoice\n  total: Int\n\n  new(total: Int)\n    @total = total\n  end\n\n  def via_at\n    @_tax(2)\n  end\n\n  def via_bare\n    _tax(2)\n  end\n\n  def via_bare_zero\n    _label\n  end\n\n  def via_bare_zero_parens\n    _label()\n  end\n\n  def via_block\n    [1, 2].map { |n| _tax(n) }\n  end\n\n  def other(invoice)\n    invoice._tax(2) rescue \"refused\"\n  end\n\n  private\n\n  def _tax(rate)\n    @total * rate\n  end\n\n  def _label\n    \"invoice #{@total}\"\n  end\nend\ninv = new Invoice(10)\nprint(inv.via_at)\nprint(inv.via_bare)\nprint(inv.via_bare_zero)\nprint(inv.via_bare_zero_parens)\nprint(inv.via_block)",
    ),
    (
        "private_refused_from_outside",
        "class Invoice\n  total: Int\n\n  new(total: Int)\n    @total = total\n  end\n\n  def via_at\n    @_tax(2)\n  end\n\n  def via_bare\n    _tax(2)\n  end\n\n  def via_bare_zero\n    _label\n  end\n\n  def via_bare_zero_parens\n    _label()\n  end\n\n  def via_block\n    [1, 2].map { |n| _tax(n) }\n  end\n\n  def other(invoice)\n    invoice._tax(2) rescue \"refused\"\n  end\n\n  private\n\n  def _tax(rate)\n    @total * rate\n  end\n\n  def _label\n    \"invoice #{@total}\"\n  end\nend\ninv = new Invoice(10)\ndef tax_of(x)\n  x._tax(2) rescue \"refused\"\nend\n\ndef label_of(x)\n  x._label rescue \"refused\"\nend\nprint(tax_of(inv))\nprint(label_of(inv))\nprint(inv.other(new Invoice(3)))",
    ),
    (
        "private_modifier_covers_one_method",
        "class A\n  private def secret\n    1\n  end\n\n  def open\n    2\n  end\nend\ndef secret_of(x)\n  x.secret rescue \"refused\"\nend\na = new A()\nprint(a.open)\nprint(secret_of(a))",
    ),
    (
        "private_bare_call_from_subclass",
        "class Base\n  private\n\n  def _double(n)\n    n * 2\n  end\nend\n\nclass Child < Base\n  def run\n    _double(21)\n  end\nend\nprint(new Child().run)",
    ),
    (
        "implicit_self_local_and_global_win",
        "def shared\n  \"global\"\nend\n\nclass C\n  def run\n    label = \"local\"\n    [label, shared()]\n  end\n\n  def label\n    \"method\"\n  end\n\n  def shared\n    \"method\"\n  end\nend\nprint(new C().run)",
    ),
    (
        // A bare call resolves once, before its arguments run, and the VM's
        // call op reuses that decision: a compiled method entered with its
        // defining class (so `super` inside an inherited one climbs from
        // there), defaults filled, globals still first, nested bare calls,
        // bare calls in a block, and an error among the arguments rescued in
        // the same method, after which bare calls still work.
        "implicit_self_calls_resolve_once",
        "def twice(x)\n  x * 2\nend\n\nclass Base\n  def greet(name, suffix = \"!\")\n    \"hi #{name}#{suffix}\"\n  end\n\n  def scale(x)\n    x * 10\n  end\n\n  def lineage\n    \"base\"\n  end\nend\n\nclass Mid < Base\n  def lineage\n    \"mid+\" + super.lineage()\n  end\nend\n\nclass Child < Mid\n  def scale(x)\n    super.scale(x) + 1\n  end\n\n  def run\n    out = []\n    out.push(greet(\"a\"))\n    out.push(greet(\"b\", \"?\"))\n    out.push(scale(2))\n    out.push(lineage())\n    out.push(_add(_add(1, 2), twice(_add(3, 4))))\n    out.push([1, 2, 3].map { |n| _add(n, twice(n)) })\n    caught = \"none\"\n    try\n      caught = _add(_fail(1), 2)\n    catch e\n      caught = \"caught #{e}\"\n    end\n    out.push(caught)\n    out.push(_add(5, 6))\n    out\n  end\n\n  private\n\n  def _add(a, b)\n    a + b\n  end\n\n  def _fail(x)\n    throw \"boom\"\n  end\nend\nprint(new Child().run)",
    ),
    (
        // `record[field] = value` with a computed name writes the field, as
        // `record.<field> = value` does; a model's `_` metadata stays
        // read-only. It used to be "invalid assignment target".
        "instance_dynamic_field_write",
        "class Doc < Model\nend\n\nclass Box\n  label: String\nend\n\ndoc = Doc.new()\nfield = \"title\"\ndoc[field] = \"t\"\nprint(doc.title)\nprint(doc[field])\nlocked = \"ok\"\nbegin_key = \"_key\"\nlocked = (doc[begin_key] = \"x\") rescue \"refused\"\nprint(locked)\nbox = new Box()\nbox[\"label\"] = \"b\"\nprint(box.label)",
    ),
    (
        // Indexing by name goes through the same private/protected check as
        // `record.field` on both engines.
        "instance_dynamic_field_private",
        "class Vault\n  private\n  secret: String\n\n  public\n  def put(name, value)\n    this[name] = value\n    this[name]\n  end\nend\n\nv = new Vault()\nprint(v.put(\"secret\", \"s1\"))\nfield = \"secret\"\nprint(v[field] rescue \"read refused\")\nout = (v[field] = \"x\") rescue \"write refused\"\nprint(out)\n",
    ),
    (
        // A bare `clock` is the time, as `clock()` is, in every position. The
        // VM used to push the native function itself: `clock - start` raised
        // "Cannot subtract float from Function".
        "bare_clock_is_the_time",
        "start = clock()\nprint(type(clock))\nprint(clock >= start)\nprint(type(clock - start))\ndef elapsed(from)\n  clock - from\nend\nprint(elapsed(start) >= 0)\nclass Timer\n  def now\n    clock\n  end\nend\nprint(type(Timer.new().now))\n[1].each do |i|\n  print(type(clock + i))\nend\nprint(type(clock.round))",
    ),
    (
        // A program's own `clock` keeps what it is.
        "bare_clock_shadowed",
        "clock = 7\nprint(clock)\ndef f(clock)\n  clock + 1\nend\nprint(f(1))",
    ),
    (
        // `send` on an instance calls the method it names, with the rest of
        // the arguments; from inside a method too (`this.send`), as a policy
        // does. The VM had no instance `send` and refused the handler.
        "instance_send",
        "class Policy\n  def edit?\n    true\n  end\n  def scaled(n, k)\n    n * k\n  end\n  def check(action)\n    this.send(action + \"?\")\n  end\nend\np = new Policy()\nprint(p.send(\"edit?\"))\nprint(p.send(\"scaled\", 3, 4))\nprint(p.check(\"edit\"))",
    ),
    (
        // A class with a static `send` of its own keeps it. The reflection
        // `send` on a class is the tree-walker's alone: the VM refuses it,
        // and `soli serve` re-runs the action on the tree-walker.
        "class_own_static_send",
        "class Mailer\n  static def send(to)\n    \"mail to #{to}\"\n  end\nend\nm = Mailer\nprint(Mailer.send(\"bob\"))\nprint(m.send(\"ann\"))",
    ),
    (
        // `contains?` is `contains`, named as the predicate it is, on strings
        // and arrays.
        "contains_predicate",
        "print(\"soli lang\".contains?(\"lang\"))\nprint(\"soli\".contains?(\"x\"))\nprint([1, 2, 3].contains?(2))\nprint([1, 2].contains?(5))",
    ),
    (
        // A subclass keeps its superclass's field defaults, with a `new` that
        // does not call `super` too, and with field defaults and no `new` its
        // superclass's constructor. On the VM the defaults-only `init` hid
        // that constructor ("expected 0, got 1") and the superclass's fields
        // stayed unset.
        "subclass_field_defaults_keep_the_superclass",
        r##"class Base
  x: Int = 5
  list: Array = []
end
class Child < Base
  y: Int = 1
end
c = new Child()
c.list.push(1)
print(c.x, c.list, c.y, new Child().list)
class Named
  name: Any = nil
  new(name)
    @name = name
  end
end
class Tagged < Named
  tag: String = "t"
end
t = new Tagged("a")
print(t.name, t.tag)
class Deeper < Tagged
  depth: Int = 2
end
d = new Deeper("b")
print(d.name, d.tag, d.depth)
class Own < Base
  a: Any = nil
  new(a)
    @a = a
  end
end
o = new Own(3)
print(o.x, o.list, o.a)"##,
    ),
    (
        // A declared field without a default starts as nil, also inside the
        // constructor, and whatever builds the instance. The VM set only
        // the fields with a default: reading any other one raised
        // "Cannot access property".
        "declared_fields_start_as_nil",
        r##"class Profile
  name: String
  bio: String
  seen: Any

  new(name)
    @seen = @bio.nil?
    @name = name
  end
end
p = Profile.new("ann")
print(p.bio, p.seen, new Profile("bob").bio, Profile("cy").bio)
class Bare
  title: String
end
print(new Bare().title, Bare().title)
class Optional < Profile
  extra: String

  new(name)
    super(name)
  end
end
o = new Optional("x")
print(o.name, o.bio, o.extra)
class Hinted < Error
  code: Int = 42
  hint: String
end
h = Hinted.new("boom")
print(h.message, h.code, h.hint)"##,
    ),
    (
        // Every class's fields are set before any constructor runs, the
        // farthest class first. The VM kept a class's defaults in its
        // compiled constructor: a parent's were lost when the child's `new`
        // did not call `super`, and re-applied by `super()` after the
        // child's (`2` came out `1`, `7` came out `0`). A field declared
        // again without a default is reset to nil; a literal array or hash
        // default is never shared.
        "field_defaults_reach_every_class_before_any_constructor",
        r##"class P1
  a: Int = 1
  p: Bool

  new()
    @p = true
  end
end
class K1 < P1
  c: Int

  new(c)
    @c = c
  end
end
k1 = new K1(3)
print(k1.a, k1.p, k1.c)
class P2
  a: Int = 1

  new()
  end
end
class K2 < P2
  a: Int = 2

  new()
    super()
  end
end
print(new K2().a)
class P3
  count: Int = 0

  new()
  end
end
class K3 < P3
  new(c)
    @count = c
    super()
  end
end
print(new K3(7).count)
class P5
  a: Int = 1
end
class K5 < P5
  b: Int = 2
end
class G5 < K5
  a: Int
  c: Int

  new(c)
    @c = c
  end
end
g = G5.new(3)
print(g.a, g.b, g.c)
class Items
  items: Array = []
  meta: Hash = {"n": 0}

  new()
  end
end
i1 = new Items()
i2 = new Items()
i1.items.push(1)
i1.meta["n"] = 5
print(i1.items, i2.items, i2.meta["n"])"##,
    ),
    (
        // A default that is not a literal runs once per instance, a
        // parent's before a child's. The VM also ran it once when the class
        // was defined, so every count was one ahead.
        "a_computed_field_default_runs_once_per_instance",
        r##"calls = 0
def next_stamp()
  calls = calls + 1
  calls * 10
end
class Stamped
  stamp: Int = next_stamp()
  tag: String = "t"
  offset: Int = -1
end
print("defined", calls)
a = new Stamped()
b = Stamped()
print(a.stamp, b.stamp, a.tag, a.offset, calls)
class Later < Stamped
  label: String = "n#{calls}"
end
c = new Later()
print(c.stamp, c.label, calls)"##,
    ),
    (
        // Without a constructor, `Klass.new({..})` / `Klass.new(k: v)` take
        // the attributes over fields started as `new Klass()` starts them,
        // and constructor arguments go nowhere. The VM ran its defaults-only
        // `init` as a constructor: "expected 0, got 1", "Undefined variable
        // 'a'". The tree-walker's attribute form set literal defaults only:
        // `x.b` and `t.n` raised there.
        "field_defaults_without_a_constructor_keep_the_attribute_form",
        r##"class D
  a: Int = 1
  b: Int
  offset: Int = -1
  list: Array = []
end
x = D.new({"a": 5})
y = D.new(a: 6)
z = D.new()
print(x.a, x.b, x.offset, y.a, y.b, y.offset, z.a, z.list)
print(new D(7).a, new D(a: 8).a, D(9).b, D(a: 9).a)
z.list.push(1)
print(D.new().list)
calls = 0
def tick()
  calls = calls + 1
  calls
end
class T
  n: Int = tick()
end
print(T.new().n, T.new({}).n, new T().n, calls)"##,
    ),
    (
        // Defaults on private and const fields, one reading a local of the
        // function the class is declared in, one reading a static, nested
        // containers copied per instance, a parent's computed default under
        // a child's literal one. A default that throws is caught where the
        // instance is built: the VM evaluated it when the class was defined,
        // outside the `try`, and the script died.
        "field_defaults_private_const_nested_and_throwing",
        r##"class Vault
  private secret: String = "s3"
  const id: Int = 7
  stamp: String = "v" + "1"

  def reveal
    "#{@secret} #{@id} #{@stamp}"
  end
end
print(new Vault().reveal, Vault().reveal)
def build(prefix)
  class Tag
    label: String = prefix + "!"
  end
  new Tag().label
end
print(build("hi"))
class Cfg
  static DEFAULT: Int = 5
  size: Int = Cfg.DEFAULT * 2
end
print(new Cfg().size)
class Nested
  data: Hash = {"list": [1, [2, 3]], "n": nil}
end
n1 = new Nested()
n2 = new Nested()
n1.data["list"][1].push(4)
print(n1.data, n2.data)
def boom()
  throw "boom"
end
class Risky
  x: Int = boom()
end
r = "none"
try
  new Risky()
catch e
  r = "caught #{e}"
end
print(r)
def compute()
  41 + 1
end
class PA
  x: Int = compute()
  y: Int = 1
end
class CA < PA
  x: Int = 2
  y: Int = compute()
end
c = new CA()
print(c.x, c.y)"##,
    ),
    (
        // A value object's field left out takes its default, by every
        // spelling, and its invariants and `with` see it. On the VM the
        // defaults-only `init` ran before the arguments were bound, which
        // held for one level only: a parent value object's defaults were
        // nil.
        "value_object_fields_take_their_defaults",
        r##"class Amount < ValueObject
  cents: Int
  currency: String = "EUR"

  invariant(:currency, "three letters") { @currency.length == 3 }
end
class Price < Amount
  vat: Int = 20
end
a = Amount.new(500)
b = Amount(600)
c = Amount.new(cents: 700)
d = Amount.new({"cents": 800})
e = a.with(cents: 900)
p = Price.new(100)
print(a.currency, b.currency, c.currency, d.currency, e.currency, e.cents)
print(p.cents, p.currency, p.vat, Price.new(cents: 5, vat: 7).vat)
print(a == Amount.new(500, "EUR"), Amount.new(1, "USD").currency)"##,
    ),
    (
        // A field default that is not a literal runs on the new instance, in
        // declaration order, whatever the spelling: the tree-walker evaluated
        // it in the caller's environment (its `this`, its locals) and in
        // hash-map order.
        "field_defaults_run_on_the_new_instance_in_declaration_order",
        r##"class Basket
  items: Array = [1, 2, 3]
  count: Int = @items.length
  label: String = "n=#{@count}"
  doubled: Int = @count * 2
end
class Shop
  items: Array = [9]
  count: Int = 40

  def build
    items = [7, 7]
    [new Basket(), Basket.new(), Basket.new({}), Basket.new(label: "x"), Basket()]
  end
end
Shop.new().build.each do |b|
  print(b.items, b.count, b.label, b.doubled)
end"##,
    ),
    (
        // Each class starts its fields, then runs its defaults that are not
        // literals, before the next class down starts its own.
        "field_defaults_run_class_by_class_from_the_farthest",
        r##"calls = []
def note(tag)
  calls.push(tag)
  tag
end
class Base1
  a: String = note("base-a")
  b: Int = 1
  c: String = "#{@a}/#{@b}"
end
class Kid1 < Base1
  b: Int = 2
  d: String = note("kid-d") + @c
end
k = Kid1.new()
print(k.a, k.b, k.c, k.d)
k2 = new Kid1()
print(k2.c, k2.d, calls)
class Plain
  stamp: Int = 5 + 5
  sym = :draft
end
class Child < Plain
  extra: Array = []
end
class Grand < Child
end
print(new Child().stamp, Child.new().sym, Grand.new().extra, Grand.new(extra: [1]).extra)"##,
    ),
    (
        // A method named `init` is a method, not the constructor. The VM kept
        // its compiled constructor as the method `init`: `new Service()` ran
        // the user's `init` and was its value, and a class with field
        // defaults and an `init(a)` raised "Wrong number of arguments" on
        // `new K()`.
        "a_method_named_init_is_not_the_constructor",
        r##"class Service
  def init
    "init-value"
  end
end
s = new Service()
print(s.is_a?("Service"), s.init)
class K
  x: Int = 1

  def init(a)
    @x = a
  end
end
k = new K()
print(k.x)
k.init(5)
print(k.x)
class Cart
  owner: String
  items: Array

  new(owner)
    @owner = owner
    @items = []
  end

  def init(items)
    @items = items
    "init-ran"
  end
end
c = Cart.new("ann")
print(c.init([1, 2]), c.owner, c.items)
d = Cart.new("bo")
print(d.owner, d.items)
class SubCart < Cart
end
e = new SubCart("cy")
print(e.owner, e.items, e.init([3]))"##,
    ),
    (
        // `Klass.new(k: v)` runs a default that is not a literal, as every
        // other spelling does: the VM read `Ticket.new` as a member and built
        // the attribute form with that default left nil.
        "named_attribute_form_runs_a_computed_default",
        r##"calls = 0
def tick()
  calls = calls + 1
  calls
end
class Ticket
  n: Int = tick()
  label: String
end
t = Ticket.new(label: "x")
print(t.n, t.label, calls)"##,
    ),
    (
        // `this` in a `static def` is the class it was called on, a
        // subclass's too. The tree-walker raised "'this' outside of class".
        "this_in_a_static_method",
        r##"class Base
  def hello
    "hi"
  end
  static def build
    this.new()
  end
  static def twice(n)
    this.helper(n) * 2
  end
  static def helper(n)
    n + 1
  end
end
class Sub < Base
end
print(Base.build().hello, Sub.build().class, Sub.twice(2))
print(Sub.build.class, Base.build.hello)
module Util
  def shout(s)
    s.upcase
  end
end
print(Util.shout("a"))"##,
    ),
    (
        // `==`/`!=` against a literal or another local compile to the fused
        // `EqualLocalConst`/`EqualLocalLocal` (and their `!=` twins), a
        // condition on any other left side to `TestEqualJump`, and literal
        // `match` arms to `EqualLocalConst`; string literals and primitive
        // equality are answered in the VM's fast tier. Every kind pair must
        // still answer as `==` does: nil against a value, Int against Float,
        // a symbol against its string, a class against its name, enums.
        "equality_fused_and_fast_paths",
        r##"def g(s)
  match s {
    "a" => 1,
    "b" => 2,
    nil => 3,
    true => 4,
    1.5 => 5,
    0 => 6,
    _ => 0
  }
end
print([g("a"), g("b"), g(nil), g(true), g(1.5), g("z"), g(1), g(0), g(false)])
def f(s, t)
  [s == "admin", s != "admin", s == t, s != t, s == nil, s != nil, s == 0, s != 0, s == 1.5, s == true]
end
print(f("admin", "admin"))
print(f("user", "admin"))
print(f(nil, nil))
print(f(nil, 0))
print(f(0.0, 0))
print(f(0, 0.0))
print(f(1.5, 1.5))
print(f(true, false))
print(f(:admin, "admin"))
print(f(:admin, :admin))
def branch(s, t)
  out = []
  out.push("eq") if s == "admin"
  out.push("ne") if s != "admin"
  out.push("ll") if s == t
  if s == nil
    out.push("nil")
  elsif s == 0
    out.push("zero")
  end
  while out.length < 3 && s != t
    out.push("loop")
  end
  out
end
print(branch("admin", "admin"))
print(branch(nil, 0))
print(branch(0.0, 0))
h = {"k": "z", "n": nil}
print(h["k"] == "z", h["k"] != "z", h["n"] == nil, h["k"] == :z)
if h["k"] == "z"
  print("hash condition")
end
print([1, 5, 2, 5].filter { |x| x == 5 })
print([1, 5.0, 2, 5].filter { |x| x != 5 })
print(["a", "b", nil].filter { |x| x == "b" })
print({"a": 1, "b": 5}.filter { |k, v| v == 5 })
class Account
end
klass = Account
print(klass == "Account", klass != "Account")
def same_class(c)
  c == "Account"
end
print(same_class(Account))
enum Status
  Active
  Inactive
end
def active(st)
  [st == Status.Active, st != Status.Active]
end
print(active(Status.parse("Active")))
print(active(Status.Inactive))
s = "x"
y = "#{s}" == "x"
print(y, "lit" == "lit", :a == :a, "a" == :a)"##,
    ),
    (
        // The fast tier answers `k -= 1`, Int `/` and `%` (and `k % 7`'s new
        // ModLocalConst), `??`, `h["k"]` and `xs[i]` in range; each declines
        // where the general arm raises or converts. Results, errors and their
        // spans must stay the tree-walker's.
        "fast_tier_division_nullish_and_index_reads",
        r##"def f(k, c, z, m, h, arr, v)
  print(k % 7, k / 7, k % c, k / c, -k % 7, -k / 7, k % -3, (k + 1) % 4)
  print(7.5 / 2.0, 7.5 % 2.0, k / 2.0, 7.0 / k, k % 2.5)
  try
    print(k / z)
  catch e
    print("div: #{e}")
  end
  try
    print(k % z)
  catch e
    print("mod: #{e}")
  end
  try
    print(k % 0)
  catch e
    print("modc: #{e}")
  end
  try
    print(m / -1)
  catch e
    print("ovf: #{e}")
  end
  try
    print(m % -1)
  catch e
    print("ovfm: #{e}")
  end
  try
    print(1.0 / 0.0)
  catch e
    print("fz: #{e}")
  end
  try
    print(k % "x")
  catch e
    print("mods: #{e}")
  end
  print(h["a"], h["zz"], arr[1], arr[-1], arr[c - 2], v ?? 3, nil ?? 4, false ?? 5)
  try
    print(arr[k])
  catch e
    print("oob: #{e}")
  end
  try
    print(arr[-9])
  catch e
    print("oobn: #{e}")
  end
  n = 3
  n -= 1
  n -= 1
  d = 2.5
  d -= 1
  x = nil
  x ??= 8
  x ??= 9
  low = m
  try
    low -= 1
  catch e
    print("decr: #{e}")
  end
  s = "abc"
  g = {"q": 1}
  print(n, d, x, s[1], g["q"] ?? 0, g["r"] ?? 0)
end
f(10, 4, 0, -9223372036854775807 - 1, {"a": 1}, [1, 2, 3], nil)
top = {"a": 1, "b": nil}
print(top["a"], top["b"] ?? "dflt", top["c"])
i = 10
evens = 0
while i > 0
  evens += 1 if i % 2 == 0
  i -= 1
end
print(evens, i)"##,
    ),
    (
        // Defaulted parameters, short positional calls and named arguments
        // bind alike: the VM runs the default prologue and short calls in its
        // fast tier and binds labels in place on the stack. The positional
        // value wins over a label for the same parameter; an explicit nil is
        // supplied. (A duplicate label and `f()` are refused by both engines,
        // with a different column and expected count: only the refusal is
        // compared.)
        "defaults_short_calls_and_named_arguments_bind_alike",
        r##"def f(a, b = 2, c = 10 + 1)
  "#{a}/#{b}/#{c}"
end
def g(x, y)
  x - y
end
print(f(1), f(1, 5), f(1, 5, 6), f(1, nil))
print(f(1, c: 7), f(b: 3, a: 4), f(c: 0, a: 9), f(1, a: 99), g(y: 1, x: 10))
i = 0
total = 0
while i < 3
  total += g(f(i).length, 1)
  i += 1
end
print(total)
try
  f(b: 1)
catch e
  print("missing: #{e}")
end
try
  f(1, zz: 1)
catch e
  print("unknown: #{e}")
end
try
  f(1, 2, 3, 4)
catch e
  print("arity: #{e}")
end
try
  f(1, 2, 3, d: 4)
catch e
  print("unknown after positional: #{e}")
end
try
  f(a: 1, a: 2)
catch e
  print("duplicate refused")
end
try
  f()
catch e
  print("none refused")
end
class Pt
  x: Int
  y: Int
  new(x: Int, y: Int = 5)
    @x = x
    @y = y
  end
end
print(Pt.new(y: 1, x: 2).x, Pt(x: 3).y)
def many(a, b = 1, c = 2, d = 3, e = 4, f = 5, g = 6, h = 7, i = 8, j = 9, k = 10, l = 11, m = 12, n = 13, o = 14, p = 15, q = 16, r = 17, s = 18)
  [a, b, s, r, j]
end
print(many(0, s: 100, r: 99, q: 1, p: 1, o: 1, n: 1, m: 1, l: 1, k: 1, i: 1, h: 1, g: 1, f: 1, e: 1, d: 1, c: 1, b: 7))"##,
    ),
    (
        // `if v`, `unless v`, `if !v` and `while k` on a local compile to the
        // fused `JumpIfLocalFalsy` / `JumpIfLocalTruthy`, which test the slot
        // in place; every truthiness class must branch as the tree-walker
        // does (`0.0` truthy, `0`, `""`, `[]`, `{}` falsy). `&&`/`||` keep the
        // tested value, so they stay unfused.
        "local_condition_fused_branches",
        r##"class Box
  v: Int
  new(v: Int)
    @v = v
  end
end
def branches(v)
  out = []
  out.push(1) if v
  out.push(2) unless v
  n = 0
  if !v
    out.push(3)
  else
    out.push(4)
  end
  k = v
  n = 0
  while k
    n = n + 1
    k = nil if n >= 2
  end
  out.push(n)
  a = v && "and"
  b = v || "or"
  out.push(a)
  out.push(b)
  out
end
[true, false, nil, 0, 1, -7, 0.0, 2.5, "", "x", [], [nil], {}, {"k": 0}, :sym, Box.new(0), fn(x) { x }].each do |v|
  print(branches(v))
end"##,
    ),
    (
        // `unless <expr>`, the postfix `unless` forms and `if !<expr>` fold
        // the `!` into one `JumpIfTrue` on the expression's value (`!!` and
        // `unless !` back into `JumpIfFalse`), in statements, tail position,
        // `while`, the ternary, comprehension filters and `match` guards.
        // Every truthiness class must branch as the tree-walker's `!` does.
        "negated_condition_jump_if_true",
        r##"class Box
  v: Int
  new(v: Int)
    @v = v
  end
end
def id(v)
  v
end
def guard(v)
  return "g" unless id(v)

  "G"
end
def tail(v)
  unless id(v) == nil
    "t"
  else
    "T"
  end
end
def forms(v)
  out = []
  out.push(1) unless id(v)
  unless [v][0]
    out.push(2)
  else
    out.push(3)
  end
  out.push(4) if !id(v)
  out.push(5) if !!id(v)
  out.push(6) unless !id(v)
  out.push(!(id(v)) ? 7 : 8)
  n = 0
  while !(n >= 2 || id(v))
    n += 1
  end
  out.push(n)
  out.push([1, 2].filter { |x| !id(v) }.length)
  out.push(match 1 { x if !id(v) => "m", _ => "M" })
  out.push(guard(v))
  out.push(tail(v))
  out
end
[true, false, nil, 0, 1, -7, 0.0, 2.5, "", "x", [], [nil], {}, {"k": 0}, :sym, Box.new(0), fn(x) { x }].each do |v|
  print(forms(v))
end
i = 5
until_like = 0
while !(i < 0)
  i -= 1
  until_like += 1 unless i % 2 == 0
end
print(until_like)
print([1, 2, 3, 4].map { |x| x unless x > 2 })
print((0..6).filter { |x| !(x % 3 == 0) })"##,
    ),
    // The ops that read their name from the constant pool by sharing it
    // rather than copying it (T33.20): AddAssignGlobal (hit and the insert of
    // a first write), AppendGlobal, AppendGlobalConst, CallSuperMethod and
    // `send`, each with a name short enough to sit inline and one long enough
    // to be reference-counted.
    (
        "borrowed_constant_names_global_updates_super_and_send",
        r##"total = 0
an_accumulator_with_a_long_name = 0
for i in [1, 2, 3] {
  total = total + i * 2
  an_accumulator_with_a_long_name = an_accumulator_with_a_long_name + i * 3
}
print(total)
print(an_accumulator_with_a_long_name)
buf = "a"
buf << "b"
list = []
list << 1
print(buf)
print(list)
out = ""
for w in [1, 2] { out = out + "x" }
print(out)
class Base
  def greet(x)
    "base #{x}"
  end
  def a_method_name_longer_than_fifteen
    "long"
  end
end
class Kid < Base
  def greet(x)
    super.greet(x) + "!"
  end
  def a_method_name_longer_than_fifteen
    super.a_method_name_longer_than_fifteen() + "er"
  end
end
k = Kid.new()
print(k.greet(1))
print(k.a_method_name_longer_than_fifteen)
print(k.send("greet", 2))
print(k.send("a_method_name_longer_than_fifteen"))"##,
    ),
    (
        // Each `while` condition is the `Loop` target. It fuses to a `*LocalLocal` / `*LocalConst` compare (C587): Int, Float and String operands, a loop that runs and one that does not.
        "fused_head_at_while_headers",
        r##"def headers(n, lim)
  i = 0
  f = 0.0
  s = "a"
  hits = 0
  while i < n
    i += 1
  end
  while f < lim
    f = f + 0.5
  end
  while s < "aaaa"
    s = s + "a"
  end
  while hits <= 3
    hits += 1
  end
  [i, f, s, hits]
end
print(headers(4, 2.0))
print(headers(0, 0.0))"##,
    ),
    (
        // The statement after an `if` or a postfix guard is the branch's merge, and fuses (C587): `IncrLocal`, `AddLocalsInPlace`, `GetLocalProperty`, `LessLocalLocal`, `AppendLocalConst` and a method call, with Int, Float, String and Array operands.
        "fused_head_after_if_and_guard",
        r##"class Box
  v: Int
  new(v)
    @v = v
  end
  def twice
    @v * 2
  end
end
def merges(k, m, h, b)
  t = 0
  y = nil
  c = 0
  buf = ""
  i = 0
  while i < 4
    c += 1 if i % 2 == 0
    i += 1
    if i > 2
      buf = buf + "x"
    end
    t = t + m
    y = h.a
    z = k < m
    buf = buf + "y"
    w = b.twice
    i += 0
    print([t, y, z, buf, w])
  end
  [c, t]
end
print(merges(1, 2, {"a": 5}, Box.new(3)))
print(merges(1.5, 2.25, {"a": "s"}, Box.new(1)))
print(merges("a", "b", {"a": [1]}, Box.new(0)))"##,
    ),
    (
        // `elsif` and `else` heads, the statement after a `match`, a defaulted function's first statement and a `catch` body's are branch targets that fuse (C587).
        "fused_head_at_elsif_match_default_catch",
        r##"def classify(v)
  r = 0
  if v < 0
    r = 1
  elsif v < 3
    r = r + v
  elsif v == 3
    r = v * 10
  else
    r = v - 1
  end
  r
end
def arms(v)
  n = 1
  out = match v {
    1 => n + 1,
    2 => n * 5,
    "s" => n - 3,
    _ => v
  }
  n += 1
  [out, n]
end
def defaults(a, b = 2, c = "z")
  a += 1
  [a, b, c]
end
def caught(x)
  n = 0
  try
    throw "boom" if x
    n += 5
  catch e
    n += 1
    n = n * 3
  end
  n += 10
  n
end
print([classify(-2), classify(2), classify(3), classify(9), classify(2.5)])
print([arms(1), arms(2), arms("s"), arms(7)])
print([defaults(1), defaults(1.5, 3), defaults(0, 0, "q")])
print([caught(true), caught(false)])"##,
    ),
    (
        // `x += 1` after an `if` is `IncrLocal` (C587): Float and String operands take the general arm, and an Int overflow raises with the statement's location.
        "fused_head_after_if_raises_like_unfused",
        r##"def bump(flag, x)
  r = 0
  if flag
    r = 1
  end
  x += 1
  x
end
print(bump(true, 1))
print(bump(false, 2.5))
try
  print(bump(true, 9223372036854775807))
catch e
  print("caught: #{e}")
end
try
  print(bump(false, "s"))
catch e
  print("caught type")
end"##,
    ),
    // --- top-level variables kept in frame slots (`compile_entry_script`) ---
    (
        // The loop of the audit: `i` and `sum` live in the main frame.
        "toplevel_loop_accumulator",
        "i = 0\nsum = 0\nwhile i < 10\n  sum = sum + i\n  i += 1\nend\nprint(sum)\nprint(i)",
    ),
    (
        // `let`, `const`, a bare `let` filled later, and a block-scoped `let`
        // that shadows a slot without touching it.
        "toplevel_let_and_const_in_slots",
        "let a = 1\nconst LIMIT = 10\nlet c\nc = a + LIMIT\nif c > 5\n  let c = 0\n  a = a + c + 1\nend\nprint(a)\nprint(c)\nprint(LIMIT)",
    ),
    (
        // Names a `def` or a block reads or writes stay globals; the loop
        // counter beside them is a slot.
        "toplevel_names_shared_with_functions_stay_global",
        "count = 0\ntotal = 0\ndef bump(n)\n  count = count + n\nend\n[1, 2, 3].each do |x|\n  total = total + x\nend\nbump(5)\nsteps = 0\nwhile steps < 3\n  steps += 1\nend\nprint(count, total, steps)",
    ),
    (
        "toplevel_lambda_held_in_a_slot",
        "double = fn(x) { x * 2 }\nacc = []\nfor v in [1, 2, 3]\n  acc.push(double(v))\nend\nprint(acc)",
    ),
    (
        // A comprehension after another element runs in a lambda of its own
        // that reads the slots; a binding `match` at a clean position does not.
        "toplevel_slots_read_by_wrapped_comprehension_and_match",
        "k = 3\nxs = [1, 2]\nrow = [k, [x * k for x in xs], match k { 3 => \"three\", _ => \"other\" }]\nprint(row)\nlabel = match k {\n  1 => \"one\",\n  n if n > 2 => \"big #{n}\",\n  _ => \"other\"\n}\nprint(label)",
    ),
    (
        // A throw caught at top level keeps the slots below its handler; a
        // string grows in a slot.
        "toplevel_slots_survive_try_and_string_growth",
        "log = \"\"\nn = 0\ntry\n  n = n + 1\n  throw \"boom\"\ncatch e\n  log = log + \"caught \"\nend\nwhile n < 4\n  log += \"#{n} \"\n  n += 1\nend\nprint(log)\nprint(n)",
    ),
    (
        // `defined` asks for a global by name at run time: no slot then.
        "toplevel_defined_sees_top_level_variables",
        "x = 5\nprint(defined(\"x\"))\ny = x + 1\nprint(y)\nprint(defined(\"y\"))",
    ),
    (
        "toplevel_slots_beside_a_class",
        "class Counter\n  value: Int\n  new(start: Int)\n    @value = start\n  end\n  def bump\n    @value = @value + 1\n  end\nend\nc = new Counter(1)\ni = 0\nwhile i < 3\n  c.bump\n  i += 1\nend\nprint(c.value)\nprint(i)",
    ),
    (
        "toplevel_compound_assignments_on_slots",
        "total = 10\ntotal -= 3\ntotal *= 2\ntotal ||= 99\nflag = \"\"\nflag ||= \"set\"\nwords = []\nwords << \"a\"\nwords << \"b\"\nname = \"x\"\nname << \"y\"\nitems = [\"a\", \"b\"]\nout = \"\"\nfor item, idx in items\n  out = out + \"#{idx}:#{item} \"\nend\nprint(total, flag, words, name, out)",
    ),
    // --- constant folding ---
    (
        // Folded at compile time on the VM, computed at run time on the
        // tree-walker; what raises is left to raise.
        "literal_arithmetic_folds_as_it_runs",
        "print(60 * 60 * 24)\nprint(-7 / 2)\nprint(-7 % 3)\nprint(7.5 / 2.5)\nprint(1 + 2.5)\nprint(0.1 + 0.2)\nprint(\"ab\" + \"cd\")\nprint(-5, -2.5, 2 * -3)\nprint((1 + 2) * 3)\nbig = (9223372036854775807 + 1) rescue \"overflow\"\nprint(big)\nzero = (1 / 0) rescue \"div0\"\nprint(zero)\ndef day_seconds()\n  60 * 60 * 24\nend\nprint(day_seconds())",
    ),
    // --- array literals made of literals (`compiler_literal_arrays.rs`) ---
    (
        // `[<literals>].includes?(x)` is one op against a constant list on
        // the VM: the four spellings, each kind of literal, the guard shapes
        // the style guide uses, a method reading `@status`, a literal over
        // several lines.
        "literal_list_membership",
        r##"class Ticket
  status: String

  new(status: String)
    @status = status
  end

  def open?
    ["up", "late", "overdue"].includes?(@status)
  end
end

def label(s)
  return "open" if ["up", "late"].includes?(s)
  return "symbol" if [:up, :late].include?(s)
  return "small" if [1, 2, 3].contains(s)
  return "flag" if [true, nil].contains?(s)

  "other"
end

print(label("up"), label("closed"), label(:late), label(2), label(nil), label(false), label(9))
print(Ticket.new("late").open?, Ticket.new("closed").open?)
print(%w[draft sent].includes?("sent"), [].includes?("x"), [-1, 60 * 60].includes?(3600))
allowed = [
  "inline",
  "inline-block",
  "table"
].includes?("table")
print(allowed)
kept = "kept"
kept = "dropped" unless ["b", "c"].includes?("a")
print(kept)"##,
    ),
    (
        // An array literal of literals is one op on the VM, and still a new
        // array on every evaluation: changing one leaves the next alone.
        "literal_array_is_new_on_each_evaluation",
        r##"def make()
  ["a", :b, 1, 2.5, true, nil]
end

first = make()
first.push("x")
first[0] = "changed"
print(first, make())
rows = []
for i in [0, 1, 2]
  row = [0, 0]
  row[1] = i
  rows.push(row)
end
print(rows)
grid = [[1, 2], [3]]
grid[0].push(9)
print(grid, [[1, 2], [3]])
print(["px-4", "py-2"].join(" "), [3, 1, 2].sort(), ["b", "a"].map { |s| s.upcase })
h = {"tags": ["a", "b"]}
h["tags"].push("c")
print(h, {"tags": ["a", "b"]})"##,
    ),
    (
        // Once a class declares a private member, the VM lets a member of
        // `self` through before any class lookup and answers an unrestricted
        // name from a per-class memo. Every verdict stays the walk's: on self
        // (`@x`, `this.x`, `@x = v`, `@m()`, in a block), off self (a field
        // read, written and indexed, a private call, a protected read from a
        // subclass and from outside), a hash dot-write.
        "privacy_verdicts_on_and_off_self",
        r##"class Base
  private token: String
  protected rank: Int
  label: String

  new(token: String, rank: Int)
    @token = token
    @rank = rank
    @label = "base"
  end

  def name
    "base-name"
  end

  def describe
    "#{@label}:#{@token}:#{@rank}:#{@_secret()}:#{this._secret()}"
  end

  def bump
    @rank = @rank + 1
    @token = @token + "!"
    this.label = "bumped"
    [1, 2].map { |n| "#{@token}#{n}#{_secret()}" }
  end

  def outranks?(other)
    @rank > other.rank
  end

  private

  def _secret
    "s"
  end
end

class Child < Base
  def name
    "child-name"
  end

  def rank_of(other)
    other.rank
  end
end

class Plain
  title: String

  new(title: String)
    @title = title
  end
end

def read_token(x)
  x.token rescue "refused"
end

def read_rank(x)
  x.rank rescue "refused"
end

def call_secret(x)
  x._secret() rescue "refused"
end

def write_token(x)
  try
    x.token = "forged"
    return "written"
  catch error
    return "refused"
  end
end

b = new Base("t", 1)
c = new Child("u", 5)
p = new Plain("hello")
print(b.describe)
print(b.bump)
print(b.describe)
print(b.label)
b.label = "outside"
print(b.label)
print(read_token(b))
print(read_token(c))
print(read_rank(c))
print(call_secret(c))
print(write_token(c))
print(c.rank_of(b))
print(b.outranks?(c))
print(c.name)
print(b.name)
print(p.title)
p.title = "changed"
print(p.title)
h = {"k": 1}
h.k = 2
print(h.k)
print(c["label"])
print(c["token"] rescue "index refused")"##,
    ),
    (
        // The memo only says which names a chain declares; which class
        // *defines* a name nearest still decides, so a subclass that redefines
        // a private method without `private` makes it public again. A block
        // calling a peer's private method is refused like the method itself.
        "privacy_nearest_definition_and_blocks",
        r##"class Base
  def shout
    "#{@_whisper}!"
  end

  def peer_whisper(other)
    other._whisper() rescue "refused"
  end

  def peers_in_block(others)
    others.map { |o| o._whisper() rescue "refused" }
  end

  private

  def _whisper
    "psst"
  end

  def reveal
    "hidden"
  end
end

class Open < Base
  def reveal
    "open"
  end
end

class Later
  def ping
    "pong"
  end
end

def reveal_of(x)
  x.reveal rescue "refused"
end

b = new Base()
o = new Open()
print(b.shout)
print(b.peer_whisper(new Base()))
print(b.peers_in_block([new Base(), new Open()]))
print(reveal_of(o))
print(reveal_of(b))
print(new Later().ping)"##,
    ),
    (
        // Instance field reads and writes complete in the VM's fast tier
        // when the general arm's field hit would: on self (`@x`, `this.y`),
        // a local, a block's upvalue, a global and a call's result; a field
        // added after construction, one holding nil or a closure, one
        // shadowing a method. A miss (`p.area`, the method), a frozen value
        // object and a refused private field are handed back and answered
        // as before, the refusals at the same line and column. `Vault` comes
        // last, so the same reads run before and after a class declares a
        // private member.
        "instance_fields_in_and_out_of_the_fast_tier",
        r##"class Point
  x: Int
  y: Int
  label: String

  new(x: Int, y: Int)
    @x = x
    @y = y
    @label = "field"
  end

  def label
    "method"
  end

  def area
    @x * @y
  end

  def walk(n)
    i = 0
    while i < n
      @x = @x + 1
      this.y = this.y + @x
      i = i + 1
    end
    [1, 2].map { |k| @x * k + this.y }
  end
end

class Amount < ValueObject
  cents: Int
end

class Email < ValueObject
  address: String

  new(address: String)
    @address = address.trim.downcase
  end
end

def make
  new Point(7, 8)
end

def poke(point, n)
  i = 0
  while i < n
    point.x = point.x + point.y
    [1].each { |k| point.y = point.y + k }
    i = i + 1
  end
  point.x
end

p = new Point(1, 2)
print(p.walk(3))
print(poke(p, 2))
print(p.x, p.y, make().x)
print(p.label)
print(p.area)
y = (p.x = 5)
print(y)
print(p.x = 9)
p.extra = "dyn"
print(p.extra)
p.extra = nil
print(p.extra)
print(p.extra.nil?)
p.cb = fn(v) { v * 2 }
f = p.cb
print(f(21))
a = Amount.new(5)
try
  a.cents = 6
catch error
  print("frozen: #{error}")
end
print(a.cents)
print(Email.new("  Ada@Example.COM ").address)

class Vault
  private code: Int

  new(code: Int)
    @code = code
  end

  def turn
    [1, 2].each { |k| @code = @code + k }
    @code
  end
end

def forge(vault)
  try
    vault.code = 7
    return "written"
  catch error
    return "refused: #{error}"
  end
end

def peek(vault)
  vault.code rescue "refused"
end

v = new Vault(1)
print(v.turn)
print(forge(v))
print(peek(v))
print(p.walk(1))
print(poke(p, 1))
print(p.label, p.area)"##,
    ),
    (
        // Paren-less zero-argument builtins: the VM's fast tier answers them
        // by method id (`method_table::bare_zero_arg`) instead of by name, and
        // `Int#to_s` / `Float#round` skip a throwaway method value. Every
        // read must still match the tree-walker's, bare and with `()`, from a
        // local and from a global, including a mutating `a.pop` and the
        // `h.len` that stays a method on a hash.
        "bare_zero_arg_builtins_match_parens",
        r##"def reads(h, s, a, n, f, d, nothing)
  print([h.length, h.size, h.empty?, h.keys, h.values, h.present?, h.blank?, h.nil?, h.class])
  print([s.length, s.upcase, s.downcase, s.trim, s.blank?, s.present?, s.to_s, s.reverse, s.chars, s.empty?])
  print([a.length, a.first, a.last, a.empty?, a.sum, a.present?, a.blank?, a.class])
  print([n.to_s, n.to_i, n.to_f, n.nil?, n.class, n.abs, f.to_s, f.to_i, f.round, f.floor, d.to_s, d.round])
  print([nothing.to_s, nothing.nil?, nothing.blank?, nothing.to_i, nothing.class, true.to_s, false.blank?])
  print([h.length(), s.upcase(), a.first(), n.to_s(), f.round(), d.round(), nothing.to_s()])
  print(h.len)
  print([a.pop, a, [].first, {}.empty?, "".blank?, (-7).to_s, 255.to_s(16), 2.567.round(2)])
end

reads({"a": 1, "b": [2]}, "  Hi There ", [3, 1, 2], 42, 2.5, 1.25D, nil)
h = {"x": nil}
s = ""
total = 0
i = 0
while i < 50
  total = total + i.to_s.length + s.length + h.length
  i = i + 1
end
print(total)
print([h.x, h.keys, s.present?, s.class])"##,
    ),
    (
        // A static call resolves in one walk of the chain and is called from
        // there: a subclass's static hides its superclass's (one reads
        // `this`, the other does not), an inherited one, defaults, recursion,
        // a method `extend` mixed in, a native static, and a wrong arity.
        "static_calls_resolve_once",
        r##"module Twice
  def twice(n)
    n * 2
  end
end

class Base
  static def who
    this
  end

  static def label(n)
    "base #{n}"
  end
end

class Svc < Base
  extend Twice

  static def who
    "svc"
  end

  static def compute(n, m = 10)
    n + m
  end

  static def fact(n)
    return 1 if n <= 1

    n * Svc.fact(n - 1)
  end
end

print(Base.who() == Base)
print(Svc.who())
print(Svc.compute(1))
print(Svc.compute(1, 2))
print(Svc.compute(3))
print(Svc.fact(5))
print(Svc.label(4))
print(Svc.twice(21))
print(Math.floor(2.7))
try
  Svc.compute()
catch e
  print("arity")
end"##,
    ),
    (
        // `obj.x ||= v` (and `&&=`, `??=`) that short-circuits writes
        // nothing and evaluates the receiver once, as on the tree-walker: a
        // frozen value object is not refused, a getter is not copied into a
        // field, and only an assigning `pick(...).x ||= v` calls `pick` twice.
        "member_compound_assign_short_circuit_writes_nothing",
        r##"class Amount < ValueObject
  cents: Int
end

class Box
  x: Int
  count: Int

  new()
    @count = 0
  end

  def label
    "computed"
  end

  def fill(v)
    @x ||= v
  end
end

def pick(log, box)
  log.push("pick")
  box
end

a = Amount.new(500)
print(a.cents ||= 7) rescue print("frozen")
print(a.cents &&= nil) rescue print("frozen")
b = Box.new()
print(b.label ||= "x")
print(b.instance_variables.includes?("@label"))
print(b.fill(3), b.fill(9), b.x)
log = []
print(pick(log, b).x ||= 4)
print(log.length)
print(pick(log, b).count ||= 4)
print(log.length)
print(pick(log, b).count &&= 8)
print(log.length)
print(pick(log, b).x ??= 1)
print(log.length)
print(b.fresh ??= 2)
print(log.length)
print(b.fresh, b.count, b.x)
r = (b.count &&= 0)
print(r, b.count)"##,
    ),
    (
        // `is_a?` / `respond_to?` on a plain instance, answered by a walk of
        // the class on the VM: inherited classes and methods, a symbol name,
        // a field (not a method), the universal members, and a wrong argument.
        "instance_is_a_and_respond_to",
        r##"class Animal
  def speak
    "..."
  end
end

class Dog < Animal
  name: String

  new(name: String)
    @name = name
  end

  def bark(times)
    "woof" * times
  end
end

d = Dog.new("rex")
print(d.is_a?("Dog"), d.is_a?("Animal"), d.is_a?("Cat"), d.is_a?("Object"))
print(d.respond_to?("bark"), d.respond_to?("speak"), d.respond_to?(:name), d.respond_to?("name"))
print(d.respond_to?("inspect"), d.respond_to?("nope"), d.respond_to?("method_missing"))
i = 0
hits = 0
while i < 20
  hits += 1 if d.is_a?("Animal") && d.respond_to?("speak")
  i += 1
end
print(hits)
try
  d.is_a?(42)
catch e
  print("bad is_a? argument")
end
try
  d.respond_to?(42)
catch e
  print("bad respond_to? argument")
end"##,
    ),
    (
        // The one-argument array calls the VM answers by method id
        // (`array_method_one_arg`), the `includes?`/`contains?` spellings
        // that now have ids, and the errors a declined argument still raises
        // by name: each must be what the interpreter says, message included.
        "array_and_string_one_arg_calls_by_id",
        r##"def dyn(v)
  v
end
xs = [1, 2, 3, "a", nil, 2.0]
print(xs.includes?(2))
print(xs.includes?(9))
print(xs.contains?(nil))
print(xs.include?("a"))
print(xs.contains(2.0))
print(xs.index_of(3))
print(xs.index_of(42))
print(xs.get(1))
print(xs.get(-1))
print(xs.get(-99))
print(xs.get(99))
print(xs.take(2))
print(xs.take(0))
print(xs.take(99))
print(xs.drop(4))
print(xs.drop(99))
print([1, "b", nil, 2.5].join("-"))
print([].join(","))
s = "hello world"
print(s.includes?("wor"))
print(s.contains?("xyz"))
print(s.include?("o w"))
print(s.contains("h"))
try { xs.get(dyn("1")) } catch e { print(e) }
try { xs.take(dyn(-1)) } catch e { print(e) }
try { xs.drop(dyn("x")) } catch e { print(e) }
try { xs.join(dyn(1)) } catch e { print(e) }
try { s.includes?(dyn(1)) } catch e { print(e) }
try { s.include?(dyn(1)) } catch e { print(e) }
try { s.contains?(dyn(1)) } catch e { print(e) }
try { s.contains(dyn(1)) } catch e { print(e) }
h = {"a": 1}
try { print(h.includes?("a")) } catch e { print(e) }
try { print(h.contains("a")) } catch e { print(e) }
try { print(5.includes?(1)) } catch e { print(e) }"##,
    ),
    (
        // Global natives run on the VM's stack slice (`call_plain_native_at`):
        // arguments in order, through a global and through a local, nested,
        // a native error and an arity error caught with the values around
        // the call intact, and the natives the VM intercepts (`defined`,
        // `const_get`) still answering from its globals.
        "global_native_calls_on_the_stack_slice",
        r##"class Widget
  def label
    "w"
  end
end
def pick(v)
  v
end
total = 0
i = 0
while i < 50
  total = total + int(str(i)) + len([i, i])
  i = i + 1
end
print(total)
print(len(str(int("12")) + "ab"))
f = int
print(f("41") + 1)
print(type(nil))
print(Math.floor(3.7))
print(JSON.stringify({"a": [1, 2]}))
try { len(1, 2) } catch e { print("arity") }
try { int(pick("abc")) } catch e { print("err #{e}") }
acc = []
[1, 2, 3].each do |x|
  try { acc.push(int(pick(x == 2 ? "zz" : str(x)))) } catch e { acc.push(-1) }
end
print(acc)
print(defined("Widget"))
print(defined("NoSuchThing"))
k = const_get("Widget")
print(k == Widget)
a = 1
b = 2
print([a, len("xyz"), b])"##,
    ),
    (
        // Instance method calls in and out of the fast tier (its
        // `CallMethod` / `CallMethodById` arm and the per-site method
        // cache): own, inherited and overridden methods, one site seeing
        // several classes, `@m()` / `this.m()`, recursion, a module's
        // method, `&.`, blocks; and what it hands to the general arm —
        // an omitted default, too few / too many arguments, a private call
        // from outside, `method_missing`, `send`, the depth limit — with
        // each error's text and position.
        "instance_method_calls_in_and_out_of_the_fast_tier",
        r##"module Greets
  def hello(who)
    "hello #{who} from #{@name}"
  end
end

class Shape
  name: String

  new(name: String)
    @name = name
  end

  include Greets

  def label
    "shape #{@name}"
  end

  def area
    0
  end

  def scaled(k)
    @area * k
  end

  def sum3(a, b, c)
    a + b + c + @area()
  end

  def twice
    [@label(), this.label()]
  end

  def fact(n)
    return 1 if n <= 1

    n * @fact(n - 1)
  end

  def corners(block)
    [1, 2].map { |i| block(i) + @area() }
  end

  def secret_twice
    [1, 2].map { |k| "#{@_secret(k)}#{this._secret(k)}" }
  end

  def length
    42
  end

  def to_s
    "Shape(#{@name})"
  end

  def tagged(tag = "plain")
    "#{tag}:#{@name}"
  end

  def deep(n)
    @deep(n + 1)
  end

  def method_missing(name, args)
    "missing #{name} #{args}"
  end

  private

  def _secret(k)
    "s#{k}"
  end
end

class Square < Shape
  side: Int

  new(side: Int)
    super("square")
    @side = side
  end

  def area
    @side * @side
  end

  def label
    "square #{@side}"
  end
end

class Cube < Square
  def volume
    @area() * @side
  end
end

def describe(x)
  x.label()
end

def total(shapes)
  sum = 0
  shapes.each do |s|
    sum = sum + s.scaled(2) + s.length()
  end
  sum
end

def attempt(label, block)
  try
    print("#{label}: #{block()}")
  catch error
    print("#{label} raised: #{error}")
  end
end

def hand_offs(x)
  attempt("defaulted", fn() { x.tagged() })
  attempt("too few", fn() { x.scaled() })
  attempt("too many", fn() { x.scaled(1, 2) })
  attempt("private", fn() { x._secret(1) })
  attempt("missing", fn() { x.whatever(1, 2) })
  attempt("send", fn() { x.send("scaled", 3) })
  attempt("deep", fn() { x.deep(0) })
end

s = new Shape("blob")
q = new Square(3)
c = new Cube(2)
print(s.label(), q.label(), c.label())
print(describe(s), describe(q), describe(c), describe(s), describe(c))
print(s.area(), q.area(), c.area(), c.volume())
print(s.scaled(5), q.scaled(5), c.scaled(5))
print(q.sum3(1, 2, 3), q.twice(), q.fact(10))
print(c.corners(fn(i) { i * 10 }))
print(q.secret_twice())
print(q.length(), q.to_s(), q.hello("ada"))
print(s.tagged("x"), q.tagged("y"))
print(total([s, q, c, q, s]))
i = 0
acc = 0
while i < 50
  acc = acc + q.scaled(i) + c.area()
  i = i + 1
end
print(acc)
n = nil
print(n&.label(), q&.label())
hand_offs(c)
print(c.volume(), describe(c))"##,
    ),
    (
        // A `return` ends a constructor; `new` is still the instance. The VM
        // compiled the `return` as returning nil, so `new Guarded(5)` was
        // nil and the next read of it raised — and so did a `next` outside a
        // loop, which ends the body too. A block's own `return` ends only
        // the block.
        "a_constructor_that_returns_early_is_still_the_instance",
        r##"class Guarded
  x: Int

  new(a)
    @x = a
    return if a > 1

    @x = 0
  end
end

class Skipping
  v: Int

  new(a)
    @v = a
    next if a > 1

    @v = 0
  end
end

class Branchy
  label: String

  new(a)
    if a > 0
      @label = "pos"
      return
    else
      @label = "neg"
    end
    @label = @label + "!"
  end
end

class Looping
  n: Int

  new(limit)
    @n = 0
    while true
      @n = @n + 1
      return if @n >= limit
    end
  end
end

class Finally
  log: String

  new
    @log = ""
    try
      @log = @log + "t"
      return
    finally
      @log = @log + "f"
    end
  end
end

class Each
  seen: Array

  new(xs)
    @seen = []
    xs.each do |x|
      return if x == 2

      @seen.push(x)
    end
    @seen.push(99)
  end
end

g = new Guarded(5)
print(g.x, Guarded.new(0).x, Guarded(9).x)
print(new Skipping(4).v, Skipping.new(1).v)
print(Branchy.new(1).label, new Branchy(-1).label)
print(new Looping(3).n)
print(Finally.new().log)
print(new Each([1, 2, 3]).seen)"##,
    ),
    (
        // Constructors that build their own class, raise through a helper,
        // take defaults and run inside a block callback: all three spellings
        // enter the same compiled constructor.
        "constructors_recurse_raise_and_run_in_callbacks",
        r##"class Node
  value: Int
  next_node: Any

  new(n)
    @value = n
    @next_node = n > 0 ? Node.new(n - 1) : nil
  end

  def depth
    @next_node.nil? ? 1 : 1 + @next_node.depth
  end
end

class Picky
  v: Int

  new(v)
    @v = Picky.check(v)
  end

  static def check(v)
    throw "bad #{v}" if v < 0

    v
  end
end

class Pair
  a: Int
  b: Int

  new(a, b = 10)
    @a = a
    @b = b
  end
end

print(Node.new(4).depth)
caught = "none"
try
  Picky.new(-1)
catch err
  caught = "caught #{err}"
end
print(caught, Picky.new(3).v)
built = [1, 2, 3].map { |x| Pair.new(x) }
print(built.map { |p| p.a + p.b })
print(Pair.new(1, 2).b, new Pair(3).b, Pair(4).b)"##,
    ),
    (
        // C571: the VM answers `Model.new(...)`, a bare `Model.new` and a
        // class value's universal members itself, as `class_member_access`
        // does — attribute form, `_` keys skipped, literal defaults, the same
        // argument errors — instead of asking a tree-walker seeded with every
        // global (~35 µs a call). A model's own static `new` still wins.
        "model_new_and_class_universal_members",
        r##"class Note < Model
  views: Int = 3
end
class Plain
end
class Own < Model
  static def new(x)
    "own #{x}"
  end
end
n = Note.new({"title": "x", "_k": 1})
print(n.title, n._k, n.views)
print(Note.new(nil).views, Note.new().views)
k = Note
print(k.nil?, Note.blank?, Note.present?, Note.class)
print(Plain.nil?, Plain.class, Plain.present?)
print(Note.new)
print(Own.new(1))
try
  Note.new(5)
catch e
  print("hash: #{e}")
end
try
  Note.new({}, {})
catch e
  print("count: #{e}")
end"##,
    ),
    (
        // C571: a model's uploader method calls the app helper from the VM's
        // globals, not through a tree-walker seeded with all of them.
        "model_uploader_method_calls_the_app_helper",
        r##"class Snap < Model
end
Snap.has_one_attached("pic")
Snap.has_many_attached("shots")
def upload_url(record, field, blob_id)
  "url:#{field}:#{blob_id}"
end
s = Snap.new()
print(s.pic_url(), s.pic_url("b1"), s.shots_urls())
print(s.pic_url(1, 2) rescue "refused")"##,
    ),
    (
        // C583: labelled arguments on an instance method keep the receiver — by `obj.`, `this.`, `@` and the bare private helper; a method of arity 0 runs once; unknown, duplicate and missing labels raise.
        "named_args_on_an_instance_method",
        r##"class Calc
  base: Int
  calls: Int

  new()
    @base = 10
    @calls = 0
  end

  def plus(a, b = 5)
    a + b + @base
  end

  def opt(a = 1, b = 2)
    @calls = @calls + 1
    a * 100 + b
  end

  def me(flag)
    this
  end

  def twice
    "#{this.plus(a: 1)} #{@plus(b: 2, a: 3)} #{_half(n: 8)} #{@_half(n: 4)}"
  end

  private

  def _half(n)
    n / 2
  end
end

c = Calc.new()
print(c.plus(a: 1, b: 2), c.plus(1, b: 3), c.plus(a: 4))
print(c.opt(b: 7), c.calls)
print(c.me(flag: true) == c, c.twice)
r = "none"
try
  c.plus(a: 1, zz: 2)
catch e
  r = "caught"
end
d = "none"
try
  c.plus(a: 1, a: 2)
catch e
  d = "caught"
end
m = "none"
try
  c.plus(b: 1)
catch e
  m = "caught"
end
print(r, d, m)"##,
    ),
    (
        // C583: a static called with labels keeps its class as `this`.
        "named_args_on_a_static_that_reads_this",
        r##"class Base
  static def helper(n)
    n + 1
  end

  static def twice(n, k = 2)
    this.helper(n) * k
  end

  static def build(v = 0)
    this.new()
  end

  def hello
    "hi"
  end
end

class Sub < Base
end

print(Base.twice(2, 3), Base.twice(n: 2), Sub.twice(n: 2, k: 5))
print(Sub.build(v: 1).class, Base.build(v: 2).hello)"##,
    ),
    (
        // C583: `nil&.m(k: side())` evaluates no argument.
        "named_args_under_safe_navigation",
        r##"class P
  def m(a, b = 1)
    a + b
  end
end

log = []

def side()
  log.push(1)
  2
end

none = nil
q = P.new()
print(none&.m(a: side()), log.length)
print(q&.m(a: side(), b: 5), log.length)"##,
    ),
    (
        // C583: a field holding a lambda binds the labels by name; `method_missing` gets them as one hash after the positional arguments.
        "named_args_reach_fields_and_method_missing",
        r##"class Box
  f: Any

  new()
    @f = fn(x) { x * 3 }
  end

  def method_missing(name, args)
    "mm #{name} #{args}"
  end
end

b = Box.new()
print(b.f(x: 2), b.nope(z: 1), b.nope(1, z: 2))"##,
    ),
    (
        // C583: a block passed with labels goes to the `&block` parameter, or is dropped when the method declares none.
        "named_args_with_a_block",
        r##"class C
  def amp(a, &blk)
    blk(a)
  end

  def n(a, b = 2, f = nil)
    "#{a} #{b} #{f.nil?}"
  end
end

c = C.new()
r1 = c.amp(a: 4) do |x|
  x + 40
end
r2 = c.n(1, b: 3) do |x|
  x
end
doubler = fn(x) { x * 2 }
r3 = c.amp(a: 5, &doubler)
print(r1, r2, r3)"##,
    ),
    (
        // C583: a bare labelled call naming a builtin inside a method still reaches the builtin (agreed before the fix; guards the global branch).
        "named_args_to_a_global_from_a_method",
        r##"class K
  def run
    "#{type(x: 1)} #{len(a: 1, b: 2)}"
  end
end

print(K.new().run)"##,
    ),
    (
        // C583: a module function called with labels (agreed before the fix).
        "named_args_on_a_module_function",
        r##"module Util
  def shout(s, bang = "!")
    s.upcase + bang
  end
end

print(Util.shout("a"), Util.shout(s: "b"), Util.shout("c", bang: "?"))"##,
    ),
    (
        // DateTime's string form is formatted by hand (no strftime); both
        // engines must print the same text in interpolation, str, to_s, join.
        "datetime_string_forms",
        "d = DateTime.from_unix(-1).utc\nprint(\"at #{d} ok\")\nprint(str(DateTime.from_unix(951782400).utc))\nprint(DateTime.from_unix(4107542399).utc.to_s)\nprint([DateTime.from_unix(0).utc, DateTime.from_unix(86399).utc].join(\" | \"))",
    ),
    // --- JSON of a class instance ---
    // Instance fields live in a hash map with a random seed per instance, and
    // the JSON writers used to follow its iteration order: two instances with
    // the same data serialised in different key orders, so this case printed
    // different bytes from one run to the next. Both engines now write the
    // SEC-013-filtered fields sorted by name.
    (
        "instance_json_sorted_and_filtered",
        r##"class Post
  title: String
  body: String
  views: Int
  category: String
  password_digest: String
  _key: String
  _errors: Array

  new()
    @title = "t"
    @body = "b"
    @views = 3
    @category = "c"
    @password_digest = "secret"
    @_key = "k1"
    @_errors = []
  end
end
print(JSON.stringify(Post.new()))
print(JSON.stringify([Post.new(), Post.new()]))"##,
    ),
    (
        // A method defined on a parent after a subclass has already looked
        // one up reaches the subclass. The tree-walker's flattened method
        // cache kept the subclass's old copy of the parent's methods.
        "define_method_on_a_parent_after_a_subclass_lookup",
        r##"class A
  def hello
    "hi"
  end
end
class B < A
end
class C < B
end
b = B.new()
c = C.new()
print(b.hello, c.hello, b.respond_to?("late"))
A.define_method("late", fn() { "late!" })
print(b.respond_to?("late"), b.late, c.late)
A.alias_method("hello2", "hello")
print(b.hello2, c.hello2)
print(b.methods.includes?("late"), c.methods.includes?("hello2"))"##,
    ),
    (
        // `"str" + x` renders `x` as interpolation does, both ways round and
        // for every kind of operand, including the floats where ryu and
        // Display disagree on notation or digits.
        "string_plus_any_value_renders_like_display",
        r##"print("n=" + 1, "n=" + -42, "n=" + 1.0, "n=" + -0.0, "n=" + (0.1 + 0.2))
print("n=" + 10000000000000000.0, 1658206780088562.2 + "!", "nan:" + "nan".to_f)
print("b" + true, false + "b", nil + "x", :sym + "x", "a" + [1, 2.5, "s"], "h" + {"k": 1})
s = "a prefix longer than the inline limit: "
print(s + 12345, 3.25 + s)
t = "x"
t += 7
t += 1.5
print(t)"##,
    ),
    (
        // String orderings, on the unfused, fused-local and test-jump ops.
        "string_orderings_on_every_comparison_op",
        r##"a = "abc"
b = "abd"
print(a < b, a <= b, a > b, a >= b, b > a, a <= a, "é" > "z")
print(a < "abd", a >= "abc", a > "ab")
n = 0
n += 1 if a < b
n += 10 if a > b
n += 100 if b >= a
print(n)
def lo(x, y)
  x < y
end
def gt(x, y)
  x > y
end
print(lo("a", "b"), gt("a", "b"))
i = 0
w = "a"
while w < "aaaa"
  w = w + "a"
  i += 1
end
print(i, w)"##,
    ),
    (
        // Instance `==`: identity for ordinary objects, structure for enum
        // values, which compare field by field even against themselves.
        "instance_equality_identity_and_enums",
        r##"enum M { None, Some(v: Float) }
class P
  n: Int
  new(n: Int)
    @n = n
  end
end
p1 = P.new(1)
p2 = P.new(1)
print(p1 == p1, p1 == p2, p1 != p2, p1 != p1)
x = M.Some("nan".to_f)
y = M.Some(1.5)
print(x == x, y == y, y == M.Some(1.5), M.None == M.None, M.None == p1, p1 == M.None)"##,
    ),
    // A block may declare fewer parameters than its iterator passes, and it
    // binds only those. The VM passed every argument, so each of these raised
    // "Wrong number of arguments: expected 0, got 1" (or "expected 1, got 2"),
    // a script stopped, and a `serve` handler was demoted after its side
    // effects had run.
    (
        "int_iterators_with_blocks_that_ignore_the_index",
        r##"def tally()
  count = 0
  3.times do
    count = count + 1
  end
  1.upto(3) { count = count + 10 }
  3.downto(1) { count = count + 100 }
  count
end
print(tally())
print(2.times { print("t") })
print(1.upto(2) { print("u") })
print(2.downto(1) { print("d") })"##,
    ),
    (
        "array_iterators_with_blocks_that_ignore_the_element",
        r##"xs = [3, 1, 2]
print(xs.each { print("e") })
print(xs.map { 5 })
print(xs.filter { true })
print(xs.select { false })
print(xs.reject { false })
print(xs.find { true })
print(xs.count { true })
print(xs.any? { true }, xs.all? { false }, xs.none? { false }, xs.one? { true })
print(xs.sort_by { 1 })
print((1..3).map { 0 })
def visits(list)
  seen = 0
  list.each { seen = seen + 1 }
  list.map { seen = seen + 1 }
  list.filter { seen = seen + 1 }
  seen
end
print(visits([7, 8]))"##,
    ),
    (
        "hash_iterators_with_blocks_that_ignore_the_entry",
        r##"h = {"a": 1, "b": 2}
print(h.each { print("h") })
h.each_key { print("k") }
h.each_value { print("v") }
print(h.map { ["k", 9] })
print(h.filter { true }, h.select { false }, h.reject { true })
print(h.keep_if { true }, h.delete_if { false })
print(h.any? { true }, h.all? { false })
print(h.transform_values { 0 }, h.transform_keys { "z" })"##,
    ),
    (
        // `each_with_index { |x| }` gets the element alone and `{ }` gets
        // nothing. A one-parameter `reduce` callback gets the `[acc, item]`
        // pair, seeded or not.
        "blocks_take_a_prefix_of_two_arguments",
        r##"[10, 20].each_with_index { |x| print(x) }
[10, 20].each_with_index { print("i") }
print([[1, 2], [3, 4]].each_with_index { |pair| print(pair) })
print([1, 2, 3].reduce(fn(pair) { pair[0] + pair[1] }, 0))
print([1, 2, 3].reduce(fn(pair) { pair[0] * pair[1] }))
print([1, 2, 3].reduce(fn() { 7 }, 0))
print([1, 2, 3].reduce(fn(acc, x) { acc + x }, 0))"##,
    ),
    (
        // Blocks bind loosely, but a direct call still checks its arity on
        // both engines. Destructuring is unchanged.
        "direct_calls_stay_strict_while_blocks_bind_loosely",
        r##"def call_with(fun, args)
  return fun(args[0]) if args.length == 1

  fun(args[0], args[1])
end
try
  call_with(fn() { 1 }, [5])
  print("not strict")
catch e
  print("a direct call stays strict")
end
try
  call_with(fn(a) { a }, [1, 2])
  print("not strict")
catch e
  print("too many stays strict")
end
table = {"f": fn() { 1 }}
try
  table.f(5)
  print("not strict")
catch e
  print("a hash entry call stays strict")
end
print([1, 2].map { |x| x * 2 })
print([[1, 2], [3, 4]].map { |a, b| a + b })"##,
    ),
    // The block iterators that run their callback inside the VM's dispatch
    // loop instead of a nested `run()` per element: each_with_index, reject,
    // find, any?/all?/none?/one?, count, sort_by, reduce/fold, Int#times/
    // upto/downto, and blocks that destructure array elements. Templates
    // (`x > 4`), the frame path (`x % 2 == 0`) and a template handing a
    // float to the frame path (`v > 1` on 2.5) must all agree.
    (
        "array_block_methods_in_the_dispatch_loop",
        r##"xs = [5, 3, 8, 1]
print(xs.each_with_index { |x, i| print("#{i}:#{x}") })
print(xs.reject { |x| x > 4 }, xs.reject { |x| x % 2 == 0 })
print(xs.find { |x| x > 4 }, xs.find { |x| x > 100 }, xs.find { |x| x % 4 == 0 })
print(xs.any? { |x| x > 7 }, xs.all? { |x| x > 0 }, xs.none? { |x| x > 7 }, xs.one? { |x| x > 7 })
print(xs.any? { |x| x > 70 }, xs.all? { |x| x > 1 }, xs.none? { |x| x > 70 }, xs.one? { |x| x > 2 })
print(xs.count { |x| x > 2 }, xs.count { |x| x % 2 == 1 })
print(xs.sort_by { |x| x }, xs.sort_by { |x| x * -1 }, ["bb", "a", "ccc"].sort_by { |s| s.length })
print([[2, "b"], [1, "a"], [2, "a"], [1, "b"]].sort_by { |p| p[0] })
print([1, 2.5, 3].count { |v| v > 1 }, [1, 2.5, 3].find { |v| v > 2 }, [0.5, 1, 2].any? { |v| v > 1 })
print([3, 1.5, 2].sort_by { |v| v * 2 }, [1, 2.5, 3].reject { |v| v > 2 })
def probe(list)
  calls = 0
  found = list.find do |x|
    calls = calls + 1
    x > 2
  end
  any = list.any? do |x|
    calls = calls + 1
    x > 1
  end
  none = list.none? do |x|
    calls = calls + 1
    x > 2
  end
  one = list.one? do |x|
    calls = calls + 1
    x > 1
  end
  all = list.all? do |x|
    calls = calls + 1
    x < 2
  end
  "#{found} #{any} #{none} #{one} #{all} #{calls}"
end
print(probe([1, 2, 3, 4]))"##,
    ),
    (
        "reduce_in_the_dispatch_loop",
        r##"print([1, 2, 3].reduce(fn(acc, x) { acc + x }, 10))
print([1, 2, 3].reduce(fn(acc, x) { acc * x }))
print([7].reduce(fn(acc, x) { acc + x }), [].reduce(fn(acc, x) { acc + x }, 5))
print([1, 2].fold(fn(acc, x) { acc - x }, 0))
print(["a", "b"].reduce(fn(acc, s) { acc + s }, ""))
print([[1, 2], [3, 4]].reduce(fn(acc, row) { acc + row.find { |v| v > 1 } }, 0))
try
  [].reduce(fn(acc, x) { acc + x })
  print("no error")
catch e
  print("reduce of an empty array raises")
end
vs = [1, 2, 3]
total = vs.reduce(fn(acc, v) {
  vs.push(v) if acc < 3
  acc + v
}, 0)
print(total, vs)"##,
    ),
    (
        "int_iterators_in_the_dispatch_loop",
        r##"def tally(n)
  total = 0
  n.times { |i| total += i }
  total
end
print(tally(5))
print(3.times { |i| print("t#{i}") })
print(2.upto(4) { |i| print("u#{i}") })
print(4.downto(2) { |i| print("d#{i}") })
print(0.times { |i| print("never") }, (0 - 3).times { |i| print("never") })
print(5.upto(3) { |i| print("never") }, 1.downto(3) { |i| print("never") })
seen = []
3.times { |i| 2.times { |j| seen.push(i * 10 + j) } }
print(seen)
print([1, 2, 3].count { |v| [2, 3, 4].any? { |w| w == v } })
fns = []
3.times do |i|
  fns.push(fn() { i })
end
print(fns[0](), fns[2]())"##,
    ),
    (
        "destructuring_blocks_in_the_dispatch_loop",
        r##"pairs = [[1, 2], [3, 4], [5]]
print(pairs.map { |a, b| [b, a] })
print(pairs.filter { |a, b| b.nil? }, pairs.reject { |a, b| a > 2 })
print(pairs.find { |a, b| a > 2 }, pairs.count { |a, b| a > 2 }, pairs.any? { |a, b| b.nil? })
print(pairs.sort_by { |a, b| a * -1 })
pairs.each { |a, b| print("#{a}-#{b}") }
print([[1, 2, 3]].map { |a, b| a + b }, [[1, 2, 3], [4, 5, 6]].map { |a, b, c| a + b + c })
print([5, 6].each_with_index.map { |v, i| v * i })
print({"a": 1, "b": 2}.entries.map { |k, v| "#{k}=#{v}" })
ps = [[1, 2], [3, 4], [5, 6]]
sums = ps.map do |a, b|
  ps.pop()
  a + b
end
print(sums, ps.length)"##,
    ),
    (
        // A throw out of each kind of loop reaches the caller's catch with its
        // value, and the next loop starts clean. A block that changes the
        // array it walks sees the live array, up to the length it started
        // with.
        "block_loops_throw_and_see_their_receiver_change",
        r##"try
  [1, 2].find { |v| throw "f#{v}" }
catch e
  print(e)
end
try
  3.times { |i| throw "t#{i}" if i == 1 }
catch e
  print(e)
end
try
  [[1, 2]].map { |a, b| throw "p#{a}#{b}" }
catch e
  print(e)
end
try
  [1, 2].reduce(fn(a, v) { throw "r#{v}" }, 0)
catch e
  print(e)
end
try
  [7, 8].each_with_index { |v, i| throw "e#{v}#{i}" if i == 1 }
catch e
  print(e)
end
try
  [1, 2].sort_by { |v| throw "s#{v}" }
catch e
  print(e)
end
try
  [1, 2].count { |v| throw "c#{v}" }
catch e
  print(e)
end
print([1, 2].map { |v| v * 2 })
ys = [1, 2, 3, 4]
seen = []
ys.each_with_index do |y, i|
  seen.push(y)
  ys.pop() if i == 0
end
print(seen, ys)
zs = [1, 2, 3]
found = zs.find do |z|
  zs.push(z + 10)
  z > 5
end
print(found, zs)
ss = [3, 1, 2, 4]
sorted = ss.sort_by do |s|
  ss.pop()
  s
end
print(sorted, ss)"##,
    ),
    // Closures over a `try` body's locals, after a throw caught in the same
    // function. The VM closed only the cells of the frames it unwound, so
    // these stayed open on slots the unwinding gave back: they read, and
    // wrote, whatever came to live there next (`y` became 101).
    (
        "closures_over_try_body_locals_survive_the_unwind",
        r##"def caught_here(fns)
  try
    let x = 5
    fns.push(fn() { x })
    fns.push(fn() { x = x + 1
      x })
    throw "e"
  catch e
    print("caught #{e}")
  end
  let y = 99
  let z = 100
  print(fns[1](), fns[1](), y, z)
  fns[0]()
end
print(caught_here([]))
def boom
  throw "b"
end
def caught_below(fns)
  try
    let x = 6
    fns.push(fn() { x })
    boom()
  catch e
    print("caught #{e}")
  end
  let y = 98
  [fns[0](), y]
end
print(caught_below([]))
def rescued(fns)
  value = (fn() {
    let w = 7
    fns.push(fn() { w })
    boom()
  })() rescue 1
  let q = 77
  [value, fns[0](), q]
end
print(rescued([]))"##,
    ),
    // `total += x`, `count += 1`, `text = text + s` into variables a block
    // captures: one `AddAssignUpvalue` each, numbers in the fast tier,
    // strings appended in place, mixed int/float, `<<`. An overflow raises
    // the same error at the same place as the tree-walker — the assignment,
    // which `big = big + one` reported at its `+` before.
    (
        "captured_accumulators_add_like_the_tree_walker",
        r##"def sums(xs)
  total = 0
  count = 0
  ratio = 0.5
  mixed = 1
  text = ""
  parts = []
  xs.each do |x|
    total += x
    count += 1
    ratio = ratio + x * 0.25
    mixed += 0.5
    text = text + "#{x},"
    text += "|"
    parts << x
  end
  [total, count, ratio, mixed, text, parts]
end
print(sums([1, 2, 3]))
def overflow
  big = 9223372036854775807
  one = 1
  [1].each do |v|
    try
      big = big + one
    catch e
      print(e)
    end
    try
      big += 1
    catch e
      print(e)
    end
    try
      big = big + one * 2
    catch e
      print(e)
    end
  end
  big
end
print(overflow())
def shared
  n = 0
  bump = fn() { n += 1 }
  [1, 2, 3].each { |x| n = n + x }
  bump()
  bump()
  n
end
print(shared())
def strings(words)
  out = ""
  words.each { |w| out = out + w }
  copy = out
  words.each { |w| out += "-" }
  [out, copy]
end
print(strings(["a", "b"]))"##,
    ),
    // Inner blocks over an outer block's parameter or local (the outer
    // frame is rerun with its cells closed), a block reused across calls of
    // its method while closures over the method's variables escape, and
    // recursion through a capturing block.
    (
        "nested_and_reused_capturing_blocks",
        r##"def grid(rows)
  out = []
  rows.each do |row|
    k = row.length
    row.each { |c| out.push(c * 10 + k) }
  end
  out
end
print(grid([[1, 2], [3], [4, 5, 6]]))
def pairs(xs)
  xs.map { |a| xs.map { |b| a * b } }
end
print(pairs([1, 2, 3]))
kept = []
def scale(xs, k, kept)
  xs.map do |x|
    kept.push(fn() { k }) if x > 2
    x * k
  end
end
print(scale([1, 2], 10, kept))
print(scale([3, 4], 100, kept))
print(scale([5], 1000, kept))
print(kept.map { |f| f() })
def walk(node, depth, seen)
  node["kids"].each { |kid| walk(kid, depth + 1, seen) }
  seen.push("#{node["name"]}@#{depth}")
  seen
end
tree = {"name": "a", "kids": [{"name": "b", "kids": [{"name": "c", "kids": []}]}, {"name": "d", "kids": []}]}
print(walk(tree, 0, []))
def tally_by(words)
  counts = {}
  words.each do |w|
    counts[w] = (counts[w] || 0) + 1
  end
  total = 0
  counts.each { |k, v| total += v }
  [counts, total]
end
print(tally_by(["x", "y", "x"]))
def deep_capture(xs, m)
  xs.map { |x| [x].map { |y| [y].map { |z| x + y + z + m } } }
end
print(deep_capture([1, 2], 100))"##,
    ),
    // A block result that fails the method — a `map` pair or a transformed
    // key that cannot be a hash key — is reported at the method's call, as
    // the native driver does: with or without a capture, from a pass that
    // reruns the block's frame, from the template that runs after one, at
    // the top level. The VM reported the op the block last left the fast
    // tier for (`3:21`, `53:24`); once capturing blocks reran their frame,
    // those diverged too.
    (
        "loop_block_result_errors_point_at_the_call",
        r##"def plain_map(h)
  try
    h.map { |k, v| [{"z": 1}, v] }
  catch e
    print("plain map: #{e}")
  end
end
plain_map({"a": 1})
def capturing_map(h, fns)
  try
    h.map do |k, v|
      fns.push(fn() { k })
      [{"z": 1}, v]
    end
  catch e
    print("capturing map: #{e}")
  end
end
capturing_map({"a": 1}, [])
def capturing_local_map(h, fns)
  try
    h.map do |k, v|
      x = v + 1
      fns.push(fn() { x })
      [{"z": x}, v]
    end
  catch e
    print("capturing local map: #{e}")
  end
end
capturing_local_map({"a": 1}, [])
def plain_keys(h)
  try
    h.transform_keys { |k| [1.5] }
  catch e
    print("plain keys: #{e}")
  end
end
plain_keys({"a": 1})
def capturing_keys(h, fns)
  try
    h.transform_keys do |k|
      fns.push(fn() { k })
      [1.5]
    end
  catch e
    print("capturing keys: #{e}")
  end
end
capturing_keys({"a": 1}, [])
def template_after_a_pass(h)
  try
    h.map { |k, v| [v, k * 2] }
  catch e
    print("template: #{e}")
  end
end
template_after_a_pass({"a": 1, 2: [1]})
top = {"a": 1}
try
  top.map { |k, v| [{"z": 1}, v] }
catch e
  print("top level: #{e}")
end
print("after", capturing_map({"b": 2}, []), [1, 2].map { |n| n * 2 })"##,
    ),    (
        // `filter` / `select` / `reject`, `find` and `sort_by` return the
        // element they handed the block — not what the receiver holds at
        // that index once the block has replaced it, nor what the block
        // reassigned its parameter to. The VM's loop keeps that element in
        // a slot of its own.
        "array_block_kept_element_is_the_one_handed_in",
        r##"fs = [1, 2, 3, 4]
r1 = fs.filter do |v|
  fs[0] = 9 if v == 1
  v = 100
  true
end
print("#{r1} #{fs}")
hs = [{"v": 1}, {"v": 2}, {"v": 3}]
r2 = hs.select do |h|
  hs[1] = {"v": 0} if h["v"] == 2
  h["v"] > 1
end
print("#{r2} #{hs}")
ss = ["a", "bb", "ccc"]
r3 = ss.reject do |s|
  ss[2] = "z" if s == "ccc"
  s.length == 2
end
print("#{r3} #{ss}")
ds = [5, 6, 7]
r4 = ds.find do |d|
  ds[1] = 60 if d == 6
  d > 5
end
print("#{r4} #{ds}")
ks = [3, 1, 2]
r5 = ks.sort_by do |k|
  ks[0] = 30 if k == 3
  k
end
print("#{r5} #{ks}")"##,
    ),
    (
        // `s[i]` reads one character without decoding the whole string
        // (an ASCII byte read, else a `chars().nth` walk), on GetIndex,
        // GetLocalIndex and GetNestedIndex. The paren-form zero-arg string
        // methods share the receiver when nothing changes, split/lines
        // build their pieces directly, and a pad that adds nothing returns
        // the receiver. Negative indexes on multi-byte text are covered by
        // `string_negative_index_counts_characters`.
        "string_index_and_shared_string_results",
        r##"def at(s, i)
  s[i]
end
def scan(s)
  out = []
  i = 0
  while i < 5
    out.push(s[i])
    i += 1
  end
  out
end
a = "hello"
m = "héllo wörld😀"
print(at(a, 0), at(a, 4), at(a, -1), at(a, -5))
print(at(m, 0), at(m, 1), at(m, 2), at(m, 7), at(m, 11))
print(scan(m))
print(m[1] + m[11])
counts = {"h": 0}
counts[a[0]] = counts[a[0]] + 1
print(counts)
try
  at(a, 5)
catch e
  print("oob")
end
try
  at(m, 11)
  print("ok11")
  at(m, 12)
catch e
  print("oob2")
end
long = "  " + "x" * 40 + "  "
print(long.trim().length, long.lstrip().length, long.rstrip().length, long.to_s() == long)
print("ABC".upcase(), "abc".upcase(), "ÉcOLE".downcase(), "école".upcase(), "MiXeD".downcase())
print("line\n".chomp(), "line".chomp(), "a\r\n".chomp().length, "abc".join())
print("a,b,,c".split(","), "a b".split(), "abc".split(""), "x".split(","))
print("one\ntwo\n".lines(), "ab".ljust(1), "ab".ljust(5, "*"), "ab".rjust(4), "ab".center(6, "-"))
print("ab".lpad(1), "ab".lpad(4, "0"), "ab".rpad(4, "."), "é".ljust(4, "é"))"##,
    ),
    (
        // A negative `s[i]` counts characters from the end on both engines.
        // The tree-walker offset it from the byte length, so on non-ASCII
        // text `"héllo"[-1]` raised there (and `[-2]` read the wrong
        // character), and its out-of-range message gave the byte length
        // where the VM gave the character count. Error texts are printed:
        // they must match too.
        "string_negative_index_counts_characters",
        r##"def at(s, i)
  s[i]
end
def attempt(s, i)
  try
    print(at(s, i))
  catch e
    print(e)
  end
end
m = "héllo"
print(m[-1], m[-2], m[-4], m[-5], at(m, -1), at(m, -4))
e = "😀ab日本"
print(e[-1], e[-2], e[-3], e[-5], e[0], e[4])
print("日本語"[-3], "aé"[-1], "éa"[-1], "éa"[-2])
attempt(m, 5)
attempt(m, -6)
attempt(e, -6)
attempt(e, 5)
attempt("", -1)"##,
    ),
    (
        // `for ch in s` yields one-character strings on both engines. The
        // tree-walker raised `cannot iterate over string` while the VM
        // iterated, so a string that reached the loop through an untyped
        // parameter or return value worked on one engine only (and the
        // checker refused a visible String). Covers non-ASCII text, the
        // index variable, break/next, an empty string, a closure-capturing
        // body (the tree-walker's per-iteration environment path), and
        // reassigning the variable mid-loop (the loop keeps its own handle).
        "for_in_string_iterates_characters",
        r##"def get(x)
  x
end
s = get("héllo 😀日")
for ch in s
  print(ch)
end
for ch, i in s
  print("#{i}:#{ch}")
end
for ch in "abc"
  print(ch.upcase)
end
out = []
for ch in get("abcdef")
  next if ch == "b"
  break if ch == "e"

  out.push(ch)
end
print(out)
for ch in get("")
  print("never")
end
fns = []
for ch in get("xyz")
  fns.push(fn() { ch })
end
print(fns.map { |f| f() })
word = get("ab")
seen = []
for ch in word
  word = "zzzz"
  seen.push(ch)
end
print(seen, word)"##,
    ),
    // A zero-argument def read bare is called on both engines. The VM read
    // `signed_in?` as the function itself, always truthy, so an auth guard
    // `return … unless signed_in?` let everyone through on the VM.
    (
        "bare_zero_argument_def_is_called",
        r##"def signed_in?
  false
end
def greeting(name = "you")
  "hi #{name}"
end
def guard
  return "redirect" unless signed_in?
  "page"
end
print(guard())
state = signed_in?
print(state, greeting, greeting.upcase)
cb = fn() { 7 }
print(cb())"##,
    ),
    (
        // A bare local whose name the function also binds with a
        // comprehension variable compiled to a VM global: the recursive call
        // overwrote the caller's `item`.
        "local_also_bound_by_comprehension_survives_recursion",
        r##"def summary(items, depth)
  item = items[0]
  if depth > 0
    summary(["inner"], depth - 1)
  end
  names = [item.upcase for item in items]
  "#{item}: #{names}"
end
print(summary(["outer", "b"], 1))"##,
    ),
    (
        // Same with a `catch` and a `for` variable.
        "local_also_bound_by_catch_and_for_survives_recursion",
        r##"def guarded(secret, depth)
  e = secret
  i = depth
  guarded("inner", depth - 1) if depth > 0
  try
    throw "boom"
  catch e
    nil
  end
  for i in [7, 8]
    nil
  end
  "#{e}:#{i}"
end
print(guarded("alice-password", 1))"##,
    ),
    // --- XML (lane L1): navigation, index hooks, display, equality,
    // building and mutation, and the block builder ---
    (
        "xml_navigation",
        r##"doc = Xml.parse("<rss v=\"2\"><channel><title>A &amp; B</title><item id=\"1\"><t>x</t></item><item id=\"2\"/></channel></rss>")
print(type(doc), doc.kind, doc.root.name)
ch = doc.root.children[0]
print(ch.name, ch.children.length, ch.nodes.length)
items = ch.children.filter { |c| c.name == "item" }
print(items.map { |i| i["id"] })
print(items[0].path, items[0].text, ch.children[0].text)
print(items[0].parent == ch, items[0] == items[1], items[0].next_sibling == items[1])
print([items[0], items[1].previous_sibling].uniq.length)
print(items[0].ancestors.map { |a| a.name })
print(doc.declaration, doc.root.attributes, doc.root.has_attribute?("v"))"##,
    ),
    (
        "xml_index_and_get",
        r##"root = Xml.parse("<a id=\"7\" x=\"y\"/>").root
print(root["id"], root.get("x"), root["nope"], root["__xml"])
k = "id"
print(root[k])
root["id"] = 8
root["x"] = nil
root.set("z", 1)
print(root)"##,
    ),
    (
        "xml_display_json_eq",
        r##"doc = Xml.parse("<r><a k=\"1\">t</a></r>")
a = doc.root.children[0]
print(a)
print("#{a}")
print(a.inspect)
print(json_stringify({"n": a}))
print(Xml.parse("<a/>") == Xml.parse("<a/>"), doc.root.children[0] == a)
print([a].includes?(doc.root.children[0]))"##,
    ),
    (
        "xml_dot_write_refused",
        r##"n = Xml.element("a")
n.text = "hi"
n.name = "b"
print(n)
try
  n.foo = 1
catch e
  print("refused")
end
try
  n.has_key("x")
catch e
  print("no has_key")
end"##,
    ),
    (
        "xml_build_and_mutate",
        r##"doc = Xml.document("rss", version: "2.0")
channel = doc.root.add("channel")
channel.add("title", "News & views")
[1, 2].each do |i|
  item = channel.add("item", id: i)
  item.add("title", "P#{i}")
  item.add_cdata("<b>#{i}</b>")
end
print(doc.to_xml(indent: 2))
print(doc.to_xml)
el = Xml.element("a", ["t", Xml.element("b")], {"x": 1})
print(el, el.to_h)
el.children[0].remove
el.prepend("s")
print(el, el.dup == el)
print(Xml.parse("<i id=\"1\"><t>A</t><g>x</g><g>y</g></i>").to_h)"##,
    ),
    (
        "xml_block_builder_do",
        r##"doc = Xml.build("feed", {"xmlns": "urn:a"}) do |f|
  f.add("title", "T")
  [1, 2].each do |i|
    f.add("entry", id: i) do |e|
      e.add("n", i * 10)
    end
  end
end
print(type(doc))
print(doc.to_xml(indent: 2))
c = doc.root.add("c", nil, {"k": "v"}) do |x|
  x["at"] = "1"
end
print(c)"##,
    ),
    (
        "xml_block_builder_fn",
        r##"root = Xml.element("r")
blk = fn(n) { n.add("viafn") }
root.add("w", &blk)
root.add("w2", fn(n) { n.add("positional") })
doc = Xml.build("d", &blk)
print(root)
print(doc.root)"##,
    ),
    (
        "xml_block_builder_throw",
        r##"root = Xml.element("r")
try
  root.add("a") do |a|
    a.add("before")
    throw "boom"
  end
catch e
  print("caught #{e}")
end
print(root)
try
  Xml.build("x") do |x|
    throw "doc boom"
  end
catch e
  print("caught #{e}")
end"##,
    ),
    (
        "xml_block_builder_break",
        r##"root = Xml.element("r")
root.add("a") do |a|
  a.add("first")
  break
end
root.add("b")
print(root)
n = 0
Xml.build("x") do |x|
  [1, 2, 3].each do |i|
    x.add("i") do |c|
      n = n + 1
      break if i == 2
      c.add("after")
    end
  end
end
print(n)"##,
    ),
    (
        "xml_no_constructor",
        r##"def attempt(label, f)
  try
    f()
    print(label, "built")
  catch e
    print(label, e)
  end
end
attempt("bare", fn() { XmlNode.new })
attempt("empty", fn() { XmlNode.new() })
attempt("positional", fn() { XmlNode.new("x") })
attempt("named", fn() { XmlNode.new(name: "x") })
attempt("call", fn() { XmlNode() })
attempt("doc bare", fn() { XmlDocument.new })
attempt("doc empty", fn() { XmlDocument.new() })
attempt("doc named", fn() { XmlDocument.new(name: "x") })"##,
    ),
    (
        "xml_safe_member_named_and_block",
        r##"q = Xml.element("q")
print(q&.add("z", id: 2))
r = Xml.element("r")
r&.add("x") do |x|
  x.add("y")
end
print(r)
print(Xml.element("q")&.to_xml(indent: 2))
nothing = nil
print(nothing&.to_xml(indent: 2))
res = nothing&.add("never", id: 1) do |x|
  print("not run")
end
print(res)
def side()
  print("evaluated")
  1
end
print(nothing&.add("z", id: side()))
print(q&.add("w", id: side()))"##,
    ),
    (
        "xml_add_is_atomic",
        r##"n = Xml.element("x")
try
  n.add("y", [n])
catch e
  print("refused")
end
print(n)"##,
    ),
    // --- XML queries (lane L2): find / find_all (lenient) and xpath /
    // at_xpath (strict), namespaces, result types and errors ---
    (
        "xml_find_and_xpath",
        r##"doc = Xml.parse("<rss><channel><title>A &amp; B</title><item id=\"1\"><t>x</t></item><item id=\"2\"/></channel></rss>")
print(doc.find("channel/title")&.text, doc.find("rss"), doc.at_xpath("channel"))
print(doc.find_all("//item/@id"), doc.xpath("rss/channel/item").length)
print(doc.find("channel/item[@id='2']") == doc.xpath("//item")[1])
print(doc.xpath("//item[last()]/@id | //title/text()"))
print(type(doc.at_xpath("/")), doc.find_all("//item[t='x']").map { |i| i["id"] })
print(doc.find("channel").find_all("item[position() > 1]"), doc.find("missing")&.text)"##,
    ),
    (
        "xml_xpath_namespaces",
        r##"feed = Xml.parse("<feed xmlns=\"urn:a\" xmlns:x=\"urn:x\"><entry><x:m k=\"v\"/></entry><entry/></feed>")
print(feed.find_all("entry").length, feed.xpath("//entry").length, feed.xpath("//xmlns:entry").length)
print(feed.xpath("//e:entry", {"e": "urn:a"}).length, feed.xpath("//entry", {"": "urn:a"}).length)
print(feed.at_xpath("//x:m/@k"), feed.at_xpath("//y:m/@k", y: "urn:x"), feed.find("//*:m"))"##,
    ),
    (
        "xml_xpath_errors",
        r##"doc = Xml.parse("<a/>")
["sum(//a)", "//q:a", "a[", "//a[@id=1]", "child::a"].each do |expr|
  try
    doc.xpath(expr)
  catch e
    print(e)
  end
end
try
  doc.find(42)
catch e
  print(e)
end
n = Xml.element("a")
n.add("b", id: 1)
print(n.at_xpath("/a/b/@id"), n.at_xpath("/"))"##,
    ),
    // --- XML streaming (lane L3): Xml.each with a do block, &fn, a throw
    // and break; Xml.reader with break; matches support find / xpath ---
    (
        "xml_each_do_block",
        r##"feed = "tests/builtins/_fixtures/xml/feed.rss"
titles = []
n = Xml.each(feed, "channel/item") do |item|
  titles.push(item.find("title").text)
  print(item["id"], item.parent, item.at_xpath("dc:creator/text()"), item.find_all("*").length)
end
print(n, titles)
def in_def(path)
  total = 0
  Xml.each(path, "item") do |item|
    total = total + item["id"].to_i
  end
  total
end
print(in_def(feed))
print(Xml.each(feed, "dc:creator") do |c|
  print(c, c.namespace)
end)"##,
    ),
    (
        "xml_each_fn_block",
        r##"feed = "tests/builtins/_fixtures/xml/feed.rss"
ids = []
collect = fn(node) { ids.push(node["id"]) }
print(Xml.each(feed, "item", &collect), ids)
print(Xml.each(feed, "item[@id='2']", fn(node) { ids.push("p" + node["id"]) }), ids)
cb = Xml.each
print(cb(feed, "title") do |t|
  ids.push(t.text)
end, ids)"##,
    ),
    (
        "xml_each_throw",
        r##"feed = "tests/builtins/_fixtures/xml/feed.rss"
hits = 0
try
  Xml.each(feed, "item") do |item|
    hits = hits + 1
    throw "stop #{item["id"]}"
  end
catch e
  print("caught", e, hits)
end
try
  Xml.each(feed, "item")
catch e
  print("no block", e)
end
try
  Xml.each(feed, "item[1]") do |item|
    print("never")
  end
catch e
  print("bad match", e)
end
print(Xml.each(feed, "item") do |item|
  nil
end)"##,
    ),
    (
        "xml_each_break_and_reader",
        r##"feed = "tests/builtins/_fixtures/xml/feed.rss"
seen = []
n = Xml.each(feed, "item") do |item|
  break if item["id"] == "1"
  seen.push(item["id"])
end
print(n, seen)
reader = Xml.reader(feed, "item")
print(reader, type(reader))
while (item = reader.next)
  print("next", item["id"], item.find("title").text)
  break if item["id"] == "1"
end
print(reader.count)
reader.close
print(reader.next, reader.count)
r2 = Xml.reader(feed, "title")
print(r2.next.text, r2.next.text, r2.next.text, r2.next, r2.count)"##,
    ),
    (
        // Card #657: a `{ |x| … }` block after a call in an `if`/`unless`/
        // `elsif`/`while` condition is the call's block, not the body.
        "brace_block_in_condition",
        r##"xs = [1, 2, 3]
if xs.any? { |x| x > 2 }
  print("any")
end
unless xs.all? { |x| x > 2 }
  print("not all")
end
if xs.none? { |x| x > 5 }
  print("none")
elsif xs.any? { |x| x > 1 }
  print("never")
end
if xs.filter { |x| x > 1 }.length > 0 { print("brace body") }
print("postfix") if xs.find { |x| x == 2 }
print("postfix unless") unless xs.any? { |x| x > 9 }
n = 0
while xs.any? { |x| x > n }
  n = n + 1
end
print(n)
if xs.any? { |x|
  x.to_s == "3"
}
  print("multiline")
end"##,
    ),
    // --- card #698: an unknown method read bare on a String / Array ---
    (
        "unknown_bare_string_method_errors",
        "try\n  x = \"s\".upcse\n  print(\"bound\")\ncatch e\n  print(e)\nend",
    ),
    (
        "unknown_bare_array_method_errors",
        "try\n  x = [1].lenght\n  print(\"bound\")\ncatch e\n  print(e)\nend",
    ),
    (
        "known_bare_string_method_binds",
        "f = \"a,b\".split\nprint(f(\",\"))\nprint(\"ab\".upcase)\nprint([3, 1].sort)",
    ),
    (
        // WebAuthn is one native class serving both engines: the verified
        // result and a rejection's message must match (an OpenSSL-signed
        // RS256 vector, read relative to the crate root).
        "webauthn_verify_and_reject",
        "v = JSON.parse(slurp(\"tests/fixtures/webauthn/vectors.json\"))[\"rs256\"]\nr = WebAuthn.verify_authentication(v[\"authentication\"], {\"challenge\": v[\"authentication_challenge\"], \"origins\": \"https://example.com\", \"rp_id\": \"example.com\", \"public_key\": v[\"public_key\"]})\nprint(r[\"user_handle\"], r[\"sign_count\"], r[\"counter_regressed\"])\no = WebAuthn.registration_options({\"rp_id\": \"example.com\", \"user_id\": \"u\", \"user_name\": \"n\"})\nprint(o[\"options\"][\"challenge\"] == o[\"challenge\"])\ntry\n  WebAuthn.verify_registration(v[\"registration\"], {\"challenge\": \"x\", \"origins\": \"https://example.com\", \"rp_id\": \"example.com\"})\ncatch err\n  print(err)\nend\nprint(WebAuthn.verify_registration({}, {\"challenge\": \"x\", \"origins\": \"o\", \"rp_id\": \"r\"}) rescue \"rescued\")",
    ),
];
/// Cases that currently diverge because of an unfixed VM bug. Keep this list in
/// sync with reality: when a fix lands, the corresponding case starts matching
/// and the test will tell you to remove it from here.
const KNOWN_DIVERGENT: &[&str] = &[
    // A block's frame carries no class, so `super` in a block of a method
    // has nothing to resolve against on the VM. Needs the compiler to hand
    // the block the class it was written in.
    "super_in_a_block_of_a_method",
    // Sub-expression comprehensions and binding `match` as a call argument
    // now compile via a clean-frame lambda (`Compiler::wrap_in_lambda`).
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
    //        Class.vm_methods (the constructor on Class.vm_init); they
    //        dispatch with the receiver in the callee slot as `this`
    //   named_attribute_form_runs_a_computed_default — the VM answers
    //        `Klass.new(k: v)` itself and runs the field defaults
];

/// Programs the type checker refuses but both engines run. It rejects a read
/// that comes before the variable's assignment in the source, even inside a
/// closure that is only called afterwards — the shapes where the VM must keep
/// hoisting a bare local rather than declare it at its first assignment. Run
/// with `--no-type-check`, the engines must still agree.
const UNCHECKED_CASES: &[(&str, &str)] = &[
    (
        "bare_local_read_by_a_closure_made_before_its_assignment",
        "def later\n  get = fn() { x }\n  x = 5\n  get()\nend\nprint(later())",
    ),
    (
        "bare_local_read_by_a_nested_def_made_before_its_assignment",
        "def outer\n  def inner\n    x + 1\n  end\n  x = 41\n  inner()\nend\nprint(outer())",
    ),
    // A top-level variable read before its first assignment is undefined
    // there on both engines: it must stay a global, not a slot reading nil.
    (
        "toplevel_read_before_assignment_stays_undefined",
        "seen = z rescue \"undefined\"\nprint(seen)\nz = 1\nprint(z)",
    ),
    // A second `let` of a top-level variable kept in a slot.
    (
        "toplevel_let_redeclared",
        "let a = 1\nlet a = a + 100\nprint(a)",
    ),
    // Membership in a list of literals across types, which the type checker
    // refuses: `==` as `Array#includes?` has it — `Int` and `Float` by value,
    // a class equal to the string of its name, arrays by value.
    (
        "literal_list_membership_across_types",
        r##"class Account
end
print([1, 2, 3].includes?(2.0), [1.5, 2.0].includes?(2), [1, "1"].includes?("1"))
print(["1"].includes?(1), [nil].includes?(false), [true].includes?(1), [0].includes?(false))
print(["up"].includes?(:up), [:up].includes?("up"), ["Account", "User"].includes?(Account))
print([1, 2].includes?([1]), [[1], 2].includes?([1]), [{"a": 1}].includes?({"a": 1}))"##,
    ),
    // The app metrics API: named `tags:`, a `do` block timed by `time` (its
    // tags positional: a labelled call with a block on a stdlib static
    // method is refused by the VM), and a refused name — the same on both
    // engines.
    (
        "metrics_api_calls",
        r##"answer = Metrics.time("checkout.time") do
  41 + 1
end
print(answer)
tagged = Metrics.time("job.run", {"job": "mail"}) do
  "done"
end
print(tagged)
Metrics.increment("orders.created", tags: {"plan": "pro"})
Metrics.increment("orders.created", 2)
Metrics.gauge("queue.depth", 3, tags: {"queue": "mail"})
Metrics.histogram("cart.items", 2.5, {"k": "v"})
Metrics.distribution("cart.items", 1)
refused = Metrics.increment("bad name") rescue "refused"
print(refused)"##,
    ),
];

/// Run `source` through the soli binary; `vm` selects the bytecode VM with
/// optional-`let` enabled. Returns the observable outcome: stdout on success,
/// or a sentinel on any non-success (error/panic) so error *text* differences
/// don't count as behavioral divergence.
fn run(source: &str, idx: usize, vm: bool) -> String {
    run_as(source, &idx.to_string(), vm, true)
}

/// `run`, with the temp file named after `tag` (tests run in parallel, so
/// each needs its own names) and `--no-type-check` unless `type_check`.
fn run_as(source: &str, tag: &str, vm: bool, type_check: bool) -> String {
    let mut path = std::env::temp_dir();
    path.push(format!("soli_diff_{}_{}.sl", std::process::id(), tag));
    std::fs::write(&path, source).expect("write temp source");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_soli"));
    cmd.arg(&path);
    if vm {
        cmd.arg("--vm").env("SOLI_VM_OPTIONAL_LET", "1");
    } else {
        // Scripts run on the VM by default now; the reference is the
        // tree-walker, by name.
        cmd.arg("--tree");
    }
    if !type_check {
        cmd.arg("--no-type-check");
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

#[test]
fn tree_walker_and_vm_agree_without_the_type_checker() {
    for (idx, (name, source)) in UNCHECKED_CASES.iter().enumerate() {
        let tag = format!("unchecked_{idx}");
        let tw = run_as(source, &tag, false, false);
        let vm = run_as(source, &tag, true, false);
        assert_ne!(tw, "<non-success>", "{name}: the tree-walker must run it");
        assert_eq!(tw, vm, "{name}: the tree-walker and the VM disagree");
    }
}

/// Run `source` on one engine, as `run_as` does, with `env` added to its
/// environment. Returns whether it exited successfully, its stdout and its
/// stderr.
fn run_with_env(source: &str, tag: &str, vm: bool, env: &[(&str, &str)]) -> (bool, String, String) {
    let mut path = std::env::temp_dir();
    path.push(format!("soli_diff_{}_{}.sl", std::process::id(), tag));
    std::fs::write(&path, source).expect("write temp source");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_soli"));
    cmd.arg(&path).envs(env.iter().copied());
    if vm {
        cmd.arg("--vm").env("SOLI_VM_OPTIONAL_LET", "1");
    } else {
        cmd.arg("--tree");
    }
    let output = cmd.output().expect("run soli");
    let _ = std::fs::remove_file(&path);
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

/// With a metrics sink configured (`SOLI_METRICS=1`), both engines store
/// app samples and read them back the same way.
#[test]
fn metrics_are_recorded_and_read_back_on_both_engines() {
    let source = r##"Metrics.increment("orders.created", tags: {"plan": "pro"})
Metrics.increment("orders.created", 2, tags: {"plan": "pro"})
Metrics.gauge("queue.depth", 7)
Metrics.histogram("cart.items", 0.2)
seconds = Metrics.time("work") do
  1
end
print(Metrics.enabled?)
print(Metrics.value("orders.created", tags: {"plan": "pro"}))
print(Metrics.value("queue.depth"))
print(Metrics.value("cart.items")["count"], Metrics.value("cart.items")["buckets"]["0.25"])
print(Metrics.value("work")["count"], seconds)
"##;
    for vm in [false, true] {
        let (ok, stdout, stderr) = run_with_env(
            source,
            &format!("metrics_on_{vm}"),
            vm,
            &[("SOLI_METRICS", "1")],
        );
        assert!(ok, "vm: {vm}: {stderr}");
        assert_eq!(stdout, "true\n3\n7\n1 1\n1 1\n", "vm: {vm}");
    }
}

/// A range that a `for` loop or a comprehension counts through is never built,
/// so `SOLI_MAX_RANGE_LEN` does not limit the range itself, with or without
/// parentheses around it, on either engine. Both engines used to build a
/// parenthesized one as an array, and refuse it past the limit, and the
/// tree-walker built a comprehension's bare one too. A range used as a value
/// is still built, and still refused; a comprehension's result is held to the
/// limit (`a_comprehension_over_a_range_is_held_to_the_range_limit`).
#[test]
fn ranges_that_loops_count_through_are_not_built() {
    let limit = [("SOLI_MAX_RANGE_LEN", "100")];
    let counted = r##"def walk(n)
  sum = 0
  for i in (0..n)
    sum += i
  end
  for i in ((0..n))
    sum += 1
  end
  for i, k in 0..n
    sum += k
  end
  sum
end

print(walk(1000))
print([i for i in (0..1000) if i % 250 == 0], [i for i in 0..1000 if i > 997])
print({(i): i for i in (0..1000) if i % 400 == 0}.length, {(i): i for i in 0..1000 if i < 2})
"##;
    for vm in [false, true] {
        let (ok, stdout, stderr) =
            run_with_env(counted, &format!("range_counted_{vm}"), vm, &limit);
        assert!(ok, "vm: {vm}: {stderr}");
        assert_eq!(
            stdout, "1000000\n[0, 250, 500, 750] [998, 999]\n3 {0 => 0, 1 => 1}\n",
            "vm: {vm}"
        );
    }

    let built = [
        "numbers = (0..1000)\nprint(numbers.length)\n",
        "print((0..1000).map { |i| i }.length)\n",
        "for i in (0..1000).reverse()\n  print(i)\nend\n",
        "print([...(0..1000)].length)\n",
    ];
    for (idx, source) in built.iter().enumerate() {
        for vm in [false, true] {
            let tag = format!("range_built_{idx}_{vm}");
            let (ok, stdout, stderr) = run_with_env(source, &tag, vm, &limit);
            assert!(
                !ok && stdout.is_empty() && stderr.contains("over the 100 limit"),
                "{source} (vm: {vm}) must still be refused: {stdout}{stderr}"
            );
        }
    }
}

/// A comprehension over a range counts through it, but it builds its result:
/// past `SOLI_MAX_RANGE_LEN` elements both engines refuse it, at the range,
/// as they refuse a range built as a value. Counting lazily had dropped the
/// limit, so `[i * i for i in (0..params["n"].to_i)]` with a huge `n` grew
/// until the allocator failed, which aborts every worker of the process. A
/// filter that keeps few elements is not refused, however long the range.
#[test]
fn a_comprehension_over_a_range_is_held_to_the_range_limit() {
    let limit = [("SOLI_MAX_RANGE_LEN", "100")];
    let kept = "print([i for i in 0..100].length, {(i): i for i in (0..100)}.length)\n\
                print([i for i in 0..100000 if i % 1000 == 0].length)\n";
    for vm in [false, true] {
        let (ok, stdout, stderr) = run_with_env(kept, &format!("compr_kept_{vm}"), vm, &limit);
        assert!(ok, "vm: {vm}: {stderr}");
        assert_eq!(stdout, "100 100\n100\n", "vm: {vm}");
    }

    let refused = [
        ("print([i for i in 0..101].length)\n", "1:19"),
        (
            "def f(n)\n  [i * i for i in (0..n)]\nend\nprint(f(1000).length)\n",
            "2:20",
        ),
        (
            "def f(n)\n  [i for i in 0..n if i >= 0]\nend\nprint(f(1000).length)\n",
            "2:15",
        ),
        ("print({(i): i for i in ((0..1000))}.length)\n", "1:26"),
        (
            "def f(n)\n  h = {(i): 1 for i in 0..n}\n  h\nend\nprint(f(1000).length)\n",
            "2:24",
        ),
    ];
    for (idx, (source, at)) in refused.iter().enumerate() {
        for vm in [false, true] {
            let tag = format!("compr_refused_{idx}_{vm}");
            let (ok, stdout, stderr) = run_with_env(source, &tag, vm, &limit);
            let error = stderr
                .lines()
                .find(|line| line.starts_with("Error:"))
                .unwrap_or_default();
            assert!(
                !ok && stdout.is_empty()
                    && error.contains("would build more than 100 elements, over the 100 limit")
                    && error.ends_with(&format!(" at {at}")),
                "{source} (vm: {vm}) must be refused at {at}: {stdout}{stderr}"
            );
        }
    }
}

/// A bound that is not an integer is reported at the range itself, with or
/// without parentheses around it, in a `for` loop or a comprehension, on both
/// engines (the VM words it `Range requires integer operands`). The VM used
/// to report a bare range at its `for` statement, and the range of a
/// comprehension compiled as a lambda at column 0.
#[test]
fn a_bound_that_is_not_an_integer_is_reported_at_the_range() {
    let cases = [
        (
            "def f(a, b)\n  for i in (a..b)\n  end\nend\nf(1.5, 3)\n",
            "2:13",
        ),
        (
            "def f(a, b)\n  for i in a..b\n  end\nend\nf(1.5, 3)\n",
            "2:12",
        ),
        (
            "def f(a, b)\n  [i for i in (a..b)]\nend\nf(1.5, 3)\n",
            "2:16",
        ),
        ("def f(a, b)\n  [i for i in a..b]\nend\nf(1.5, 3)\n", "2:15"),
        (
            "def f(a, b)\n  print({(i): i for i in ((a..b))})\nend\nf(1, \"3\")\n",
            "2:28",
        ),
    ];
    for (idx, (source, at)) in cases.iter().enumerate() {
        for vm in [false, true] {
            let tag = format!("range_bound_{idx}_{vm}");
            let (ok, _, stderr) = run_with_env(source, &tag, vm, &[]);
            let error = stderr
                .lines()
                .find(|line| line.starts_with("Error:"))
                .unwrap_or_default();
            assert!(
                !ok && error.contains("integer") && error.ends_with(&format!(" at {at}")),
                "{source} (vm: {vm}) must fail at {at}: {stderr}"
            );
        }
    }
}

// --- Classes loaded the way a server loads them -----------------------------
//
// `soli serve` defines every class of an application on the tree-walker (the
// app loader), copies the interpreter's globals into each worker's VM, and
// runs the action there through `Vm::call_method_bound`. The VM compiles such
// a class's constructor and methods from the tree-walker's AST when it first
// reaches them. A script's classes are compiled by the VM itself, so CASES
// never meets one: a constructor calling `super(...)` agreed there and failed
// in a served app ("super used outside of a subclass method"), which demoted
// the handler to the interpreter — or, inside a `try`, answered with the VM's
// own error.

/// (name, source). Each source declares `class Probe` with a zero-argument
/// `run` that returns a string, and runs cleanly on the tree-walker.
const APP_CASES: &[(&str, &str)] = &[
    (
        "app_constructor_calls_super",
        r##"class Point
  x: Int
  y: Int

  new(x, y)
    @x = x
    @y = y
  end
end

class Diagonal < Point
  new(n)
    super(n, n)
  end
end

class Probe
  def run
    a = Diagonal.new(4)
    b = new Diagonal(5)
    "#{a.x},#{a.y} #{b.x},#{b.y}"
  end
end"##,
    ),
    (
        // The middle constructor is reached through `super(...)`: its own
        // `super(...)` must go one level further up.
        "app_three_constructors_call_super",
        r##"class G1
  a: Int

  new(a)
    @a = a
  end
end

class G2 < G1
  b: Int

  new(a)
    super(a)
    @b = a * 10
  end
end

class G3 < G2
  c: Int

  new(a)
    super(a)
    @c = a * 100
  end
end

class Probe
  def run
    g = G3.new(1)
    "#{g.a} #{g.b} #{g.c}"
  end
end"##,
    ),
    (
        // K2 inherits K1's constructor: reached from K3's `super(...)`, it
        // runs as K1's, so its `super(...)` reaches K0. Run as K2's, it would
        // call itself until the stack ran out.
        "app_super_reaches_an_inherited_constructor",
        r##"class K0
  k0: Int

  new(a)
    @k0 = a
  end
end

class K1 < K0
  k1: Int

  new(a)
    super(a + 1)
    @k1 = a
  end
end

class K2 < K1
end

class K3 < K2
  k3: Int

  new(a)
    super(a * 2)
    @k3 = a
  end
end

class Probe
  def run
    k = K3.new(3)
    plain = K2.new(10)
    "#{k.k0} #{k.k1} #{k.k3} #{plain.k0} #{plain.k1}"
  end
end"##,
    ),
    (
        // The documented custom errors, built, thrown and rescued. Inside the
        // `try`, the VM used to catch its own "super used outside of a
        // subclass method" and answer with it, with no demotion.
        "app_error_subclasses_pass_their_message_up",
        r##"class AppError < Error
end

class Overdrawn < AppError
  amount: Int

  new(amount: Int)
    super("overdrawn by #{amount}", {"amount": amount})
    @amount = amount
  end
end

class Refused < DomainError
  new(why: String)
    super("refused: #{why}")
  end
end

class Probe
  def run
    built = Overdrawn.new(5)
    caught = "none"
    try
      throw Overdrawn.new(7)
    catch err
      caught = "#{err.class} #{err.message} #{err.details["amount"]}"
    end
    typed = "none"
    try
      throw Refused.new("closed")
    catch DomainError err
      typed = err.to_s
    end
    rescued = Overdrawn.new(9) rescue "rescued"
    "#{built.message} #{built.amount} | #{caught} | #{typed} | #{rescued.class}"
  end
end"##,
    ),
    (
        // The error-handling guide's hierarchy, with a typed catch.
        "app_documented_error_hierarchy",
        r##"class AppError
  message: String

  new(msg: String)
    this.message = msg
  end
end

class NotFoundError < AppError
  new(msg: String)
    super(msg)
  end
end

class Probe
  def run
    seen = "none"
    try
      throw NotFoundError.new("missing")
    catch AppError e
      seen = "App error: " + e.message
    end
    seen
  end
end"##,
    ),
    (
        // `super.m()` into a parent method that calls `super.m()` itself.
        "app_three_level_super_method_chain",
        r##"class LA
  def m
    1
  end
end

class LB < LA
  def m
    super.m() + 10
  end
end

class LC < LB
  def m
    super.m() + 100
  end
end

class Probe
  def run
    "#{new LC().m()} #{LB.new().m()}"
  end
end"##,
    ),
    (
        // `super.run()` in the action itself, as `super.show(req)` in a
        // controller that extends another.
        "app_action_calls_super",
        r##"class BaseProbe
  def run
    "base"
  end
end

class Probe < BaseProbe
  def run
    "probe+" + super.run()
  end
end"##,
    ),
    (
        // An inherited action runs as the class that declares it: its
        // `super.run()` reaches Root. Run as Probe's, it would call itself.
        "app_inherited_action_calls_super",
        r##"class Root
  def run
    "root"
  end
end

class Mid < Root
  def run
    "mid+" + super.run()
  end
end

class Probe < Mid
end"##,
    ),
    (
        // A value object whose constructor hands its argument to the parent's,
        // with the parent's invariants checked on the VM.
        "app_value_object_constructor_calls_super",
        r##"class Email < ValueObject
  address: String

  new(address: String)
    @address = address.trim.downcase
  end

  invariant("must contain @") { @address.includes?("@") }
  invariant("must not contain spaces") { !@address.includes?(" ") }
end

class WorkEmail < Email
  new(user: String)
    super(user + "@Work.Example")
  end
end

class Probe
  def run
    work = WorkEmail.new("  Ada")
    refused = "none"
    try
      WorkEmail.new("Ada Lovelace")
      refused = "built"
    catch InvariantViolation err
      refused = err.message
    end
    "#{work.address} #{work.class} #{refused}"
  end
end"##,
    ),
    (
        // A served app's classes get their fields like a script's: every
        // default (inherited ones, and one that is not a literal, run per
        // instance) and nil without one, whatever builds the instance. The VM
        // set none of them on a class the tree-walker defined: the first read
        // raised "Cannot access property" and demoted the handler.
        "app_field_defaults_and_nil",
        r##"calls = 0

def next_stamp()
  calls = calls + 1
  calls
end

class WithDefaults
  count: Int = 5
  label: String = "dflt"
  v: Int

  new(v)
    @v = v
  end
end

class ZeroCtorDefaults
  count: Int = 7
  made: Bool

  new()
    @made = true
  end
end

class ParentD
  a: Int = 1
  b: Int = -2
end

class KidD < ParentD
  c: Int

  new(c)
    @c = c
  end
end

class Profile
  name: String
  bio: String

  new(name)
    @name = name
  end
end

class Stamped
  stamp: Int = next_stamp()
  tags: Array = []
end

class Hinted < Error
  code: Int = 42
  hint: String
end

class Probe
  def run
    d1 = WithDefaults.new(1)
    d2 = new WithDefaults(2)
    d3 = KidD.new(3)
    d4 = new KidD(4)
    d5 = ParentD.new()
    d6 = new ParentD()
    d8 = ZeroCtorDefaults.new()
    u1 = Profile.new("ann")
    s1 = new Stamped()
    s2 = Stamped.new()
    s1.tags.push(1)
    h = Hinted.new("boom")
    first = "#{d1.count} #{d1.label} #{d2.count} #{d3.a} #{d3.b} #{d4.a} #{d5.a} #{d6.b}"
    second = "#{d8.count} #{d8.made} #{u1.bio.nil?} #{s1.stamp} #{s2.stamp} #{s2.tags.length}"
    "#{first} #{second} #{h.message} #{h.code} #{h.hint.nil?} #{calls}"
  end
end"##,
    ),
    (
        // The documented value object, built in a served app. A field left
        // out takes its default; the VM gave it nil, silently — or, with the
        // invariant reading it, "Cannot access property 'length' on null".
        "app_value_object_field_defaults",
        r##"class Amount < ValueObject
  cents: Int
  currency: String = "EUR"

  invariant(:cents, "must not be negative") { @cents >= 0 }
  invariant(:currency, "three letters") { @currency.length == 3 }

  def to_display -> String
    symbol = @currency == "EUR" ? "€" : @currency
    "#{@cents / 100} #{symbol}"
  end
end

class Probe
  def run
    v1 = Amount.new(500)
    v2 = Amount(600)
    v3 = Amount.new(cents: 700)
    v4 = Amount.new(800, "USD")
    v5 = v1.with(cents: 900)
    shown = "none"
    try
      shown = Amount.new(1000).to_display
    catch err
      shown = "caught: #{err}"
    end
    currencies = "#{v1.currency} #{v2.currency} #{v3.currency} #{v4.currency} #{v5.currency}"
    "#{currencies} #{v5.cents} #{shown} #{v1 == Amount.new(500, "EUR")}"
  end
end"##,
    ),
    (
        // A served class's constructor is compiled once, in constructor mode,
        // and entered as an ordinary frame. Every way out of it — the end, an
        // early `return`, one inside `try`/`finally` — still gives the
        // instance, by every spelling; an inherited constructor, `super`, a
        // raise from a helper, recursion, a block callback and the attribute
        // form behave as on the tree-walker.
        "app_constructors_run_as_frames",
        r##"class Guarded
  x: Int

  new(a)
    @x = a
    return if a > 1

    @x = 0
  end
end

class Finally
  log: String

  new
    @log = ""
    try
      @log = @log + "t"
      return
    finally
      @log = @log + "f"
    end
  end
end

class Node
  value: Int
  next_node: Any

  new(n)
    @value = n
    @next_node = n > 0 ? Node.new(n - 1) : nil
  end

  def depth
    @next_node.nil? ? 1 : 1 + @next_node.depth
  end
end

class Picky
  v: Int

  new(v)
    @v = Picky.check(v)
  end

  static def check(v)
    throw "bad #{v}" if v < 0

    v
  end
end

class Pair
  a: Int
  b: Int

  new(a, b = 10)
    @a = a
    @b = b
  end
end

class K1
  log: String

  new(a)
    @log = "k1:#{a}"
  end
end

class K2 < K1
end

class K3 < K2
  new(a)
    super(a + 1)
    @log = @log + ";k3"
  end
end

class Bare
  note: String
  count: Int = 3
end

class Probe
  def run
    early = "#{Guarded.new(5).x} #{new Guarded(0).x} #{Guarded(7).x} #{Finally.new().log}"
    caught = "none"
    try
      Picky.new(-1)
    catch err
      caught = "caught #{err}"
    end
    pairs = [1, 2].map { |x| Pair.new(x) }.map { |p| p.a + p.b }
    chain = "#{K3.new(1).log} #{K2.new(5).log}"
    bare = Bare.new({"note": "n"})
    nodes = Node.new(3).depth
    "#{early} | #{nodes} #{caught} #{Picky.new(2).v} | #{pairs} #{Pair.new(1, 2).b} | #{chain} | #{bare.note} #{bare.count} #{Bare.new().count}"
  end
end"##,
    ),
    (
        // A parent's compiled method must not hide the subclass's own
        // (was tracked in KNOWN_APP_DIVERGENT until 2.29.4's level-by-level
        // dispatch).
        "app_a_compiled_parent_method_hides_the_override",
        r##"class Pa
  def m
    "Pa"
  end
end

class Ca < Pa
  def m
    "Ca"
  end
end

class Probe
  def run
    first = Pa.new().m()
    second = Ca.new().m()
    "#{first} #{second}"
  end
end"##,
    ),
    (
        // C583: on served (tree-walker) classes a labelled method call ran with the method itself as `this`: `ident` returned the Function, `@_scale(factor: 2)` failed on `@count` and demoted the action.
        "app_named_args_keep_the_receiver",
        r##"class Pt
  x: Int
  calls: Int

  new(x)
    @x = x
    @calls = 0
  end

  def ident(flag)
    this
  end

  def opt(a = 1, b = 2)
    @calls = @calls + 1
    a * 100 + b
  end

  def greet(name)
    "hi #{name}"
  end
end

class Probe
  count: Int

  def run
    @count = 3
    p = Pt.new(1)
    r = p.ident(flag: true)
    o = p.opt(b: 7)
    first = "#{type(r)} #{r == p} #{o} #{p.calls} #{p.greet(name: "bob")}"
    "#{first} #{@_scale(factor: 2)} #{_scale(factor: 4)} #{this._scale(factor: 1)}"
  end

  private

  def _scale(factor)
    factor * @count
  end
end"##,
    ),
    (
        // C583: a served `static def` reading `this`, called with labels.
        "app_named_args_on_a_static_that_reads_this",
        r##"class Base
  static def helper(n)
    n + 1
  end

  static def twice(n, k = 2)
    this.helper(n) * k
  end

  static def label(prefix = "<")
    "#{prefix}#{this.helper(9)}"
  end
end

class Sub < Base
end

class Probe
  def run
    "#{Base.twice(n: 2)} #{Sub.twice(n: 2, k: 5)} #{Sub.label(prefix: "+")}"
  end
end"##,
    ),
    (
        // C583: a block to a `&block` parameter, a lambda field and `method_missing`, with labels, on served classes.
        "app_named_args_with_a_block_a_field_and_method_missing",
        r##"class Box
  f: Any

  new()
    @f = fn(x) { x * 3 }
  end

  def amp(a, &blk)
    blk(a)
  end

  def method_missing(name, args)
    "mm #{name} #{args}"
  end
end

class Probe
  def run
    b = Box.new()
    r = b.amp(a: 4) do |x|
      x + 40
    end
    "#{b.f(x: 2)} #{r} #{b.nope(z: 1)}"
  end
end"##,
    ),
    (
        // A served class's method named `init`, once the VM has compiled it,
        // is not the constructor: `Cart.new("bo")` ran it ("init-ran") and
        // `c.init([1])` re-ran the constructor's body.
        "app_a_method_named_init_is_not_the_constructor",
        r##"class Cart
  owner: String
  items: Array

  new(owner)
    @owner = owner
    @items = []
  end

  def init(items)
    @items = items
    "init-ran"
  end
end

class Probe
  def run
    c = Cart.new("ann")
    first = c.init([1, 2])
    d = Cart.new("bo")
    e = new Cart("cy")
    "#{first} #{c.owner} #{c.items} #{d.owner} #{d.items} #{e.owner} #{e.init([3])}"
  end
end"##,
    ),
    (
        // A served class's defaults that are not literals run on the new
        // instance, in declaration order, whichever spelling builds it — not
        // in the action's environment (the tree-walker read the controller's
        // `@items` there).
        "app_field_defaults_run_on_the_new_instance",
        r##"class Basket
  items: Array = [1, 2, 3]
  count: Int = @items.length
  label: String = "n=#{@count}"
end

class Base1
  a: String = "base"
  c: String = "#{@a}/#{@b}"
  b: Int = 1
end

class Kid1 < Base1
  b: Int = 2
  d: String = @c + "!"
end

class Probe
  items: Array = [9]

  def run
    items = [7, 7]
    built = [new Basket(), Basket.new(), Basket.new({}), Basket.new(label: "x"), Basket()]
    kid = Kid1.new()
    shown = built.map { |b| "#{b.count}:#{b.label}" }
    "#{shown} #{kid.b} #{kid.c} #{kid.d}"
  end
end"##,
    ),
];

/// APP_CASES that still diverge, with KNOWN_DIVERGENT's contract.
const KNOWN_APP_DIVERGENT: &[&str] = &[
    // Empty. Fixed and locked in by this harness:
    //   app_a_compiled_parent_method_hides_the_override — dispatch resolves
    //   the nearest definition level by level (2.29.4's
    //   `resolve_instance_method`, and on this branch the per-site method
    //   cache and `SelfOrGlobalCallee`), so a parent's compiled method no
    //   longer hides a subclass's own.
];

fn parse(source: &str) -> solilang::ast::Program {
    let tokens = solilang::lexer::Scanner::new(source)
        .scan_tokens()
        .expect("the case must lex");
    solilang::parser::Parser::new(tokens)
        .parse()
        .expect("the case must parse")
}

/// `Probe.new().run()` on the tree-walker.
fn run_app_on_tree_walker(source: &str) -> Result<String, String> {
    let program = parse(&format!("{source}\nprobe_result = Probe.new().run()\n"));
    let mut interpreter = solilang::interpreter::Interpreter::new();
    interpreter.interpret(&program).map_err(|e| e.to_string())?;
    let result = interpreter.global_env().borrow().get("probe_result");
    result
        .map(|value| value.to_string())
        .ok_or_else(|| "no probe_result".to_string())
}

/// `Probe#run` as a served request runs an action: the classes defined by the
/// tree-walker, the action on a VM seeded with the interpreter's globals.
fn run_app_on_served_vm(source: &str) -> Result<String, String> {
    use solilang::interpreter::value::{Instance, Value};
    let program = parse(source);
    let mut interpreter = solilang::interpreter::Interpreter::new();
    interpreter.interpret(&program).map_err(|e| e.to_string())?;
    let mut vm = solilang::vm::Vm::new();
    for (name, value) in interpreter.global_env().borrow().get_all_bindings() {
        vm.globals.insert(name, value);
    }
    let Some(Value::Class(probe)) = vm.globals.get("Probe").cloned() else {
        return Err("no class Probe".to_string());
    };
    let run = probe
        .find_method("run")
        .ok_or_else(|| "no Probe#run".to_string())?;
    let instance = Value::Instance(std::rc::Rc::new(std::cell::RefCell::new(Instance::new(
        probe,
    ))));
    vm.call_method_bound(&run, instance, None, solilang::span::Span::default())
        .map(|value| value.to_string())
        .map_err(|e| e.to_string())
}

#[test]
fn app_loaded_classes_agree_on_both_engines() {
    let known: std::collections::HashSet<&str> = KNOWN_APP_DIVERGENT.iter().copied().collect();

    let mut new_divergences: Vec<String> = Vec::new();
    let mut fixed: Vec<&str> = Vec::new();

    for (name, source) in APP_CASES {
        let tw = run_app_on_tree_walker(source);
        assert!(
            tw.is_ok(),
            "{name} must run on the tree-walker, the reference: {tw:?}"
        );
        let vm = run_app_on_served_vm(source);
        match (tw != vm, known.contains(name)) {
            (true, false) => new_divergences.push(format!(
                "  [NEW DIVERGENCE] {name}\n    tree-walker: {tw:?}\n    served vm:   {vm:?}"
            )),
            (false, true) => fixed.push(name),
            _ => {}
        }
    }

    let mut msg = String::new();
    if !new_divergences.is_empty() {
        msg.push_str(&format!(
            "{} app-loaded program(s) give a different result on the VM a server runs \
             them on than on the tree-walker:\n{}\n",
            new_divergences.len(),
            new_divergences.join("\n")
        ));
    }
    if !fixed.is_empty() {
        msg.push_str(&format!(
            "{} known-divergent app case(s) now AGREE — remove them from \
             KNOWN_APP_DIVERGENT to lock in the fix: {:?}\n",
            fixed.len(),
            fixed
        ));
    }
    assert!(msg.is_empty(), "\n{msg}");
}

/// `sort_by` (block and string-key forms) and `order` over keys of mixed
/// types, NaN and nil. Their comparator is no total order — values of
/// different types and NaN compare Equal — and std's sort panics on that
/// past 20 elements: the script died with exit 101, a served request with a
/// panic `rescue` could not catch. Each form must return, on both engines,
/// the same deterministic and stable order. (The differential cases above
/// cannot pin this: two engines that both crash agree.)
#[test]
fn sort_by_over_mixed_type_and_nan_keys_returns_on_both_engines() {
    let source = r#"
pool = [3, "b", 1.5, 2, "a", 7, 0.25, "c", 5, 1, "NaN".to_f, nil]
rows = (0..72).map { |i| {"k": pool[(i * 7 + i / 3) % 12], "i": i} }
def stable(sorted)
  (1..sorted.length).all? do |j|
    a = sorted[j - 1]
    b = sorted[j]
    same = type(a["k"]) == type(b["k"]) && a["k"] == b["k"]
    !same || a["i"] < b["i"]
  end
end
def ids(sorted)
  sorted.map { |r| r["i"] }.join(",")
end
by_block = rows.sort_by { |r| r["k"] }
by_key = rows.sort_by("k")
by_order = rows.order("k")
print(ids(by_block))
print(ids(by_key))
print(ids(by_order))
print(stable(by_block), stable(by_key), stable(by_order))
"#;
    let tree = run(source, 90_000, false);
    let vm = run(source, 90_001, true);
    assert_ne!(
        tree, "<non-success>",
        "sort_by must not crash the tree-walker"
    );
    assert_ne!(vm, "<non-success>", "sort_by must not crash the VM");
    assert_eq!(tree, vm, "both engines must give the same order");
    assert_eq!(
        tree,
        run(source, 90_002, false),
        "the order is deterministic"
    );

    let lines: Vec<&str> = tree.lines().collect();
    assert_eq!(lines.len(), 4, "{tree}");
    for line in &lines[..3] {
        let mut ids: Vec<usize> = line.split(',').map(|n| n.parse().unwrap()).collect();
        ids.sort_unstable();
        assert_eq!(ids, (0..72).collect::<Vec<_>>(), "every row once: {line}");
    }
    assert_eq!(
        lines[3], "true true true",
        "rows with equal keys keep their order"
    );
}
