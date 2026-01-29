// ============================================================================
// Nullish Coalescing Operator Test Suite
// ============================================================================

describe("Nullish Coalescing Operator ??", fn() {
    test("returns right operand when left is null", fn() {
        let result = null ?? "default";
        assert_eq(result, "default");
    });

    test("returns left operand when left is not null", fn() {
        let result = "value" ?? "default";
        assert_eq(result, "value");
    });

    test("returns right operand when left is undefined (null equivalent)", fn() {
        let result = null ?? 42;
        assert_eq(result, 42);
    });

    test("works with numbers", fn() {
        assert_eq(0 ?? 100, 0);
        assert_eq(null ?? 100, 100);
    });

    test("works with booleans", fn() {
        assert_eq(false ?? true, false);
        assert_eq(null ?? true, true);
    });

    test("works with empty string", fn() {
        assert_eq("" ?? "default", "");
        assert_eq(null ?? "default", "default");
    });

    test("works with empty array", fn() {
        assert_eq([] ?? [1, 2, 3], []);
        assert_eq(null ?? [1, 2, 3], [1, 2, 3]);
    });

    test("works with empty hash", fn() {
        assert_eq({} ?? {"key": "value"}, {});
        assert_eq(null ?? {"key": "value"}, {"key": "value"});
    });

    test("chained nullish coalescing", fn() {
        let result = null ?? null ?? "final";
        assert_eq(result, "final");
    });

    test("first non-null value is returned", fn() {
        let result = null ?? null ?? "third" ?? "fourth";
        assert_eq(result, "third");
    });

    test("with variables", fn() {
        let a = null;
        let b = "found";
        assert_eq(a ?? b, "found");

        let c = "value";
        assert_eq(c ?? b, "value");
    });

    test("combined with other operators", fn() {
        let result = null ?? 5 + 3;
        assert_eq(result, 8);

        let x = null ?? (true && false);
        assert_eq(x, false);
    });

    test("in function parameters", fn() {
        fn greet(name) {
            return "Hello, " + (name ?? "Guest") + "!";
        }

        assert_eq(greet(null), "Hello, Guest!");
        assert_eq(greet("Alice"), "Hello, Alice!");
    });

    test("with method calls on default value", fn() {
        let result = null ?? "default";
        assert_eq(result.uppercase(), "DEFAULT");
    });

    test("precedence with logical AND", fn() {
        let result = true && (null ?? "fallback");
        assert_eq(result, "fallback");

        let result2 = false && (null ?? "fallback");
        assert_eq(result2, false);
    });

    test("precedence with logical OR", fn() {
        let result = false || (null ?? "fallback");
        assert_eq(result, "fallback");

        let result2 = true || (null ?? "fallback");
        assert_eq(result2, true);
    });

    test("with pipeline operator", fn() {
        let result = (null ?? [1, 2, 3]).length();
        assert_eq(result, 3);

        let result2 = (["a", "b"] ?? [1, 2]).first();
        assert_eq(result2, "a");
    });
});

describe("Nullish Coalescing with Not Keyword", fn() {
    test("not null coalescing result", fn() {
        assert_eq(not (null ?? "default"), false);
        assert_eq(not ("value" ?? "default"), false);
    });

    test("not on nullish result", fn() {
        let result = null ?? null;
        assert_eq(not result, true);
    });
});

describe("Edge Cases", fn() {
    test("nullish coalescing with functions returning null", fn() {
        fn get_value(flag) {
            if (flag) {
                return "found";
            }
            return null;
        }

        assert_eq(get_value(false) ?? "default", "default");
        assert_eq(get_value(true) ?? "default", "found");
    });

    test("nested nullish coalescing", fn() {
        let config = null;
        let result = (config ?? { db: null }).db ?? "sqlite";
        assert_eq(result, "sqlite");

        let config2 = { db: "postgresql" };
        let result2 = (config2 ?? { db: null }).db ?? "sqlite";
        assert_eq(result2, "postgresql");
    });

    test("in array literals", fn() {
        let arr = [1, null ?? 2, 3];
        assert_eq(arr, [1, 2, 3]);

        let arr2 = [1, "a" ?? 2, 3];
        assert_eq(arr2, [1, "a", 3]);
    });

    test("in hash literals", fn() {
        let h = { a: null ?? 1, b: "value" ?? 2 };
        assert_eq(h.a, 1);
        assert_eq(h.b, "value");
    });
});
