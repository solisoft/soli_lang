//! Integration tests for the type checker (`solilang::type_check`).
//!
//! These tests exercise the lexer → parser → type-checker pipeline against
//! real Soli source. They cover both the happy paths the checker should
//! accept and the error conditions it should reject. Every `expect_error`
//! case asserts on a specific `TypeError` variant so the tests fail loudly
//! if the checker silently accepts something it shouldn't.

use solilang::error::TypeError;
use solilang::type_check;

// ---------- helpers ----------

fn check_ok(source: &str) {
    match type_check(source) {
        Ok(()) => {}
        Err(errors) => panic!(
            "expected program to type-check, got errors:\n{}",
            errors
                .iter()
                .map(|e| format!("  - {e}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

fn check_err(source: &str) -> Vec<TypeError> {
    match type_check(source) {
        Ok(()) => panic!("expected type error, but program type-checked"),
        Err(errors) => errors,
    }
}

fn assert_any<P: Fn(&TypeError) -> bool>(errors: &[TypeError], pred: P, label: &str) {
    assert!(
        errors.iter().any(pred),
        "expected at least one error matching {label}, got:\n{}",
        errors
            .iter()
            .map(|e| format!("  - {e:?}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// =====================================================================
// Literals & basic let / const
// =====================================================================

#[test]
fn literal_int_typechecks() {
    check_ok("let x = 42;");
}

#[test]
fn literal_float_typechecks() {
    check_ok("let x = 3.14;");
}

#[test]
fn literal_string_typechecks() {
    check_ok(r#"let x = "hello";"#);
}

#[test]
fn literal_bool_typechecks() {
    check_ok("let x = true; let y = false;");
}

#[test]
fn literal_null_typechecks() {
    check_ok("let x = null;");
}

#[test]
fn let_with_matching_annotation() {
    check_ok("let x: Int = 5;");
}

#[test]
fn let_int_to_float_widens() {
    check_ok("let x: Float = 5;");
}

#[test]
fn let_with_mismatched_annotation_errors() {
    let errors = check_err(r#"let x: Int = "hello";"#);
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch on let x: Int = \"hello\"",
    );
}

#[test]
fn let_string_to_int_errors() {
    let errors = check_err(r#"let x: String = 42;"#);
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch",
    );
}

#[test]
fn const_with_mismatched_annotation_errors() {
    let errors = check_err(r#"const X: Int = "hello";"#);
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch",
    );
}

#[test]
fn const_typechecks() {
    check_ok("const MAX = 100;");
}

// =====================================================================
// Binary operators
// =====================================================================

#[test]
fn int_addition_typechecks() {
    check_ok("let x: Int = 1 + 2;");
}

#[test]
fn int_plus_float_widens_to_float() {
    check_ok("let x: Float = 1 + 2.0;");
}

#[test]
fn string_concat_typechecks() {
    check_ok(r#"let x: String = "a" + "b";"#);
}

#[test]
fn string_plus_int_yields_string() {
    // `+` with a String operand always yields String per operators.rs.
    check_ok(r#"let x: String = "a" + 1;"#);
}

#[test]
fn array_plus_array_typechecks() {
    check_ok("let x = [1, 2] + [3, 4];");
}

#[test]
fn bool_plus_int_errors() {
    let errors = check_err("let x = true + 5;");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::General { .. }),
        "General add error",
    );
}

#[test]
fn comparison_returns_bool() {
    check_ok("let x: Bool = 1 < 2;");
}

#[test]
fn compare_string_lt_int_errors() {
    let errors = check_err(r#"let x = "a" < 1;"#);
    assert_any(
        &errors,
        |e| matches!(e, TypeError::General { .. }),
        "General compare",
    );
}

#[test]
fn negate_int_typechecks() {
    check_ok("let x: Int = -5;");
}

#[test]
fn negate_string_errors() {
    let errors = check_err(r#"let x = -"hi";"#);
    assert_any(
        &errors,
        |e| matches!(e, TypeError::General { .. }),
        "General negate",
    );
}

#[test]
fn not_returns_bool() {
    check_ok("let x: Bool = !true;");
}

#[test]
fn range_two_ints_typechecks() {
    check_ok("let r = 1..10;");
}

#[test]
fn range_with_float_errors() {
    let errors = check_err("let r = 1.5..10;");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::General { .. }),
        "General range",
    );
}

#[test]
fn equal_returns_bool_for_any_types() {
    // Equality is allowed on any pair of types and returns Bool.
    check_ok(r#"let x: Bool = "a" == 1;"#);
}

// =====================================================================
// Variables / assignment
// =====================================================================

#[test]
fn undefined_variable_errors() {
    let errors = check_err("let x = unknown_var;");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::UndefinedVariable(name, _, _) if name == "unknown_var"),
        "UndefinedVariable(unknown_var)",
    );
}

#[test]
fn assigning_to_new_variable_auto_defines() {
    // `x = 5` without `let` auto-defines via check_assign_expr.
    check_ok("x = 5;");
}

#[test]
fn reassigning_to_compatible_type_typechecks() {
    check_ok("let x: Int = 1; x = 2;");
}

// NOTE: there is no test asserting that reassigning a `let x: Int` with a
// String produces a Mismatch — the auto-define path would have already
// shadowed if it weren't a let. Here `x` is defined as Int, so the assign
// path checks types. Verify:
#[test]
fn reassigning_with_wrong_type_errors() {
    let errors = check_err(r#"let x: Int = 1; x = "no";"#);
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch on reassign",
    );
}

// =====================================================================
// Functions
// =====================================================================

#[test]
fn function_decl_and_call_typechecks() {
    check_ok(
        "
        fn add(a: Int, b: Int) -> Int { return a + b; }
        let r: Int = add(1, 2);
        ",
    );
}

#[test]
fn function_arity_too_many_errors() {
    let errors = check_err(
        "
        fn add(a: Int, b: Int) -> Int { return a + b; }
        let r = add(1, 2, 3);
        ",
    );
    assert_any(
        &errors,
        |e| {
            matches!(
                e,
                TypeError::WrongArity {
                    expected: 2,
                    got: 3,
                    ..
                }
            )
        },
        "WrongArity(2,3)",
    );
}

#[test]
fn function_call_wrong_arg_type_errors() {
    let errors = check_err(
        r#"
        fn add(a: Int, b: Int) -> Int { return a + b; }
        let r = add(1, "x");
        "#,
    );
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch on arg type",
    );
}

#[test]
fn calling_non_function_errors() {
    let errors = check_err("let x: Int = 5; x();");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::NotCallable(_, _)),
        "NotCallable",
    );
}

#[test]
fn return_type_mismatch_errors() {
    let errors = check_err(
        r#"
        fn f() -> Int { return "hi"; }
        "#,
    );
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch on return",
    );
}

#[test]
fn return_void_function_typechecks() {
    check_ok("fn noop() -> Void { return; }");
}

#[test]
fn lambda_typechecks() {
    check_ok("let f = fn(x: Int) -> Int { return x + 1; };");
}

#[test]
fn lambda_pipe_syntax_typechecks() {
    check_ok("let double = |x| { return x * 2; };");
}

// =====================================================================
// If / while / for control flow
// =====================================================================

#[test]
fn if_statement_with_bool_typechecks() {
    check_ok("if true { let x = 1; }");
}

#[test]
fn if_statement_with_int_condition_typechecks() {
    // Every value has a truthiness (`0`, `""`, `[]`, `{}`, `nil` and `false`
    // are falsy), so any type may stand as a condition.
    check_ok("if 5 { let x = 1; }");
}

#[test]
fn if_statement_with_any_typechecks() {
    // The statement-form if accepts Bool|Any|Unknown.
    check_ok(
        "
        fn opaque(a: Any) -> Any { return a; }
        if opaque(1) { let x = 1; }
        ",
    );
}

#[test]
fn while_with_int_condition_typechecks() {
    check_ok("let n = 3\nwhile n { n = n - 1 }");
}

#[test]
fn for_over_array_typechecks() {
    check_ok(
        "
        let xs: Int[] = [1, 2, 3];
        for x in xs { let y: Int = x; }
        ",
    );
}

// A bare `Array` annotation is a type the checker does not model: `for` must
// let it through, as `xs.each do |x| … end` and `if` / `while` always did.
#[test]
fn for_over_a_bare_array_annotation_typechecks() {
    check_ok(
        "
def total(xs: Array) -> Int
  sum = 0
  for x in xs
    sum = sum + x
  end
  sum
end

let ys: Array = [1, 2]
for y in ys
  print(y)
end
",
    );
}

// `@errors = errors` sets a field; a method of the same name (here a static
// one) is not the type of that field.
#[test]
fn a_field_write_is_not_checked_against_a_method_of_the_same_name() {
    check_ok(
        "
class Form
  static def errors(infos: Hash, cgv: Bool) -> Int[]
    []
  end

  def prepare(errors: Int[])
    @errors = errors
  end
end
",
    );
}

// A string iterates by character on both engines, so `for ch in s` with a
// visible String is accepted and binds `ch` as a String.
#[test]
fn for_over_a_string_typechecks_and_binds_a_string() {
    check_ok(
        "
let word: String = \"héllo\"
for ch, i in word
  let c: String = ch
  let n: Int = i
end
",
    );
    let errors = check_err("for ch in \"abc\" { let n: Int = ch; }");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch: the loop variable is a String",
    );
}

#[test]
fn for_over_int_errors() {
    let errors = check_err("for x in 5 { let y = x; }");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::General { .. }),
        "General iterate",
    );
}

// =====================================================================
// Arrays / Hashes
// =====================================================================

#[test]
fn array_literal_homogeneous_typechecks() {
    check_ok("let xs: Int[] = [1, 2, 3];");
}

#[test]
fn array_index_with_int_typechecks() {
    check_ok("let xs = [1, 2, 3]; let y: Int = xs[0];");
}

#[test]
fn array_index_with_bool_errors() {
    let errors = check_err("let xs = [1, 2, 3]; let y = xs[true];");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { expected, .. } if expected == "Int"),
        "Mismatch with expected=Int on array index",
    );
}

