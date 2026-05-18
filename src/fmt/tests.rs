//! Integration tests for the formatter — canonical-output checks and
//! idempotency (`fmt(fmt(x)) == fmt(x)`).

use super::format_source;

fn assert_fmt(input: &str, expected: &str) {
    let actual = format_source(input).expect("format_source failed");
    assert_eq!(
        actual.trim_end(),
        expected.trim_end(),
        "\n--- expected ---\n{}\n--- actual ---\n{}\n",
        expected,
        actual
    );
}

fn assert_idempotent(input: &str) {
    let once = format_source(input).expect("first format failed");
    let twice = format_source(&once).expect("second format failed");
    assert_eq!(
        once, twice,
        "fmt is not idempotent:\n--- once ---\n{}\n--- twice ---\n{}\n",
        once, twice
    );
}

#[test]
fn simple_let_const() {
    assert_fmt("let   x=1\nconst   Y=2\n", "let x = 1\nconst Y = 2\n");
}

#[test]
fn binary_operator_spacing() {
    assert_fmt("let z=a+b*c-d\n", "let z = a + b * c - d\n");
}

#[test]
fn if_block_uses_ruby_style() {
    assert_fmt(
        "if x { print(1) } else { print(2) }\n",
        "if x\n  print(1)\nelse\n  print(2)\nend\n",
    );
}

#[test]
fn nested_function_uses_two_space_indent() {
    assert_fmt(
        "fn outer { fn inner { 1 } }\n",
        "fn outer\n  fn inner\n    1\n  end\nend\n",
    );
}

#[test]
fn class_with_methods_separated_by_blank_line() {
    // Methods keep `()` even when empty (matches the task-orchestrator
    // convention `static def run_state_root()`).
    let src = "class A < B\n  def first\n    1\n  end\n  def second\n    2\n  end\nend\n";
    let expected =
        "class A < B\n  def first()\n    1\n  end\n\n  def second()\n    2\n  end\nend\n";
    assert_fmt(src, expected);
}

#[test]
fn single_stmt_block_if_collapses_to_postfix() {
    // Idiomatic Soli prefers postfix `expr if cond` for guard clauses
    // (per the language docs). The formatter rewrites block-form `if cond
    // <single-stmt> end` to postfix when it fits on one line.
    let src = "fn f(x)\n  if x == 0\n    return 0\n  end\n  return x * 2\nend\n";
    let expected = "fn f(x)\n  return 0 if x == 0\n  return x * 2\nend\n";
    assert_fmt(src, expected);
}

#[test]
fn hash_literal_spacing() {
    assert_fmt(
        "let h={\"a\":1,\"b\":2}\n",
        "let h = {\"a\": 1, \"b\": 2}\n",
    );
}

#[test]
fn array_literal_spacing() {
    assert_fmt("let a=[1,2,3]\n", "let a = [1, 2, 3]\n");
}

#[test]
fn member_and_index_no_spaces() {
    assert_fmt("let v = obj . field [ 0 ]\n", "let v = obj.field[0]\n");
}

#[test]
fn comment_preserved_above_statement() {
    let src = "# top comment\nlet x = 1\n";
    let expected = "# top comment\nlet x = 1\n";
    assert_fmt(src, expected);
}

#[test]
fn slash_slash_comment_normalized_to_hash() {
    let src = "// a\nlet x = 1\n";
    let expected = "# a\nlet x = 1\n";
    assert_fmt(src, expected);
}

#[test]
fn blank_line_between_top_level_statements_preserved() {
    let src = "let x = 1\n\nlet y = 2\n";
    assert_fmt(src, src);
}

#[test]
fn three_or_more_blank_lines_collapse_to_one() {
    let src = "let x = 1\n\n\n\nlet y = 2\n";
    let expected = "let x = 1\n\nlet y = 2\n";
    assert_fmt(src, expected);
}

#[test]
fn import_statement() {
    assert_fmt("import \"./foo.sl\"\n", "import \"./foo.sl\"\n");
}

#[test]
fn call_with_named_args() {
    assert_fmt(
        "configure(port:3000,host:\"x\")\n",
        "configure(port: 3000, host: \"x\")\n",
    );
}

#[test]
fn lambda_inline_form() {
    assert_fmt("let f=fn(x){return x*2}\n", "let f = fn(x) { x * 2 }\n");
}

#[test]
fn idempotent_controller_sample() {
    let src = "# A controller\nclass PostsController < Controller\n  def index(req)\n    let posts = Post.all()\n    return render(\"posts/index\", {\"posts\": posts})\n  end\nend\n";
    assert_idempotent(src);
}

