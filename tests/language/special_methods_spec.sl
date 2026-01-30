// ============================================================================
// Special Methods (? suffix) Test Suite
// ============================================================================

describe("Predicate Methods with ?", fn() {
    test("empty? method on array", fn() {
        let arr = [];
        assert(arr.empty?());

        let arr2 = [1, 2, 3];
        assert_not(arr2.empty?());
    });

    test("empty? method on string", fn() {
        let s = "";
        assert(s.empty?());

        let s2 = "hello";
        assert_not(s2.empty?());
    });

    test("include? method on array", fn() {
        let arr = [1, 2, 3];
        assert(arr.include?(2));
        assert_not(arr.include?(5));
    });

    test("include? method on string", fn() {
        let s = "hello world";
        assert(s.include?("world"));
        assert_not(s.include?("xyz"));
    });

    test("starts_with? method", fn() {
        let s = "hello world";
        assert(s.starts_with?("hello"));
        assert_not(s.starts_with?("world"));
    });

    test("ends_with? method", fn() {
        let s = "hello world";
        assert(s.ends_with?("world"));
        assert_not(s.ends_with?("hello"));
    });

    test("predicate methods in conditions", fn() {
        let items = [1, 2, 3];
        if (items.empty?()) {
            assert(false);
        } else {
            assert(true);
        }
    });

    test("chained predicate checks", fn() {
        let s = "hello";
        assert(s.include?("ell") && s.starts_with?("he"));
        // Test that neither xyz is included nor does it end with "xyz"
        assert_not(s.include?("xyz") || s.ends_with?("xyz"));
    });
});

describe("Pipeline Operator", fn() {
    test("simple pipeline", fn() {
        let result = 5 |> fn(x) { return x * 2; } |> fn(x) { return x + 1; };
        assert_eq(result, 11);
    });

    test("pipeline with lambda", fn() {
        let result = "hello" |> fn(s) { return s.upcase(); };
        assert_eq(result, "HELLO");
    });

    test("pipeline with multiple steps", fn() {
        let result = [1, 2, 3, 4, 5]
            |> fn(arr) { return arr.filter(fn(x) { return x > 2; }); }
            |> fn(arr) { return arr.map(fn(x) { return x * 2; }); };
        assert_eq(result, [6, 8, 10]);
    });

    test("pipeline with method via lambda", fn() {
        let result = "hello" |> fn(s) { return s.length(); };
        assert_eq(result, 5);
    });

    test("pipeline preserves types", fn() {
        let result = 10 |> fn(x) { return x + 5; };
        assert_eq(result, 15);
        assert(type(result) == "int");
    });

    test("pipeline with string operations", fn() {
        let result = "  hello world  " |> fn(s) { return s.trim(); } |> fn(s) { return s.upcase(); };
        assert_eq(result, "HELLO WORLD");
    });
});

describe("Question Mark Operator", fn() {
    test("ternary operator true branch", fn() {
        let result = true ? "yes" : "no";
        assert_eq(result, "yes");
    });

    test("ternary operator false branch", fn() {
        let result = false ? "yes" : "no";
        assert_eq(result, "no");
    });

    test("ternary in expressions", fn() {
        let x = 5;
        let result = x > 0 ? "positive" : "non-positive";
        assert_eq(result, "positive");
    });

    test("nested ternary", fn() {
        let x = 0;
        let result = x > 0 ? "positive" : x < 0 ? "negative" : "zero";
        assert_eq(result, "zero");
    });
});
