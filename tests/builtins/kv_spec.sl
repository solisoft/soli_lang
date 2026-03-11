// ============================================================================
// KV Class Test Suite
// ============================================================================
// Tests for SoliKV key-value operations via the KV class
// Requires a running SoliKV instance (SOLIKV_RESP_HOST, default localhost:6380)
// ============================================================================

// Detect SoliKV availability
let __solikv_available = false
try
    let __pong = KV.ping()
    __solikv_available = true
catch e
end

fn skip_unless_solikv()
    if not __solikv_available
        return null
    end
end

describe("KV basic operations", fn() {
    test("KV.set() and KV.get() round-trip", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:basic", "hello")
        let result = KV.get("test:basic")
        assert_eq(result, "hello")
        KV.delete("test:basic")
    })

    test("KV.get() returns null for missing key", fn() {
        if not __solikv_available
            return null
        end
        let result = KV.get("test:nonexistent_key_xyz")
        assert_null(result)
    })

    test("KV.set() stores integers as strings", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:int", 42)
        let result = KV.get("test:int")
        assert_eq(result, "42")
        KV.delete("test:int")
    })

    test("KV.set() with TTL", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:ttl", "expires", 60)
        let ttl = KV.ttl("test:ttl")
        assert(ttl > 0)
        assert(ttl <= 60)
        KV.delete("test:ttl")
    })

    test("KV.delete() removes key", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:del", "value")
        let removed = KV.delete("test:del")
        assert(removed)
        assert_null(KV.get("test:del"))
    })

    test("KV.delete() returns false for missing key", fn() {
        if not __solikv_available
            return null
        end
        let removed = KV.delete("test:nonexistent_del_xyz")
        assert_not(removed)
    })

    test("KV.exists() checks key existence", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:exists", "yes")
        assert(KV.exists("test:exists"))
        KV.delete("test:exists")
        assert_not(KV.exists("test:exists"))
    })
})

describe("KV key management", fn() {
    test("KV.keys() returns matching keys", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:keys:a", "1")
        KV.set("test:keys:b", "2")
        let keys = KV.keys("test:keys:*")
        assert(len(keys) >= 2)
        KV.delete("test:keys:a")
        KV.delete("test:keys:b")
    })

    test("KV.ttl() returns null for missing key", fn() {
        if not __solikv_available
            return null
        end
        let result = KV.ttl("test:missing_ttl_xyz")
        assert_null(result)
    })

    test("KV.expire() sets TTL on existing key", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:expire", "value")
        let ok = KV.expire("test:expire", 120)
        assert(ok)
        let ttl = KV.ttl("test:expire")
        assert(ttl > 0)
        assert(ttl <= 120)
        KV.delete("test:expire")
    })

    test("KV.persist() removes TTL", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:persist", "value", 60)
        KV.persist("test:persist")
        let ttl = KV.ttl("test:persist")
        assert_null(ttl)
        KV.delete("test:persist")
    })

    // KV.rename() skipped — not supported by SoliKV
})

describe("KV numeric operations", fn() {
    test("KV.incr() increments by 1", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:counter")
        let val = KV.incr("test:counter")
        assert_eq(val, 1)
        let val2 = KV.incr("test:counter")
        assert_eq(val2, 2)
        KV.delete("test:counter")
    })

    test("KV.decr() decrements by 1", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:decr")
        KV.set("test:decr", "10")
        let val = KV.decr("test:decr")
        assert_eq(val, 9)
        KV.delete("test:decr")
    })

    test("KV.incrby() increments by amount", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:incrby")
        let val = KV.incrby("test:incrby", 5)
        assert_eq(val, 5)
        let val2 = KV.incrby("test:incrby", 3)
        assert_eq(val2, 8)
        KV.delete("test:incrby")
    })

    test("KV.decrby() decrements by amount", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:decrby")
        KV.incrby("test:decrby", 10)
        let val = KV.decrby("test:decrby", 3)
        assert_eq(val, 7)
        KV.delete("test:decrby")
    })
})

