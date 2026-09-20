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

# These three forms all ran correctly and were rejected by `soli check`, which
# is as costly as missing a real error: the checker refused code the runtime
# was happy to execute, and in one case the form it refused is the one the
# project's own conventions teach. Pinned here so a future change to the
# operator table cannot quietly take them away again.
describe("forms the type checker used to reject", fn() {
    test("|| answers an operand, not a boolean", fn() {
        # `a || b` is `a` when `a` is truthy and `b` otherwise. The checker
        # answered Bool unconditionally, so the `let` passed and the *use*
        # failed with "expected String, found Bool".
        let host: String = getenv("SOLI_SPEC_UNSET_HOST") || "localhost";
        assert_eq(host, "localhost");

        let n: Int = 0 || 42;
        assert_eq(n, 42);

        let kept: Int = 7 || 42;
        assert_eq(kept, 7);
    });

    test("&& answers an operand too", fn() {
        let a: Int = 1 && 5;
        assert_eq(a, 5);
        assert_eq(false && 5, false);
    });

    test("|| still works as a condition", fn() {
        # Widening the result type must not break the ordinary use.
        let seen = false;
        if false || true { seen = true; }
        assert(seen);
    });

    test("a string repeats by an int, either way round", fn() {
        assert_eq("ab" * 3, "ababab");
        assert_eq(3 * "ab", "ababab");
        assert_eq("-" * 5, "-----");
    });

    test("an array does not repeat", fn() {
        # `[1, 2] * 2` raises at run time, so the checker is right to refuse it
        # and the rule above must not be widened to cover arrays.
        try {
            let bad = [1, 2] * 2;
            assert(false, "expected a runtime error");
        } catch _e {
            assert(true);
        }
    });
});

# The falsy set, pinned. Three documentation surfaces claimed for a long time
# that "only false and null are falsy; 0 and "" are truthy", which is the
# opposite of what both engines do — and one of those surfaces is the CLAUDE.md
# copied into every scaffolded application.
describe("truthiness", fn() {
    test("the whole falsy set", fn() {
        assert_eq(false || "f", "f");
        assert_eq(null  || "f", "f");
        assert_eq(0     || "f", "f");
        assert_eq(""    || "f", "f");
        assert_eq([]    || "f", "f");
        assert_eq({}    || "f", "f");
    });

    test("0.0 is truthy even though 0 is not", fn() {
        assert_eq(0.0 || "f", 0.0);
        assert_eq(0   || "f", "f");
    });

    test("everything else is truthy", fn() {
        assert_eq(1        || "f", 1);
        assert_eq("x"      || "f", "x");
        assert_eq([0]      || "f", [0]);
        assert_eq(true     || "f", true);
    });
});
