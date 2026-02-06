// ============================================================================
// Functions Test Suite
// ============================================================================

describe("Functions", fn() {
    test("function declaration and call", fn() {
        fn add(a, b) {
            return a + b;
        }
        assert_eq(add(2, 3), 5);
    });

    test("function with no parameters", fn() {
        fn greet() {
            return "hello";
        }
        assert_eq(greet(), "hello");
    });

    test("function with typed parameters", fn() {
        fn multiply(a: Int, b: Int) {
            return a * b;
        }
        assert_eq(multiply(3, 4), 12);
    });

    test("function with return type", fn() {
        fn square(x: Int) -> Int {
            return x * x;
        }
        assert_eq(square(5), 25);
    });

    test("function with default parameter", fn() {
        fn greet(name: String = "World") {
            return "Hello " + name;
        }
        assert_eq(greet(), "Hello World");
        assert_eq(greet("Alice"), "Hello Alice");
    });

    test("function with named parameters", fn() {
        fn configure(host: String = "localhost", port: Int = 8080, debug: Bool = false) {
            return host + ":" + str(port) + " (debug: " + str(debug) + ")";
        }
        assert_eq(configure(), "localhost:8080 (debug: false)");
        assert_eq(configure(port: 3000), "localhost:3000 (debug: false)");
        assert_eq(configure(host: "example.com", port: 443), "example.com:443 (debug: false)");
        assert_eq(configure(port: 9000, debug: true, host: "api.example.com"), "api.example.com:9000 (debug: true)");
    });

    test("function with mixed positional and named parameters", fn() {
        fn sum(a: Int, b: Int = 10, c: Int = 20) {
            return a + b + c;
        }
        assert_eq(sum(1), 31);
        assert_eq(sum(1, 2), 23);
        assert_eq(sum(1, c: 5), 16);
    });

    test("function returning null implicitly", fn() {
        fn nothing() {
            let x = 1;
        }
        assert_null(nothing());
    });

    test("early return", fn() {
        fn check(x) {
            if (x < 0) {
                return "negative";
            }
            return "non-negative";
        }
        assert_eq(check(-5), "negative");
        assert_eq(check(5), "non-negative");
    });

    test("recursive function", fn() {
        fn factorial(n) {
            if (n <= 1) {
                return 1;
            }
            return n * factorial(n - 1);
        }
        assert_eq(factorial(5), 120);
    });

    test("function as first-class value", fn() {
        fn double(x) {
            return x * 2;
        }
        let f = double;
        assert_eq(f(5), 10);
    });

    test("higher-order function", fn() {
        fn apply(f, x) {
            return f(x);
        }
        fn triple(x) {
            return x * 3;
        }
        assert_eq(apply(triple, 4), 12);
    });
});

describe("Implicit Returns", fn() {
    test("implicit return from expression", fn() {
        fn add(a, b) {
            a + b
        }
        assert_eq(add(2, 3), 5);
    });

    test("implicit return from function call", fn() {
        fn double(x) { x * 2 }
        fn call_double(x) {
            double(x)
        }
        assert_eq(call_double(5), 10);
    });

    test("implicit return from if/else", fn() {
        fn abs(x) {
            if (x < 0) { -x } else { x }
        }
        assert_eq(abs(-5), 5);
        assert_eq(abs(5), 5);
    });

    test("implicit return from block lambda", fn() {
        let items = [1, 2, 3];
        let doubled = items.map(fn(x) { x * 2 });
        assert_eq(doubled, [2, 4, 6]);
    });

    test("let as last statement returns null", fn() {
        fn nothing() {
            let x = 1;
        }
        assert_null(nothing());
    });

    test("explicit return still works", fn() {
        fn early(x) {
            if (x < 0) { return "negative"; }
            "non-negative"
        }
        assert_eq(early(-1), "negative");
        assert_eq(early(1), "non-negative");
    });
});

describe("Closures", fn() {
    test("closure captures outer variable", fn() {
        let multiplier = 3;
        let multiply = fn(x) { return x * multiplier; };
        assert_eq(multiply(5), 15);
    });

    test("closure captures multiple variables", fn() {
        let a = 10;
        let b = 20;
        let sum = fn() { return a + b; };
        assert_eq(sum(), 30);
    });

    test("closure factory", fn() {
        fn makeAdder(n) {
            return fn(x) { return x + n; };
        }
        let add5 = makeAdder(5);
        let add10 = makeAdder(10);
        assert_eq(add5(3), 8);
        assert_eq(add10(3), 13);
    });

    test("closure maintains separate state", fn() {
        fn makeCounter() {
            let count = 0;
            return fn() {
                count = count + 1;
                return count;
            };
        }
        let counter1 = makeCounter();
        let counter2 = makeCounter();
        assert_eq(counter1(), 1);
        assert_eq(counter1(), 2);
        assert_eq(counter2(), 1);
    });
});
