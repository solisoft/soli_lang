// ============================================================================
// Solilang Language Features Test Suite
// ============================================================================
// Comprehensive tests for language statements, expressions, classes, functions
// ============================================================================

// ----------------------------------------------------------------------------
// Variable Declarations
// ----------------------------------------------------------------------------
describe("Variable Declarations", fn() {
    test("let declares a variable", fn() {
        let x = 10;
        assert_eq(x, 10);
    });

    test("let with type annotation", fn() {
        let x: Int = 42;
        assert_eq(x, 42);
        assert_eq(type(x), "int");
    });

    test("let can be reassigned", fn() {
        let x = 1;
        x = 2;
        assert_eq(x, 2);
    });

    test("multiple variable declarations", fn() {
        let a = 1;
        let b = 2;
        let c = 3;
        assert_eq(a + b + c, 6);
    });
});

// ----------------------------------------------------------------------------
// Literals
// ----------------------------------------------------------------------------
describe("Literals", fn() {
    test("integer literals", fn() {
        assert_eq(42, 42);
        assert_eq(-10, -10);
        assert_eq(0, 0);
    });

    test("float literals", fn() {
        assert_eq(3.14, 3.14);
        assert_eq(-2.5, -2.5);
        assert_eq(0.0, 0.0);
    });

    test("string literals with double quotes", fn() {
        assert_eq("hello", "hello");
        assert_eq("", "");
    });

    test("boolean literals", fn() {
        assert_eq(true, true);
        assert_eq(false, false);
    });

    test("null literal", fn() {
        assert_null(null);
    });

    test("array literals", fn() {
        let arr = [1, 2, 3];
        assert_eq(len(arr), 3);
        assert_eq(arr[0], 1);
        assert_eq(arr[1], 2);
        assert_eq(arr[2], 3);
    });

    test("empty array literal", fn() {
        let arr = [];
        assert_eq(len(arr), 0);
    });

    test("nested array literals", fn() {
        let arr = [[1, 2], [3, 4]];
        assert_eq(arr[0][0], 1);
        assert_eq(arr[1][1], 4);
    });

    test("hash literals with fat arrow", fn() {
        let h = {"a" => 1, "b" => 2};
        assert_eq(h["a"], 1);
        assert_eq(h["b"], 2);
    });

    test("hash literals with colon", fn() {
        let h = {a: 1, b: 2};
        assert_eq(h["a"], 1);
        assert_eq(h["b"], 2);
    });

    test("empty hash literal", fn() {
        let h = {};
        assert_eq(len(h), 0);
    });
});

// ----------------------------------------------------------------------------
// String Interpolation
// ----------------------------------------------------------------------------
describe("String Interpolation", fn() {
    test("basic interpolation with \\()", fn() {
        let name = "World";
        let greeting = "Hello \(name)!";
        assert_eq(greeting, "Hello World!");
    });

    test("interpolation with expressions", fn() {
        let a = 2;
        let b = 3;
        let result = "Sum is \(a + b)";
        assert_eq(result, "Sum is 5");
    });

    test("multiple interpolations", fn() {
        let first = "John";
        let last = "Doe";
        let full = "\(first) \(last)";
        assert_eq(full, "John Doe");
    });

    test("nested expression in interpolation", fn() {
        let x = 10;
        let msg = "Double is \(x * 2)";
        assert_eq(msg, "Double is 20");
    });
});

// ----------------------------------------------------------------------------
// Arithmetic Operators
// ----------------------------------------------------------------------------
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

// ----------------------------------------------------------------------------
// Comparison Operators
// ----------------------------------------------------------------------------
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

// ----------------------------------------------------------------------------
// Logical Operators
// ----------------------------------------------------------------------------
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

// ----------------------------------------------------------------------------
// Ternary Operator
// ----------------------------------------------------------------------------
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

