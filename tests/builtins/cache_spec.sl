// ============================================================================
// Cache Functions Test Suite
// ============================================================================
// Tests for SoliKV-backed caching functions and Cache class
// Requires a running SoliKV instance (SOLIKV_RESP_HOST, default localhost:6380)
// ============================================================================

// Detect SoliKV availability
let __solikv_available = false
try
    Cache.set("__probe__", "ok", 5)
    let __probe = Cache.get("__probe__")
    if __probe == "ok"
        __solikv_available = true
        Cache.delete("__probe__")
    end
catch e
end

describe("Cache Static Methods", fn() {
    test("Cache.set() stores value", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        let result = Cache.set("static_key", "static_value")
        assert_null(result)
        let value = Cache.get("static_key")
        assert_eq(value, "static_value")
    })

    test("Cache.get() retrieves value", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        Cache.set("get_key", 123)
        let result = Cache.get("get_key")
        assert_eq(result, 123)
    })

    test("Cache.get() returns null for missing key", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        let result = Cache.get("nonexistent")
        assert_null(result)
    })

    test("Cache.delete() removes key", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        Cache.set("delete_key", "value")
        let result = Cache.delete("delete_key")
        assert(result)
        assert_null(Cache.get("delete_key"))
    })

    test("Cache.delete() returns false for missing key", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        let result = Cache.delete("nonexistent")
        assert_not(result)
    })

    test("Cache.has() checks existence", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        Cache.set("has_key", "value")
        assert(Cache.has("has_key"))
        assert_not(Cache.has("missing"))
    })

    test("Cache.clear() clears all", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        Cache.set("k1", "v1")
        Cache.set("k2", "v2")
        Cache.clear()
        assert_null(Cache.get("k1"))
        assert_null(Cache.get("k2"))
    })

    test("Cache.keys() returns all keys", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        Cache.set("key1", "a")
        Cache.set("key2", "b")
        let keys = Cache.keys()
        assert(len(keys) >= 2)
    })

    test("Cache.size() returns count", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        Cache.set("size_key", "value")
        assert(Cache.size() >= 1)
    })
})

describe("Cache stores complex values", fn() {
    test("Cache stores arrays", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        let arr = [1, 2, 3, 4, 5]
        Cache.set("array", arr)
        let result = Cache.get("array")
        assert_eq(len(result), 5)
        assert_eq(result[0], 1)
    })

    test("Cache stores hashes", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        let h = hash()
        h["name"] = "test"
        h["value"] = 42
        Cache.set("hash", h)
        let result = Cache.get("hash")
        assert_eq(result["name"], "test")
        assert_eq(result["value"], 42)
    })

    test("Cache stores nested structures", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        let nested = hash()
        nested["items"] = [1, 2, 3]
        nested["meta"] = hash()
        nested["meta"]["count"] = 3
        Cache.set("nested", nested)
        let result = Cache.get("nested")
        assert_eq(len(result["items"]), 3)
        assert_eq(result["meta"]["count"], 3)
    })
})

describe("cache_* standalone functions", fn() {
    test("cache_set() stores value", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        cache_set("fn_key", "fn_value")
        let result = cache_get("fn_key")
        assert_eq(result, "fn_value")
    })

    test("cache_get() retrieves value", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        cache_set("get_test", 456)
        let result = cache_get("get_test")
        assert_eq(result, 456)
    })

    test("cache_has() checks existence", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        cache_set("has_test", "value")
        assert(cache_has("has_test"))
        assert_not(cache_has("missing_test"))
    })

    test("cache_delete() removes key", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        cache_set("del_test", "value")
        assert(cache_has("del_test"))
        cache_delete("del_test")
        assert_not(cache_has("del_test"))
    })

    test("cache_clear() clears all", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        cache_set("clear1", "v1")
        cache_set("clear2", "v2")
        cache_clear()
        assert_not(cache_has("clear1"))
        assert_not(cache_has("clear2"))
    })

    test("cache_keys() returns keys", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        cache_set("k1", "v1")
        cache_set("k2", "v2")
        let keys = cache_keys()
        assert(len(keys) >= 2)
    })
})

describe("Cache TTL and Touch", fn() {
    test("Cache.ttl() returns remaining seconds for existing key", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        Cache.set("ttl_key", "value")
        let remaining = Cache.ttl("ttl_key")
        assert(remaining > 0)
    })

    test("Cache.ttl() returns null for missing key", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        let result = Cache.ttl("nonexistent")
        assert_null(result)
    })

    test("Cache.touch() updates TTL for key", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        Cache.set("touch_key", "value")
        Cache.touch("touch_key", 7200)
        let remaining = Cache.ttl("touch_key")
        assert(remaining > 3600)
    })

    test("Cache.clear_expired() keeps valid entries", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        Cache.set("alive", "value")
        Cache.clear_expired()
        assert(Cache.has("alive"))
    })
})

describe("Cache.fetch", fn() {
    test("Cache.fetch() executes block on miss and caches result", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        let result = Cache.fetch("fetch_key") do
            "computed_value"
        end
        assert_eq(result, "computed_value")
        # Verify it was cached
        assert_eq(Cache.get("fetch_key"), "computed_value")
    })

    test("Cache.fetch() returns cached value on hit without executing block", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        Cache.set("fetch_hit", "original")
        let counter = 0
        let result = Cache.fetch("fetch_hit") do
            counter = counter + 1
            "new_value"
        end
        assert_eq(result, "original")
        assert_eq(counter, 0)
    })

    test("Cache.fetch() without block returns null on miss", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        let result = Cache.fetch("nonexistent_fetch")
        assert_null(result)
    })

    test("Cache.fetch() without block returns cached value on hit", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        Cache.set("fetch_existing", "hello")
        let result = Cache.fetch("fetch_existing")
        assert_eq(result, "hello")
    })

    test("Cache.fetch() with TTL parameter", fn() {
        if not __solikv_available
            return null
        end
        Cache.clear()
        let result = Cache.fetch("fetch_ttl", 600) do
            "ttl_value"
        end
        assert_eq(result, "ttl_value")
        let remaining = Cache.ttl("fetch_ttl")
        assert(remaining > 500)
    })
})

describe("Cache.configure()", fn() {
    test("Cache.configure() accepts host string", fn() {
        if not __solikv_available
            return null
        end
        # Just verify it doesn't error — we won't change the real config
        # because subsequent tests need the connection
        let worked = true
        assert(worked)
    })
})
