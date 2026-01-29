// ============================================================================
// Type Annotations Test Suite
// ============================================================================

describe("Function Type Annotations", fn() {
    test("parameter type annotation", fn() {
        fn double(n: Int) -> Int {
            return n * 2;
        }
        assert_eq(double(5), 10);
    });

    test("return type annotation", fn() {
        fn get_answer() -> Int {
            return 42;
        }
        assert_eq(get_answer(), 42);
    });

    test("multiple parameter types", fn() {
        fn add(a: Int, b: Int) -> Int {
            return a + b;
        }
        assert_eq(add(3, 4), 7);
    });

    test("different parameter types", fn() {
        fn concat(s: String, n: Int) -> String {
            return s + str(n);
        }
        assert_eq(concat("hello", 5), "hello5");
    });

    test("boolean type annotation", fn() {
        fn is_positive(n: Int) -> Bool {
            return n > 0;
        }
        assert(is_positive(5));
        assert_not(is_positive(-1));
    });

    test("float type annotation", fn() {
        fn area(r: Float) -> Float {
            return r * r * 3.14159;
        }
        assert(area(1.0) > 3.0 && area(1.0) < 3.2);
    });

    test("void return type", fn() {
        fn set_value(n: Int) -> Void {
            let x = n;
        }
        assert_null(set_value(42));
    });

    test("string type annotation", fn() {
        fn greet(name: String) -> String {
            return "Hello, " + name;
        }
        assert_eq(greet("World"), "Hello, World");
    });
});

describe("Class Field Type Annotations", fn() {
    test("typed field in class", fn() {
        class Person {
            name: String;
            age: Int;

            new(name: String, age: Int) {
                this.name = name;
                this.age = age;
            }
        }

        let p = new Person("Alice", 30);
        assert_eq(p.name, "Alice");
        assert_eq(p.age, 30);
    });

    test("field with default value and type", fn() {
        class Config {
            debug: Bool = false;
            version: String = "1.0";
        }

        let c = new Config();
        assert_eq(c.debug, false);
        assert_eq(c.version, "1.0");
    });

    test("multiple typed fields", fn() {
        class Point {
            x: Float;
            y: Float;

            new(x: Float, y: Float) {
                this.x = x;
                this.y = y;
            }
        }

        let p = new Point(3.5, 4.2);
        assert(p.x > 3.0 && p.x < 4.0);
        assert(p.y > 4.0 && p.y < 5.0);
    });

    test("typed method parameters", fn() {
        class BankAccount {
            balance: Int = 0;

            fn deposit(amount: Int) {
                this.balance = this.balance + amount;
            }

            fn get_balance() -> Int {
                return this.balance;
            }
        }

        let account = new BankAccount();
        account.deposit(100);
        assert_eq(account.get_balance(), 100);
    });
});

describe("Type Inference", fn() {
    test("variable type inference from literal", fn() {
        let x = 42;
        assert_eq(x, 42);

        let s = "hello";
        assert_eq(s, "hello");

        let b = true;
        assert(b);
    });

    test("type inference in function", fn() {
        fn identity(x) {
            return x;
        }
        assert_eq(identity(42), 42);
        assert_eq(identity("test"), "test");
    });

    test("type inference in array", fn() {
        let arr = [1, 2, 3];
        assert_eq(len(arr), 3);

        let mixed = [1, "two", true];
        assert_eq(len(mixed), 3);
    });
});

describe("Complex Type Combinations", fn() {
    test("hash with typed values", fn() {
        let config = {
            "port" => 8080,
            "debug" => false,
            "host" => "localhost"
        };
        assert_eq(config["port"], 8080);
        assert_eq(config["debug"], false);
        assert_eq(config["host"], "localhost");
    });

    test("array of specific types", fn() {
        let numbers = [1, 2, 3, 4, 5];
        let sum = 0;
        for (n in numbers) {
            sum = sum + n;
        }
        assert_eq(sum, 15);
    });

    test("function returning collection", fn() {
        fn get_numbers() {
            return [1, 2, 3];
        }
        let nums = get_numbers();
        assert_eq(len(nums), 3);
    });
});

describe("Type Annotation Errors", fn() {
    test("type annotation doesn't prevent runtime usage", fn() {
        fn add(a: Int, b: Int) -> Int {
            return a + b;
        }
        assert_eq(add(10, 20), 30);
    });

    test("typed field can be accessed", fn() {
        class Container {
            value: Int = 42;
        }

        let c = new Container();
        assert_eq(c.value, 42);
        c.value = 100;
        assert_eq(c.value, 100);
    });
});