#[test]
fn array_method_map_typechecks() {
    check_ok("let xs = [1, 2, 3]; let ys = xs.map(fn(x) x * 2);");
}

#[test]
fn array_method_unknown_errors() {
    let errors = check_err("let xs = [1, 2, 3]; let y = xs.no_such_method;");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::NoSuchMember { member, .. } if member == "no_such_method"),
        "NoSuchMember(no_such_method)",
    );
}

#[test]
fn hash_literal_typechecks() {
    check_ok(r#"let h = {"a": 1, "b": 2};"#);
}

#[test]
fn hash_with_array_key_errors() {
    // Arrays are not hashable per check_hash_expr.
    let errors = check_err(r#"let h = {[1]: "x"};"#);
    assert_any(
        &errors,
        |e| matches!(e, TypeError::General { message, .. } if message.contains("hash key")),
        "General 'cannot be used as a hash key'",
    );
}

#[test]
fn hash_index_with_correct_key_type_typechecks() {
    check_ok(r#"let h = {"a": 1}; let v = h["a"];"#);
}

#[test]
fn string_method_length_typechecks() {
    check_ok(r#"let n: Int = "hello".length();"#);
}

#[test]
fn string_method_unknown_errors() {
    let errors = check_err(r#"let x = "hi".not_a_method;"#);
    assert_any(
        &errors,
        |e| matches!(e, TypeError::NoSuchMember { member, .. } if member == "not_a_method"),
        "NoSuchMember(not_a_method)",
    );
}

// =====================================================================
// Classes / interfaces / this / super
// =====================================================================

#[test]
fn class_with_method_typechecks() {
    check_ok(
        "
        class Person {
            name: String;
            new(name: String) { this.name = name; }
            fn greet() -> String { return \"hi \" + this.name; }
        }
        ",
    );
}

#[test]
fn class_extends_with_super_typechecks() {
    // Constructors must have a non-empty body — the parser rejects `new() {}`.
    check_ok(
        r#"
        class A {
            x: Int;
            new() { this.x = 0; }
            fn hello() -> String { return "A"; }
        }
        class B extends A {
            new() { super(); }
        }
        "#,
    );
}

#[test]
fn this_outside_class_errors_in_function() {
    let errors = check_err("fn foo() { this.x; }");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::ThisOutsideClass(_)),
        "ThisOutsideClass",
    );
}

#[test]
fn super_outside_class_errors_in_function() {
    let errors = check_err("fn foo() { super(); }");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::SuperOutsideClass(_)),
        "SuperOutsideClass",
    );
}

#[test]
fn super_in_class_without_superclass_errors() {
    let errors = check_err(
        "
        class A {
            fn x() { super(); }
        }
        ",
    );
    assert_any(
        &errors,
        |e| matches!(e, TypeError::NoSuperclass(name, _) if name == "A"),
        "NoSuperclass(A)",
    );
}

#[test]
fn class_implementing_interface_correctly_typechecks() {
    check_ok(
        "
        interface Greeter {
            fn greet() -> String;
        }
        class Hi implements Greeter {
            fn greet() -> String { return \"hi\"; }
        }
        ",
    );
}

#[test]
fn class_missing_interface_method_errors() {
    let errors = check_err(
        "
        interface Greeter {
            fn greet() -> String;
        }
        class Bad implements Greeter {
        }
        ",
    );
    assert_any(
        &errors,
        |e| matches!(e, TypeError::General { message, .. } if message.contains("does not implement")),
        "General 'does not implement'",
    );
}

#[test]
fn class_interface_signature_mismatch_errors() {
    let errors = check_err(
        "
        interface Greeter {
            fn greet() -> String;
        }
        class Bad implements Greeter {
            fn greet() -> Int { return 1; }
        }
        ",
    );
    assert_any(
        &errors,
        |e| matches!(e, TypeError::General { message, .. } if message.contains("does not match")),
        "General 'does not match interface signature'",
    );
}

// (member access on instances of `new ClassName()` is currently
// silently accepted; see `bug_member_access_on_new_instance_is_silently_accepted`
// in the bug-pinning section below.)

// =====================================================================
// Try / catch / throw
// =====================================================================

#[test]
fn try_catch_typechecks() {
    check_ok(
        r#"
        try {
            let x = 1;
        } catch error {
            let y = error;
        }
        "#,
    );
}

#[test]
fn throw_statement_typechecks() {
    check_ok(r#"throw "boom";"#);
}

// =====================================================================
// Pipelines
// =====================================================================

#[test]
fn pipeline_with_compatible_function_typechecks() {
    check_ok(
        "
        fn double(x: Int) -> Int { return x * 2; }
        let r: Int = 5 |> double();
        ",
    );
}

#[test]
fn pipeline_lhs_type_mismatch_errors() {
    let errors = check_err(
        r#"
        fn double(x: Int) -> Int { return x * 2; }
        let r = "hi" |> double();
        "#,
    );
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch on pipeline LHS",
    );
}

// =====================================================================
// Match
// =====================================================================

#[test]
fn match_with_literal_typechecks() {
    check_ok(
        r#"
        let x = 5;
        let r = match x {
            1 => "one",
            _ => "other",
        };
        "#,
    );
}

#[test]
fn match_with_typed_pattern_typechecks() {
    // Constructors require a non-empty body — the parser rejects `new() {}`.
    check_ok(
        r#"
        class A {
            x: Int;
            new() { this.x = 0; }
        }
        fn handle(a: Any) -> String {
            return match a {
                value: A => "a",
                _ => "other",
            };
        }
        "#,
    );
}

// =====================================================================
// Rescue / nullish coalescing / postfix
// =====================================================================

#[test]
fn rescue_typechecks() {
    check_ok(r#"let x = 1 rescue 0;"#);
}

#[test]
fn rescue_null_fallback_does_not_make_the_value_null() {
    // The shape that failed a whole script before its first line: the
    // fallback's `Null` became the variable's type, so the index after the
    // nil check was refused as "cannot index Null".
    check_ok(
        r#"
def vectors_for(texts)
  return [[1.0, 2.0]]
end

def first_vector
  vectors = vectors_for([]) rescue null
  if vectors.nil?
    return null
  end
  return vectors[0]
end
"#,
    );
}

#[test]
fn rescue_with_same_type_on_both_sides_keeps_it() {
    // Widening, not erasure: two `Int`s stay an `Int`, so a wrong
    // annotation is still caught.
    check_ok(r#"let x: Int = 1 rescue 0;"#);
    let errors = check_err(r#"let x: String = 1 rescue 0;"#);
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch on let x: String = 1 rescue 0",
    );
}

#[test]
fn bare_nil_assignment_can_be_reassigned() {
    // `x = nil` without `let` reads like `let x = nil`: nothing known yet.
    check_ok(
        r#"
def load
  records = null
  records = [[1.0]]
  return records[0]
end
"#,
    );
}

#[test]
fn nullish_coalescing_typechecks() {
    check_ok(r#"let x: Int = null ?? 0;"#);
}

#[test]
fn postfix_increment_typechecks() {
    check_ok("let x = 1; x++;");
}

#[test]
fn compound_assign_typechecks() {
    check_ok("let x = 1; x += 2;");
}

// =====================================================================
// Comprehensions / interpolation / spread
// =====================================================================

#[test]
fn list_comprehension_typechecks() {
    check_ok("let xs = [n * 2 for n in [1, 2, 3]];");
}

#[test]
fn list_comprehension_non_bool_filter_errors() {
    let errors = check_err("let xs = [n for n in [1, 2, 3] if 5];");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch on filter",
    );
}

#[test]
fn interpolated_string_typechecks() {
    // Soli's lexer uses `#{...}` for interpolation (not `\(...)`).
    check_ok(r#"let n = 5; let s: String = "n=#{n}";"#);
}

// =====================================================================
// Bug-pinning tests
//
// The tests below document behaviour the type checker should arguably
// reject but currently accepts. They are `#[ignore]`'d so the suite stays
// green; remove the `#[ignore]` once the underlying bug is fixed and the
// test will turn into a real regression check.
// =====================================================================

/// BUG: `resolve_type` silently turns an unknown type name into
/// `Type::Unknown`, which `is_assignable_to` treats as compatible with
/// everything. As a result, a typo in a type annotation (e.g.
/// `let x: NotARealType = 5`) is silently accepted instead of producing
/// `TypeError::UndefinedType`. See `src/types/checker/mod.rs::resolve_type`.
#[test]
#[ignore = "BUG: unknown type names should error, not become Type::Unknown"]
fn bug_unknown_type_name_in_annotation_is_silently_accepted() {
    let errors = check_err("let x: NotARealType = 42;");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::UndefinedType(name, _) if name == "NotARealType"),
        "UndefinedType(NotARealType)",
    );
}

/// `new ClassName()` builds an instance of the class (#693): a member the
/// class does not have is refused, as on `this`.
#[test]
fn member_access_on_new_instance_is_checked() {
    let errors = check_err(
        "
        class A {
            x: Int;
            new() { this.x = 1; }
        }
        let a = new A();
        let y = a.no_such;
        ",
    );
    assert_any(
        &errors,
        |e| matches!(e, TypeError::NoSuchMember { member, .. } if member == "no_such"),
        "NoSuchMember(no_such)",
    );
}

// Note: we initially had tests for `throw` *as an expression* and for `if`
// *as expression* — the corresponding type-checker code paths
// (`check_throw_expr`, `check_if_expr` strict-Bool branch) exist but appear
// unreachable from the current grammar: the parser rejects
// `let x = throw "...";` and `let x = if c {..} else {..};`. If those grammar
// features are added later, those checker paths will suddenly become live — at
// which point `check_throw_expr` will panic with `unimplemented!()`. Worth
// fixing pre-emptively, or removing the dead code.
//
// The parallel `await` case is gone: the `await`/`async` keywords and the dead
// `ExprKind::Await` variant were removed. `await(...)` is now an ordinary call
// to the `await()` builtin, so no `ExprKind::Await` / `check_await_expr` path
// exists to become live.

#[test]
fn an_instance_is_indexed_by_field_name() {
    // `this[name]` / `record[field] = value` read and write a field on both
    // engines; the checker refused the program ("cannot index Box").
    check_ok(
        r#"
class Box
  label: String

  def put(name, value)
    this[name] = value
    this[name]
  end
end

box = new Box()
field = "label"
box[field] = "b"
print(box[field])
"#,
    );
}

// ---------- value objects and domain errors ----------

#[test]
fn value_objects_and_domain_errors_typecheck() {
    check_ok(
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
end
class InsufficientFunds < DomainError
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
class Amount2 < ValueObject
  cents: Int
  currency: String = "EUR"
end
print(Amount2.new({"cents": 5}).to_h)
print(Amount2.new(cents: 7).to_h)
print(Amount2.new(9, "USD").to_h)
print(Amount2.new({"cents": 5}) == Amount2.new(5))"##,
    );
}

/// `C.new(...)` is a construction, as `new C(...)` is: `soli check` types the
/// receiver from either spelling and refuses a private call on it.
#[test]
fn klass_new_types_the_receiver_like_new_klass() {
    let source = |construction: &str| {
        format!(
            "class Vault
  private

  def secret
    1
  end
end
v = {construction}
v.secret
{construction}.secret
"
        )
    };
    for construction in ["Vault.new()", "new Vault()"] {
        let errors = check_err(&source(construction));
        let private_calls = errors
            .iter()
            .filter(|e| {
                matches!(e, TypeError::General { message, .. }
                    if message == "private method 'secret' called for an instance of Vault")
            })
            .count();
        assert_eq!(private_calls, 2, "{construction}: {errors:?}");
    }
    // `.new` on a variable that holds no known class types nothing.
    check_ok("def build(klass)\n  made = klass.new()\n  made.anything\nend\n");
}

/// A nested class named from inside its outer class: `Inner.new(...)` is
/// left to the runtime, as `new Inner(...)` is.
#[test]
fn klass_new_on_a_nested_class_from_its_outer_class() {
    for construction in ["Inner.new(x)", "new Inner(x)"] {
        check_ok(&format!(
            "class Outer
  def build(x)
    {construction}
  end

  class Inner
    value: Any

    new(value)
      @value = value
    end
  end
end
"
        ));
    }
}

// ---------- match rest bindings ----------

/// `...rest` in an array or hash pattern binds a name the arm can use. The
/// checker used to skip it, so any read of `rest` was an "Undefined variable"
/// and the program never ran on either engine.
#[test]
fn match_rest_bindings_are_defined_in_the_arm() {
    check_ok(
        "def head_tail(xs)\n  match xs {\n    [first, ...rest] => [first, rest.length, rest],\n    _ => nil\n  }\nend\n\
         def others(h)\n  match h {\n    {name: n, ...rest} => rest,\n    _ => nil\n  }\nend\n\
         print(head_tail([1, 2, 3]))\nprint(others({\"name\": \"a\", \"age\": 1}))",
    );
}

// #678: a local named after a builtin. The assignment used to fail as
// "expected (Any, Any) -> String, found String", naming neither the builtin
// nor the fix; the call of a deliberately shadowed name as "Cannot call
// non-function type 'String'".
#[test]
fn assigning_over_a_builtin_names_the_builtin() {
    let errors = check_err("def probe\n  page_url = \"x\"\n  page_url.length\nend\n");
    assert_any(
        &errors,
        |e| {
            matches!(e, TypeError::General { message, .. }
                if message.contains("builtin page_url()") && message.contains("rename"))
        },
        "a message naming the builtin page_url()",
    );
}

#[test]
fn calling_a_local_that_shadows_a_builtin_names_the_builtin() {
    let errors = check_err("def probe\n  let stream = \"x\"\n  stream(1, 2)\nend\n");
    assert_any(
        &errors,
        |e| {
            matches!(e, TypeError::General { message, .. }
                if message.contains("`stream` is a local String here, shadowing the builtin stream(); rename it"))
        },
        "a message naming the shadowed builtin stream()",
    );
}

#[test]
fn calling_an_ordinary_non_function_is_still_not_callable() {
    let errors = check_err("def probe\n  let total = \"x\"\n  total(1)\nend\n");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::NotCallable(_, _)),
        "NotCallable",
    );
}

// =====================================================================
// #693 phase 1: the false positives the in-repo corpus surfaced
// =====================================================================

#[test]
fn conditions_take_any_value() {
    check_ok(
        r#"
xs = [1, 2]
h = {"a": 1}
f = fn(x) { x }
if xs.length
  print("some")
end
if h
  print("hash")
end
print("fn") if f
t = h ? 1 : 2
"#,
    );
}

#[test]
fn hash_has_key_answers_a_bool() {
    check_ok(
        r#"
const SCRIPTS = {"a": {"x": 1}}
let found: Bool = SCRIPTS.has_key("a")
"#,
    );
}

#[test]
fn reassigning_an_unannotated_local_widens_it() {
    check_ok(
        r#"
def score(n)
  total = 0
  total = total * 1.5
  label = "x"
  label = {"k": 1}
  label["k"]
end
"#,
    );
}

#[test]
fn reassigning_an_annotated_local_is_still_refused() {
    let errors = check_err("let x: Int = 1\nx = \"s\"\n");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch on annotated reassignment",
    );
}

#[test]
fn reassigning_a_typed_parameter_is_still_refused() {
    let errors = check_err("def f(x: Int)\n  x = \"s\"\n  x\nend\n");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch on typed parameter reassignment",
    );
}