#[test]
fn idempotent_class_with_static_method() {
    let src = "class Run\n  static def run_state_root\n    \"/tmp\"\n  end\n\n  static def run_log_path(repo)\n    run_state_root() + \"/\" + repo\n  end\nend\n";
    assert_idempotent(src);
}

#[test]
fn idempotent_nested_control_flow() {
    let src = "fn f(x)\n  if x > 0\n    while x > 0\n      x = x - 1\n    end\n  end\nend\n";
    assert_idempotent(src);
}

#[test]
fn idempotent_match_expression() {
    let src = "fn label(v)\n  match v {\n    42 => \"answer\",\n    _ => \"other\",\n  }\nend\n";
    assert_idempotent(src);
}

#[test]
fn idempotent_test_with_inline_lambda_assertion() {
    // Regression: `test("...", fn() { assert_eq(a, b) })` used to oscillate
    // between forms across fmt passes — the lambda's inline check used raw
    // source byte length (which depends on whether the source has the body
    // wrapped or inline) and the call's break heuristic added a +8 safety
    // margin that triggered false-positive wraps on borderline lines.
    let src = "describe(\"x\", fn() {\n  describe(\"y\", fn() {\n    test(\"⚙ prefix returns text-slate-500\", fn() {\n      assert_eq(task_log_line_class(\"⚙ some log\"), \"text-slate-500\")\n    })\n\n    test(\"returns empty when no reviews for the task\", fn() { assert_eq(CodeReview.for_task(\"p\", \"s\").length(), 0) })\n  })\n})\n";
    assert_idempotent(src);
}

#[test]
fn parse_error_propagates() {
    let res = format_source("class { broken");
    assert!(res.is_err(), "expected parse error, got {:?}", res);
}

// ----------------------------------------------------------------------------
// Round-trip safety: the formatter must never emit syntax the parser rejects.
// `assert_round_trip` verifies that the formatted output re-parses cleanly.
// ----------------------------------------------------------------------------

fn assert_round_trip(input: &str) {
    let formatted = format_source(input).expect("first format failed");
    // The output must lex + parse cleanly. We don't compare to a canonical
    // string here — that's what assert_fmt is for; this only catches
    // surface syntax the parser rejects.
    let tokens = crate::lexer::Scanner::new(&formatted)
        .scan_tokens()
        .unwrap_or_else(|e| {
            panic!(
                "formatted output failed to lex: {:?}\n---formatted---\n{}",
                e, formatted
            )
        });
    let _ = crate::parser::Parser::new(tokens)
        .parse()
        .unwrap_or_else(|e| {
            panic!(
                "formatted output failed to parse: {:?}\n---formatted---\n{}",
                e, formatted
            )
        });
}

// ---- Bug 1: string interpolation must emit Ruby-style #{...} ----

#[test]
fn string_interpolation_uses_hash_braces() {
    assert_fmt(
        "let name = \"Alice\"\nprint(\"Hello #{name}!\")\n",
        "let name = \"Alice\"\nprint(\"Hello #{name}!\")\n",
    );
}

#[test]
fn string_interpolation_round_trips() {
    assert_round_trip("let n = 1\nprint(\"n = #{n}\")\n");
}

#[test]
fn string_interpolation_idempotent() {
    assert_idempotent("let x = 1\nlet s = \"x=#{x}\"\n");
}

// ---- Bug 2: ternary `?:` must round-trip as `?:`, not `if-then-else` ----

#[test]
fn ternary_keeps_question_colon_form() {
    assert_fmt(
        "let s = x > 5 ? \"big\" : \"small\"\n",
        "let s = x > 5 ? \"big\" : \"small\"\n",
    );
}

#[test]
fn ternary_round_trips() {
    assert_round_trip("let s = x > 0 ? \"pos\" : \"neg\"\n");
}

#[test]
fn nested_ternary_round_trips() {
    assert_round_trip("let g = n >= 90 ? \"A\" : n >= 80 ? \"B\" : \"C\"\n");
}

// ---- Bug 3: interface bodies need `{ }` braces and `fn` keyword ----

#[test]
fn interface_uses_brace_body() {
    let src = "interface Drawable { fn draw() -> String }\n";
    let formatted = format_source(src).expect("fmt failed");
    assert!(
        formatted.contains("interface Drawable {"),
        "expected braces, got:\n{}",
        formatted
    );
    assert!(
        formatted.contains("fn draw"),
        "interface methods must use `fn`, got:\n{}",
        formatted
    );
    assert!(
        !formatted.contains("def draw"),
        "interface methods must NOT use `def`, got:\n{}",
        formatted
    );
}

#[test]
fn interface_round_trips() {
    assert_round_trip("interface Printable { fn print() -> String\nfn name() -> String }\n");
}

// ---- Bug 4: postfix `if` / `unless` must round-trip in postfix form ----

