// ============================================================================
// Test: alias_method on primitive types
// ============================================================================

describe("alias_method on primitives", fn() {
    test("alias an existing user method on Int", fn() {
        Int.define_method("squared", fn() { this * this })
        Int.alias_method("squared_alias", "squared")
        assert_eq(6.squared_alias(), 36)
    });

    test("alias on String", fn() {
        String.define_method("yell", fn() { this + "!" })
        String.alias_method("yell2", "yell")
        assert_eq("hi".yell2(), "hi!")
    });
});
