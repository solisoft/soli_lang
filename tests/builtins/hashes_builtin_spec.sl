// ============================================================================
// Hash Functions Test Suite
// ============================================================================

describe("Hash Creation", fn() {
    test("hash() creates empty hash", fn() {
        let h = hash();
        assert_eq(type(h), "hash");
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

describe("Hash Access Methods", fn() {
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

    test("hash.key access with string key", fn() {
        let h = {name: "Bob"};
        assert_eq(h.name, "Bob");
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

    test(".length() method returns size", fn() {
        let h = {"a" => 1, "b" => 2, "c" => 3};
        assert_eq(h.length(), 3);
    });

    test(".get() retrieves value by key", fn() {
        let h = {"x" => 10, "y" => 20};
        assert_eq(h.get("x"), 10);
        assert_eq(h.get("z"), null);
    });

    test(".get() with default value", fn() {
        let h = {"a" => 1};
        assert_eq(h.get("a"), 1);
    });
});

describe("Hash Modification Methods", fn() {
    test("has_key() checks for key existence", fn() {
        let h = hash();
        h["a"] = 1;
        assert(has_key(h, "a"));
        assert_not(has_key(h, "b"));
    });

    test("delete() removes key from hash", fn() {
        let h = hash();
        h["a"] = 1;
        h["b"] = 2;
        let deleted = delete(h, "a");
        assert_eq(deleted, 1);
        assert_not(has_key(h, "a"));
        assert_eq(len(h), 1);
    });

    test("delete() on non-existent key", fn() {
        let h = hash();
        let deleted = delete(h, "missing");
        assert_null(deleted);
    });

    test(".delete() method removes key", fn() {
        let h = {"a" => 1, "b" => 2};
        let deleted = h.delete("a");
        assert_eq(deleted, 1);
        assert_not(has_key(h, "a"));
    });

    test(".clear() removes all entries", fn() {
        let h = hash();
        h["a"] = 1;
        h["b"] = 2;
        h.clear();
        assert_eq(len(h), 0);
    });

    test(".set() updates or adds key-value", fn() {
        let h = {"a" => 1};
        h.set("a", 10);
        h.set("b", 20);
        assert_eq(h["a"], 10);
        assert_eq(h["b"], 20);
    });
});

describe("Hash Transformation Methods", fn() {
    test("merge() combines two hashes", fn() {
        let h1 = hash();
        h1["a"] = 1;
        let h2 = hash();
        h2["b"] = 2;
        let merged = merge(h1, h2);
        assert_eq(len(merged), 2);
        assert_eq(merged["a"], 1);
        assert_eq(merged["b"], 2);
    });

    test("merge() overwrites duplicate keys", fn() {
        let h1 = {"a" => 1, "b" => 2};
        let h2 = {"b" => 20, "c" => 3};
        let merged = merge(h1, h2);
        assert_eq(merged["a"], 1);
        assert_eq(merged["b"], 20);
        assert_eq(merged["c"], 3);
    });

    test(".merge() method combines hashes", fn() {
        let h1 = {"a" => 1};
        let h2 = {"b" => 2};
        let merged = h1.merge(h2);
        assert_eq(len(merged), 2);
    });
});

describe("Hash Enumeration Methods", fn() {
    test("keys() returns all keys", fn() {
        let h = hash();
        h["a"] = 1;
        h["b"] = 2;
        let k = keys(h);
        assert_eq(len(k), 2);
        assert_contains(k, "a");
        assert_contains(k, "b");
    });

    test("keys() on empty hash", fn() {
        let h = hash();
        let k = keys(h);
        assert_eq(len(k), 0);
    });

    test("values() returns all values", fn() {
        let h = hash();
        h["a"] = 1;
        h["b"] = 2;
        let v = values(h);
        assert_eq(len(v), 2);
        assert_contains(v, 1);
        assert_contains(v, 2);
    });

    test("values() on empty hash", fn() {
        let h = hash();
        let v = values(h);
        assert_eq(len(v), 0);
    });

    test("entries() returns key-value pairs", fn() {
        let h = hash();
        h["a"] = 1;
        let e = entries(h);
        assert_eq(len(e), 1);
        assert_eq(e[0][0], "a");
        assert_eq(e[0][1], 1);
    });

    test("entries() on empty hash", fn() {
        let h = hash();
        let e = entries(h);
        assert_eq(len(e), 0);
    });

    test("from_entries() creates hash from pairs", fn() {
        let pairs = [["a", 1], ["b", 2]];
        let h = from_entries(pairs);
        assert_eq(h["a"], 1);
        assert_eq(h["b"], 2);
    });

    test("from_entries() on empty array", fn() {
        let h = from_entries([]);
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

describe("Hash Assertions", fn() {
    test("assert_hash_has_key works", fn() {
        let h = hash();
        h["key"] = "value";
        assert_hash_has_key(h, "key");
    });
});
