// ============================================================================
// Operators Test Suite
// ============================================================================

describe("Arithmetic Operators", fn() {
    test("addition", fn() {
        assert_eq(2 + 3, 5);
        assert_eq(-1 + 1, 0);
        assert_eq(1.5 + 2.5, 4.0);
    });

    test("subtraction", fn() {
        assert_eq(5 - 3, 2);
        assert_eq(0 - 5, -5);
        assert_eq(3.5 - 1.5, 2.0);
    });

    test("multiplication", fn() {
        assert_eq(3 * 4, 12);
        assert_eq(-2 * 3, -6);
        assert_eq(2.5 * 2, 5.0);
    });

    test("division", fn() {
        assert_eq(10 / 2, 5);
        assert_eq(7 / 2, 3);
        assert_eq(7.0 / 2.0, 3.5);
    });

    test("modulo", fn() {
        assert_eq(10 % 3, 1);
        assert_eq(15 % 5, 0);
        assert_eq(7 % 2, 1);
    });

    test("unary negation", fn() {
        let x = 5;
        assert_eq(-x, -5);
        assert_eq(-(-x), 5);
    });

    test("operator precedence", fn() {
        assert_eq(2 + 3 * 4, 14);
        assert_eq((2 + 3) * 4, 20);
        assert_eq(10 - 4 / 2, 8);
    });

    test("string concatenation with +", fn() {
        assert_eq("hello" + " " + "world", "hello world");
    });
});

describe("Comparison Operators", fn() {
    test("equality", fn() {
        assert(1 == 1);
        assert("a" == "a");
        assert_not(1 == 2);
    });

    test("inequality", fn() {
        assert(1 != 2);
        assert("a" != "b");
        assert_not(1 != 1);
    });

    test("less than", fn() {
        assert(1 < 2);
        assert_not(2 < 1);
        assert_not(1 < 1);
    });

    test("less than or equal", fn() {
        assert(1 <= 2);
        assert(1 <= 1);
        assert_not(2 <= 1);
    });

    test("greater than", fn() {
        assert(2 > 1);
        assert_not(1 > 2);
        assert_not(1 > 1);
    });

    test("greater than or equal", fn() {
        assert(2 >= 1);
        assert(1 >= 1);
        assert_not(1 >= 2);
    });
});

describe("Logical Operators", fn() {
    test("logical AND", fn() {
        assert(true && true);
        assert_not(true && false);
        assert_not(false && true);
        assert_not(false && false);
    });

    test("logical OR", fn() {
        assert(true || true);
        assert(true || false);
        assert(false || true);
        assert_not(false || false);
    });

    test("logical NOT", fn() {
        assert(!false);
        assert_not(!true);
    });

    test("short-circuit AND", fn() {
        let called = false;
        let result = false && (called = true);
        assert_not(called);
    });

    test("short-circuit OR", fn() {
        let called = false;
        let result = true || (called = true);
        assert_not(called);
    });

    test("combined logical operators", fn() {
        assert((true && true) || false);
        assert(!(false && true));
        assert((1 < 2) && (3 > 2));
    });
});

describe("Ternary Operator", fn() {
    test("ternary returns true branch", fn() {
        let result = true ? "yes" : "no";
        assert_eq(result, "yes");
    });

    test("ternary returns false branch", fn() {
        let result = false ? "yes" : "no";
        assert_eq(result, "no");
    });

    test("ternary with expressions", fn() {
        let x = 10;
        let result = x > 5 ? "big" : "small";
        assert_eq(result, "big");
    });

    test("nested ternary", fn() {
        let x = 5;
        let result = x < 0 ? "negative" : x == 0 ? "zero" : "positive";
        assert_eq(result, "positive");
    });
});
