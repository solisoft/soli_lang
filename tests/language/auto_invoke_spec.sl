// ============================================================================
// Auto-Invoke (Ruby-style no-parentheses method calls) Test Suite
// ============================================================================

describe("Auto-invoke built-in Array methods", fn() {
    test("arr.length auto-invokes", fn() {
        let arr = [1, 2, 3];
        assert_eq(arr.length, 3);
    });

    test("arr.length() still works", fn() {
        let arr = [1, 2, 3];
        assert_eq(arr.length(), 3);
    });

    test("arr.first auto-invokes", fn() {
        let arr = [10, 20, 30];
        assert_eq(arr.first, 10);
    });

    test("arr.last auto-invokes", fn() {
        let arr = [10, 20, 30];
        assert_eq(arr.last, 30);
    });

    test("arr.empty? auto-invokes", fn() {
        assert_eq([].empty?, true);
        assert_eq([1].empty?, false);
    });

    test("arr.reverse auto-invokes", fn() {
        assert_eq([1, 2, 3].reverse, [3, 2, 1]);
    });

    test("arr.sort auto-invokes", fn() {
        assert_eq([3, 1, 2].sort, [1, 2, 3]);
    });

    test("arr.uniq auto-invokes", fn() {
        assert_eq([1, 2, 2, 3, 3].uniq, [1, 2, 3]);
    });

    test("arr.compact auto-invokes", fn() {
        assert_eq([1, null, 2, null, 3].compact, [1, 2, 3]);
    });

    test("arr.flatten auto-invokes", fn() {
        assert_eq([[1, 2], [3, 4]].flatten, [1, 2, 3, 4]);
    });

    test("arr.sum auto-invokes", fn() {
        assert_eq([1, 2, 3, 4].sum, 10);
    });

    test("arr.min auto-invokes", fn() {
        assert_eq([3, 1, 2].min, 1);
    });

    test("arr.max auto-invokes", fn() {
        assert_eq([3, 1, 2].max, 3);
    });

    test("arr.to_string auto-invokes", fn() {
        assert_eq([1, 2, 3].to_string, "[1, 2, 3]");
    });

    test("methods with args still need parens", fn() {
        let arr = [1, 2, 3];
        let doubled = arr.map(fn(x) { x * 2 });
        assert_eq(doubled, [2, 4, 6]);
    });

    test("include? with args still works", fn() {
        let arr = [1, 2, 3];
        assert_eq(arr.include?(2), true);
        assert_eq(arr.include?(4), false);
    });
});

describe("Auto-invoke built-in String methods", fn() {
    test("str.length auto-invokes", fn() {
        assert_eq("hello".length, 5);
    });

    test("str.upcase auto-invokes", fn() {
        assert_eq("hello".upcase, "HELLO");
    });

    test("str.downcase auto-invokes", fn() {
        assert_eq("HELLO".downcase, "hello");
    });

    test("str.trim auto-invokes", fn() {
        assert_eq("  hello  ".trim, "hello");
    });

    test("str.reverse auto-invokes", fn() {
        assert_eq("hello".reverse, "olleh");
    });

    test("str.capitalize auto-invokes", fn() {
        assert_eq("hello".capitalize, "Hello");
    });

    test("str.empty? auto-invokes", fn() {
        assert_eq("".empty?, true);
        assert_eq("hello".empty?, false);
    });

    test("str.chars auto-invokes", fn() {
        assert_eq("abc".chars, ["a", "b", "c"]);
    });

    test("str.bytes auto-invokes", fn() {
        assert_eq("abc".bytes, [97, 98, 99]);
    });

    test("str.upcase() with parens still works", fn() {
        assert_eq("hello".upcase(), "HELLO");
    });

    test("methods with args still need parens", fn() {
        assert_eq("hello world".split(" "), ["hello", "world"]);
    });
});

