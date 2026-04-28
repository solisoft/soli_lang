// ============================================================================
// Compound Assignment & Postfix Operators Test Suite
// ============================================================================

describe("Compound Assignment Operators", fn() {
    test("plus-equals (+=)", fn() {
        let a = 1
        a += 1
        assert_eq(a, 2);

        a += 10
        assert_eq(a, 12);
    });

    test("minus-equals (-=)", fn() {
        let a = 10
        a -= 3
        assert_eq(a, 7);

        a -= 7
        assert_eq(a, 0);
    });

    test("star-equals (*=)", fn() {
        let a = 3
        a *= 4
        assert_eq(a, 12);

        a *= 0
        assert_eq(a, 0);
    });

    test("slash-equals (/=)", fn() {
        let a = 10
        a /= 2
        assert_eq(a, 5);

        a /= 5
        assert_eq(a, 1);
    });

    test("percent-equals (%=)", fn() {
        let a = 10
        a %= 3
        assert_eq(a, 1);

        let b = 15
        b %= 5
        assert_eq(b, 0);
    });

    test("compound assignment with floats", fn() {
        let x = 1.5
        x += 0.5
        assert_eq(x, 2.0);

        x *= 3.0
        assert_eq(x, 6.0);

        x -= 1.0
        assert_eq(x, 5.0);

        x /= 2.0
        assert_eq(x, 2.5);
    });

    test("compound assignment with strings", fn() {
        let s = "hello"
        s += " world"
        assert_eq(s, "hello world");
    });

    test("chained compound assignments", fn() {
        let a = 1
        a += 1
        a *= 3
        a -= 1
        a /= 2
        assert_eq(a, 2);
    });
});

describe("Logical Compound Assignment Operators", fn() {
    test("or-equals (||=) assigns when target is null", fn() {
        let a = null
        a ||= 42
        assert_eq(a, 42);
    });

    test("or-equals (||=) assigns when target is false", fn() {
        let a = false
        a ||= "fallback"
        assert_eq(a, "fallback");
    });

    test("or-equals (||=) keeps existing truthy value", fn() {
        let a = 7
        a ||= 99
        assert_eq(a, 7);

        let s = "hello"
        s ||= "ignored"
        assert_eq(s, "hello");
    });

    test("or-equals (||=) on hash member", fn() {
        let h = {"name": "Alice"}
        h["age"] ||= 30
        assert_eq(h["age"], 30);
        h["name"] ||= "Bob"
        assert_eq(h["name"], "Alice");
    });

    test("and-equals (&&=) assigns when target is truthy", fn() {
        let a = 1
        a &&= 99
        assert_eq(a, 99);
    });

    test("and-equals (&&=) keeps falsy value", fn() {
        let a = null
        a &&= 99
        assert_eq(a, null);

        let b = false
        b &&= 99
        assert_eq(b, false);
    });

    test("nullish-equals (??=) only assigns when null", fn() {
        let a = null
        a ??= 42
        assert_eq(a, 42);

        let b = false
        b ??= 99
        assert_eq(b, false);

        let c = 0
        c ??= 99
        assert_eq(c, 0);

        let d = "x"
        d ??= "y"
        assert_eq(d, "x");
    });

    test("logical compound on member access", fn() {
        let user = {"name": null}
        user.name ||= "default"
        assert_eq(user.name, "default");
    });
});

describe("Postfix Increment Operator (++)", fn() {
    test("basic increment", fn() {
        let a = 1
        a++
        assert_eq(a, 2);
    });

    test("returns old value", fn() {
        let a = 5
        let b = a++
        assert_eq(b, 5);
        assert_eq(a, 6);
    });

    test("multiple increments", fn() {
        let a = 0
        a++
        a++
        a++
        assert_eq(a, 3);
    });
});

describe("Postfix Decrement Operator (--)", fn() {
    test("basic decrement", fn() {
        let a = 5
        a--
        assert_eq(a, 4);
    });

    test("returns old value", fn() {
        let a = 10
        let b = a--
        assert_eq(b, 10);
        assert_eq(a, 9);
    });

    test("decrement to negative", fn() {
        let a = 0
        a--
        assert_eq(a, -1);
    });
});

describe("Combined Operators", fn() {
    test("increment and compound assignment together", fn() {
        let a = 1
        a++
        a += 10
        assert_eq(a, 12);

        a--
        a -= 5
        assert_eq(a, 6);
    });

    test("in loops", fn() {
        let sum = 0
        let i = 0
        while (i < 5) {
            sum += i
            i++
        }
        assert_eq(sum, 10);
        assert_eq(i, 5);
    });
});
