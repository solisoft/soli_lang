# String.define_method adds a method to every string value.
#
# Primitive extensions live for the whole worker process, not just this file:
# every name here starts with `spec_` so it cannot collide with a method another
# spec relies on, and the one test that shadows a builtin restores it.

describe("String.define_method") do
  test("adds a method callable on any string") do
    assert_null(String.define_method("spec_shout", fn() { this + "!!!" }))
    assert_eq("hi".spec_shout, "hi!!!")
    assert_eq("ok".spec_shout(), "ok!!!")
  end

  test("a method with arguments") do
    String.define_method("spec_repeat_n", fn(count) {
      (0..count).map { |_i| this }.join("")
    })
    assert_eq("ab".spec_repeat_n(3), "ababab")
    assert_eq("ab".spec_repeat_n(0), "")
  end

  test("accepts a symbol as the name") do
    String.define_method(:spec_symbol_named, fn() { this.length })
    assert_eq("abc".spec_symbol_named, 3)
  end

  test("builtin methods still work alongside") do
    String.define_method("spec_noop", fn() { this })
    assert_eq("HELLO".downcase, "hello")
    assert_eq("hello".length, 5)
  end

  test("raises unless given a function") do
    assert_raises("define_method expects a function as second argument") do
      String.define_method("spec_bad", [5][0])
    end
  end

  test("raises unless given a name") do
    assert_raises("define_method expects method name as first argument") do
      String.define_method([5][0], fn() { 1 })
    end
  end
end

describe("shadowing a builtin String method") do
  # String#to_s returns the string itself; put that back so the override
  # cannot leak into the specs that run after this one in the same worker.
  after_each() do
    String.define_method("to_s", fn() { this })
  end

  test("the user method wins") do
    String.define_method("to_s", fn() { this.upcase })
    assert_eq("hi".to_s, "HI")
  end

  test("interpolation does not call it") do
    String.define_method("to_s", fn() { this.upcase })
    assert_eq("#{"hi"}", "hi")
  end

  test("the restored to_s returns the string itself") do
    assert_eq("hi".to_s, "hi")
  end
end
