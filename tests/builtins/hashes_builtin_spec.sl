// ============================================================================
// Hash Functions Test Suite
// ============================================================================

describe("Hash Creation", fn() {
    test("hash() creates empty hash", fn() {
        let h = hash();
        assert_eq(len(h), 0);
    });

    test("empty hash literal", fn() {
        let h = {};
        assert_eq(len(h), 0);
    });

    test("hash literal with entries", fn() {
        let h = {"a" => 1, "b" => 2};
        assert_eq(len(h), 2);
    });

    test("hash with different key types", fn() {
        let h = {
            "string" => "value1",
            123 => "value2",
            true => "value3"
        };
        assert_eq(len(h), 3);
    });
});

describe("Hash Access", fn() {
    test("len() returns hash size", fn() {
        let h = hash();
        h["a"] = 1;
        h["b"] = 2;
        assert_eq(len(h), 2);
    });

    test("hash[key] access", fn() {
        let h = {"name" => "Alice", "age" => 30};
        assert_eq(h["name"], "Alice");
        assert_eq(h["age"], 30);
    });

    test("hash[key] = value assignment", fn() {
        let h = hash();
        h["key"] = "value";
        assert_eq(h["key"], "value");
    });

    test("accessing non-existent key", fn() {
        let h = hash();
        assert_null(h["missing"]);
    });
});

describe("Hash.from_entries", fn() {
    test("creates hash from key-value pairs", fn() {
        let pair1 = ["a", 1];
        let pair2 = ["b", 2];
        let h = Hash.from_entries([pair1, pair2]);
        assert_eq(h["a"], 1);
        assert_eq(h["b"], 2);
        assert_eq(len(h), 2);
    });

    test("roundtrip with entries()", fn() {
        let original = {"x" => 10, "y" => 20, "z" => 30};
        let rebuilt = Hash.from_entries(original.entries());
        assert_eq(rebuilt["x"], 10);
        assert_eq(rebuilt["y"], 20);
        assert_eq(rebuilt["z"], 30);
        assert_eq(len(rebuilt), 3);
    });

    test("empty array creates empty hash", fn() {
        let h = Hash.from_entries([]);
        assert_eq(len(h), 0);
    });
});

describe("Hash Edge Cases", fn() {
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

    test("hash with null values", fn() {
        let h = {"a" => null, "b" => 2};
        assert_null(h["a"]);
        assert_eq(h["b"], 2);
    });

    test("hash key with spaces", fn() {
        let h = {"first name" => "John", "last name" => "Doe"};
        assert_eq(h["first name"], "John");
        assert_eq(h["last name"], "Doe");
    });

    test("hash with boolean keys", fn() {
        let h = {true => "yes", false => "no"};
        assert_eq(h[true], "yes");
        assert_eq(h[false], "no");
    });

    test("hash with integer keys", fn() {
        let h = {1 => "one", 2 => "two"};
        assert_eq(h[1], "one");
        assert_eq(h[2], "two");
    });
});

describe("Hash.dig", fn() {
    test("dig nested hash keys", fn() {
        let h = {"user" => {"profile" => {"name" => "Alice"}}};
        assert_eq(h.dig("user", "profile", "name"), "Alice");
    });

    test("dig returns null for missing key", fn() {
        let h = {"user" => {"name" => "Alice"}};
        assert_null(h.dig("user", "age"));
    });

    test("dig returns null for missing intermediate key", fn() {
        let h = {"user" => {"profile" => {"name" => "Bob"}}};
        assert_null(h.dig("user", "settings", "theme"));
    });

    test("dig with array index", fn() {
        let h = {"items" => [10, 20, 30]};
        assert_eq(h.dig("items", 0), 10);
        assert_eq(h.dig("items", 1), 20);
        assert_eq(h.dig("items", 2), 30);
    });

    test("dig with mixed hash and array", fn() {
        let h = {"users" => [{"name" => "Alice"}, {"name" => "Bob"}]};
        assert_eq(h.dig("users", 0, "name"), "Alice");
        assert_eq(h.dig("users", 1, "name"), "Bob");
    });

    test("dig returns null for out of bounds array index", fn() {
        let h = {"items" => [10, 20]};
        assert_null(h.dig("items", 5));
    });

    test("dig on empty hash returns null", fn() {
        let h = {};
        assert_null(h.dig("missing"));
    });

    test("dig with single key", fn() {
        let h = {"name" => "Alice"};
        assert_eq(h.dig("name"), "Alice");
    });
});
