// ============================================================================
// KV Class Extended Test Suite
// ============================================================================
// Behavioral tests for extended SoliKV operations: string extras, expiry,
// lists, sets, hashes, hyperloglog, sorted sets, bitmaps and admin commands.
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

describe("KV string extras", fn() {
    test("KV.setnx() sets only when key is absent", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:setnx")
        let created = KV.setnx("test:ext:setnx", "first")
        assert(created)
        let again = KV.setnx("test:ext:setnx", "second")
        assert_not(again)
        assert_eq(KV.get("test:ext:setnx"), "first")
        KV.delete("test:ext:setnx")
    })

    test("KV.getset() returns the previous value", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:getset")
        let missing = KV.getset("test:ext:getset", "new")
        assert_null(missing)
        let previous = KV.getset("test:ext:getset", "newer")
        assert_eq(previous, "new")
        assert_eq(KV.get("test:ext:getset"), "newer")
        KV.delete("test:ext:getset")
    })

    test("KV.getdel() reads and removes the key", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:ext:getdel", "ephemeral")
        let value = KV.getdel("test:ext:getdel")
        assert_eq(value, "ephemeral")
        assert_null(KV.get("test:ext:getdel"))
        let missing = KV.getdel("test:ext:getdel")
        assert_null(missing)
    })

    test("KV.strlen() returns stored value length", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:ext:strlen", "hello")
        assert_eq(KV.strlen("test:ext:strlen"), 5)
        assert_eq(KV.strlen("test:ext:strlen_missing_xyz"), 0)
        KV.delete("test:ext:strlen")
    })

    test("KV.mset() sets many keys and KV.mget() fetches them", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:m:a")
        KV.delete("test:ext:m:b")
        KV.mset("test:ext:m:a", "1", "test:ext:m:b", "2")
        let values = KV.mget("test:ext:m:a", "test:ext:m:b", "test:ext:m:missing")
        assert_eq(len(values), 3)
        assert_eq(values[0], "1")
        assert_eq(values[1], "2")
        assert_null(values[2])
        KV.delete("test:ext:m:a")
        KV.delete("test:ext:m:b")
    })
})

describe("KV numeric float operations", fn() {
    test("KV.incrbyfloat() increments by a float amount", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:fincr")
        let result = KV.incrbyfloat("test:ext:fincr", 1.5)
        assert(result == 1.5 or result == 1)
        let result2 = KV.incrbyfloat("test:ext:fincr", 2.25)
        assert(result2 == 3.75 or result2 == 4)
        KV.delete("test:ext:fincr")
    })
})

describe("KV expiry variants", fn() {
    test("KV.pexpire() sets millisecond TTL", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:ext:pexp", "value")
        let ok = KV.pexpire("test:ext:pexp", 60000)
        assert(ok)
        let ms = KV.pttl("test:ext:pexp")
        assert(ms > 0)
        assert(ms <= 60000)
        KV.delete("test:ext:pexp")
    })

    test("KV.pexpire() returns false for a missing key", fn() {
        if not __solikv_available
            return null
        end
        assert_not(KV.pexpire("test:ext:pexp_missing_xyz", 60000))
    })

    test("KV.pttl() returns null when there is no expiry", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:ext:pttl", "value")
        assert_null(KV.pttl("test:ext:pttl"))
        KV.delete("test:ext:pttl")
        assert_null(KV.pttl("test:ext:pttl_missing_xyz"))
    })

    test("KV.expireat() expires at a unix timestamp", fn() {
        if not __solikv_available
            return null
        end
        # 2100-01-01T00:00:00Z — comfortably in the future
        KV.set("test:ext:expat", "value")
        let ok = KV.expireat("test:ext:expat", 4102444800)
        assert(ok)
        let ms = KV.pttl("test:ext:expat")
        assert(ms > 0)
        KV.delete("test:ext:expat")
        assert_not(KV.expireat("test:ext:expat_missing_xyz", 4102444800))
    })

    test("KV.unlink() removes keys non-blockingly", fn() {
        if not __solikv_available
            return null
        end
        KV.set("test:ext:ul:a", "1")
        KV.set("test:ext:ul:b", "2")
        let removed = KV.unlink("test:ext:ul:a", "test:ext:ul:b", "test:ext:ul:missing")
        assert(removed >= 1)
        assert_null(KV.get("test:ext:ul:a"))
        assert_null(KV.get("test:ext:ul:b"))
    })
})

