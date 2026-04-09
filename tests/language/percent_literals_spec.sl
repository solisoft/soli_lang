// ============================================================================
// Percent Literal Arrays Test Suite
// ============================================================================

describe("%w[] String Array Literals", fn() {
    test("basic string array", fn() {
        let arr = %w[demo test]
        assert_eq(len(arr), 2)
        assert_eq(arr[0], "demo")
        assert_eq(arr[1], "test")
    });

    test("three elements", fn() {
        let arr = %w[foo bar baz]
        assert_eq(arr, ["foo", "bar", "baz"])
    });

    test("empty array", fn() {
        let arr = %w[]
        assert_eq(len(arr), 0)
        assert_eq(arr, [])
    });

    test("single element", fn() {
        let arr = %w[hello]
        assert_eq(len(arr), 1)
        assert_eq(arr[0], "hello")
    });

    test("equivalent to regular array", fn() {
        assert_eq(%w[a b c], ["a", "b", "c"])
    });

    test("elements separated by multiple spaces", fn() {
        let arr = %w[one  two   three]
        assert_eq(len(arr), 3)
        assert_eq(arr[0], "one")
        assert_eq(arr[1], "two")
        assert_eq(arr[2], "three")
    });

    test("multiline array", fn() {
        let arr = %w[
            one
            two
            three
        ]
        assert_eq(len(arr), 3)
        assert_eq(arr[0], "one")
        assert_eq(arr[1], "two")
        assert_eq(arr[2], "three")
    });

    test("works with array methods", fn() {
        let arr = %w[hello world]
        let upper = arr.map(fn(s) s.upcase())
        assert_eq(upper, ["HELLO", "WORLD"])
    });

    test("works with spread operator", fn() {
        let arr = %w[one two]
        let combined = [...arr, "three"]
        assert_eq(combined, ["one", "two", "three"])
    });

    test("in assignment", fn() {
        let words = %w[apple banana cherry]
        assert_eq(words[1], "banana")
    });
});

describe("%i[] Symbol Array Literals", fn() {
    test("basic symbol array", fn() {
        let arr = %i[demo test]
        assert_eq(len(arr), 2)
        assert_eq(arr[0], :demo)
        assert_eq(arr[1], :test)
    });

    test("three elements", fn() {
        let arr = %i[get post put delete]
        assert_eq(len(arr), 4)
        assert_eq(arr[0], :get)
        assert_eq(arr[1], :post)
        assert_eq(arr[2], :put)
        assert_eq(arr[3], :delete)
    });

    test("empty array", fn() {
        let arr = %i[]
        assert_eq(len(arr), 0)
        assert_eq(arr, [])
    });

    test("single element", fn() {
        let arr = %i[hello]
        assert_eq(len(arr), 1)
        assert_eq(arr[0], :hello)
    });

    test("equivalent to regular symbol array", fn() {
        assert_eq(%i[a b c], [:a, :b, :c])
    });

    test("multiline array", fn() {
        let arr = %i[
            read
            write
            execute
        ]
        assert_eq(len(arr), 3)
        assert_eq(arr[0], :read)
        assert_eq(arr[1], :write)
        assert_eq(arr[2], :execute)
    });

    test("elements are symbols not strings", fn() {
        let arr = %i[foo bar]
        assert_eq(arr[0].class, "symbol")
        assert_eq(arr[1].class, "symbol")
    });

    test("symbols work with methods", fn() {
        let arr = %i[read write execute]
        assert(arr[0] == :read)
        assert(arr[1] == :write)
        assert(arr[2] == :execute)
    });
});

