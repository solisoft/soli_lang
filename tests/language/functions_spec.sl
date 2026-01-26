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