// ----------------------------------------------------------------------------
// If/Else Statements
// ----------------------------------------------------------------------------
describe("If/Else Statements", fn() {
    test("if true executes block", fn() {
        let result = 0;
        if (true) {
            result = 1;
        }
        assert_eq(result, 1);
    });

    test("if false skips block", fn() {
        let result = 0;
        if (false) {
            result = 1;
        }
        assert_eq(result, 0);
    });

    test("if-else executes else branch", fn() {
        let result = 0;
        if (false) {
            result = 1;
        } else {
            result = 2;
        }
        assert_eq(result, 2);
    });

    test("if-else if-else chain", fn() {
        let x = 2;
        let result = "";
        if (x == 1) {
            result = "one";
        } else if (x == 2) {
            result = "two";
        } else {
            result = "other";
        }
        assert_eq(result, "two");
    });

    test("nested if statements", fn() {
        let a = true;
        let b = true;
        let result = 0;
        if (a) {
            if (b) {
                result = 1;
            }
        }
        assert_eq(result, 1);
    });

    test("if with complex condition", fn() {
        let x = 5;
        let y = 10;
        let result = "";
        if (x > 0 && y > 0) {
            result = "both positive";
        }
        assert_eq(result, "both positive");
    });
});

// ----------------------------------------------------------------------------
// While Loops
// ----------------------------------------------------------------------------
describe("While Loops", fn() {
    test("while loop iterates", fn() {
        let count = 0;
        while (count < 5) {
            count = count + 1;
        }
        assert_eq(count, 5);
    });

    test("while loop with false condition never executes", fn() {
        let executed = false;
        while (false) {
            executed = true;
        }
        assert_not(executed);
    });

    test("while loop with complex condition", fn() {
        let i = 0;
        let sum = 0;
        while (i < 10 && sum < 20) {
            sum = sum + i;
            i = i + 1;
        }
        assert(sum >= 20 || i >= 10);
    });
});

// ----------------------------------------------------------------------------
// For-In Loops
// ----------------------------------------------------------------------------
describe("For-In Loops", fn() {
    test("for-in iterates over array", fn() {
        let arr = [1, 2, 3];
        let sum = 0;
        for (x in arr) {
            sum = sum + x;
        }
        assert_eq(sum, 6);
    });

    test("for-in iterates over range", fn() {
        let sum = 0;
        for (i in range(1, 5)) {
            sum = sum + i;
        }
        assert_eq(sum, 10);
    });

    test("for-in with empty array", fn() {
        let count = 0;
        for (x in []) {
            count = count + 1;
        }
        assert_eq(count, 0);
    });

    test("for-in can access loop variable", fn() {
        let result = [];
        for (i in range(0, 3)) {
            push(result, i * 2);
        }
        assert_eq(result[0], 0);
        assert_eq(result[1], 2);
        assert_eq(result[2], 4);
    });

    test("nested for-in loops", fn() {
        let sum = 0;
        for (i in range(0, 3)) {
            for (j in range(0, 3)) {
                sum = sum + 1;
            }
        }
        assert_eq(sum, 9);
    });
});

// ----------------------------------------------------------------------------
// Functions
// ----------------------------------------------------------------------------
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

// ----------------------------------------------------------------------------
// Anonymous Functions / Lambdas
// ----------------------------------------------------------------------------
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

// ----------------------------------------------------------------------------
// Closures
// ----------------------------------------------------------------------------
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

// ----------------------------------------------------------------------------
// Classes
// ----------------------------------------------------------------------------
describe("Classes", fn() {
    test("class declaration and instantiation", fn() {
        class Point {
            x: Int;
            y: Int;

            new(x: Int, y: Int) {
                this.x = x;
                this.y = y;
            }
        }
        let p = new Point(3, 4);
        assert_eq(p.x, 3);
        assert_eq(p.y, 4);
    });

    test("class with methods", fn() {
        class Rectangle {
            width: Int;
            height: Int;

            new(w: Int, h: Int) {
                this.width = w;
                this.height = h;
            }

            fn area() {
                return this.width * this.height;
            }

            fn perimeter() {
                return 2 * (this.width + this.height);
            }
        }
        let rect = new Rectangle(5, 3);
        assert_eq(rect.area(), 15);
        assert_eq(rect.perimeter(), 16);
    });

    test("class with default field values", fn() {
        class Counter {
            count: Int = 0;

            fn increment() {
                this.count = this.count + 1;
            }

            fn get() {
                return this.count;
            }
        }
        let c = new Counter();
        assert_eq(c.get(), 0);
        c.increment();
        c.increment();
        assert_eq(c.get(), 2);
    });

    test("class method chaining", fn() {
        class Builder {
            value: String = "";

            fn add(s: String) {
                this.value = this.value + s;
                return this;
            }

            fn build() {
                return this.value;
            }
        }
        let result = new Builder().add("Hello").add(" ").add("World").build();
        assert_eq(result, "Hello World");
    });
});

