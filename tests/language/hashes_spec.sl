// ============================================================================
// Hashes Test Suite
// ============================================================================

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

    test("hash literal as last expression (implicit return)", fn() {
        fn make_response() {
            { "status": 200, "body": "ok" }
        }
        let r = make_response();
        assert_eq(r["status"], 200);
        assert_eq(r["body"], "ok");
    });

    test("empty hash as last expression", fn() {
        fn empty() {
            {}
        }
        assert_eq(empty(), {});
    });

    // Dot notation tests
    test("dot notation access", fn() {
        let person = {"name": "Alice", "age": 30};
        assert_eq(person.name, "Alice");
        assert_eq(person.age, 30);
    });

    test("dot notation nested access", fn() {
        let user = {"profile": {"email": "alice@example.com"}};
        assert_eq(user.profile.email, "alice@example.com");
    });

    test("dot notation assignment", fn() {
        let person = {"name": "Alice"};
        person.name = "Bob";
        assert_eq(person.name, "Bob");
    });

    test("dot notation add new key", fn() {
        let person = {"name": "Alice"};
        person.age = 30;
        assert_eq(person.name, "Alice");
        assert_eq(person.age, 30);
    });

    test("dot notation with method chaining", fn() {
        let data = {"items": [1, 2, 3]};
        assert_eq(data.items.length, 3);
    });

    // Safe navigation tests
    test("safe navigation with &.", fn() {
        let user = {"profile": null};
        assert_eq(user&.profile&.email, null);
    });

    test("safe navigation returns null on missing key", fn() {
        let user = {"name": "Alice"};
        assert_eq(user&.email, null);
    });

    // Hash methods tests
    test("hash length method", fn() {
        let h = {"a": 1, "b": 2};
        assert_eq(h.length, 2);
    });

    test("hash len method", fn() {
        let h = {"a": 1, "b": 2, "c": 3};
        assert_eq(h.len(), 3);
    });

    test("hash keys method", fn() {
        let h = {"a": 1, "b": 2};
        let k = h.keys;
        assert_eq(k.contains("a"), true);
        assert_eq(k.contains("b"), true);
    });

    test("hash values method", fn() {
        let h = {"a": 1, "b": 2};
        let v = h.values;
        assert_eq(v.contains(1), true);
        assert_eq(v.contains(2), true);
    });

    test("hash has_key method", fn() {
        let h = {"a": 1, "b": 2};
        assert(h.has_key("a"));
        assert(!h.has_key("c"));
    });

    test("hash delete method", fn() {
        let h = {"a": 1, "b": 2};
        h.delete("a");
        assert_eq(h.length, 1);
        assert_eq(h["a"], null);
    });

    test("hash merge method", fn() {
        let h1 = {"a": 1, "b": 2};
        let h2 = {"b": 3, "c": 4};
        let merged = h1.merge(h2);
        assert_eq(merged["a"], 1);
        assert_eq(merged["b"], 3);
        assert_eq(merged["c"], 4);
    });

    test("hash clear method", fn() {
        let h = {"a": 1, "b": 2};
        h.clear;
        assert_eq(h.length, 0);
    });
});
