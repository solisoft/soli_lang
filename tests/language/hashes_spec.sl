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

    test("hash shift returns first pair", fn() {
        let h = {"a": 1, "b": 2};
        let pair = h.shift();
        assert_eq(pair[0], "a");
        assert_eq(pair[1], 1);
    });

    test("hash shift removes first pair", fn() {
        let h = {"a": 1, "b": 2};
        h.shift();
        assert_eq(h.len(), 1);
    });

    test("hash shift on empty hash", fn() {
        let h = {};
        assert_eq(h.shift(), null);
    });

    test("hash flatten", fn() {
        let h = {"a": 1, "b": 2};
        let flat = h.flatten();
        assert_eq(flat.len(), 2);
        assert_eq(flat[0][0], "a");
        assert_eq(flat[0][1], 1);
        assert_eq(flat[1][0], "b");
        assert_eq(flat[1][1], 2);
    });

    test("hash values_at", fn() {
        let h = {"a": 1, "b": 2, "c": 3};
        let vals = h.values_at("a", "c");
        assert_eq(vals[0], 1);
        assert_eq(vals[1], 3);
    });

    test("hash values_at with missing key returns null", fn() {
        let h = {"a": 1};
        let vals = h.values_at("a", "missing");
        assert_eq(vals[0], 1);
        assert_eq(vals[1], null);
    });

    test("hash key inverse lookup", fn() {
        let h = {"a": 1, "b": 2, "c": 3};
        assert_eq(h.key(2), "b");
    });

    test("hash key returns null if value not found", fn() {
        let h = {"a": 1};
        assert_eq(h.key(99), null);
    });

    test("hash has_value? returns true", fn() {
        let h = {"a": 1, "b": 2};
        assert(h.has_value?(2));
    });

    test("hash has_value? returns false", fn() {
        let h = {"a": 1};
        assert(!h.has_value?(99));
    });

    test("hash value? alias", fn() {
        let h = {"a": 1};
        assert(h.value?(1));
        assert(!h.value?(99));
    });

    test("hash to_h returns self", fn() {
        let h = {"a": 1, "b": 2};
        let h2 = h.to_h();
        assert_eq(h2["a"], 1);
        assert_eq(h2["b"], 2);
    });

    test("hash each_key iterates keys", fn() {
        let h = {"a": 1, "b": 2, "c": 3};
        let keys = [];
        h.each_key(|k| keys.push(k));
        assert_eq(keys.len(), 3);
        assert(keys.contains("a"));
        assert(keys.contains("b"));
        assert(keys.contains("c"));
    });

    test("hash each_value iterates values", fn() {
        let h = {"a": 1, "b": 2, "c": 3};
        let vals = [];
        h.each_value(|v| vals.push(v));
        assert_eq(vals.len(), 3);
        assert(vals.contains(1));
        assert(vals.contains(2));
        assert(vals.contains(3));
    });

    test("hash keep_if keeps matching entries", fn() {
        let h = {"a": 1, "b": 2, "c": 3};
        let kept = h.keep_if(|k, v| v >= 2);
        assert(kept.has_key("b"));
        assert(kept.has_key("c"));
        assert(!kept.has_key("a"));
    });

    test("hash delete_if removes matching entries", fn() {
        let h = {"a": 1, "b": 2, "c": 3};
        let kept = h.delete_if(|k, v| v >= 2);
        assert(kept.has_key("a"));
        assert(!kept.has_key("b"));
        assert(!kept.has_key("c"));
    });

    test("hash update alias for merge", fn() {
        let h = {"a": 1};
        let updated = h.update({"b": 2});
        assert_eq(updated["a"], 1);
        assert_eq(updated["b"], 2);
    });

    test("hash all? returns true when all match", fn() {
        let h = {"a": 2, "b": 4};
        assert(h.all?(|k, v| v % 2 == 0));
    });

    test("hash all? returns false when any fails", fn() {
        let h = {"a": 2, "b": 3};
        assert(!h.all?(|k, v| v % 2 == 0));
    });

    test("hash any? returns true when any matches", fn() {
        let h = {"a": 1, "b": 2};
        assert(h.any?(|k, v| v == 2));
    });

    test("hash any? returns false when none match", fn() {
        let h = {"a": 1, "b": 3};
        assert(!h.any?(|k, v| v == 2));
    });

    test("hash assoc returns pair", fn() {
        let h = {"a": 1, "b": 2};
        let pair = h.assoc("b");
        assert_eq(pair[0], "b");
        assert_eq(pair[1], 2);
    });

    test("hash assoc returns null for missing key", fn() {
        let h = {"a": 1};
        assert_eq(h.assoc("missing"), null);
    });

    test("hash rassoc returns pair by value", fn() {
        let h = {"a": 1, "b": 2};
        let pair = h.rassoc(2);
        assert_eq(pair[0], "b");
        assert_eq(pair[1], 2);
    });

    test("hash rassoc returns null for missing value", fn() {
        let h = {"a": 1};
        assert_eq(h.rassoc(99), null);
    });

    test("hash fetch_values returns values", fn() {
        let h = {"a": 1, "b": 2};
        let vals = h.fetch_values("a", "b");
        assert_eq(vals[0], 1);
        assert_eq(vals[1], 2);
    });
});
