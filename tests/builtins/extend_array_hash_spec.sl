// ============================================================================
// Test: extending Array and Hash with user-defined methods
// ============================================================================

describe("Extending Array", fn() {
    test("define_method on Array", fn() {
        Array.define_method("second", fn() { this[1] })
        assert_eq([10, 20, 30].second(), 20)
    });

    test("user method coexists with builtin Array methods", fn() {
        let a = [1, 2, 3]
        assert_eq(a.length, 3)
        assert_eq(a.map(fn(x) { x * 2 }), [2, 4, 6])
    });
});

describe("Extending Hash", fn() {
    test("define_method on Hash", fn() {
        Hash.define_method("size_label", fn() { "n=" + str(this.length) })
        assert_eq({"a": 1, "b": 2}.size_label(), "n=2")
    });

    test("user method on Hash wins over key fallback", fn() {
        // A hash with key "marker" — calling `.marker` would normally return
        // the value; with a user method registered, the method wins.
        Hash.define_method("marker", fn() { "method-wins" })
        let h = {"marker": "key-value"}
        assert_eq(h.marker(), "method-wins")
    });
});
