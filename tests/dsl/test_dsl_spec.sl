# The test DSL, specified from the inside: hook order across nested describes,
# skip/pending, assert_raises, the optional assertion message, and expect().
# Some tests read what the previous one left behind, so they rely on tests
# running in the order they are declared.

hook_log = []
teardown_log = []
before_all_runs = 0
alias_runs = []

# The message an assertion fails with, without its " at line:col" suffix, or
# nil when it passes. assert_raises lets a failed assertion through on purpose
# (a failing spec inside its block must still fail the test), so the failure
# path of an assertion is read with try/catch.
def failure_of(check)
  try
    check()
  catch error
    return Regex.replace(" at \\d+:\\d+$", "#{error}", "")
  end

  nil
end

describe("before_each and before_all") do
  before_all() do
    before_all_runs += 1
  end

  before_each() do
    hook_log = ["outer before_each"]
  end

  test("before_each runs before the test") do
    assert_eq(hook_log, ["outer before_each"])
    hook_log.push("left behind by the first test")
  end

  test("before_each runs again before every test, on a fresh value") do
    assert_eq(hook_log, ["outer before_each"])
  end

  test("before_all ran once for the whole describe") do
    assert_eq(before_all_runs, 1)
  end

  describe("a nested describe") do
    before_each() do
      hook_log.push("inner before_each")
    end

    test("inherits the enclosing before_each, which runs first") do
      assert_eq(hook_log, ["outer before_each", "inner before_each"])
    end

    context("two levels down") do
      before_each() do
        hook_log.push("innermost before_each")
      end

      test("runs every enclosing before_each, outermost first") do
        expected = ["outer before_each", "inner before_each", "innermost before_each"]
        assert_eq(hook_log, expected)
      end
    end
  end

  test("does not reach the nested describe's before_each") do
    assert_eq(hook_log, ["outer before_each"])
  end
end

describe("after_each") do
  before_all() do
    teardown_log = []
  end

  after_each() do
    teardown_log.push("outer after_each")
  end

  describe("in a nested describe") do
    after_each() do
      teardown_log.push("inner after_each")
    end

    test("has not run before the first test") do
      assert_eq(teardown_log, [])
    end

    test("ran after the previous test, innermost first") do
      assert_eq(teardown_log, ["inner after_each", "outer after_each"])
    end

    test("still runs after a skipped test") do
      teardown_log = []
      skip("stops here; teardown runs anyway")
    end

    test("so the skipped test's teardown is on the log") do
      assert_eq(teardown_log, ["inner after_each", "outer after_each"])
    end
  end
end

describe("skip and pending") do
  test("skip(reason) stops a test before a failing assertion") do
    skip("not on this platform")
    assert(false)
  end

  test("pending(reason) stops a test too") do
    pending("not written yet")
    assert_eq(1, 2)
  end

  test("pending without a reason") do
    pending()
    assert(false)
  end

  describe("from a before_each") do
    before_each() do
      skip("the whole group is waiting on a service")
    end

    test("marks the test not run instead of running it") do
      assert(false)
    end
  end

  describe("pending from a before_each") do
    before_each() do
      pending("the whole group is unwritten")
    end

    test("marks the test not run too") do
      assert_eq(1, 2)
    end
  end

  test("a test after them runs normally") do
    assert_eq(1 + 1, 2)
  end
end

describe("assert_raises") do
  test("with a block and no fragment returns the message") do
    message = assert_raises() do
      throw "boom"
    end
    assert_eq(message, "boom")
  end

  test("with a fragment passes when the message contains it") do
    message = assert_raises("oo") do
      throw "boom"
    end
    assert_eq(message, "boom")
  end

  test("takes a lambda instead of a block") do
    assert_match(assert_raises(fn() { JSON.parse("{") }), "JSON")
  end

  test("catches a runtime error and returns its message") do
    message = assert_raises("Division by zero") do
      1 / 0
    end
    assert_match(message, "^Division by zero")
  end

  test("fails when the block raises nothing") do
    message = failure_of(fn() { assert_raises(fn() { 1 }) })
    assert_eq(message, "expected the block to raise, but it raised nothing")
  end

  test("fails when nothing is raised, naming the fragment") do
    message = failure_of(fn() { assert_raises("boom", fn() { 1 }) })
    assert_eq(message, "expected the block to raise an error containing \"boom\", but it raised nothing")
  end

  test("fails when the message does not contain the fragment") do
    message = failure_of(fn() { assert_raises("zz", fn() { 1 / 0 }) })
    assert_match(message, "^expected an error containing \"zz\", got \"Division by zero")
  end

  test("lets a failed assertion inside the block through") do
    message = failure_of(fn() { assert_raises(fn() { assert_eq(1, 2) }) })
    assert_eq(message, "expected 2, got 1")
  end
end

describe("assertion messages") do
  test("a passing assertion returns 1") do
    assert_eq(assert_eq(1, 1), 1)
  end

  test("a failure says what was expected and what came instead") do
    assert_eq(failure_of(fn() { assert_eq(1, 2) }), "expected 2, got 1")
    assert_eq(failure_of(fn() { assert_ne(1, 1) }), "expected a value other than 1")
    assert_eq(failure_of(fn() { assert_null(1) }), "expected nil, got 1")
    assert_eq(failure_of(fn() { assert_not_null(nil) }), "expected a value, got nil")
    assert_eq(failure_of(fn() { assert_match("abc", "^b") }), "expected \"abc\" to match /^b/")
    assert_eq(failure_of(fn() { assert_contains([1], 2) }), "expected [1] to contain 2")
    assert_eq(failure_of(fn() { assert_hash_has_key({"a": 1}, "b") }), "expected {a => 1} to have key \"b\"")
  end

  test("the optional message is prefixed to the failure") do
    assert_eq(failure_of(fn() { assert_eq(1, 2, "totals differ") }), "totals differ: expected 2, got 1")
    assert_eq(failure_of(fn() { assert(false, "nope") }), "nope: assertion failed")
  end

  test("the optional message must be a String") do
    assert_eq(failure_of(fn() { assert(false, 3) }), "assert: the message must be a String, got int")
  end

  test("a missing argument is refused") do
    assert_eq(failure_of(fn() { assert() }), "assert expects 1 argument and an optional message, got 0")
  end

  test("Int and Float compare by value") do
    assert_eq(1, 1.0)
    assert_gt(3, 2.5)
    assert_lt(2.5, 3)
    assert_eq(failure_of(fn() { assert_lt(3, 2.5) }), "expected 3 to be less than 2.5")
  end
end

describe("expect") do
  test("holds the value under test") do
    assert_eq(expect(42).actual, 42)
  end

  test("to_have_key checks a hash's keys") do
    expect({"name": "Ada"}).to_have_key("name")
    assert_eq(failure_of(fn() { expect({"a": 1}).to_have_key("b") }), "Expected {a => 1} to have key \"b\"")
  end

  test("comparisons mix Int and Float") do
    expect(10).to_be_greater_than(9.5)
    expect(9.5).to_be_less_than(10)
    expect(10).to_be_greater_than_or_equal(10.0)
    expect(1).to_equal(1.0)
  end
end

describe("aliases") do
  it("it() registers a test that runs") do
    alias_runs.push("it")
    assert_eq(alias_runs, ["it"])
  end

  specify("specify() registers one too, run in declaration order") do
    alias_runs.push("specify")
    assert_eq(alias_runs, ["it", "specify"])
  end

  context("context() groups tests like describe()") do
    test("and runs them after the enclosing tests") do
      alias_runs.push("context")
      assert_eq(alias_runs.join(", "), "it, specify, context")
    end
  end
end