// ----------------------------------------------------------------------------
// Class Inheritance
// ----------------------------------------------------------------------------
describe("Class Inheritance", fn() {
    test("subclass inherits from superclass", fn() {
        class Animal {
            name: String;

            new(name: String) {
                this.name = name;
            }

            fn speak() {
                return "...";
            }
        }

        class Dog extends Animal {
            fn speak() {
                return "Woof!";
            }
        }

        let dog = new Dog("Buddy");
        assert_eq(dog.name, "Buddy");
        assert_eq(dog.speak(), "Woof!");
    });

    test("subclass can call super", fn() {
        class Base {
            fn getValue() {
                return 10;
            }
        }

        class Derived extends Base {
            fn getValue() {
                return super.getValue() + 5;
            }
        }

        let d = new Derived();
        assert_eq(d.getValue(), 15);
    });

    test("instanceof-like behavior with type", fn() {
        class MyClass {
            value: Int = 42;
        }
        let obj = new MyClass();
        assert_eq(type(obj), "MyClass");
    });
});

// ----------------------------------------------------------------------------
// Try/Catch/Finally
// ----------------------------------------------------------------------------
describe("Try/Catch/Finally", fn() {
    test("try without error executes normally", fn() {
        let result = 0;
        try {
            result = 42;
        } catch (e) {
            result = -1;
        }
        assert_eq(result, 42);
    });

    test("catch handles thrown error", fn() {
        let result = "";
        try {
            throw "error message";
            result = "not reached";
        } catch (e) {
            result = "caught";
        }
        assert_eq(result, "caught");
    });

    test("finally always executes after try", fn() {
        let finally_ran = false;
        try {
            let x = 1;
        } catch (e) {
            let x = 2;
        } finally {
            finally_ran = true;
        }
        assert(finally_ran);
    });

    test("finally runs after catch", fn() {
        let sequence = [];
        try {
            throw "error";
        } catch (e) {
            push(sequence, "catch");
        } finally {
            push(sequence, "finally");
        }
        assert_eq(len(sequence), 2);
        assert_eq(sequence[0], "catch");
        assert_eq(sequence[1], "finally");
    });

    test("nested try/catch", fn() {
        let result = "";
        try {
            try {
                throw "inner";
            } catch (e) {
                result = "inner caught";
                throw "outer";
            }
        } catch (e) {
            result = result + " outer caught";
        }
        assert_eq(result, "inner caught outer caught");
    });
});

// ----------------------------------------------------------------------------
// Array Operations
// ----------------------------------------------------------------------------
describe("Array Operations", fn() {
    test("array indexing", fn() {
        let arr = ["a", "b", "c"];
        assert_eq(arr[0], "a");
        assert_eq(arr[1], "b");
        assert_eq(arr[2], "c");
    });

    test("array index assignment", fn() {
        let arr = [1, 2, 3];
        arr[1] = 20;
        assert_eq(arr[1], 20);
    });

    test("array spread operator", fn() {
        let a = [1, 2];
        let b = [3, 4];
        let c = [...a, ...b];
        assert_eq(len(c), 4);
        assert_eq(c[0], 1);
        assert_eq(c[3], 4);
    });

    test("array of mixed types", fn() {
        let arr = [1, "two", true, null];
        assert_eq(arr[0], 1);
        assert_eq(arr[1], "two");
        assert_eq(arr[2], true);
        assert_null(arr[3]);
    });
});

