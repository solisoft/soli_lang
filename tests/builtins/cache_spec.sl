// ============================================================================
// Cache Functions Test Suite
// ============================================================================
// Tests for in-memory caching functions and Cache class
// ============================================================================

describe("Cache Static Methods", fn() {
    test("Cache.set() stores value", fn() {
        Cache.clear();
        let result = Cache.set("static_key", "static_value");
        assert_null(result);
        let value = Cache.get("static_key");
        assert_eq(value, "static_value");
    });

    test("Cache.get() retrieves value", fn() {
        Cache.clear();
        Cache.set("get_key", 123);
        let result = Cache.get("get_key");
        assert_eq(result, 123);
    });

    test("Cache.get() returns null for missing key", fn() {
        Cache.clear();
        let result = Cache.get("nonexistent");
        assert_null(result);
    });

    test("Cache.delete() removes key", fn() {
        Cache.clear();
        Cache.set("delete_key", "value");
        let result = Cache.delete("delete_key");
        assert(result);
        assert_null(Cache.get("delete_key"));
    });

    test("Cache.delete() returns false for missing key", fn() {
        Cache.clear();
        let result = Cache.delete("nonexistent");
        assert_not(result);
    });

    test("Cache.has() checks existence", fn() {
        Cache.clear();
        Cache.set("has_key", "value");
        assert(Cache.has("has_key"));
        assert_not(Cache.has("missing"));
    });

    test("Cache.clear() clears all", fn() {
        Cache.clear();
        Cache.set("k1", "v1");
        Cache.set("k2", "v2");
        Cache.clear();
        assert_null(Cache.get("k1"));
        assert_null(Cache.get("k2"));
    });

    test("Cache.keys() returns all keys", fn() {
        Cache.clear();
        Cache.set("key1", "a");
        Cache.set("key2", "b");
        let keys = Cache.keys();
        assert(len(keys) >= 2);
    });

    test("Cache.size() returns count", fn() {
        Cache.clear();
        let size = Cache.size();
        Cache.set("size_key", "value");
        assert_eq(Cache.size(), size + 1);
    });

    test("Cache.ttl() returns remaining time", fn() {
        Cache.clear();
        Cache.set("ttl_key", "value", 300);
        let ttl = Cache.ttl("ttl_key");
        assert(ttl > 0);
        assert(ttl <= 300);
    });

    test("Cache.touch() updates TTL", fn() {
        Cache.clear();
        Cache.set("touch_key", "value", 300);
        let before = Cache.ttl("touch_key");
        Cache.touch("touch_key", 600);
        let after = Cache.ttl("touch_key");
        assert(after > before);
    });
});

describe("Cache Instance", fn() {
    test("cache() returns Cache instance", fn() {
        let c = cache();
        assert_not_null(c);
    });

    test("cache instance shares store with static methods", fn() {
        Cache.clear();
        let c = cache();
        Cache.set("shared", "from_static");
        let result = c.get("shared");
        assert_eq(result, "from_static");
        
        c.set("from_instance", 42);
        let result2 = Cache.get("from_instance");
        assert_eq(result2, 42);
    });
});

describe("Cache Values", fn() {
    test("Cache stores string values", fn() {
        Cache.clear();
        Cache.set("string_key", "hello");
        assert_eq(Cache.get("string_key"), "hello");
    });

    test("Cache stores numeric values", fn() {
        Cache.clear();
        Cache.set("int_key", 42);
        Cache.set("float_key", 3.14);
        assert_eq(Cache.get("int_key"), 42);
        assert_eq(Cache.get("float_key"), 3.14);
    });

    test("Cache stores hash values", fn() {
        Cache.clear();
        let data = hash();
        data["name"] = "Alice";
        data["age"] = 30;
        Cache.set("hash_key", data);
        let result = Cache.get("hash_key");
        assert_eq(result["name"], "Alice");
        assert_eq(result["age"], 30);
    });

    test("Cache stores array values", fn() {
        Cache.clear();
        Cache.set("array_key", [1, 2, 3]);
        let result = Cache.get("array_key");
        assert_eq(len(result), 3);
        assert_eq(result[0], 1);
    });

    test("Cache stores with custom TTL", fn() {
        Cache.clear();
        Cache.set("ttl_key", "short_lived", 5);
        assert_eq(Cache.get("ttl_key"), "short_lived");
    });
});

describe("Cache Expiration", fn() {
    test("Cache.has() returns false for expired key", fn() {
        Cache.clear();
        Cache.set("expiring", "value", 1);
        sleep(1.1);
        assert_not(Cache.has("expiring"));
    });

    test("Cache.ttl() returns null for expired key", fn() {
        Cache.clear();
        Cache.set("expiring", "value", 1);
        sleep(1.1);
        let ttl = Cache.ttl("expiring");
        assert_null(ttl);
    });

    test("Cache.clear_expired() removes only expired", fn() {
        Cache.clear();
        Cache.set("permanent", "stays", 3600);
        Cache.set("temporary", "expires", 1);
        sleep(1.1);
        Cache.clear_expired();
        assert_not_null(Cache.get("permanent"));
        assert_null(Cache.get("temporary"));
    });
});

describe("Global Functions (Backward Compatibility)", fn() {
    test("cache_set() works", fn() {
        cache_clear();
        cache_set("global_key", "global_value");
        assert_eq(cache_get("global_key"), "global_value");
    });

    test("cache_get() works", fn() {
        cache_clear();
        cache_set("get_key", 123);
        assert_eq(cache_get("get_key"), 123);
    });

    test("cache_has() works", fn() {
        cache_clear();
        cache_set("has_key", "value");
        assert(cache_has("has_key"));
    });

    test("cache_delete() works", fn() {
        cache_clear();
        cache_set("delete_key", "value");
        assert(cache_delete("delete_key"));
        assert_null(cache_get("delete_key"));
    });

    test("cache_clear() works", fn() {
        cache_clear();
        cache_set("k1", "v1");
        cache_set("k2", "v2");
        cache_clear();
        assert_null(cache_get("k1"));
    });

    test("cache_keys() works", fn() {
        cache_clear();
        cache_set("key1", "a");
        cache_set("key2", "b");
        let keys = cache_keys();
        assert(len(keys) >= 2);
    });

    test("cache_size() works", fn() {
        cache_clear();
        let size = cache_size();
        cache_set("size_key", "value");
        assert_eq(cache_size(), size + 1);
    });

    test("cache_ttl() works", fn() {
        cache_clear();
        cache_set("ttl_key", "value", 300);
        let ttl = cache_ttl("ttl_key");
        assert(ttl > 0);
    });

    test("cache_touch() works", fn() {
        cache_clear();
        cache_set("touch_key", "value", 300);
        let before = cache_ttl("touch_key");
        cache_touch("touch_key", 600);
        let after = cache_ttl("touch_key");
        assert(after > before);
    });

    test("cache_config() works", fn() {
        let result = cache_config(1800, 5000);
        assert_null(result);
    });
});

describe("Cache Configuration", fn() {
    test("Cache shares configuration with global functions", fn() {
        cache_config(1800, 5000);
        Cache.clear();
        Cache.set("config_test", "value");
        assert_eq(cache_size(), 1);
    });
});

describe("Cache Integration", fn() {
    test("static methods and global functions share store", fn() {
        Cache.clear();
        Cache.set("static_key", "from_static");
        let global_val = cache_get("static_key");
        assert_eq(global_val, "from_static");
        
        cache_set("global_key", "from_global");
        let static_val = Cache.get("global_key");
        assert_eq(static_val, "from_global");
    });
});
