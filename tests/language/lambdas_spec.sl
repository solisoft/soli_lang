// ============================================================================
// Anonymous Functions (Lambdas) Test Suite
// ============================================================================

describe("Anonymous Functions", fn() {
    test("fn() syntax", fn() {
        let add = fn(a, b) { return a + b; };
        assert_eq(add(2, 3), 5);
    });

    test("lambda with pipe syntax", fn() {
        let double = |x| { return x * 2; };
        assert_eq(double(5), 10);
    });

    test("lambda with multiple parameters", fn() {
        let sum = |a, b, c| { return a + b + c; };
        assert_eq(sum(1, 2, 3), 6);
    });

    test("lambda as callback", fn() {
        fn apply(f, x) {
            return f(x);
        }
        let result = apply(|x| { return x * x; }, 4);
        assert_eq(result, 16);
    });

    test("immediately invoked lambda", fn() {
        let result = (fn(x) { return x + 1; })(5);
        assert_eq(result, 6);
    });
});

describe("Lambdas and this Keyword", fn() {
    test("arrow lambda captures this from enclosing scope", fn() {
        class Outer {
            value: Int = 100;
            fn get_closure() {
                return |x| this.value + x;
            }
        }
        let o = new Outer();
        let closure = o.get_closure();
        assert_eq(closure(5), 105);
    });

    test("fn() {} lambda captures this", fn() {
        class Container {
            factor: Int = 10;
            fn multiplier() {
                return fn(x) { return x * this.factor; };
            }
        }
        let c = new Container();
        let closure = c.multiplier();
        assert_eq(closure(5), 50);
    });

    test("nested lambdas with this", fn() {
        class Outer {
            base: Int = 10;
            fn create_nested() {
                return fn(y) {
                    return fn(z) {
                        return this.base + y + z;
                    };
                };
            }
        }
        let o = new Outer();
        let inner = o.create_nested()(5);
        assert_eq(inner(3), 18);
    });

    test("this in method callback", fn() {
        class Processor {
            multiplier: Int = 2;
            fn process(items: Array) {
                return items.map(fn(x) { return x * this.multiplier; });
            }
        }
        let p = new Processor();
        let result = p.process([1, 2, 3]);
        assert_eq(result[0], 2);
        assert_eq(result[1], 4);
        assert_eq(result[2], 6);
    });
});

describe("Lambda Edge Cases", fn() {
    test("lambda with no parameters", fn() {
        let getFive = fn() { return 5; };
        assert_eq(getFive(), 5);
    });

    test("lambda with typed parameters", fn() {
        let add = fn(a: Int, b: Int) { return a + b; };
        assert_eq(add(10, 20), 30);
    });

    test("lambda with typed return", fn() {
        let square = fn(x: Int) -> Int { return x * x; };
        assert_eq(square(7), 49);
    });

    test("lambda stored in array", fn() {
        let ops = [
            fn(x) { return x + 1; },
            fn(x) { return x * 2; }
        ];
        assert_eq(ops[0](5), 6);
        assert_eq(ops[1](5), 10);
    });

    test("lambda stored in hash", fn() {
        let funcs = {
            "double" => fn(x) { return x * 2; },
            "triple" => fn(x) { return x * 3; }
        };
        assert_eq(funcs["double"](5), 10);
        assert_eq(funcs["triple"](5), 15);
    });
});