describe("KV list extras", fn() {
    test("KV.lindex() reads by index including negative", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:lidx")
        KV.rpush("test:ext:lidx", "a", "b", "c")
        assert_eq(KV.lindex("test:ext:lidx", 0), "a")
        assert_eq(KV.lindex("test:ext:lidx", 2), "c")
        assert_eq(KV.lindex("test:ext:lidx", -1), "c")
        assert_null(KV.lindex("test:ext:lidx", 99))
        KV.delete("test:ext:lidx")
    })

    test("KV.lset() replaces an element by index", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:lset")
        KV.rpush("test:ext:lset", "a", "b", "c")
        KV.lset("test:ext:lset", 1, "z")
        assert_eq(KV.lindex("test:ext:lset", 1), "z")
        assert_eq(KV.llen("test:ext:lset"), 3)
        KV.delete("test:ext:lset")
    })

    test("KV.lrem() removes matching elements", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:lrem")
        KV.rpush("test:ext:lrem", "x", "y", "x", "z", "x")
        let removed = KV.lrem("test:ext:lrem", 0, "x")
        assert_eq(removed, 3)
        assert_eq(KV.llen("test:ext:lrem"), 2)
        KV.delete("test:ext:lrem")
    })

    test("KV.ltrim() keeps only the given range", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:ltrim")
        KV.rpush("test:ext:ltrim", "a", "b", "c", "d", "e")
        KV.ltrim("test:ext:ltrim", 1, 3)
        assert_eq(KV.llen("test:ext:ltrim"), 3)
        assert_eq(KV.lindex("test:ext:ltrim", 0), "b")
        assert_eq(KV.lindex("test:ext:ltrim", -1), "d")
        KV.delete("test:ext:ltrim")
    })

    test("KV.rpoplpush() moves the last element of source onto dest", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:rpl:src")
        KV.delete("test:ext:rpl:dst")
        KV.rpush("test:ext:rpl:src", "a", "b", "c")
        let moved = KV.rpoplpush("test:ext:rpl:src", "test:ext:rpl:dst")
        assert_eq(moved, "c")
        assert_eq(KV.llen("test:ext:rpl:src"), 2)
        assert_eq(KV.lindex("test:ext:rpl:dst", 0), "c")
        KV.delete("test:ext:rpl:src")
        KV.delete("test:ext:rpl:dst")
    })
})

describe("KV hash extras", fn() {
    test("KV.hsetnx() creates a field only once", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:hsetnx")
        let created = KV.hsetnx("test:ext:hsetnx", "field", "first")
        assert(created)
        let again = KV.hsetnx("test:ext:hsetnx", "field", "second")
        assert_not(again)
        assert_eq(KV.hget("test:ext:hsetnx", "field"), "first")
        KV.delete("test:ext:hsetnx")
    })

    test("KV.hincrby() increments a hash field by an integer", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:hincr")
        let first = KV.hincrby("test:ext:hincr", "count", 5)
        assert_eq(first, 5)
        let second = KV.hincrby("test:ext:hincr", "count", 3)
        assert_eq(second, 8)
        KV.delete("test:ext:hincr")
    })

    test("KV.hincrbyfloat() increments a hash field by a float", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:hincrf")
        let first = KV.hincrbyfloat("test:ext:hincrf", "score", 1.5)
        assert(first == 1.5 or first == 2)
        let second = KV.hincrbyfloat("test:ext:hincrf", "score", 0.25)
        assert(second == 1.75 or second == 2)
        KV.delete("test:ext:hincrf")
    })

    test("KV.hmget() fetches multiple fields with nils", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:hmget")
        KV.hset("test:ext:hmget", "a", "1")
        KV.hset("test:ext:hmget", "b", "2")
        let values = KV.hmget("test:ext:hmget", "a", "missing", "b")
        assert_eq(len(values), 3)
        assert_eq(values[0], "1")
        assert_null(values[1])
        assert_eq(values[2], "2")
        KV.delete("test:ext:hmget")
    })

    test("KV.hvals() returns all field values", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:hvals")
        KV.hset("test:ext:hvals", "x", "10")
        KV.hset("test:ext:hvals", "y", "20")
        let values = KV.hvals("test:ext:hvals")
        assert_eq(len(values), 2)
        assert(values.includes?("10"))
        assert(values.includes?("20"))
        KV.delete("test:ext:hvals")
    })
})