// ----------------------------------------------------------------------------
// Hash Operations
// ----------------------------------------------------------------------------
describe("Hash Operations", fn() {
    test("hash key access", fn() {
        let h = {"name" => "Alice", "age" => 30};
        assert_eq(h["name"], "Alice");
        assert_eq(h["age"], 30);
    });

    test("hash key assignment", fn() {
        let h = hash();
        h["key"] = "value";
        assert_eq(h["key"], "value");
    });

    test("hash with integer keys", fn() {
        let h = {1 => "one", 2 => "two"};
        assert_eq(h[1], "one");
        assert_eq(h[2], "two");
    });

    test("hash dot notation for string keys", fn() {
        let h = {name: "Bob"};
        assert_eq(h["name"], "Bob");
    });

    test("nested hash access", fn() {
        let h = {
            "person" => {
                "name" => "Alice",
                "address" => {
                    "city" => "NYC"
                }
            }
        };
        assert_eq(h["person"]["name"], "Alice");
        assert_eq(h["person"]["address"]["city"], "NYC");
    });
});

// ----------------------------------------------------------------------------
// List Comprehensions
// ----------------------------------------------------------------------------
describe("List Comprehensions", fn() {
    test("basic list comprehension", fn() {
        let squares = [x * x for x in range(1, 5)];
        assert_eq(len(squares), 4);
        assert_eq(squares[0], 1);
        assert_eq(squares[1], 4);
        assert_eq(squares[2], 9);
        assert_eq(squares[3], 16);
    });

    test("list comprehension with condition", fn() {
        let evens = [x for x in range(1, 10) if x % 2 == 0];
        assert_eq(len(evens), 4);
        assert_eq(evens[0], 2);
        assert_eq(evens[1], 4);
    });

    test("list comprehension with transformation", fn() {
        let words = ["hello", "world"];
        let upper = [upcase(w) for w in words];
        assert_eq(upper[0], "HELLO");
        assert_eq(upper[1], "WORLD");
    });
});

// ----------------------------------------------------------------------------
// Pipeline Operator
// ----------------------------------------------------------------------------
describe("Pipeline Operator", fn() {
    test("basic pipeline", fn() {
        fn double(x) { return x * 2; }
        fn addOne(x) { return x + 1; }

        let result = 5 |> double();
        assert_eq(result, 10);
    });

    test("chained pipeline", fn() {
        fn double(x) { return x * 2; }
        fn addTen(x) { return x + 10; }

        let result = 5 |> double() |> addTen();
        assert_eq(result, 20);
    });
});

// ----------------------------------------------------------------------------
// Pattern Matching
// ----------------------------------------------------------------------------
describe("Pattern Matching", fn() {
    test("match with literal patterns", fn() {
        fn describe(x) {
            return match x {
                0 => "zero",
                1 => "one",
                _ => "other"
            };
        }
        assert_eq(describe(0), "zero");
        assert_eq(describe(1), "one");
        assert_eq(describe(99), "other");
    });

    test("match with variable binding", fn() {
        fn describe(x) {
            return match x {
                0 => "zero",
                n => "number: " + str(n)
            };
        }
        assert_eq(describe(0), "zero");
        assert_eq(describe(42), "number: 42");
    });

    test("match with guards", fn() {
        fn classify(x) {
            return match x {
                n if n < 0 => "negative",
                n if n == 0 => "zero",
                n if n > 0 => "positive"
            };
        }
        assert_eq(classify(-5), "negative");
        assert_eq(classify(0), "zero");
        assert_eq(classify(5), "positive");
    });

    test("match with array patterns", fn() {
        fn first(arr) {
            return match arr {
                [] => "empty",
                [x] => "single: " + str(x),
                [x, y] => "pair: " + str(x) + ", " + str(y),
                _ => "many"
            };
        }
        assert_eq(first([]), "empty");
        assert_eq(first([1]), "single: 1");
        assert_eq(first([1, 2]), "pair: 1, 2");
        assert_eq(first([1, 2, 3]), "many");
    });
});

