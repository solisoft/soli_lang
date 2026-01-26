// ============================================================================
// Literals Test Suite
// ============================================================================

describe("Literals", fn() {
    test("integer literals", fn() {
        assert_eq(42, 42);
        assert_eq(-10, -10);
        assert_eq(0, 0);
    });

    test("float literals", fn() {
        assert_eq(3.14, 3.14);
        assert_eq(-2.5, -2.5);
        assert_eq(0.0, 0.0);
    });

    test("string literals with double quotes", fn() {
        assert_eq("hello", "hello");
        assert_eq("", "");
    });

    test("boolean literals", fn() {
        assert_eq(true, true);
        assert_eq(false, false);
    });

    test("null literal", fn() {
        assert_null(null);
    });

    test("array literals", fn() {
        let arr = [1, 2, 3];
        assert_eq(len(arr), 3);
        assert_eq(arr[0], 1);
        assert_eq(arr[1], 2);
        assert_eq(arr[2], 3);
    });

    test("empty array literal", fn() {
        let arr = [];
        assert_eq(len(arr), 0);
    });

    test("nested array literals", fn() {
        let arr = [[1, 2], [3, 4]];
        assert_eq(arr[0][0], 1);
        assert_eq(arr[1][1], 4);
    });

    test("hash literals with fat arrow", fn() {
        let h = {"a" => 1, "b" => 2};
        assert_eq(h["a"], 1);
        assert_eq(h["b"], 2);
    });

    test("hash literals with colon", fn() {
        let h = {a: 1, b: 2};
        assert_eq(h["a"], 1);
        assert_eq(h["b"], 2);
    });

    test("empty hash literal", fn() {
        let h = {};
        assert_eq(len(h), 0);
    });
});