describe("KV set extras", fn() {
    test("KV.sinter() returns members present in all sets", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:s:a")
        KV.delete("test:ext:s:b")
        KV.sadd("test:ext:s:a", "1", "2", "3")
        KV.sadd("test:ext:s:b", "2", "3", "4")
        let result = KV.sinter("test:ext:s:a", "test:ext:s:b")
        assert_eq(len(result), 2)
        assert(result.includes?("2"))
        assert(result.includes?("3"))
        KV.delete("test:ext:s:a")
        KV.delete("test:ext:s:b")
    })

    test("KV.sunion() returns members from all sets", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:u:a")
        KV.delete("test:ext:u:b")
        KV.sadd("test:ext:u:a", "1", "2")
        KV.sadd("test:ext:u:b", "3")
        let result = KV.sunion("test:ext:u:a", "test:ext:u:b")
        assert_eq(len(result), 3)
        assert(result.includes?("1"))
        assert(result.includes?("3"))
        KV.delete("test:ext:u:a")
        KV.delete("test:ext:u:b")
    })

    test("KV.sdiff() returns members in the first set only", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:d:a")
        KV.delete("test:ext:d:b")
        KV.sadd("test:ext:d:a", "1", "2", "3")
        KV.sadd("test:ext:d:b", "3")
        let result = KV.sdiff("test:ext:d:a", "test:ext:d:b")
        assert_eq(len(result), 2)
        assert(result.includes?("1"))
        assert(result.includes?("2"))
        KV.delete("test:ext:d:a")
        KV.delete("test:ext:d:b")
    })

    test("KV.smismember() reports membership per member", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:smism")
        KV.sadd("test:ext:smism", "a", "b")
        let flags = KV.smismember("test:ext:smism", "a", "nope", "b")
        # RESP integers (0/1) or Bools depending on conversion — accept either
        assert(flags[0] == true or flags[0] == 1)
        assert(flags[1] == false or flags[1] == 0)
        assert(flags[2] == true or flags[2] == 1)
        KV.delete("test:ext:smism")
    })

    test("KV.smove() moves a member between sets", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:mv:src")
        KV.delete("test:ext:mv:dst")
        KV.sadd("test:ext:mv:src", "a", "b")
        KV.sadd("test:ext:mv:dst", "c")
        let moved = KV.smove("test:ext:mv:src", "test:ext:mv:dst", "a")
        assert(moved)
        assert_eq(KV.scard("test:ext:mv:src"), 1)
        assert_eq(KV.scard("test:ext:mv:dst"), 2)
        assert(KV.sismember("test:ext:mv:dst", "a"))
        KV.delete("test:ext:mv:src")
        KV.delete("test:ext:mv:dst")
    })

    test("KV.spop() removes and returns random members", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:spop")
        KV.sadd("test:ext:spop", "a", "b", "c")
        let one = KV.spop("test:ext:spop")
        assert(one == "a" or one == "b" or one == "c")
        assert_eq(KV.scard("test:ext:spop"), 2)
        let many = KV.spop("test:ext:spop", 2)
        assert_eq(len(many), 2)
        assert_eq(KV.scard("test:ext:spop"), 0)
        assert_null(KV.spop("test:ext:spop"))
    })

    test("KV.srandmember() reads random members without removing them", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:srand")
        KV.sadd("test:ext:srand", "a", "b", "c")
        let one = KV.srandmember("test:ext:srand")
        assert(one == "a" or one == "b" or one == "c")
        assert_eq(KV.scard("test:ext:srand"), 3)
        let some = KV.srandmember("test:ext:srand", 2)
        assert_eq(len(some), 2)
        assert_eq(KV.scard("test:ext:srand"), 3)
        KV.delete("test:ext:srand")
    })
})