describe("Auto-invoke built-in Hash methods", fn() {
    test("hash.length auto-invokes", fn() {
        assert_eq({a: 1, b: 2, c: 3}.length, 3);
    });

    test("hash.keys auto-invokes", fn() {
        assert_eq({a: 1, b: 2}.keys, ["a", "b"]);
    });

    test("hash.values auto-invokes", fn() {
        assert_eq({a: 1, b: 2}.values, [1, 2]);
    });

    test("hash.empty? auto-invokes", fn() {
        assert_eq({}.empty?, true);
        assert_eq({a: 1}.empty?, false);
    });

    test("hash.to_string auto-invokes", fn() {
        let h = {a: 1};
        let s = h.to_string;
        assert_eq(type(s), "string");
    });

    test("hash.compact auto-invokes", fn() {
        let h = {a: 1, b: null, c: 3};
        assert_eq(h.compact, {a: 1, c: 3});
    });

    test("hash field access still works", fn() {
        let h = {name: "Soli", version: 1};
        assert_eq(h.name, "Soli");
        assert_eq(h.version, 1);
    });
});

describe("Auto-invoke user-defined methods", fn() {
    test("zero-arg instance method auto-invokes", fn() {
        class Dog {
            name: String;

            new(name: String) {
                this.name = name;
            }

            fn bark() {
                "Woof!"
            }
        }
        let d = new Dog("Rex");
        assert_eq(d.bark, "Woof!");
    });

    test("zero-arg method with parens still works", fn() {
        class Dog {
            name: String;

            new(name: String) {
                this.name = name;
            }

            fn bark() {
                "Woof!"
            }
        }
        let d = new Dog("Rex");
        assert_eq(d.bark(), "Woof!");
    });

    test("field access is NOT auto-invoked", fn() {
        class Box {
            value: Int;

            new(v: Int) {
                this.value = v;
            }
        }
        let b = new Box(42);
        assert_eq(b.value, 42);
    });

    test("lambda field is NOT auto-invoked", fn() {
        class Box {
            action: Function;

            new() {
                this.action = fn() { "called" };
            }
        }
        let b = new Box();
        assert_eq(type(b.action), "Function");
    });

    test("methods with args still need parens", fn() {
        class Dog {
            name: String;

            new(name: String) {
                this.name = name;
            }

            fn greet(person) {
                "Hello #{person}, I'm #{this.name}"
            }
        }
        let d = new Dog("Rex");
        assert_eq(d.greet("Alice"), "Hello Alice, I'm Rex");
    });

    test("method with default params auto-invokes when all params have defaults", fn() {
        class Greeter {
            fn hello(name = "World") {
                "Hello #{name}!"
            }
        }
        let g = new Greeter();
        assert_eq(g.hello, "Hello World!");
    });
});

describe("Method chaining with auto-invoke", fn() {
    test("arr.sort.first chains correctly", fn() {
        assert_eq([3, 1, 2].sort.first, 1);
    });

    test("arr.sort.last chains correctly", fn() {
        assert_eq([3, 1, 2].sort.last, 3);
    });

    test("arr.sort.reverse chains correctly", fn() {
        assert_eq([3, 1, 2].sort.reverse, [3, 2, 1]);
    });

    test("arr.reverse.first chains correctly", fn() {
        assert_eq([1, 2, 3].reverse.first, 3);
    });

    test("str.upcase.reverse chains correctly", fn() {
        assert_eq("hello".upcase.reverse, "OLLEH");
    });

    test("str.trim.upcase chains correctly", fn() {
        assert_eq("  hello  ".trim.upcase, "HELLO");
    });

    test("hash.keys.length chains correctly", fn() {
        assert_eq({a: 1, b: 2, c: 3}.keys.length, 3);
    });

    test("hash.values.sort chains correctly", fn() {
        assert_eq({a: 3, b: 1, c: 2}.values.sort, [1, 2, 3]);
    });

    test("chain auto-invoke with explicit call", fn() {
        let result = [3, 1, 2].sort.map(fn(x) { x * 10 });
        assert_eq(result, [10, 20, 30]);
    });
});

describe("Safe navigation with auto-invoke", fn() {
    test("null&.method returns null", fn() {
        let x = null;
        assert_null(x&.length);
    });

    test("non-null&.method auto-invokes", fn() {
        let arr = [1, 2, 3];
        assert_eq(arr&.length, 3);
    });

    test("safe nav with chaining", fn() {
        let arr = [3, 1, 2];
        assert_eq(arr&.sort&.first, 1);
    });
});
