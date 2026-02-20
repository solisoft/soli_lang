// ============================================================================
// Command-Style Calls Test Suite
// ============================================================================
// Tests for calling functions without parentheses: `print x` instead of `print(x)`

describe("Command-style calls with literals", fn() {
    test("string literal", fn() {
        fn echo(x) { return x; }
        let result = echo "hello";
        assert_eq(result, "hello");
    });

    test("integer literal", fn() {
        fn echo(x) { return x; }
        let result = echo 42;
        assert_eq(result, 42);
    });

    test("float literal", fn() {
        fn echo(x) { return x; }
        let result = echo 3.14;
        assert_eq(result, 3.14);
    });

    test("boolean literal", fn() {
        fn echo(x) { return x; }
        let t = echo true;
        let f = echo false;
        assert_eq(t, true);
        assert_eq(f, false);
    });

    test("null literal", fn() {
        fn echo(x) { return x; }
        let result = echo null;
        assert_null(result);
    });

    test("interpolated string", fn() {
        fn echo(x) { return x; }
        let name = "world";
        let result = echo "hello #{name}";
        assert_eq(result, "hello world");
    });
});

describe("Command-style calls with variables", fn() {
    test("simple variable", fn() {
        fn echo(x) { return x; }
        let msg = "hello";
        let result = echo msg;
        assert_eq(result, "hello");
    });

    test("variable holding number", fn() {
        fn double(x) { return x * 2; }
        let n = 21;
        let result = double n;
        assert_eq(result, 42);
    });

    test("variable holding boolean", fn() {
        fn negate(x) { return !x; }
        let flag = true;
        let result = negate flag;
        assert_eq(result, false);
    });

    test("variable holding array", fn() {
        fn first(arr) { return arr[0]; }
        let items = [10, 20, 30];
        let result = first items;
        assert_eq(result, 10);
    });

    test("variable holding hash", fn() {
        fn get_name(h) { return h["name"]; }
        let person = { "name": "Alice" };
        let result = get_name person;
        assert_eq(result, "Alice");
    });
});

describe("Command-style calls with multiple arguments", fn() {
    test("two string arguments", fn() {
        fn concat(a, b) { return a + " " + b; }
        let result = concat "hello", "world";
        assert_eq(result, "hello world");
    });

    test("two variable arguments", fn() {
        fn add(a, b) { return a + b; }
        let x = 10;
        let y = 20;
        let result = add x, y;
        assert_eq(result, 30);
    });

    test("mixed literal and variable", fn() {
        fn add(a, b) { return a + b; }
        let x = 10;
        let result = add x, 5;
        assert_eq(result, 15);
    });

    test("three arguments", fn() {
        fn sum3(a, b, c) { return a + b + c; }
        let a = 1;
        let b = 2;
        let c = 3;
        let result = sum3 a, b, c;
        assert_eq(result, 6);
    });
});

describe("Command-style calls in different contexts", fn() {
    test("inside if body", fn() {
        fn echo(x) { return x; }
        let result = null;
        let x = "yes";
        if (true) {
            result = echo x;
        }
        assert_eq(result, "yes");
    });

    test("inside function body with end syntax", fn() {
        fn echo(x)
            return x
        end

        let msg = "hello";
        let result = echo msg;
        assert_eq(result, "hello");
    });

    test("result used in expression", fn() {
        fn double(x) { return x * 2; }
        let n = 5;
        let result = (double n) + 1;
        assert_eq(result, 11);
    });

    test("chained with parentheses call", fn() {
        fn add(a, b) { return a + b; }
        fn double(x) { return x * 2; }
        let x = 3;
        let result = double(add x, 2);
        assert_eq(result, 10);
    });
});

describe("Command-style calls do not break multi-line code", fn() {
    test("function body with separate statements", fn() {
        let log = [];
        fn process(x) {
            log.push(x);
            log.push(x * 2);
            return log;
        }
        let result = process(5);
        assert_eq(result, [5, 10]);
    });

    test("variables on consecutive lines stay independent", fn() {
        let a = 1;
        let b = 2;
        let c = a;
        let d = b;
        assert_eq(c, 1);
        assert_eq(d, 2);
    });

    test("command call followed by another statement", fn() {
        fn echo(x) { return x; }
        let msg = "hi";
        let result = echo msg;
        let other = 42;
        assert_eq(result, "hi");
        assert_eq(other, 42);
    });
});

describe("Parentheses call still works", fn() {
    test("standard parenthesized call", fn() {
        fn add(a, b) { return a + b; }
        assert_eq(add(2, 3), 5);
    });

    test("parenthesized call with variable", fn() {
        fn double(x) { return x * 2; }
        let n = 7;
        assert_eq(double(n), 14);
    });

    test("both styles produce same result", fn() {
        fn echo(x) { return x; }
        let val = "test";
        let a = echo val;
        let b = echo(val);
        assert_eq(a, b);
    });
});
