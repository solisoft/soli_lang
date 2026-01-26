// ============================================================================
// Variables Test Suite
// ============================================================================

describe("Variable Declarations", fn() {
    test("let declares a variable", fn() {
        let x = 10;
        assert_eq(x, 10);
    });

    test("let with type annotation", fn() {
        let x: Int = 42;
        assert_eq(x, 42);
        assert_eq(type(x), "int");
    });

    test("let can be reassigned", fn() {
        let x = 1;
        x = 2;
        assert_eq(x, 2);
    });

    test("multiple variable declarations", fn() {
        let a = 1;
        let b = 2;
        let c = 3;
        assert_eq(a + b + c, 6);
    });
});