#[test]
fn a_local_named_like_a_modelled_non_builtin_is_a_fresh_local() {
    // `push` is no global at run time; `push = 0.0` is a plain local.
    check_ok("def f()\n  push = 0.0\n  push += 1.0\n  push\nend\n");
}

#[test]
fn a_nullable_parameter_accepts_nil() {
    check_ok(
        r#"
def f(x: Int?) -> Int
  return 0 if x.nil?

  x + 1
end
f(nil)
f(2)
"#,
    );
}

#[test]
fn a_nullable_slot_takes_nil_on_reassignment_and_return() {
    check_ok(
        r#"
def g(x: String?) -> String?
  x = nil
  return nil
end
let y: Int? = nil
y = 3
"#,
    );
}

#[test]
fn a_nullable_parameter_still_refuses_another_type() {
    let errors = check_err("def f(x: Int?)\n  x\nend\nf(\"s\")\n");
    assert!(!errors.is_empty());
}

#[test]
fn two_errors_in_one_def_body_are_both_reported() {
    let errors = check_err(
        r#"
def f()
  let a: Int = "one"
  let b: String = 2
  a
end
"#,
    );
    assert_eq!(errors.len(), 2, "{errors:?}");
}

#[test]
fn an_error_in_a_block_does_not_hide_the_rest_of_the_def() {
    let errors = check_err(
        r#"
def f(xs)
  xs.each do |x|
    let a: Int = "one"
  end
  let b: String = 2
end
"#,
    );
    assert_eq!(errors.len(), 2, "{errors:?}");
}

