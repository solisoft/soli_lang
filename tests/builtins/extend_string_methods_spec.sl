// ============================================================================
// Test: extending String with user-defined methods via String.define_method
// ============================================================================

describe("Extending String", fn() {
    test("define_method adds new method callable on string values", fn() {
        String.define_method("shout", fn() { this + "!!!" })
        assert_eq("hi".shout(), "hi!!!")
        assert_eq("ok".shout(), "ok!!!")
    });

    test("user method with args on String", fn() {
        String.define_method("repeat_n", fn(n) {
            let result = ""
            let i = 0
            while i < n
                result = result + this
                i = i + 1
            end
            result
        })
        assert_eq("ab".repeat_n(3), "ababab")
    });

    test("builtin String methods still work alongside user methods", fn() {
        assert_eq("HELLO".downcase, "hello")
        assert_eq("hello".length, 5)
    });

    test("user method shadows builtin on collision", fn() {
        // Override `to_s` to return uppercase. Subsequent .to_s calls use ours.
        String.define_method("to_s", fn() { this.upcase })
        assert_eq("hi".to_s, "HI")
    });
});