#[test]
fn postfix_if_preserved() {
    assert_fmt(
        "let x = 10\nprint(\"big\") if x > 5\n",
        "let x = 10\nprint(\"big\") if x > 5\n",
    );
}

#[test]
fn postfix_unless_preserved() {
    assert_fmt(
        "let x = 10\nprint(\"small\") unless x > 5\n",
        "let x = 10\nprint(\"small\") unless x > 5\n",
    );
}

#[test]
fn postfix_unless_strips_synthetic_not() {
    // Parser desugars `expr unless cond` to `If { !cond, then: expr }`.
    // The formatter must emit `unless cond` again, NOT `unless !cond`.
    let formatted = format_source("let y = 3\nprint(\"x\") unless y > 5\n").unwrap();
    assert!(
        formatted.contains("unless y > 5"),
        "must emit `unless y > 5`, got:\n{}",
        formatted
    );
    assert!(
        !formatted.contains("unless !"),
        "must NOT double-negate the unless cond, got:\n{}",
        formatted
    );
}

#[test]
fn block_if_with_else_stays_block_form() {
    // Block form is preserved when there's an else branch (postfix has
    // no else-form), so the formatter must not collapse it to postfix.
    let src = "if x > 5\n  print(\"big\")\nelse\n  print(\"small\")\nend\n";
    assert_fmt(src, src);
}

#[test]
fn postfix_if_round_trips() {
    assert_round_trip("let a = 10\nprint(\"ok\") if a > 0\n");
}

#[test]
fn postfix_unless_round_trips() {
    assert_round_trip("let a = 10\nprint(\"ok\") unless a < 0\n");
}

#[test]
fn postfix_return_if_round_trips() {
    assert_round_trip("fn f(x)\n  return null if x == 0\n  return x * 2\nend\n");
}

#[test]
fn postfix_if_idempotent() {
    assert_idempotent("let a = 10\nprint(\"big\") if a > 5\nprint(\"small\") unless a > 5\n");
}

// ---- Bug 5: static blocks need `{ ... }` braces (no `end` form) ----

#[test]
fn static_block_uses_braces() {
    let src = "class Hooks\n  static {\n    this.x = 1\n  }\nend\n";
    let formatted = format_source(src).expect("fmt failed");
    assert!(
        formatted.contains("static {"),
        "static block must keep braces, got:\n{}",
        formatted
    );
    assert!(
        !formatted.contains("static\n"),
        "static must NOT use end-form, got:\n{}",
        formatted
    );
}

#[test]
fn static_block_round_trips() {
    assert_round_trip(
        "class Hooks\n  static {\n    this.before_action = fn(req) { req }\n  }\nend\n",
    );
}

#[test]
fn static_block_idempotent() {
    assert_idempotent("class A\n  static {\n    this.x = 1\n    this.y = 2\n  }\nend\n");
}

// ---- Bug 6: block argument `&fn(...)` must become `&{ |params| body }` ----

#[test]
fn block_arg_uses_brace_block_form() {
    let src = "let r = [1, 2, 3].map(&fn(x) { x * 2 })\n";
    let formatted = format_source(src).expect("fmt failed");
    assert!(
        !formatted.contains("&fn("),
        "block arg must not emit &fn(...), got:\n{}",
        formatted
    );
    assert!(
        formatted.contains("&{"),
        "block arg should use &{{ |...| ... }}, got:\n{}",
        formatted
    );
}

#[test]
fn block_arg_round_trips() {
    assert_round_trip("let r = [1, 2, 3].map(&fn(x) { x * 2 })\n");
}

#[test]
fn block_arg_variable_reference_preserved() {
    // `.map(&double)` — block arg that's a variable reference, not a lambda.
    // The `&identifier` form must round-trip as-is.
    assert_round_trip("let f = fn(x) { x * 2 }\nlet r = [1, 2].map(&f)\n");
}

#[test]
fn block_arg_idempotent() {
    assert_idempotent("let r = [1, 2, 3].map(&fn(x) { x * 2 }).filter(&fn(y) { y > 2 })\n");
}

// ---- Corpus-level safety: a bouquet of constructs together ----

#[test]
fn fmt_then_reparse_complex_program() {
    let src = r#"
# Demo
class Counter
  count: Int

  new()
    this.count = 0
  end

  def increment() -> Void
    this.count = this.count + 1
  end

  def reset_if_big() -> Void
    return if this.count < 10

    this.count = 0
  end
end

interface Named { fn name() -> String }

let c = new Counter()
c.increment()
print("count = #{c.count}")
let label = c.count > 5 ? "big" : "small"
print(label) if c.count > 0
print("none") unless c.count > 0
[1, 2, 3].each(&fn(x) { print(x) })
"#;
    assert_round_trip(src);
    assert_idempotent(src);
}