#[test]
fn a_name_bound_by_a_failing_statement_does_not_cascade() {
    let errors = check_err(
        r#"
def f()
  let a: Int = "one"
  a + 1
end
"#,
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
}

#[test]
fn a_locals_of_one_failing_def_do_not_leak_into_the_next() {
    // A failing def used to leave its scope pushed, so the next def saw its
    // locals: `reach = 0.0` there made `reach[0]` "cannot index Float" here.
    let errors = check_err(
        r#"
def first()
  reach = 0.0
  let broken: Int = "x"
  reach
end

def second(sys)
  reach = [1.0, 2.0]
  reach[0]
end
"#,
    );
    assert_eq!(errors.len(), 1, "{errors:?}");
}

#[test]
fn a_block_inside_a_def_keeps_the_defs_return_type() {
    let errors = check_err(
        r#"
def f(xs) -> Int
  xs.each do |x|
    print(x)
  end
  return "not an int"
end
"#,
    );
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "return mismatch after a block",
    );
}

#[test]
fn a_bare_assignment_to_a_def_of_the_file_is_still_refused() {
    // Both engines rebind the global function: after `helper = 5` in `f`,
    // `helper()` is "Cannot call non-function value". Widening must not hide
    // it, wherever the def sits in the file relative to the assignment.
    for src in [
        "def helper\n  1\nend\ndef f\n  helper = 5\n  helper\nend\n",
        "class K\n  def go\n    helper = 5\n    helper\n  end\nend\ndef helper\n  1\nend\n",
    ] {
        let errors = check_err(src);
        assert_any(
            &errors,
            |e| e.to_string().contains("rebinds rather than shadows"),
            "rebinding a def of the file",
        );
    }
    // `let` shadows instead, at run time and here.
    check_ok("def helper\n  1\nend\ndef f\n  let helper = 5\n  helper\nend\n");
}

