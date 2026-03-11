// ============================================================================
// Symbol Test Suite
// ============================================================================

describe("Symbol Literals", fn() {
    test("basic symbol creation", fn() {
        let s = :name
        assert_eq(s, :name)
    });

    test("symbol type name", fn() {
        assert_eq(:name.class, "symbol")
    });

    test("symbol is truthy", fn() {
        let result = :name ? true : false
        assert(result)
    });

    test("symbol inspect shows colon prefix", fn() {
        assert_eq(:name.inspect, ":name")
        assert_eq(:hello_world.inspect, ":hello_world")
    });

    test("symbol to_s returns string without colon", fn() {
        assert_eq(:name.to_s, "name")
        assert_eq(:hello.to_string, "hello")
    });

    test("symbol nil? returns false", fn() {
        assert_eq(:name.nil?, false)
    });

    test("symbol with trailing ? and !", fn() {
        let s1 = :empty?
        assert_eq(s1.inspect, ":empty?")
        let s2 = :save!
        assert_eq(s2.inspect, ":save!")
    });
});

describe("Symbol Equality", fn() {
    test("same symbols are equal", fn() {
        assert_eq(:name, :name)
        assert_eq(:foo, :foo)
    });

    test("different symbols are not equal", fn() {
        assert(:name != :age)
    });

    test("symbols are not equal to strings", fn() {
        assert(:name != "name")
        assert("name" != :name)
    });
});

describe("Symbol as Hash Keys", fn() {
    test("symbol keys in hash literals", fn() {
        let h = { :name: "John", :age: 30 }
        assert_eq(h[:name], "John")
        assert_eq(h[:age], 30)
    });

    test("symbol keys distinct from string keys", fn() {
        let h = { :name: "sym_value" }
        h["name"] = "str_value"
        assert_eq(h[:name], "sym_value")
        assert_eq(h["name"], "str_value")
        assert_eq(h.length, 2)
    });

    test("hash with fat arrow and symbol keys", fn() {
        let h = { :x => 1, :y => 2 }
        assert_eq(h[:x], 1)
        assert_eq(h[:y], 2)
    });
});

describe("String.to_sym", fn() {
    test("converts string to symbol", fn() {
        let s = "hello"
        let sym = s.to_sym
        assert_eq(sym, :hello)
        assert_eq(sym.class, "symbol")
    });

    test("round-trip string to symbol to string", fn() {
        let original = "test"
        let sym = original.to_sym
        let back = sym.to_s
        assert_eq(back, original)
    });
});

describe("Symbol with &: shorthand", fn() {
    test("&:method shorthand works with symbols", fn() {
        let arr = [1, 2, 3]
        let strings = arr.map(&:to_s)
        assert_eq(strings, ["1", "2", "3"])
    });

    test("&:method with predicate", fn() {
        let arr = [1, 2, 3, 4, 5, 6]
        let evens = arr.filter(&:even?)
        assert_eq(evens, [2, 4, 6])
    });
});

describe("Ternary with Symbols", fn() {
    test("ternary returns symbols correctly", fn() {
        let result = true ? :yes : :no
        assert_eq(result, :yes)
        let result2 = false ? :yes : :no
        assert_eq(result2, :no)
    });
});
