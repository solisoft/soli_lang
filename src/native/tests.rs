//! Unit tests for the native tier: which functions qualify, and a seeded
//! differential fuzz of the generated code against the tree-walker.

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::eligibility::analyze;
use super::*;
use crate::interpreter::value::Value;
use crate::interpreter::Interpreter;

fn parse(source: &str) -> Program {
    let tokens = crate::lexer::Scanner::new(source)
        .scan_tokens()
        .expect("lex");
    crate::parser::Parser::new(tokens).parse().expect("parse")
}

fn accepted(source: &str) -> Vec<String> {
    let program = parse(source);
    analyze(&program, &|_| false)
        .funcs
        .iter()
        .map(|f| f.name.clone())
        .collect()
}

fn refusal(source: &str, name: &str) -> String {
    let program = parse(source);
    let analysis = analyze(&program, &|_| false);
    analysis
        .refusals
        .iter()
        .find(|r| r.name == name)
        .map(|r| r.reason.clone())
        .unwrap_or_else(|| panic!("`{name}` was not refused: {:?}", analysis.refusals))
}

#[test]
fn a_typed_numeric_function_qualifies() {
    let src = "def fib(n: Int) -> Int\n  return n if n < 2\n\n  fib(n - 1) + fib(n - 2)\nend\n";
    assert_eq!(accepted(src), vec!["fib"]);
}

#[test]
fn an_untyped_function_is_not_even_a_candidate() {
    let program = parse("def double(n)\n  n * 2\nend\n");
    let analysis = analyze(&program, &|_| false);
    assert!(analysis.funcs.is_empty());
    assert!(analysis.refusals.is_empty(), "{:?}", analysis.refusals);
}

#[test]
fn refusals_say_why() {
    assert_eq!(
        refusal("def f(n: Int) -> Int\n  print(n)\n  n\nend\n", "f"),
        "calls `print` (line 2)"
    );
    assert_eq!(
        refusal("def f(n: Int) -> Int\n  \"#{n}\".length\nend\n", "f"),
        "calls a method or reads a field (line 2)"
    );
    assert_eq!(
        refusal("def f(n: Int) -> String\n  \"x\"\nend\n", "f"),
        "returns String, not Int, Float or Bool"
    );
    assert_eq!(
        refusal("def f(n: Int) -> Int\n  n * 1.5\nend\n", "f"),
        "its last expression is a Float where Int is declared"
    );
    assert_eq!(
        refusal("def f(n: Int) -> Int\n  x = 1\n  x = 2.5\n  n\nend\n", "f"),
        "`x` holds an Int and is assigned a Float (line 3)"
    );
    assert_eq!(
        refusal("def f(n: Int = 1) -> Int\n  n\nend\n", "f"),
        "parameter `n` has a default value"
    );
}

#[test]
fn assigning_a_global_name_is_refused() {
    // The engines would write the global, not a local.
    let src = "total = 0\ndef f(n: Int) -> Int\n  total = n\n  total\nend\n";
    assert_eq!(
        refusal(src, "f"),
        "assigns `total`, which names a global (line 3)"
    );
    // A builtin counts as a global too.
    let program = parse("def f(n: Int) -> Int\n  print = n\n  n\nend\n");
    let analysis = analyze(&program, &|name| name == "print");
    assert!(analysis.funcs.is_empty());
}

#[test]
fn a_variable_set_inside_a_branch_is_not_read_after_it() {
    // The tree-walker scopes `y` to the `if`; the VM does not. Refused so
    // neither reading matters.
    let src = "def f(n: Int) -> Int\n  if n > 0\n    y = 1\n  end\n  y\nend\n";
    assert_eq!(
        refusal(src, "f"),
        "reads `y`, which is not a local (line 5)"
    );
    let postfix = "def f(n: Int) -> Int\n  y = 1 if n > 0\n  y\nend\n";
    assert_eq!(
        refusal(postfix, "f"),
        "reads `y`, which is not a local (line 3)"
    );
}

#[test]
fn calling_a_refused_function_refuses_the_caller() {
    let src = "def noisy(n: Int) -> Int\n  print(n)\n  n\nend\n\
               def quiet(n: Int) -> Int\n  noisy(n) + 1\nend\n";
    assert!(accepted(src).is_empty());
    assert_eq!(refusal(src, "quiet"), "calls `noisy` (line 6)");
}