describe("%n[] Number Array Literals", fn() {
    test("basic number array", fn() {
        let arr = %n[1 2 3]
        assert_eq(len(arr), 3)
        assert_eq(arr[0], 1)
        assert_eq(arr[1], 2)
        assert_eq(arr[2], 3)
    });

    test("with floats", fn() {
        let arr = %n[1.5 2.5 3.5]
        assert_eq(len(arr), 3)
        assert_eq(arr[0], 1.5)
        assert_eq(arr[1], 2.5)
        assert_eq(arr[2], 3.5)
    });

    test("mixed integers and floats", fn() {
        let arr = %n[1 2.5 3]
        assert_eq(len(arr), 3)
        assert_eq(arr[0], 1)
        assert_eq(arr[1], 2.5)
        assert_eq(arr[2], 3)
    });

    test("empty array", fn() {
        let arr = %n[]
        assert_eq(len(arr), 0)
        assert_eq(arr, [])
    });

    test("single element", fn() {
        let arr = %n[42]
        assert_eq(len(arr), 1)
        assert_eq(arr[0], 42)
    });

    test("equivalent to regular number array", fn() {
        assert_eq(%n[1 2 3], [1, 2, 3])
    });

    test("multiline array", fn() {
        let arr = %n[
            10
            20
            30
        ]
        assert_eq(len(arr), 3)
        assert_eq(arr[0], 10)
        assert_eq(arr[1], 20)
        assert_eq(arr[2], 30)
    });

    test("elements are numbers", fn() {
        let arr = %n[1 2 3]
        assert_eq(arr[0].class, "int")
        assert_eq(arr[1].class, "int")
    });

    test("with array methods", fn() {
        let arr = %n[1 2 3 4 5]
        let sum = arr.reduce(fn(acc, x) acc + x, 0)
        assert_eq(sum, 15)
    });

    test("in arithmetic operations", fn() {
        let arr = %n[10 20 30]
        assert_eq(arr[0] + arr[1], 30)
        assert_eq(arr[2] - arr[0], 20)
    });

    test("negative numbers", fn() {
        let arr = %n[-5 0 5]
        assert_eq(len(arr), 3)
        assert_eq(arr[0], -5)
        assert_eq(arr[1], 0)
        assert_eq(arr[2], 5)
    });

    test("integers vs floats have correct classes", fn() {
        let ints = %n[1 2 3]
        assert_eq(ints[0].class, "int")
        assert_eq(ints[1].class, "int")
        assert_eq(ints[2].class, "int")

        let floats = %n[1.5 2.5 3.5]
        assert_eq(floats[0].class, "float")
        assert_eq(floats[1].class, "float")
        assert_eq(floats[2].class, "float")

        let mixed = %n[1 2.5 3]
        assert_eq(mixed[0].class, "int")
        assert_eq(mixed[1].class, "float")
        assert_eq(mixed[2].class, "int")
    });

    test("decimals with D suffix", fn() {
        let arr = %n[1.5D 2.5D 3D]
        assert_eq(len(arr), 3)
        assert_eq(arr[0].class, "decimal")
        assert_eq(arr[1].class, "decimal")
        assert_eq(arr[2].class, "decimal")
    });

    test("mixed int float decimal", fn() {
        let arr = %n[1 2.5 3.5D]
        assert_eq(arr[0].class, "int")
        assert_eq(arr[1].class, "float")
        assert_eq(arr[2].class, "decimal")
    });

    test("decimals in arithmetic", fn() {
        let arr = %n[1.5D 2.5D]
        assert_eq(arr[0].class, "decimal")
        assert_eq(arr[1].class, "decimal")
    });
});

describe("Percent Literals with Symbols", fn() {
    test("symbols used as hash keys", fn() {
        let keys = %i[name email phone]
        let h = {}
        keys.each(fn(k) { h[k] = "value" })
        assert_eq(h[:name], "value")
        assert_eq(h[:email], "value")
        assert_eq(h[:phone], "value")
    });

    test("symbols as method names", fn() {
        let actions = %i[before_save after_create]
        assert_eq(actions[0], :before_save)
        assert_eq(actions[1], :after_create)
    });
});

describe("Percent Literals Edge Cases", fn() {
    test("with underscores in words", fn() {
        let arr = %w[hello_world foo_bar]
        assert_eq(arr[0], "hello_world")
        assert_eq(arr[1], "foo_bar")
    });

    test("with numbers in words", fn() {
        let arr = %w[test1 test2 test3]
        assert_eq(arr, ["test1", "test2", "test3"])
    });

    test("tabs as separators", fn() {
        let arr = %w[one	two	three]
        assert_eq(len(arr), 3)
    });

    test("mixed newlines and spaces", fn() {
        let arr = %w[
            first
            second   third
        ]
        assert_eq(len(arr), 3)
    });
});

describe("Percent Literals in Context", fn() {
    test("in function return", fn() {
        fn get_tags() {
            return %w[ruby javascript python]
        }
        let tags = get_tags()
        assert_eq(len(tags), 3)
        assert_eq(tags[0], "ruby")
    });

    test("in conditional expression", fn() {
        let env = "staging"
        let allowed = %w[dev staging prod]
        let is_allowed = allowed.includes?(env) ? "yes" : "no"
        assert_eq(is_allowed, "yes")
    });

    test("chained with array methods", fn() {
        let words = %w[hello world foo bar]
        let result = words
            .filter(fn(w) w.length > 3)
            .map(fn(w) w.upcase())
        assert_eq(result, ["HELLO", "WORLD"])
    });

    test("as constant", fn() {
        const ENVIRONMENTS = %w[development staging production]
        assert_eq(len(ENVIRONMENTS), 3)
        assert_eq(ENVIRONMENTS[0], "development")
    });

    test("nested percent literals", fn() {
        let arr = [%w[a b], %w[c d]]
        assert_eq(arr[0], ["a", "b"])
        assert_eq(arr[1], ["c", "d"])
    });
});