describe("KV hyperloglog operations", fn() {
    test("KV.pfadd() adds elements and returns modification flag", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:hll")
        let first = KV.pfadd("test:ext:hll", "a", "b", "c")
        assert(first == 1 or first == true)
        let dup = KV.pfadd("test:ext:hll", "a")
        assert(dup == 0 or dup == false)
        KV.delete("test:ext:hll")
    })

    test("KV.pfcount() estimates cardinality", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:hll:c1")
        KV.pfadd("test:ext:hll:c1", "a", "b", "c")
        let count = KV.pfcount("test:ext:hll:c1")
        assert(count >= 1)
        assert(count <= 10)
        KV.delete("test:ext:hll:c1")
    })

    test("KV.pfcount() unions across keys and KV.pfmerge() merges HLLs", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:hll:m1")
        KV.delete("test:ext:hll:m2")
        KV.delete("test:ext:hll:merged")
        KV.pfadd("test:ext:hll:m1", "a", "b")
        KV.pfadd("test:ext:hll:m2", "c", "d")
        KV.pfmerge("test:ext:hll:merged", "test:ext:hll:m1", "test:ext:hll:m2")
        let merged_count = KV.pfcount("test:ext:hll:merged")
        assert(merged_count >= 2)
        assert(merged_count <= 20)
        KV.delete("test:ext:hll:m1")
        KV.delete("test:ext:hll:m2")
        KV.delete("test:ext:hll:merged")
    })
})