#[test]
fn a_name_bound_twice_is_not_a_kernel() {
    let twice = "def f(n: Int) -> Int\n  n\nend\ndef f(n: Int) -> Int\n  n + 1\nend\n";
    assert_eq!(refusal(twice, "f"), "defined more than once");
    let rebound = "def f(n: Int) -> Int\n  n\nend\nf = 3\n";
    assert_eq!(
        refusal(rebound, "f"),
        "the name is also bound by something other than this def"
    );
}

// ---- running kernels -------------------------------------------------------

/// Compile `program`'s kernels once and look each of `names` up in them.
fn kernels(program: &Program, names: &[&str]) -> (KernelSet, Vec<KernelRef>) {
    let set = analyze_and_compile(program, &|_| false).expect("kernels compile");
    let refs = names
        .iter()
        .map(|name| {
            let decl = program
                .statements
                .iter()
                .find_map(|s| match &s.kind {
                    crate::ast::StmtKind::Function(d) if d.name == *name => Some(&**d),
                    _ => None,
                })
                .expect("declaration");
            set.for_decl(decl).expect("a kernel for the declaration")
        })
        .collect();
    (set, refs)
}

fn call(k: &KernelRef, args: &[Value]) -> Option<Value> {
    let id = k.id();
    let name = k.name().to_string();
    k.try_call(args, 0, |n| (n == name).then_some(id))
}

#[test]
fn overflow_and_division_decline() {
    let program = parse(
        "def mul(a: Int, b: Int) -> Int\n  a * b\nend\n\
         def quo(a: Int, b: Int) -> Int\n  a / b\nend\n\
         def rem(a: Int, b: Int) -> Int\n  a % b\nend\n\
         def neg(a: Int) -> Int\n  -a\nend\n\
         def fquo(a: Float, b: Float) -> Float\n  a / b\nend\n\
         def frem(a: Float, b: Float) -> Float\n  a % b\nend\n",
    );
    let (_set, k) = kernels(&program, &["mul", "quo", "rem", "neg", "fquo", "frem"]);
    let [mul, quo, rem, neg, fquo, frem] = &k[..] else {
        unreachable!()
    };
    let int = Value::Int;
    assert_eq!(call(mul, &[int(6), int(7)]), Some(int(42)));
    assert_eq!(call(mul, &[int(i64::MAX), int(2)]), None);
    assert_eq!(call(quo, &[int(-7), int(2)]), Some(int(-3)));
    assert_eq!(call(quo, &[int(1), int(0)]), None);
    assert_eq!(call(quo, &[int(i64::MIN), int(-1)]), None);
    assert_eq!(call(rem, &[int(-7), int(2)]), Some(int(-1)));
    assert_eq!(call(rem, &[int(i64::MIN), int(-1)]), None);
    assert_eq!(call(neg, &[int(i64::MIN)]), None);
    assert_eq!(call(neg, &[int(5)]), Some(int(-5)));
    assert_eq!(call(fquo, &[Value::Float(1.0), Value::Float(0.0)]), None);
    assert_eq!(call(fquo, &[Value::Float(1.0), Value::Float(-0.0)]), None);
    match call(frem, &[Value::Float(1.0), Value::Float(0.0)]) {
        Some(Value::Float(f)) => assert!(f.is_nan()),
        other => panic!("Float % 0.0 is NaN, not {other:?}"),
    }
    // Wrong argument types decline rather than misread the bits.
    assert_eq!(call(mul, &[Value::Float(2.0), int(3)]), None);
    assert_eq!(call(mul, &[int(3)]), None);
}

#[test]
fn the_depth_limit_declines() {
    let program = parse("def down(n: Int) -> Int\n  return 0 if n == 0\n\n  down(n - 1)\nend\n");
    let (_set, k) = kernels(&program, &["down"]);
    let down = &k[0];
    let limit = crate::interpreter::executor::MAX_CALL_DEPTH as i64;
    assert_eq!(call(down, &[Value::Int(limit - 1)]), Some(Value::Int(0)));
    assert_eq!(call(down, &[Value::Int(limit)]), None);
}

#[test]
fn a_callee_that_is_not_bound_declines() {
    let program = parse(
        "def outer(n: Int) -> Int\n  inner(n) + 1\nend\ndef inner(n: Int) -> Int\n  n * 2\nend\n",
    );
    let (_set, k) = kernels(&program, &["outer", "inner"]);
    let (outer, inner) = (&k[0], &k[1]);
    let (outer_id, inner_id) = (outer.id(), inner.id());
    let all = |n: &str| match n {
        "outer" => Some(outer_id),
        "inner" => Some(inner_id),
        _ => None,
    };
    assert_eq!(
        outer.try_call(&[Value::Int(4)], 0, all),
        Some(Value::Int(9))
    );
    let only_outer = |n: &str| (n == "outer").then_some(outer_id);
    assert_eq!(outer.try_call(&[Value::Int(4)], 0, only_outer), None);
}