// ----------------------------------------------------------------------------
// Postfix If/Unless
// ----------------------------------------------------------------------------
describe("Postfix Conditionals", fn() {
    test("postfix if executes when true", fn() {
        let result = 0;
        result = 42 if (true);
        assert_eq(result, 42);
    });

    test("postfix if skips when false", fn() {
        let result = 0;
        result = 42 if (false);
        assert_eq(result, 0);
    });

    test("postfix unless executes when false", fn() {
        let result = 0;
        result = 42 unless (false);
        assert_eq(result, 42);
    });

    test("postfix unless skips when true", fn() {
        let result = 0;
        result = 42 unless (true);
        assert_eq(result, 0);
    });
});

// ----------------------------------------------------------------------------
// Scope and Shadowing
// ----------------------------------------------------------------------------
describe("Scope and Shadowing", fn() {
    test("inner scope shadows outer variable", fn() {
        let x = 1;
        {
            let x = 2;
            assert_eq(x, 2);
        }
        assert_eq(x, 1);
    });

    test("inner scope can access outer variable", fn() {
        let outer = 10;
        let result = 0;
        {
            result = outer + 5;
        }
        assert_eq(result, 15);
    });

    test("function has its own scope", fn() {
        let x = 100;
        fn setX() {
            let x = 50;
            return x;
        }
        assert_eq(setX(), 50);
        assert_eq(x, 100);
    });
});

// ----------------------------------------------------------------------------
// Edge Cases and Special Behaviors
// ----------------------------------------------------------------------------
describe("Edge Cases", fn() {
    test("empty block returns null", fn() {
        let result = {
        };
        assert_null(result);
    });

    test("chained comparisons", fn() {
        let x = 5;
        assert(x > 0 && x < 10);
    });

    test("deeply nested expressions", fn() {
        let result = ((1 + 2) * (3 + 4)) + ((5 - 2) * (8 / 4));
        assert_eq(result, 27);
    });

    test("array of functions", fn() {
        let funcs = [
            fn(x) { return x + 1; },
            fn(x) { return x * 2; },
            fn(x) { return x - 3; }
        ];
        assert_eq(funcs[0](10), 11);
        assert_eq(funcs[1](10), 20);
        assert_eq(funcs[2](10), 7);
    });

    test("hash of functions", fn() {
        let ops = {
            "add" => fn(a, b) { return a + b; },
            "sub" => fn(a, b) { return a - b; }
        };
        assert_eq(ops["add"](5, 3), 8);
        assert_eq(ops["sub"](5, 3), 2);
    });

    test("method on literal", fn() {
        assert_eq(len("hello"), 5);
        assert_eq(len([1, 2, 3]), 3);
    });

    test("boolean coercion in conditions", fn() {
        let result = "";
        if (1) { result = "truthy"; }
        assert_eq(result, "truthy");

        result = "";
        if ("non-empty") { result = "truthy"; }
        assert_eq(result, "truthy");
    });
});

// ----------------------------------------------------------------------------
// Multiline Strings
// ----------------------------------------------------------------------------
describe("Multiline Strings", fn() {
    test("basic multiline string", fn() {
        let text = [[hello
world]];
        assert_contains(text, "hello");
        assert_contains(text, "world");
    });

    test("multiline string preserves newlines", fn() {
        let text = [[line1
line2]];
        assert_contains(text, "\n");
    });

    test("multiline string is raw (no escape processing)", fn() {
        let text = [[hello\nworld]];
        assert_contains(text, "\\n");
    });

    test("multiline string with multiple lines", fn() {
        let text = [[first
second
third]];
        assert_contains(text, "first");
        assert_contains(text, "second");
        assert_contains(text, "third");
    });

    test("multiline string with single bracket", fn() {
        let text = [[contains ] single bracket]];
        assert_contains(text, "]");
    });

    test("empty multiline string", fn() {
        let text = [[]];
        assert_eq(text, "");
    });
});
