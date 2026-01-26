// ============================================================================
// Type Conversion Test Suite
// ============================================================================

describe("Type Conversion", fn() {
    test("str() converts values to string", fn() {
        assert_eq(str(42), "42");
        assert_eq(str(3.14), "3.14");
        assert_eq(str(true), "true");
        assert_eq(str(false), "false");
        assert_eq(str(null), "null");
    });

    test("int() converts values to integer", fn() {
        assert_eq(int("42"), 42);
        assert_eq(int(3.14), 3);
        assert_eq(int(3.99), 3);
        assert_eq(int(-3.14), -3);
    });

    test("float() converts values to float", fn() {
        assert_eq(float("3.14"), 3.14);
        assert_eq(float(42), 42.0);
    });

    test("type() returns type name", fn() {
        assert_eq(type(42), "int");
        assert_eq(type(3.14), "float");
        assert_eq(type("hello"), "string");
        assert_eq(type(true), "bool");
        assert_eq(type(null), "null");
        assert_eq(type([1, 2, 3]), "array");
        assert_eq(type(hash()), "hash");
    });
});