// ---- differential fuzz -----------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Gen {
    Int,
    Float,
    Bool,
}

struct Fuzz {
    rng: StdRng,
    /// Generating the helper `h(a, b)`: no Float parameters to read, no calls.
    in_helper: bool,
}

impl Fuzz {
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.rng.gen_range(0..items.len())]
    }

    fn expr(&mut self, ty: Gen, depth: u32) -> String {
        let leaf = depth == 0 || self.rng.gen_bool(0.25);
        match ty {
            Gen::Int if leaf => self
                .pick(&["a", "b", "0", "1", "2", "7", "-3", "9223372036854775807"])
                .to_string(),
            Gen::Float if leaf && self.in_helper => {
                self.pick(&["0.0", "1.5", "-2.25", "1000000.5"]).to_string()
            }
            Gen::Float if leaf => self
                .pick(&["x", "y", "0.0", "1.5", "-2.25", "1000000.5"])
                .to_string(),
            Gen::Bool if leaf => self.pick(&["true", "false"]).to_string(),
            // A call to the helper, which the kernel inlines.
            Gen::Int if !self.in_helper && self.rng.gen_bool(0.2) => format!(
                "h({}, {})",
                self.expr(Gen::Int, depth - 1),
                self.expr(Gen::Int, depth - 1)
            ),
            Gen::Int => match self.rng.gen_range(0..5) {
                0 | 1 => {
                    let op = self.pick(&["+", "-", "*", "/", "%"]);
                    format!(
                        "({} {op} {})",
                        self.expr(Gen::Int, depth - 1),
                        self.expr(Gen::Int, depth - 1)
                    )
                }
                2 => format!("(-({}))", self.expr(Gen::Int, depth - 1)),
                3 => format!(
                    "({} ? {} : {})",
                    self.expr(Gen::Bool, depth - 1),
                    self.expr(Gen::Int, depth - 1),
                    self.expr(Gen::Int, depth - 1)
                ),
                _ => {
                    let op = self.pick(&["&&", "||"]);
                    format!(
                        "({} {op} {})",
                        self.expr(Gen::Int, depth - 1),
                        self.expr(Gen::Int, depth - 1)
                    )
                }
            },
            Gen::Float => match self.rng.gen_range(0..4) {
                0 => {
                    let op = self.pick(&["+", "-", "*", "/", "%"]);
                    format!(
                        "({} {op} {})",
                        self.expr(Gen::Float, depth - 1),
                        self.expr(Gen::Float, depth - 1)
                    )
                }
                1 => {
                    let op = self.pick(&["+", "-", "*", "/", "%"]);
                    format!(
                        "({} {op} {})",
                        self.expr(Gen::Int, depth - 1),
                        self.expr(Gen::Float, depth - 1)
                    )
                }
                2 => {
                    let op = self.pick(&["+", "-", "*", "/", "%"]);
                    format!(
                        "({} {op} {})",
                        self.expr(Gen::Float, depth - 1),
                        self.expr(Gen::Int, depth - 1)
                    )
                }
                _ => format!("(-({}))", self.expr(Gen::Float, depth - 1)),
            },
            Gen::Bool => match self.rng.gen_range(0..4) {
                0 => {
                    let op = self.pick(&["<", "<=", ">", ">=", "==", "!="]);
                    let (l, r) = (self.numeric(depth - 1), self.numeric(depth - 1));
                    format!("({l} {op} {r})")
                }
                1 => {
                    let operand = match self.rng.gen_range(0..3) {
                        0 => self.expr(Gen::Int, depth - 1),
                        1 => self.expr(Gen::Float, depth - 1),
                        _ => self.expr(Gen::Bool, depth - 1),
                    };
                    format!("(!({operand}))")
                }
                2 => {
                    let op = self.pick(&["&&", "||", "==", "!="]);
                    format!(
                        "({} {op} {})",
                        self.expr(Gen::Bool, depth - 1),
                        self.expr(Gen::Bool, depth - 1)
                    )
                }
                _ => format!(
                    "({} ? {} : {})",
                    self.expr(Gen::Bool, depth - 1),
                    self.expr(Gen::Bool, depth - 1),
                    self.expr(Gen::Bool, depth - 1)
                ),
            },
        }
    }

    fn numeric(&mut self, depth: u32) -> String {
        if self.rng.gen_bool(0.5) {
            self.expr(Gen::Int, depth)
        } else {
            self.expr(Gen::Float, depth)
        }
    }
}

