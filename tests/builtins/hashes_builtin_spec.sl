// ============================================================================
// Hash Functions Test Suite
// ============================================================================

describe("Hash Functions", fn() {
    test("hash() creates empty hash", fn() {
        let h = hash();
        assert_eq(type(h), "hash");
        assert_eq(len(h), 0);
    });

    test("len() returns hash size", fn() {
        let h = hash();
        h["a"] = 1;
        h["b"] = 2;
        assert_eq(len(h), 2);
    });

    test("keys() returns all keys", fn() {
        let h = hash();
        h["a"] = 1;
        h["b"] = 2;
        let k = keys(h);
        assert_eq(len(k), 2);
        assert_contains(k, "a");
        assert_contains(k, "b");
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

    test("entries() returns key-value pairs", fn() {
        let h = hash();
        h["a"] = 1;
        let e = entries(h);
        assert_eq(len(e), 1);
        assert_eq(e[0][0], "a");
        assert_eq(e[0][1], 1);
    });

    test("from_entries() creates hash from pairs", fn() {
        let pairs = [["a", 1], ["b", 2]];
        let h = from_entries(pairs);
        assert_eq(h["a"], 1);
        assert_eq(h["b"], 2);
    });

    test("clear() removes all entries", fn() {
        let h = hash();
        h["a"] = 1;
        h["b"] = 2;
        clear(h);
        assert_eq(len(h), 0);
    });

    test("assert_hash_has_key works", fn() {
        let h = hash();
        h["key"] = "value";
        assert_hash_has_key(h, "key");
    });
});