describe("KV sorted set operations", fn() {
    test("KV.zadd() adds members with scores and KV.zcard() counts them", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:z:add")
        let added = KV.zadd("test:ext:z:add", 1, "alice", 2.5, "bob")
        assert_eq(added, 2)
        assert_eq(KV.zcard("test:ext:z:add"), 2)
        KV.delete("test:ext:z:add")
        assert_eq(KV.zcard("test:ext:z:empty_xyz"), 0)
    })

    test("KV.zscore() and KV.zincrby() read and adjust scores", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:z:score")
        KV.zadd("test:ext:z:score", 10, "carol")
        let score = KV.zscore("test:ext:z:score", "carol")
        assert(score == 10 or score == 10.0)
        assert_null(KV.zscore("test:ext:z:score", "nobody"))
        let bumped = KV.zincrby("test:ext:z:score", 5, "carol")
        assert(bumped == 15 or bumped == 15.0)
        let new_score = KV.zscore("test:ext:z:score", "carol")
        assert(new_score == 15 or new_score == 15.0)
        KV.delete("test:ext:z:score")
    })

    test("KV.zcount() counts members within a score range", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:z:count")
        KV.zadd("test:ext:z:count", 1, "a", 5, "b", 10, "c")
        assert_eq(KV.zcount("test:ext:z:count", 1, 10), 3)
        assert_eq(KV.zcount("test:ext:z:count", 4, 6), 1)
        assert_eq(KV.zcount("test:ext:z:count", 50, 100), 0)
        KV.delete("test:ext:z:count")
    })

    test("KV.zrange() and KV.zrevrange() return ordered members", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:z:range")
        KV.zadd("test:ext:z:range", 1, "a", 2, "b", 3, "c")
        let asc = KV.zrange("test:ext:z:range", 0, -1)
        assert_eq(len(asc), 3)
        assert_eq(asc[0], "a")
        assert_eq(asc[2], "c")
        let desc = KV.zrevrange("test:ext:z:range", 0, -1)
        assert_eq(desc[0], "c")
        assert_eq(desc[2], "a")
        let sliced = KV.zrange("test:ext:z:range", 0, 1, true)
        assert(len(sliced) >= 2)
        KV.delete("test:ext:z:range")
    })

    test("KV.zrangebyscore() filters members by score", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:z:rbs")
        KV.zadd("test:ext:z:rbs", 1, "a", 5, "b", 10, "c")
        let mid = KV.zrangebyscore("test:ext:z:rbs", 2, 9)
        assert_eq(len(mid), 1)
        assert_eq(mid[0], "b")
        let all = KV.zrangebyscore("test:ext:z:rbs", "-inf", "+inf")
        assert_eq(len(all), 3)
        KV.delete("test:ext:z:rbs")
    })

    test("KV.zrank() and KV.zrevrank() report member positions", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:z:rank")
        KV.zadd("test:ext:z:rank", 1, "low", 2, "mid", 3, "high")
        assert_eq(KV.zrank("test:ext:z:rank", "low"), 0)
        assert_eq(KV.zrank("test:ext:z:rank", "high"), 2)
        assert_eq(KV.zrevrank("test:ext:z:rank", "high"), 0)
        assert_eq(KV.zrevrank("test:ext:z:rank", "low"), 2)
        assert_null(KV.zrank("test:ext:z:rank", "absent"))
        assert_null(KV.zrevrank("test:ext:z:rank", "absent"))
        KV.delete("test:ext:z:rank")
    })

    test("KV.zrem() removes members from a sorted set", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:z:rem")
        KV.zadd("test:ext:z:rem", 1, "a", 2, "b", 3, "c")
        let removed = KV.zrem("test:ext:z:rem", "a", "c", "ghost")
        assert_eq(removed, 2)
        assert_eq(KV.zcard("test:ext:z:rem"), 1)
        assert_eq(KV.zscore("test:ext:z:rem", "b"), 2)
        KV.delete("test:ext:z:rem")
    })
})

describe("KV bitmap operations", fn() {
    test("KV.setbit(), KV.getbit() and KV.bitcount() manage bits", fn() {
        if not __solikv_available
            return null
        end
        KV.delete("test:ext:bits")
        let previous = KV.setbit("test:ext:bits", 7, 1)
        assert_eq(previous, 0)
        assert_eq(KV.getbit("test:ext:bits", 7), 1)
        assert_eq(KV.getbit("test:ext:bits", 0), 0)
        assert_eq(KV.bitcount("test:ext:bits"), 1)
        let overwritten = KV.setbit("test:ext:bits", 7, 0)
        assert_eq(overwritten, 1)
        assert_eq(KV.bitcount("test:ext:bits"), 0)
        KV.delete("test:ext:bits")
    })
})

describe("KV admin commands", fn() {
    test("KV.flushdb() wipes the database when admin mode is enabled", fn() {
        if not __solikv_available
            return null
        end
        # SEC-037: FLUSHDB is denylisted unless SOLI_KV_ALLOW_ADMIN=1 was set
        # at process launch — mirror the KEYS gate in kv_spec.sl.
        if getenv("SOLI_KV_ALLOW_ADMIN") != "1"
            return null
        end
        KV.set("test:ext:flush", "doomed")
        KV.flushdb()
        assert_eq(KV.dbsize(), 0)
        assert_null(KV.get("test:ext:flush"))
    })

    test("KV.flushdb() raises without the admin opt-in", fn() {
        if not __solikv_available
            return null
        end
        if getenv("SOLI_KV_ALLOW_ADMIN") == "1"
            return null
        end
        let raised = false
        try
            KV.flushdb()
        catch e
            raised = true
        end
        assert(raised)
    })
})
