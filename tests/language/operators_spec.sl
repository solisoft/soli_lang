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

describe("Postfix Increment/Decrement", fn() {
    test("postfix increment returns old value", fn() {
        let x = 5;
        let old = x++;
        assert_eq(old, 5);
        assert_eq(x, 6);
    });

    test("postfix decrement returns old value", fn() {
        let x = 5;
        let old = x--;
        assert_eq(old, 5);
        assert_eq(x, 4);
    });

    test("postfix increment on variable", fn() {
        let counter = 0;
        counter++;
        assert_eq(counter, 1);
        counter++;
        assert_eq(counter, 2);
    });

    test("postfix decrement on variable", fn() {
        let counter = 10;
        counter--;
        assert_eq(counter, 9);
        counter--;
        assert_eq(counter, 8);
    });

    test("postfix increment with assignment", fn() {
        let x = 5;
        let result = x++ + 10;
        assert_eq(x, 6);
        assert_eq(result, 15);
    });

    test("postfix decrement with assignment", fn() {
        let x = 5;
        let result = x-- + 10;
        assert_eq(x, 4);
        assert_eq(result, 15);
    });

    test("postfix increment on float", fn() {
        let x = 5.0;
        let old = x++;
        assert_eq(old, 5.0);
        assert_eq(x, 6.0);
    });

    test("multiple postfix operations", fn() {
        let x = 1;
        x++;
        x++;
        x--;
        assert_eq(x, 2);
    });
});

describe("Shovel operator (<<)", fn() {
    test("appends to an array", fn() {
        let a = [1, 2, 3];
        a << 4;
        assert_eq(a, [1, 2, 3, 4]);
    });

    test("returns the array for chaining", fn() {
        let a = [];
        let r = a << 1;
        assert_eq(r, [1]);
    });

    test("works with mixed types", fn() {
        let a = [1, "two"];
        a << 3.0;
        a << true;
        assert_eq(a.length, 4);
    });

    test("errors on non-array LHS", fn() {
        try {
            let n = 5;
            n << 1;
            assert(false, "expected error");
        } catch _e {
            assert(true);
        }
    });
});
