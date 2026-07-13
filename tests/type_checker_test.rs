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
        |e| matches!(e, TypeError::UndefinedVariable(name, _) if name == "unknown_var"),
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
fn if_statement_with_int_condition_errors() {
    let errors = check_err("if 5 { let x = 1; }");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch on if cond",
    );
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
fn while_with_int_condition_errors() {
    let errors = check_err("while 5 { let x = 1; }");
    assert_any(
        &errors,
        |e| matches!(e, TypeError::Mismatch { .. }),
        "Mismatch while cond",
    );
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

/// BUG: `check_new_expr` always returns `Type::Unknown` for `new ClassName()`
/// (see `src/types/checker/expressions/objects.rs:87-111`). Since
/// `Type::Unknown` is permissively assignable to/from anything, every
/// downstream member access on the instance silently passes — including
/// references to fields and methods that don't exist on the class. The type
/// checker therefore cannot catch typos like `instance.no_such_field`,
/// undermining the whole point of declaring the class.
#[test]
#[ignore = "BUG: `new ClassName()` returns Type::Unknown, hiding member-access errors"]
fn bug_member_access_on_new_instance_is_silently_accepted() {
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