#[test]
fn a_top_level_failing_statement_does_not_cascade() {
    let errors = check_err("let a: Int = \"one\"\nprint(a + 1)\nb = a\n");
    assert_eq!(errors.len(), 1, "{errors:?}");
}

// #693 phase 2: instances, instance variables and `def` arity.

#[test]
fn klass_new_builds_a_checked_instance() {
    let errors = check_err(
        "class Pricing\n  def total(cents: Int) -> Int\n    cents * 2\n  end\nend\npricing = Pricing.new()\npricing.totl(3)\n",
    );
    assert_any(
        &errors,
        |e| matches!(e, TypeError::NoSuchMember { member, .. } if member == "totl"),
        "NoSuchMember(totl)",
    );
}

#[test]
fn an_instance_variable_the_constructor_assigns_is_a_field() {
    check_ok(
        "class Point\n  new(x)\n    @x = x\n  end\n\n  def shifted\n    @x + 1\n  end\nend\np = Point.new(1)\nprint(p.x)\nprint(p.shifted)\n",
    );
}

#[test]
fn a_def_is_checked_for_arity_with_its_defaults() {
    let src = "def greet(name, greeting = \"Hi\")\n  \"#{greeting} #{name}\"\nend\n";
    check_ok(&format!(
        "{src}greet(\"a\")\ngreet(\"a\", \"Yo\")\ngreet(name: \"a\")\n"
    ));
    let errors = check_err(&format!("{src}greet(\"a\", \"b\", \"c\")\n"));
    assert_any(
        &errors,
        |e| {
            matches!(
                e,
                TypeError::WrongArity {
                    expected: 2,
                    got: 3,
                    ..
                }
            )
        },
        "WrongArity(2, 3)",
    );
    let errors = check_err(&format!("{src}greet()\n"));
    assert_any(
        &errors,
        |e| {
            matches!(
                e,
                TypeError::WrongArity {
                    expected: 1,
                    got: 0,
                    ..
                }
            )
        },
        "WrongArity(1, 0)",
    );
}

#[test]
fn a_class_with_its_own_static_new_is_not_assumed_to_build_itself() {
    check_ok(
        "class Cache\n  static def new(name)\n    {\"name\": name}\n  end\nend\nc = Cache.new(\"x\")\nprint(c.anything)\n",
    );
}

#[test]
fn a_private_static_method_read_on_its_class_is_accepted() {
    // Neither engine enforces visibility on `Klass._helper(…)`.
    check_ok(
        "class Feed\n  static def run\n    Feed._text(1)\n  end\n\n  private\n\n  static def _text(x)\n    x\n  end\nend\nprint(Feed.run)\nprint(Feed._text(2))\n",
    );
}

#[test]
fn a_job_class_has_the_job_facade() {
    let source = "class EmailJob\n  static def perform(args: Hash)\n    args\n  end\nend\nEmailJob.perform_later({})\nEmailJob.perform_now({})\n";
    let ok = solilang::type_check_source_with_ambient(
        source,
        Some(std::path::Path::new("app/jobs/email_job.sl")),
        &[],
    );
    assert!(ok.is_ok(), "{ok:?}");
    assert!(solilang::type_check_source_with_ambient(source, None, &[]).is_err());
}
