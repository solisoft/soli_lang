# alias_method on primitive classes copies a user-defined method under a new
# name. Extensions outlive this file, so every name starts with `spec_`.

describe("alias_method on primitives") do
  test("aliases a user method on Int") do
    Int.define_method("spec_cubed", fn() { this * this * this })
    assert_null(Int.alias_method("spec_cubed_alias", "spec_cubed"))
    assert_eq(2.spec_cubed_alias, 8)
  end

  test("aliases a user method on String") do
    String.define_method("spec_yell", fn() { this + "!" })
    String.alias_method("spec_yell2", "spec_yell")
    assert_eq("hi".spec_yell2, "hi!")
  end

  test("the alias keeps the original body when the method is redefined") do
    String.define_method("spec_greet", fn() { "hello #{this}" })
    String.alias_method("spec_greet_v1", "spec_greet")
    String.define_method("spec_greet", fn() { "hi #{this}" })
    assert_eq("bob".spec_greet, "hi bob")
    assert_eq("bob".spec_greet_v1, "hello bob")
  end

  test("raises when the method does not exist") do
    assert_raises("alias_method: method 'spec_nope' not found") do
      String.alias_method("spec_alias", "spec_nope")
    end
  end
end