describe("KV list operations", fn() {
    test("KV.lpush() and KV.rpush() add to list", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:list")
        KV.rpush("test:list", "a")
        KV.rpush("test:list", "b")
        KV.lpush("test:list", "z")
        let length = KV.llen("test:list")
        assert_eq(length, 3)
        KV.delete("test:list")
    })

    test("KV.lpop() and KV.rpop() remove from list", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:list2")
        KV.rpush("test:list2", "a")
        KV.rpush("test:list2", "b")
        KV.rpush("test:list2", "c")
        let first = KV.lpop("test:list2")
        assert_eq(first, "a")
        let last = KV.rpop("test:list2")
        assert_eq(last, "c")
        KV.delete("test:list2")
    })

    test("KV.lrange() returns range of elements", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:range")
        KV.rpush("test:range", "a")
        KV.rpush("test:range", "b")
        KV.rpush("test:range", "c")
        let all = KV.lrange("test:range", 0, -1)
        assert_eq(len(all), 3)
        KV.delete("test:range")
    })

    test("KV.llen() returns list length", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:llen")
        KV.rpush("test:llen", "x")
        KV.rpush("test:llen", "y")
        assert_eq(KV.llen("test:llen"), 2)
        KV.delete("test:llen")
    })
})

describe("KV set operations", fn() {
    test("KV.sadd() and KV.smembers()", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:set")
        KV.sadd("test:set", "a")
        KV.sadd("test:set", "b")
        KV.sadd("test:set", "a") # duplicate
        let members = KV.smembers("test:set")
        assert_eq(len(members), 2)
        KV.delete("test:set")
    })

    test("KV.sismember() checks membership", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:set2")
        KV.sadd("test:set2", "x")
        assert(KV.sismember("test:set2", "x"))
        assert_not(KV.sismember("test:set2", "y"))
        KV.delete("test:set2")
    })

    test("KV.srem() removes members", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:set3")
        KV.sadd("test:set3", "a")
        KV.sadd("test:set3", "b")
        KV.srem("test:set3", "a")
        assert_eq(KV.scard("test:set3"), 1)
        KV.delete("test:set3")
    })

    test("KV.scard() returns set size", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:scard")
        KV.sadd("test:scard", "1")
        KV.sadd("test:scard", "2")
        KV.sadd("test:scard", "3")
        assert_eq(KV.scard("test:scard"), 3)
        KV.delete("test:scard")
    })
})

describe("KV hash operations", fn() {
    test("KV.hset() and KV.hget()", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:hmap")
        KV.hset("test:hmap", "field1", "value1")
        let val = KV.hget("test:hmap", "field1")
        assert_eq(val, "value1")
        KV.delete("test:hmap")
    })

    test("KV.hgetall() returns all fields", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:hmap2")
        KV.hset("test:hmap2", "name", "Alice")
        KV.hset("test:hmap2", "age", "30")
        let all = KV.hgetall("test:hmap2")
        assert_eq(all["name"], "Alice")
        assert_eq(all["age"], "30")
        KV.delete("test:hmap2")
    })

    test("KV.hdel() removes hash fields", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:hmap3")
        KV.hset("test:hmap3", "a", "1")
        KV.hset("test:hmap3", "b", "2")
        KV.hdel("test:hmap3", "a")
        assert_eq(KV.hlen("test:hmap3"), 1)
        KV.delete("test:hmap3")
    })

    test("KV.hexists() checks field existence", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:hmap4")
        KV.hset("test:hmap4", "exists", "yes")
        assert(KV.hexists("test:hmap4", "exists"))
        assert_not(KV.hexists("test:hmap4", "nope"))
        KV.delete("test:hmap4")
    })

    test("KV.hkeys() returns field names", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:hmap5")
        KV.hset("test:hmap5", "x", "1")
        KV.hset("test:hmap5", "y", "2")
        let keys = KV.hkeys("test:hmap5")
        assert_eq(len(keys), 2)
        KV.delete("test:hmap5")
    })

    test("KV.hlen() returns field count", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:hmap6")
        KV.hset("test:hmap6", "a", "1")
        KV.hset("test:hmap6", "b", "2")
        KV.hset("test:hmap6", "c", "3")
        assert_eq(KV.hlen("test:hmap6"), 3)
        KV.delete("test:hmap6")
    })
})

describe("KV server commands", fn() {
    test("KV.ping() returns PONG", fn() {
        if not __solikv_available
            return null
        end
        let result = KV.ping()
        assert(result != null)
    })

    test("KV.dbsize() returns a number", fn() {
        if not __solikv_available
            return null
        end
        let size = KV.dbsize()
        assert(size >= 0)
    })

    test("KV.cmd() runs raw commands", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:raw", "hello")
        let result = KV.cmd("GET", "test:raw")
        assert(result != null)
        KV.delete("test:raw")
    })
})
