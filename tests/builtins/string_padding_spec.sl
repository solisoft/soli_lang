// ============================================================================
// String Padding Methods Test Suite
// ============================================================================

describe("String lpad/rpad", fn() {
    test("lpad adds spaces by default", fn() {
        assert_eq("hello".lpad(10), "     hello");
    });

    test("lpad does not truncate if string is longer", fn() {
        assert_eq("hello".lpad(3), "hello");
    });

    test("lpad with exact length returns original", fn() {
        assert_eq("hello".lpad(5), "hello");
    });

    test("rpad adds spaces by default", fn() {
        assert_eq("hello".rpad(10), "hello     ");
    });

    test("rpad does not truncate if string is longer", fn() {
        assert_eq("hello".rpad(3), "hello");
    });

    test("rpad with exact length returns original", fn() {
        assert_eq("hello".rpad(5), "hello");
    });

    test("lpad with empty string", fn() {
        assert_eq("".lpad(5), "     ");
    });

    test("rpad with empty string", fn() {
        assert_eq("".rpad(5), "     ");
    });
});
