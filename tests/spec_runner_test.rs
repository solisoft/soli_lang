//! How `soli test` runs a spec file: hooks, hook failures and `assert_raises`.
//!
//! Each case runs a spec source through the same entry point the runner uses,
//! and judges the file's outcome. These live in their own test binary because
//! the runner's progress counters are process-global: unit tests in the library
//! would share them with `test_progress`'s own test.

/// Run a spec source; `Ok(assertions)` when every test passed, else the
/// runner's failure message.
fn run_spec(source: &str) -> Result<i64, String> {
    let (assertions, result) =
        solilang::run_with_path_and_coverage(source, None, false, None, None, &[]);
    result.map(|_| assertions).map_err(|e| e.to_string())
}

fn failure(source: &str) -> String {
    match run_spec(source) {
        Ok(assertions) => {
            panic!("expected the spec to fail; it passed with {assertions} assertions")
        }
        Err(message) => message,
    }
}

#[test]
fn a_nested_suite_runs_every_enclosing_before_each_outermost_first() {
    let source = r#"
log = []

describe("outer") do
  before_each() do
    log.push("outer")
  end

  describe("inner") do
    before_each() do
      log.push("inner")
    end

    test("sees both hooks, in order") do
      assert_eq(log, ["outer", "inner"])
    end
  end
end
"#;
    assert_eq!(run_spec(source), Ok(1));
}

#[test]
fn after_each_hooks_run_innermost_first() {
    let source = r#"
log = []

describe("outer") do
  after_each() do
    log.push("outer")
  end

  describe("inner") do
    after_each() do
      log.push("inner")
    end

    test("first") do
      assert(true)
    end

    test("second sees the first test's teardown") do
      assert_eq(log, ["inner", "outer"])
    end
  end
end
"#;
    assert_eq!(run_spec(source), Ok(2));
}

#[test]
fn a_failing_before_each_fails_the_test_and_skips_its_body() {
    let source = r#"
ran = false

describe("setup") do
  before_each() do
    throw "no fixture"
  end

  test("never reached") do
    ran = true
    assert(true)
  end
end
"#;
    let message = failure(source);
    assert!(message.contains("never reached: before_each:"), "{message}");
    assert!(message.contains("no fixture"), "{message}");
}

#[test]
fn skip_in_a_before_each_marks_the_test_not_run() {
    let source = r#"
describe("needs a service") do
  before_each() do
    skip("service is down")
  end

  test("would fail") do
    assert(false)
  end
end
"#;
    assert_eq!(run_spec(source), Ok(0));
}

#[test]
fn a_failing_after_each_fails_the_test() {
    let source = r#"
describe("teardown") do
  after_each() do
    throw "cleanup broke"
  end

  test("passes on its own") do
    assert(true)
  end
end
"#;
    let message = failure(source);
    assert!(
        message.contains("passes on its own: after_each:"),
        "{message}"
    );
}

#[test]
fn a_failing_before_all_fails_the_suite_without_running_it() {
    let source = r#"
describe("suite") do
  before_all() do
    throw "no server"
  end

  test("one") do
    assert(true)
  end

  test("two") do
    assert(true)
  end
end
"#;
    let message = failure(source);
    assert!(
        message.contains("suite (before_all, 2 test(s) not run)"),
        "{message}"
    );
}

#[test]
fn assert_eq_names_both_values() {
    let source = r#"
describe("values") do
  test("compares") do
    assert_eq([1, 2], [1, 3])
  end
end
"#;
    let message = failure(source);
    assert!(message.contains("expected [1, 3], got [1, 2]"), "{message}");
}

#[test]
fn assert_eq_prefixes_an_optional_message() {
    let source = r#"
describe("values") do
  test("compares") do
    assert_eq(1, 2, "the count")
  end
end
"#;
    let message = failure(source);
    assert!(
        message.contains("the count: expected 2, got 1"),
        "{message}"
    );
}

#[test]
fn assert_raises_returns_the_message_of_the_error_it_caught() {
    let source = r#"
describe("errors") do
  test("block form") do
    message = assert_raises("out of stock") do
      throw "item 7 is out of stock"
    end
    assert_eq(message, "item 7 is out of stock")
  end

  test("function form") do
    assert_raises(fn() { 1 / 0 })
  end
end
"#;
    assert_eq!(run_spec(source), Ok(3));
}

#[test]
fn assert_raises_fails_when_the_block_raises_nothing() {
    let source = r#"
describe("errors") do
  test("quiet block") do
    assert_raises("boom") do
      1 + 1
    end
  end
end
"#;
    let message = failure(source);
    assert!(
        message.contains("expected the block to raise an error containing \"boom\""),
        "{message}"
    );
}

#[test]
fn assert_raises_fails_on_a_different_error() {
    let source = r#"
describe("errors") do
  test("wrong error") do
    assert_raises("not found") do
      throw "permission denied"
    end
  end
end
"#;
    let message = failure(source);
    assert!(
        message.contains("expected an error containing \"not found\", got \"permission denied\""),
        "{message}"
    );
}

#[test]
fn assert_raises_does_not_swallow_a_failed_assertion_inside_its_block() {
    // The block raises — but because an assertion failed, not because the
    // code under test refused something. Passing here would hide a real bug.
    let source = r#"
describe("errors") do
  test("assertion inside") do
    assert_raises() do
      assert_eq(1, 2)
    end
  end
end
"#;
    let message = failure(source);
    assert!(message.contains("expected 2, got 1"), "{message}");
}

#[test]
fn expect_to_have_key_checks_hash_keys() {
    let passing = r#"
describe("hashes") do
  test("has the key") do
    expect({"id": 1}).to_have_key("id")
  end
end
"#;
    assert_eq!(run_spec(passing), Ok(1));

    let failing = r#"
describe("hashes") do
  test("lacks the key") do
    expect({"id": 1}).to_have_key("name")
  end
end
"#;
    let message = failure(failing);
    assert!(message.contains("to have key \"name\""), "{message}");
}

#[test]
fn comparison_matchers_accept_an_int_and_a_float() {
    let source = r#"
describe("numbers") do
  test("mixed") do
    expect(3).to_be_less_than(3.5)
    expect(2.5).to_be_greater_than_or_equal(2)
    assert_gt(4, 3.9)
    assert_lt(0.5, 1)
  end
end
"#;
    assert_eq!(run_spec(source), Ok(4));
}
