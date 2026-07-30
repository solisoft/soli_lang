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
    // Named declarations canonicalise to `def`, whichever keyword the source
    // used — `fn` is the lambda keyword.
    assert_fmt(
        "fn outer { fn inner { 1 } }\n",
        "def outer\n  def inner\n    1\n  end\nend\n",
    );
}

#[test]
fn class_with_methods_separated_by_blank_line() {
    // Empty parens are dropped on no-arg methods (`def first()` -> `def first`),
    // matching Soli's optional-parens convention.
    let src = "class A < B\n  def first()\n    1\n  end\n  def second()\n    2\n  end\nend\n";
    let expected = "class A < B\n  def first\n    1\n  end\n\n  def second\n    2\n  end\nend\n";
    assert_fmt(src, expected);
}

#[test]
fn single_stmt_block_if_collapses_to_postfix() {
    // Idiomatic Soli prefers postfix `expr if cond` for guard clauses
    // (per the language docs). The formatter rewrites block-form `if cond
    // <single-stmt> end` to postfix when it fits on one line.
    let src = "fn f(x)\n  if x == 0\n    return 0\n  end\n  return x * 2\nend\n";
    let expected = "def f(x)\n  return 0 if x == 0\n  return x * 2\nend\n";
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
fn blank_line_after_return_guard() {
    // An early return is separated from the body that follows.
    let src = "def f(x)\n  return nil if x.blank?\n  work(x)\nend\n";
    let expected = "def f(x)\n  return nil if x.blank?\n\n  work(x)\nend\n";
    assert_fmt(src, expected);
    assert_idempotent(src);
}

#[test]
fn no_blank_line_between_consecutive_returns() {
    // A run of guards reads as one paragraph — the blank goes after the last
    // of them, not between each pair.
    let src = "def f(x)\n  return 0 if x.nil?\n  return 1 if x == 0\n  compute(x)\nend\n";
    let expected = "def f(x)\n  return 0 if x.nil?\n  return 1 if x == 0\n\n  compute(x)\nend\n";
    assert_fmt(src, expected);
    assert_idempotent(src);
}

#[test]
fn multiline_raw_string_keeps_its_brackets() {
    // Regression: a `[[ … ]]` SDBQL query was re-emitted as an escaped
    // double-quoted string — semantics preserved, but the query collapsed onto
    // one line whose length then tripped `style/line-length`. Raw literals are
    // copied from source instead.
    let src = "def up(db)\n  db.query([[\n    FOR p IN posts\n      RETURN p\n  ]])\nend\n";
    assert_fmt(src, src);
    assert_idempotent(src);
    let out = format_source(src).unwrap();
    assert!(
        !out.contains("\\n"),
        "raw string must not be escaped:\n{}",
        out
    );
}

#[test]
fn single_line_raw_string_keeps_its_r_prefix() {
    let src = "let win = r\"C:\\Users\\name\"\nlet re = r\"\\d+\\.\\d+\"\n";
    assert_fmt(src, src);
    assert_idempotent(src);
}

#[test]
fn raw_string_holding_a_single_bracket_round_trips() {
    // `]` alone is content inside `[[ … ]]`; only `]]` closes. The value check
    // in `raw_string_source` is what keeps this from mis-slicing.
    assert_fmt("let odd = [[a]b]]\n", "let odd = [[a]b]]\n");
    assert_round_trip("let odd = [[a]b]]\n");
}

#[test]
fn escaped_string_with_newline_stays_escaped() {
    // Only *raw* literals are copied from source — a `"a\nb"` keeps the form
    // the author chose rather than being rewritten into brackets.
    assert_fmt(
        "let esc = \"line\\nbreak\"\n",
        "let esc = \"line\\nbreak\"\n",
    );
}

#[test]
fn no_blank_line_between_return_and_end() {
    // Nothing to separate the return from — the next line is the `end`.
    let src = "def f(x)\n  log(x)\n  return x * 2\nend\n";
    assert_fmt(src, src);
    assert_idempotent(src);
}

#[test]
fn existing_blank_line_after_return_not_doubled() {
    let src = "def f(x)\n  return nil if x.blank?\n\n  work(x)\nend\n";
    assert_fmt(src, src);
    assert_idempotent(src);
}

#[test]
fn blank_line_after_return_guard_precedes_next_statements_comment() {
    // The blank separates the guard from the paragraph below it, so it lands
    // above the comment that leads that paragraph — not between the comment
    // and the statement it documents.
    let src = "def f(x)\n  return nil if x.blank?\n  # then do the work\n  work(x)\nend\n";
    let expected = "def f(x)\n  return nil if x.blank?\n\n  # then do the work\n  work(x)\nend\n";
    assert_fmt(src, expected);
    assert_idempotent(src);
}

#[test]
fn no_blank_line_before_comment_starting_block_body() {
    // Regression: a comment as the first line of an `if` body must not gain a
    // spurious blank line above it. The block opener line is now recorded so
    // the comment's gap is measured from the opener, not from the statement
    // before the block (which previously looked like a paragraph break).
    let src = "fn f\n  x = 1\n  if cond\n    # first body line\n    y = 2\n  end\nend\n";
    let out = format_source(src).unwrap();
    assert!(
        !out.contains("if cond\n\n"),
        "no blank line should be inserted between `if` and its leading body comment:\n{}",
        out
    );
    assert_idempotent(src);
}

#[test]
fn long_typed_signature_wraps_under_line_limit() {
    // Regression: the param-list width estimate must account for the `: Type`
    // annotations, so a wide typed signature breaks across lines instead of
    // overflowing the line-length limit the linter enforces.
    let src = "class C\n  def self.init_params(referer: String, product_id: String, file: String, locale: String, country: String, code_integration: String) -> Hash\n    return {}\n  end\nend\n";
    let out = format_source(src).unwrap();
    assert!(
        out.lines().all(|l| l.chars().count() <= 120),
        "signature must wrap so no line exceeds 120 chars:\n{}",
        out
    );
    assert!(
        out.contains("static def init_params(\n"),
        "params should break onto their own lines:\n{}",
        out
    );
    assert_idempotent(src);
}

#[test]
fn long_named_arg_call_wraps_under_line_limit() {
    // Regression: the call-arg width estimate must count the `name: ` prefix of
    // named arguments, so a wide named-argument call breaks instead of
    // overflowing the line-length limit.
    let src = "class C\n  def show\n    params = CswClient.base_params(referer, image_height: SalesupFlow.image_height(), image_width: SalesupFlow.image_width())\n    return params\n  end\nend\n";
    let out = format_source(src).unwrap();
    assert!(
        out.lines().all(|l| l.chars().count() <= 120),
        "call must wrap so no line exceeds 120 chars:\n{}",
        out
    );
    assert_idempotent(src);
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

// ---- Block argument syntax: `&{ |params| body }` and `&identifier` ----

#[test]
fn block_arg_variable_reference_preserved() {
    // `.map(&double)` — block arg that's a variable reference, not a lambda.
    // The `&identifier` form must round-trip as-is.
    assert_round_trip("let f = fn(x) { x * 2 }\nlet r = [1, 2].map(&f)\n");
}

#[test]
fn inline_lambda_arg_round_trips() {
    assert_round_trip("let r = [1, 2, 3].map(|x| x * 2)\n");
}

#[test]
fn inline_lambda_arg_idempotent() {
    assert_idempotent("let r = [1, 2, 3].map(|x| x * 2).filter(|y| y > 2)\n");
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
let xs = [1, 2, 3]
xs.each(|x| print(x))
"#;
    assert_round_trip(src);
    assert_idempotent(src);
}

// ---------------------------------------------------------------------------
// Regressions: `soli fmt` output must survive `soli lint`.
//
// All four of these shipped together and were caught by running `soli fmt`
// followed by `soli lint` on a real app: the formatter emitted code that
// either failed to parse, exceeded the line limit, or quietly meant something
// different from what went in.
// ---------------------------------------------------------------------------

/// Asserts no line of the formatted output exceeds the limit `style/line-length`
/// enforces. The lint rule measures bytes (`str::len`), so this does too.
fn assert_within_line_limit(input: &str) {
    let out = format_source(input).expect("format failed");
    let long: Vec<_> = out
        .lines()
        .filter(|l| l.len() > super::printer::MAX_LINE_LENGTH)
        .collect();
    assert!(
        long.is_empty(),
        "formatter emitted lines over {} bytes (style/line-length would reject):\n{}\n--- full output ---\n{}",
        super::printer::MAX_LINE_LENGTH,
        long.join("\n"),
        out
    );
}

#[test]
fn block_unless_over_or_keeps_its_parens() {
    // `unless a || b` desugars to `Not(Or(a, b))`. Printing the operand bare
    // gives `!a || b` — that is `(!a) || b`, a different program. This was a
    // silent behaviour change, not just a formatting nit.
    let src =
        "fn f(a, b)\n  unless a || b\n    return \"neither\"\n  end\n  return \"some\"\nend\n";
    let expected = "def f(a, b)\n  return \"neither\" if !(a || b)\n  return \"some\"\nend\n";
    assert_fmt(src, expected);
    assert_round_trip(src);
    assert_idempotent(src);
}

#[test]
fn block_unless_over_and_keeps_its_parens() {
    let src = "fn f(a, b)\n  unless a && b\n    return 1\n  end\n  return 2\nend\n";
    let expected = "def f(a, b)\n  return 1 if !(a && b)\n  return 2\nend\n";
    assert_fmt(src, expected);
    assert_idempotent(src);
}

#[test]
fn wrapped_logical_keeps_the_operator_on_the_first_line() {
    // A Soli statement ends at its line break, so a continuation opening with
    // `&&` is a parse error. The operator has to trail.
    let src = "fn f(attrs, list)\n  if attrs.keys().includes?(\"title\") && ProductList.title_taken?(list.point_of_sale_id, attrs[\"title\"], list._key)\n    return unprocessable()\n  end\n  return ok()\nend\n";
    let out = format_source(src).expect("format failed");
    assert!(
        !out.lines()
            .any(|l| l.trim_start().starts_with("&&") || l.trim_start().starts_with("||")),
        "a continuation line must not open with a logical operator:\n{}",
        out
    );
    assert_round_trip(src);
    assert_idempotent(src);
    assert_within_line_limit(src);
}

#[test]
fn wrapped_logical_in_postfix_guard_reparses() {
    let src = "fn f(point_of_sale)\n  unless point_of_sale.nil? || point_of_sale.delivery_enabled == true || point_of_sale.pickup_enabled == true\n    return \"no delivery here at all\"\n  end\n  return null\nend\n";
    assert_round_trip(src);
    assert_idempotent(src);
    assert_within_line_limit(src);
}

#[test]
fn wide_string_arg_does_not_overflow_the_line() {
    // The per-arg width estimate used to be clamped at 60, so a call with one
    // long string argument was judged to fit and printed past the limit.
    let src = "fn f(point_of_sale_id)\n  builder = Cart.where(\"doc.point_of_sale_id == @pos && doc.paid_at != null && doc.delivered_at == null\", {\"pos\": point_of_sale_id})\n  return builder\nend\n";
    assert_within_line_limit(src);
    assert_round_trip(src);
    assert_idempotent(src);
}

#[test]
fn many_arg_call_of_long_strings_wraps() {
    let src = "fn f(produit)\n  response = deposer_json(\"/admin/products/#{produit._key}/picture\", \"picture\", \"point.png\", \"image/png\", \"des-octets\")\n  return response\nend\n";
    assert_within_line_limit(src);
    assert_round_trip(src);
    assert_idempotent(src);
}

#[test]
fn short_array_of_long_strings_wraps_on_width() {
    // Three elements starting below column 20 hit neither count-based break
    // rule, so the array printed on one 121-char line.
    let src = "fn f\n  for chemin in [\"/admin/products/inconnu\", \"/admin/products/inconnu/delete\", \"/admin/products/inconnu/picture/delete\"]\n    print(chemin)\n  end\nend\n";
    assert_within_line_limit(src);
    assert_round_trip(src);
    assert_idempotent(src);
}

#[test]
fn string_literal_width_counts_both_quotes() {
    // A string literal's span excludes its closing quote; the estimator has to
    // compute the width itself or every call containing strings reads narrow.
    let e = crate::ast::expr::Expr {
        kind: crate::ast::expr::ExprKind::StringLiteral("abc".to_string()),
        span: crate::span::Span::default(),
    };
    assert_eq!(super::expressions::ast_inline_width("", &e), 5); // `"abc"`
}

// ---------------------------------------------------------------------------
// Canonical spellings for named declarations and block arguments.
// ---------------------------------------------------------------------------

#[test]
fn top_level_declaration_uses_def_not_fn() {
    // `fn` is the lambda keyword. Named declarations are `def` at every level,
    // matching the controllers the language docs show.
    assert_fmt(
        "def index(req)\n  return render(\"home\")\nend\n",
        "def index(req)\n  return render(\"home\")\nend\n",
    );
    assert_idempotent("def index(req)\n  return render(\"home\")\nend\n");
}

#[test]
fn lambdas_keep_the_fn_keyword() {
    // Only *declarations* move to `def` — a lambda expression stays `fn`.
    assert_fmt(
        "let add = fn(a, b) { a + b }\n",
        "let add = fn(a, b) { a + b }\n",
    );
}

#[test]
fn interface_members_stay_fn() {
    // The parser only accepts `fn` inside an interface body.
    assert_round_trip("interface Named { fn name() -> String }\n");
}

#[test]
fn class_body_dsl_keeps_do_end() {
    // `do … end` and `&{ … }` parse to the same Lambda, so the formatter picks
    // one. It must be `do … end` — rewriting a declarative DSL into `&{ … }`
    // was unreadable and fought the documented idiom.
    let src = "class Cart < Model\n  state_machine :state do\n    event :pay do\n      transition from: A, to: B\n    end\n  end\nend\n";
    let out = format_source(src).expect("format failed");
    assert!(
        out.contains("state_machine(:state) do") && out.contains("event(:pay) do"),
        "declarative DSL blocks must keep `do … end`:\n{}",
        out
    );
    assert!(
        !out.contains("&{"),
        "no `&{{ … }}` rewrite expected:\n{}",
        out
    );
    assert_round_trip(src);
    assert_idempotent(src);
}

#[test]
fn method_call_block_needs_no_empty_parens() {
    let src = "def run\n  items.each do |item|\n    print(item)\n  end\nend\n";
    let out = format_source(src).expect("format failed");
    assert!(
        out.contains("items.each do |item|"),
        "expected bare `items.each do |item|`, got:\n{}",
        out
    );
    assert_round_trip(src);
    assert_idempotent(src);
}

#[test]
fn form_builder_block_round_trips() {
    let src = "def show(post)\n  form_with(post) do |f|\n    f.submit(\"Save\")\n  end\nend\n";
    let out = format_source(src).expect("format failed");
    assert!(
        out.contains("form_with(post) do |f|"),
        "expected `form_with(post) do |f|`, got:\n{}",
        out
    );
    assert_round_trip(src);
    assert_idempotent(src);
}

#[test]
fn block_in_a_condition_falls_back_to_brace_form() {
    // A `do` in an `if` condition would be swallowed by the surrounding
    // grammar, so a block argument there keeps the unambiguous `&{ … }` form
    // and the output still has to reparse.
    let src = "def run(items)\n  if items.any(&{ |x| x.active })\n    print(1)\n  end\nend\n";
    let out = format_source(src).expect("format failed");
    assert!(
        !out.lines().any(|l| l.contains("if ") && l.contains(" do")),
        "a condition must not sprout a trailing `do`:\n{}",
        out
    );
    assert_round_trip(src);
    assert_idempotent(src);
}

#[test]
fn symbol_to_proc_shorthand_round_trips() {
    // `&:total` lowers to `|__it| __it.total`; printing the lowered form back
    // out lost the shorthand the source was written with.
    assert_fmt(
        "def f(lines)\n  return int(lines.map(&:total).sum)\nend\n",
        "def f(lines)\n  return int(lines.map(&:total).sum)\nend\n",
    );
    assert_round_trip("def f(lines)\n  return lines.map(&:total)\nend\n");
    assert_idempotent("def f(lines)\n  return int(lines.map(&:total).sum)\nend\n");
}

#[test]
fn symbol_to_proc_with_predicate_suffix_round_trips() {
    assert_fmt(
        "def f(rows)\n  return rows.filter(&:active?)\nend\n",
        "def f(rows)\n  return rows.filter(&:active?)\nend\n",
    );
    assert_idempotent("def f(rows)\n  return rows.filter(&:active?)\nend\n");
}
