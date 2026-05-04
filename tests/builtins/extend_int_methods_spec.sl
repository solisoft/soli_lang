// ============================================================================
// Test: extending Int with user-defined methods via Int.define_method
// ============================================================================

describe("Extending Int", fn() {
    test("define_method adds a method callable on Int values", fn() {
        Int.define_method("triple", fn() { this * 3 })
        assert_eq(4.triple(), 12)
        assert_eq((-2).triple(), -6)
    });

    test("define_method with args", fn() {
        Int.define_method("plus_n", fn(n) { this + n })
        assert_eq(10.plus_n(5), 15)
        assert_eq(0.plus_n(7), 7)
    });

    test("zero-arg user method auto-invokes without parens", fn() {
        Int.define_method("squared", fn() { this * this })
        assert_eq(5.squared, 25)
        assert_eq(5.squared(), 25)
    });

    test("user method shadows is fine alongside builtin", fn() {
        // Builtin .abs still works.
        assert_eq((-7).abs, 7)
    });
});