/// Same value, and for a Float the same bits — `-0.0` prints differently from
/// `0.0` — except that any NaN matches any NaN (the payload is not
/// observable from Soli).
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => {
            (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits()
        }
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        _ => false,
    }
}

#[test]
fn generated_code_agrees_with_the_tree_walker() {
    let ints = [0, 1, -1, 2, -7, 3, i64::MAX, i64::MIN, 1 << 32];
    let floats = [0.0, -0.0, 1.5, -2.5, f64::NAN, f64::INFINITY, 1e300];
    let mut fuzz = Fuzz {
        rng: StdRng::seed_from_u64(0x5011_c0de),
        in_helper: false,
    };
    let mut compared = 0usize;
    let mut declined_on_success = 0usize;
    let mut compiled_with_calls = 0usize;

    for case in 0..400 {
        let ret = match case % 3 {
            0 => Gen::Int,
            1 => Gen::Float,
            _ => Gen::Bool,
        };
        fuzz.in_helper = true;
        let helper = fuzz.expr(Gen::Int, 3);
        fuzz.in_helper = false;
        let body = fuzz.expr(ret, 4);
        let ret_name = match ret {
            Gen::Int => "Int",
            Gen::Float => "Float",
            Gen::Bool => "Bool",
        };
        let source = format!(
            "def h(a: Int, b: Int) -> Int\n  {helper}\nend\n\
             def k(a: Int, b: Int, x: Float, y: Float) -> {ret_name}\n  {body}\nend\n"
        );
        let program = parse(&source);
        let Some(set) = analyze_and_compile(&program, &|_| false) else {
            // `&&`/`||` over mixed types, say: not a kernel, nothing to compare.
            continue;
        };
        let decl_of = |i: usize| match &program.statements[i].kind {
            crate::ast::StmtKind::Function(decl) => decl,
            _ => unreachable!(),
        };
        let Some(k) = set.for_decl(decl_of(1)) else {
            // `k` refused (it calls `h`, and `h` mixes types under `&&`).
            continue;
        };
        if body.contains("h(") {
            compiled_with_calls += 1;
        }
        let h_id = set.for_decl(decl_of(0)).map(|h| h.id());
        let k_id = k.id();
        let bound = |name: &str| match name {
            "k" => Some(k_id),
            "h" => h_id,
            _ => None,
        };

        let mut interp = Interpreter::new();
        interp.interpret(&program).expect("define k");
        let Some(Value::Function(func)) = interp.environment.borrow().get("k") else {
            panic!("k is not a function");
        };

        for i in 0..24 {
            let a = ints[(case + i) % ints.len()];
            let b = ints[(case * 7 + i * 3) % ints.len()];
            let x = floats[(case + i * 5) % floats.len()];
            let y = floats[(case * 3 + i) % floats.len()];
            let args = vec![
                Value::Int(a),
                Value::Int(b),
                Value::Float(x),
                Value::Float(y),
            ];
            let reference = interp.call_function(&func, args.clone());
            let native = k.try_call(&args, 0, bound);
            match (&reference, &native) {
                (Ok(want), Some(got)) => assert!(
                    same_value(want, got),
                    "{source}\nwith a={a} b={b} x={x} y={y}: tree-walker {want:?}, kernel {got:?}"
                ),
                (Ok(want), None) => {
                    declined_on_success += 1;
                    if declined_on_success <= 5 {
                        eprintln!("declined, tree-walker gave {want:?}: a={a} b={b} x={x} y={y}\n{source}");
                    }
                }
                (Err(_), None) => {}
                (Err(e), Some(got)) => panic!(
                    "{source}\nwith a={a} b={b} x={x} y={y}: tree-walker raised `{e}`, kernel returned {got:?}"
                ),
            }
            compared += 1;
        }
    }
    assert!(compared > 4000, "only {compared} comparisons ran");
    // Calls to `h` are inlined into `k`: enough of them must have run.
    assert!(
        compiled_with_calls > 60,
        "only {compiled_with_calls} compiled programs called the helper"
    );
    // A pure expression body only declines on an error, so a decline next to
    // a successful reference means the kernel gave up where it need not have.
    assert_eq!(
        declined_on_success, 0,
        "kernels declined calls the tree-walker completed"
    );
}
