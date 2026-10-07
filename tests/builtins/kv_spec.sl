# The KV class against a live SoliKV, one describe per data type.
# Every key lives under "test:kv:" and is deleted after each test.

used_keys = []

def kv_key(name)
  key = "test:kv:#{name}"
  used_keys.push(key)
  key
end

def admin_enabled?
  ["1", "true", "yes"].includes?(getenv("SOLI_KV_ALLOW_ADMIN"))
end

describe("KV") do
  before_each() do
    used_keys = []
    requires_solikv()
  end

  after_each() do
    used_keys.each do |key|
      KV.delete(key)
    end
  end

  describe("strings") do
    test("set and get round-trip a string") do
      key = kv_key("greeting")
      assert_null(KV.set(key, "hello"))
      assert_eq(KV.get(key), "hello")
    end

    test("get returns nil for a missing key") do
      assert_null(KV.get(kv_key("missing")))
    end

    test("stores non-string values as their string form") do
      int_key = kv_key("int")
      float_key = kv_key("float")
      bool_key = kv_key("bool")
      KV.set(int_key, 42)
      KV.set(float_key, 1.5)
      KV.set(bool_key, true)
      assert_eq(KV.get(int_key), "42")
      assert_eq(KV.get(float_key), "1.5")
      assert_eq(KV.get(bool_key), "true")
    end

    test("stores an empty string as an existing key") do
      key = kv_key("empty")
      KV.set(key, "")
      assert_eq(KV.get(key), "")
      assert(KV.exists(key))
    end

    test("set with a TTL expires the key in that many seconds") do
      key = kv_key("ttl")
      KV.set(key, "expires", 60)
      ttl = KV.ttl(key)
      assert_gt(ttl, 0)
      assert(ttl <= 60)
    end

    test("set without a TTL clears an earlier one") do
      key = kv_key("ttl_reset")
      KV.set(key, "x", 60)
      KV.set(key, "y")
      assert_null(KV.ttl(key))
      assert_eq(KV.get(key), "y")
    end

    test("set refuses the wrong number of arguments") do
      assert_raises("KV.set() expects 2 or 3 arguments") do
        KV.set("only_a_key")
      end
    end

    test("get refuses a non-string key") do
      assert_raises("KV.get() expects string key, got int") do
        KV.get(42)
      end
    end

    test("setnx sets only when the key is absent") do
      key = kv_key("setnx")
      assert(KV.setnx(key, "first"))
      assert_not(KV.setnx(key, "second"))
      assert_eq(KV.get(key), "first")
    end

    test("getset returns the previous value") do
      key = kv_key("getset")
      assert_null(KV.getset(key, "new"))
      assert_eq(KV.getset(key, "newer"), "new")
      assert_eq(KV.get(key), "newer")
    end

    test("getdel reads and removes the key") do
      key = kv_key("getdel")
      KV.set(key, "ephemeral")
      assert_eq(KV.getdel(key), "ephemeral")
      assert_null(KV.get(key))
      assert_null(KV.getdel(key))
    end

    test("append returns the new length, creating the key when missing") do
      existing = kv_key("append")
      fresh = kv_key("append_new")
      KV.set(existing, "ab")
      assert_eq(KV.append(existing, "cd"), 4)
      assert_eq(KV.get(existing), "abcd")
      assert_eq(KV.append(fresh, "xy"), 2)
      assert_eq(KV.get(fresh), "xy")
    end

    test("strlen returns the stored length, 0 for a missing key") do
      key = kv_key("strlen")
      KV.set(key, "hello")
      assert_eq(KV.strlen(key), 5)
      assert_eq(KV.strlen(kv_key("strlen_missing")), 0)
    end

    test("mset sets many keys and mget fetches them with nil for misses") do
      first = kv_key("m:a")
      second = kv_key("m:b")
      assert_null(KV.mset(first, "1", second, "2"))
      assert_eq(KV.mget(first, second, kv_key("m:missing")), ["1", "2", nil])
    end
  end

  describe("counters") do
    test("incr starts a missing key at 1") do
      key = kv_key("counter")
      assert_eq(KV.incr(key), 1)
      assert_eq(KV.incr(key), 2)
      assert_eq(KV.get(key), "2")
    end

    test("decr decrements a stored number") do
      key = kv_key("decr")
      KV.set(key, "10")
      assert_eq(KV.decr(key), 9)
    end

    test("incrby and decrby move by an amount and can go negative") do
      key = kv_key("incrby")
      assert_eq(KV.incrby(key, 5), 5)
      assert_eq(KV.incrby(key, 3), 8)
      assert_eq(KV.decrby(key, 3), 5)
      assert_eq(KV.decrby(key, 9), -4)
    end

    test("incrbyfloat returns a Float") do
      key = kv_key("float_counter")
      assert_eq(KV.incrbyfloat(key, 1.5), 1.5)
      assert_eq(KV.incrbyfloat(key, 2.25), 3.75)
      assert_eq(KV.incrbyfloat(key, -4.0), -0.25)
      assert_eq(KV.get(key), "-0.25")
    end

    test("incr refuses a value that is not an integer") do
      key = kv_key("not_a_number")
      KV.set(key, "abc")
      assert_raises("not an integer") do
        KV.incr(key)
      end
    end
  end

  describe("keys and expiry") do
    test("delete returns true when it removed the key") do
      key = kv_key("del")
      KV.set(key, "value")
      assert(KV.delete(key))
      assert_null(KV.get(key))
    end

    test("delete returns false for a missing key") do
      assert_not(KV.delete(kv_key("del_missing")))
    end

    test("exists follows the key's lifetime") do
      key = kv_key("exists")
      assert_not(KV.exists(key))
      KV.set(key, "yes")
      assert(KV.exists(key))
      KV.delete(key)
      assert_not(KV.exists(key))
    end

    test("unlink removes several keys and counts only those that existed") do
      first = kv_key("ul:a")
      second = kv_key("ul:b")
      KV.set(first, "1")
      KV.set(second, "2")
      assert_eq(KV.unlink(first, second, kv_key("ul:missing")), 2)
      assert_null(KV.get(first))
      assert_null(KV.get(second))
    end

    test("type names the data type, none for a missing key") do
      string_key = kv_key("type:string")
      list_key = kv_key("type:list")
      set_key = kv_key("type:set")
      hash_key = kv_key("type:hash")
      zset_key = kv_key("type:zset")
      KV.set(string_key, "x")
      KV.rpush(list_key, "a")
      KV.sadd(set_key, "a")
      KV.hset(hash_key, "a", "1")
      KV.zadd(zset_key, 1, "a")
      assert_eq(KV.type(string_key), "string")
      assert_eq(KV.type(list_key), "list")
      assert_eq(KV.type(set_key), "set")
      assert_eq(KV.type(hash_key), "hash")
      assert_eq(KV.type(zset_key), "zset")
      assert_eq(KV.type(kv_key("type:missing")), "none")
    end

    test("rename moves the value to the new key") do
      pending("SoliKV 0.4.3 RENAME answers OK and loses the value — the client sends the right command")
      source = kv_key("rename:src")
      dest = kv_key("rename:dst")
      KV.set(source, "moved")
      KV.rename(source, dest)
      assert_eq(KV.get(dest), "moved")
      assert_not(KV.exists(source))
    end

    test("touch counts the keys that exist") do
      pending("SoliKV 0.4.3 has no TOUCH — the client sends the right command")
      first = kv_key("touch:a")
      second = kv_key("touch:b")
      KV.set(first, "1")
      KV.set(second, "2")
      assert_eq(KV.touch(first, second, kv_key("touch:missing")), 2)
    end

    test("ttl is nil for a missing key and for a key without expiry") do
      key = kv_key("no_ttl")
      KV.set(key, "value")
      assert_null(KV.ttl(key))
      assert_null(KV.ttl(kv_key("ttl_missing")))
    end

    test("expire sets a TTL on an existing key only") do
      key = kv_key("expire")
      KV.set(key, "value")
      assert(KV.expire(key, 120))
      ttl = KV.ttl(key)
      assert_gt(ttl, 0)
      assert(ttl <= 120)
      assert_not(KV.expire(kv_key("expire_missing"), 120))
    end

    test("persist removes the TTL") do
      key = kv_key("persist")
      KV.set(key, "value", 60)
      assert(KV.persist(key))
      assert_null(KV.ttl(key))
      assert_eq(KV.get(key), "value")
      assert_not(KV.persist(kv_key("persist_missing")))
    end

    test("pexpire sets a TTL in milliseconds") do
      key = kv_key("pexpire")
      KV.set(key, "value")
      assert(KV.pexpire(key, 60000))
      milliseconds = KV.pttl(key)
      assert_gt(milliseconds, 0)
      assert(milliseconds <= 60000)
    end

    test("pexpire returns false for a missing key") do
      assert_not(KV.pexpire(kv_key("pexpire_missing"), 60000))
    end

    test("pttl is nil when there is no expiry or no key") do
      key = kv_key("pttl")
      KV.set(key, "value")
      assert_null(KV.pttl(key))
      assert_null(KV.pttl(kv_key("pttl_missing")))
    end

    test("expireat expires at a unix timestamp") do
      key = kv_key("expireat")
      KV.set(key, "value")
      # 2100-01-01T00:00:00Z — comfortably in the future
      assert(KV.expireat(key, 4102444800))
      assert_gt(KV.pttl(key), 0)
      assert_not(KV.expireat(kv_key("expireat_missing"), 4102444800))
    end
  end

  describe("lists") do
    test("lpush and rpush return the new length") do
      key = kv_key("list")
      assert_eq(KV.rpush(key, "a"), 1)
      assert_eq(KV.rpush(key, "b", "c"), 3)
      assert_eq(KV.lpush(key, "z"), 4)
      assert_eq(KV.lrange(key, 0, -1), ["z", "a", "b", "c"])
    end

    test("lpop and rpop remove from each end") do
      key = kv_key("pop")
      KV.rpush(key, "a", "b", "c")
      assert_eq(KV.lpop(key), "a")
      assert_eq(KV.rpop(key), "c")
      assert_eq(KV.lrange(key, 0, -1), ["b"])
    end

    test("lpop and rpop return nil on a missing list") do
      key = kv_key("pop_missing")
      assert_null(KV.lpop(key))
      assert_null(KV.rpop(key))
    end

    test("lrange returns a slice and an empty array for a missing list") do
      key = kv_key("range")
      KV.rpush(key, "a", "b", "c")
      assert_eq(KV.lrange(key, 1, 1), ["b"])
      assert_eq(KV.lrange(key, 0, 1), ["a", "b"])
      assert_eq(KV.lrange(kv_key("range_missing"), 0, -1), [])
    end

    test("llen counts elements, 0 for a missing list") do
      key = kv_key("llen")
      KV.rpush(key, "x", "y")
      assert_eq(KV.llen(key), 2)
      assert_eq(KV.llen(kv_key("llen_missing")), 0)
    end

    test("lindex reads by index, negative from the tail") do
      key = kv_key("lindex")
      KV.rpush(key, "a", "b", "c")
      assert_eq(KV.lindex(key, 0), "a")
      assert_eq(KV.lindex(key, 2), "c")
      assert_eq(KV.lindex(key, -1), "c")
      assert_null(KV.lindex(key, 99))
    end

    test("lset replaces an element by index") do
      key = kv_key("lset")
      KV.rpush(key, "a", "b", "c")
      assert_null(KV.lset(key, 1, "z"))
      assert_eq(KV.lrange(key, 0, -1), ["a", "z", "c"])
    end

    test("lrem with count 0 removes every match") do
      key = kv_key("lrem")
      KV.rpush(key, "x", "y", "x", "z", "x")
      assert_eq(KV.lrem(key, 0, "x"), 3)
      assert_eq(KV.lrange(key, 0, -1), ["y", "z"])
    end

    test("ltrim keeps only the given range") do
      key = kv_key("ltrim")
      KV.rpush(key, "a", "b", "c", "d", "e")
      assert_null(KV.ltrim(key, 1, 3))
      assert_eq(KV.lrange(key, 0, -1), ["b", "c", "d"])
    end

    test("rpoplpush moves the last element of source onto dest") do
      pending("SoliKV 0.4.3 has no RPOPLPUSH — the client sends the right command")
      source = kv_key("rpl:src")
      dest = kv_key("rpl:dst")
      KV.rpush(source, "a", "b", "c")
      assert_eq(KV.rpoplpush(source, dest), "c")
      assert_eq(KV.lrange(source, 0, -1), ["a", "b"])
      assert_eq(KV.lrange(dest, 0, -1), ["c"])
    end

    test("a list command on a string key raises WRONGTYPE") do
      key = kv_key("wrongtype")
      KV.set(key, "x")
      assert_raises("WRONGTYPE") do
        KV.lpush(key, "a")
      end
    end
  end

  describe("sets") do
    test("sadd counts only new members and smembers lists them once") do
      key = kv_key("set")
      assert_eq(KV.sadd(key, "a", "b"), 2)
      assert_eq(KV.sadd(key, "a"), 0)
      assert_eq(KV.smembers(key).sort, ["a", "b"])
    end

    test("smembers is empty for a missing set") do
      assert_eq(KV.smembers(kv_key("set_missing")), [])
    end

    test("sismember checks membership") do
      key = kv_key("sismember")
      KV.sadd(key, "x")
      assert(KV.sismember(key, "x"))
      assert_not(KV.sismember(key, "y"))
      assert_not(KV.sismember(kv_key("sismember_missing"), "x"))
    end

    test("srem removes members and returns how many") do
      key = kv_key("srem")
      KV.sadd(key, "a", "b")
      assert_eq(KV.srem(key, "a"), 1)
      assert_eq(KV.srem(key, "ghost"), 0)
      assert_eq(KV.smembers(key), ["b"])
    end

    test("scard returns the set size, 0 for a missing set") do
      key = kv_key("scard")
      KV.sadd(key, "1", "2", "3")
      assert_eq(KV.scard(key), 3)
      assert_eq(KV.scard(kv_key("scard_missing")), 0)
    end

    test("spop removes and returns random members") do
      key = kv_key("spop")
      KV.sadd(key, "a", "b", "c")
      one = KV.spop(key)
      assert_contains(["a", "b", "c"], one)
      assert_eq(KV.scard(key), 2)
      many = KV.spop(key, 2)
      assert_eq(([one] + many).sort, ["a", "b", "c"])
      assert_eq(KV.scard(key), 0)
      assert_null(KV.spop(key))
    end

    test("srandmember reads random members without removing them") do
      key = kv_key("srandmember")
      KV.sadd(key, "a", "b", "c")
      assert_contains(["a", "b", "c"], KV.srandmember(key))
      some = KV.srandmember(key, 2)
      assert_eq(some.length, 2)
      assert_eq(some.uniq.length, 2)
      assert(some.all? { |member| ["a", "b", "c"].includes?(member) })
      assert_eq(KV.scard(key), 3)
      assert_null(KV.srandmember(kv_key("srandmember_missing")))
    end

    test("sinter returns members present in all sets") do
      pending("SoliKV 0.4.3 SINTER only reads the first key — the client sends the right command")
      first = kv_key("sinter:a")
      second = kv_key("sinter:b")
      KV.sadd(first, "1", "2", "3")
      KV.sadd(second, "2", "3", "4")
      assert_eq(KV.sinter(first, second).sort, ["2", "3"])
    end

    test("sunion returns members from all sets") do
      pending("SoliKV 0.4.3 SUNION only reads the first key — the client sends the right command")
      first = kv_key("sunion:a")
      second = kv_key("sunion:b")
      KV.sadd(first, "1", "2")
      KV.sadd(second, "3")
      assert_eq(KV.sunion(first, second).sort, ["1", "2", "3"])
    end

    test("sdiff returns members in the first set only") do
      pending("SoliKV 0.4.3 SDIFF only reads the first key — the client sends the right command")
      first = kv_key("sdiff:a")
      second = kv_key("sdiff:b")
      KV.sadd(first, "1", "2", "3")
      KV.sadd(second, "3")
      assert_eq(KV.sdiff(first, second).sort, ["1", "2"])
    end

    test("smismember reports membership per member") do
      pending("SoliKV 0.4.3 has no SMISMEMBER — the client sends the right command")
      key = kv_key("smismember")
      KV.sadd(key, "a", "b")
      assert_eq(KV.smismember(key, "a", "nope", "b"), [true, false, true])
    end

    test("smove moves a member between sets") do
      pending("SoliKV 0.4.3 SMOVE does not add to the destination — the client sends the right command")
      source = kv_key("smove:src")
      dest = kv_key("smove:dst")
      KV.sadd(source, "a", "b")
      KV.sadd(dest, "c")
      assert(KV.smove(source, dest, "a"))
      assert_eq(KV.smembers(source), ["b"])
      assert_eq(KV.smembers(dest).sort, ["a", "c"])
    end
  end

  describe("hashes") do
    test("hset and hget round-trip a field") do
      key = kv_key("hash")
      KV.hset(key, "field1", "value1")
      assert_eq(KV.hget(key, "field1"), "value1")
      assert_null(KV.hget(key, "missing"))
    end

    test("hset returns 1 for a new field and 0 for an update") do
      key = kv_key("hset_count")
      assert_eq(KV.hset(key, "field", "v1"), 1)
      assert_eq(KV.hset(key, "field", "v2"), 0)
      assert_eq(KV.hget(key, "field"), "v2")
    end

    test("hgetall returns every field, an empty hash for a missing key") do
      key = kv_key("hgetall")
      KV.hset(key, "name", "Alice")
      KV.hset(key, "age", "30")
      assert_eq(KV.hgetall(key), {"name": "Alice", "age": "30"})
      assert_eq(KV.hgetall(kv_key("hgetall_missing")), {})
    end

    test("hdel removes fields") do
      key = kv_key("hdel")
      KV.hset(key, "a", "1")
      KV.hset(key, "b", "2")
      assert_eq(KV.hdel(key, "a"), 1)
      assert_eq(KV.hkeys(key), ["b"])
    end

    test("hexists checks field existence") do
      key = kv_key("hexists")
      KV.hset(key, "exists", "yes")
      assert(KV.hexists(key, "exists"))
      assert_not(KV.hexists(key, "nope"))
      assert_not(KV.hexists(kv_key("hexists_missing"), "exists"))
    end

    test("hkeys and hvals list names and values") do
      key = kv_key("hkeys")
      KV.hset(key, "x", "10")
      KV.hset(key, "y", "20")
      assert_eq(KV.hkeys(key).sort, ["x", "y"])
      assert_eq(KV.hvals(key).sort, ["10", "20"])
      assert_eq(KV.hkeys(kv_key("hkeys_missing")), [])
    end

    test("hlen counts fields, 0 for a missing key") do
      key = kv_key("hlen")
      KV.hset(key, "a", "1")
      KV.hset(key, "b", "2")
      KV.hset(key, "c", "3")
      assert_eq(KV.hlen(key), 3)
      assert_eq(KV.hlen(kv_key("hlen_missing")), 0)
    end

    test("hsetnx creates a field only once") do
      key = kv_key("hsetnx")
      assert(KV.hsetnx(key, "field", "first"))
      assert_not(KV.hsetnx(key, "field", "second"))
      assert_eq(KV.hget(key, "field"), "first")
    end

    test("hincrby increments a field by an integer") do
      key = kv_key("hincrby")
      assert_eq(KV.hincrby(key, "count", 5), 5)
      assert_eq(KV.hincrby(key, "count", 3), 8)
      assert_eq(KV.hget(key, "count"), "8")
    end

    test("hincrbyfloat increments a field by a float") do
      key = kv_key("hincrbyfloat")
      assert_eq(KV.hincrbyfloat(key, "score", 1.5), 1.5)
      assert_eq(KV.hincrbyfloat(key, "score", 0.25), 1.75)
    end

    test("hmget fetches several fields with nil for misses") do
      key = kv_key("hmget")
      KV.hset(key, "a", "1")
      KV.hset(key, "b", "2")
      assert_eq(KV.hmget(key, "a", "missing", "b"), ["1", nil, "2"])
    end
  end

  describe("sorted sets") do
    test("zadd counts new members and zcard counts them all") do
      key = kv_key("zadd")
      assert_eq(KV.zadd(key, 1, "alice", 2.5, "bob"), 2)
      assert_eq(KV.zadd(key, 3, "alice"), 0)
      assert_eq(KV.zcard(key), 2)
      assert_eq(KV.zcard(kv_key("zcard_missing")), 0)
    end

    test("zscore and zincrby read and adjust scores as Floats") do
      key = kv_key("zscore")
      KV.zadd(key, 10, "carol")
      assert_eq(KV.zscore(key, "carol"), 10.0)
      assert_null(KV.zscore(key, "nobody"))
      assert_eq(KV.zincrby(key, 5, "carol"), 15.0)
      assert_eq(KV.zscore(key, "carol"), 15.0)
    end

    test("zcount counts members within an inclusive score range") do
      key = kv_key("zcount")
      KV.zadd(key, 1, "a", 5, "b", 10, "c")
      assert_eq(KV.zcount(key, 1, 10), 3)
      assert_eq(KV.zcount(key, 4, 6), 1)
      assert_eq(KV.zcount(key, 50, 100), 0)
    end

    test("zrange and zrevrange return members by rank") do
      key = kv_key("zrange")
      KV.zadd(key, 1, "a", 2, "b", 3, "c")
      assert_eq(KV.zrange(key, 0, -1), ["a", "b", "c"])
      assert_eq(KV.zrevrange(key, 0, -1), ["c", "b", "a"])
      assert_eq(KV.zrange(key, 0, 1), ["a", "b"])
      assert_eq(KV.zrange(kv_key("zrange_missing"), 0, -1), [])
    end

    test("zrange with scores interleaves members and scores") do
      key = kv_key("zrange_scores")
      KV.zadd(key, 1, "a", 2.5, "b")
      assert_eq(KV.zrange(key, 0, -1, true), ["a", "1", "b", "2.5"])
    end

    test("zrangebyscore filters members by score") do
      key = kv_key("zrangebyscore")
      KV.zadd(key, 1, "a", 5, "b", 10, "c")
      assert_eq(KV.zrangebyscore(key, 2, 9), ["b"])
      assert_eq(KV.zrangebyscore(key, "-inf", "+inf"), ["a", "b", "c"])
    end

    test("zrank and zrevrank report positions, nil for an absent member") do
      key = kv_key("zrank")
      KV.zadd(key, 1, "low", 2, "mid", 3, "high")
      assert_eq(KV.zrank(key, "low"), 0)
      assert_eq(KV.zrank(key, "high"), 2)
      assert_eq(KV.zrevrank(key, "high"), 0)
      assert_eq(KV.zrevrank(key, "low"), 2)
      assert_null(KV.zrank(key, "absent"))
      assert_null(KV.zrevrank(key, "absent"))
    end

    test("zrem removes members and counts only those present") do
      key = kv_key("zrem")
      KV.zadd(key, 1, "a", 2, "b", 3, "c")
      assert_eq(KV.zrem(key, "a", "c", "ghost"), 2)
      assert_eq(KV.zrange(key, 0, -1), ["b"])
      assert_eq(KV.zscore(key, "b"), 2.0)
    end
  end

  describe("hyperloglog") do
    test("pfadd reports whether the estimate changed") do
      key = kv_key("hll")
      assert_eq(KV.pfadd(key, "a", "b", "c"), 1)
      assert_eq(KV.pfadd(key, "a"), 0)
    end

    test("pfcount estimates a small cardinality exactly") do
      key = kv_key("hll:count")
      KV.pfadd(key, "a", "b", "c", "a")
      assert_eq(KV.pfcount(key), 3)
      assert_eq(KV.pfcount(kv_key("hll:missing")), 0)
    end

    test("pfmerge merges sources and pfcount unions several keys") do
      first = kv_key("hll:m1")
      second = kv_key("hll:m2")
      merged = kv_key("hll:merged")
      KV.pfadd(first, "a", "b", "c")
      KV.pfadd(second, "c", "d")
      KV.pfmerge(merged, first, second)
      assert_eq(KV.pfcount(merged), 4)
      assert_eq(KV.pfcount(first, second), 4)
    end
  end

  describe("bitmaps") do
    test("setbit returns the previous bit and getbit/bitcount read it back") do
      key = kv_key("bits")
      assert_eq(KV.setbit(key, 7, 1), 0)
      assert_eq(KV.getbit(key, 7), 1)
      assert_eq(KV.getbit(key, 0), 0)
      assert_eq(KV.bitcount(key), 1)
      assert_eq(KV.setbit(key, 7, 0), 1)
      assert_eq(KV.bitcount(key), 0)
    end

    test("a missing key reads as all zero bits") do
      key = kv_key("bits_missing")
      assert_eq(KV.getbit(key, 3), 0)
      assert_eq(KV.bitcount(key), 0)
    end
  end

  describe("server") do
    test("ping answers PONG") do
      assert_eq(KV.ping, "PONG")
    end

    test("dbsize counts the keys, including a fresh one") do
      key = kv_key("dbsize")
      KV.set(key, "x")
      assert_gt(KV.dbsize, 0)
    end

    test("cmd runs a raw command and returns its reply") do
      key = kv_key("raw")
      assert_eq(KV.cmd("SET", key, "hello"), "OK")
      assert_eq(KV.cmd("GET", key), "hello")
      assert_eq(KV.cmd("ECHO", "hi"), "hi")
      assert_eq(KV.cmd("EXISTS", kv_key("raw_missing")), 0)
    end
  end
end

describe("KV admin commands without SOLI_KV_ALLOW_ADMIN") do
  before_each() do
    used_keys = []
    requires_solikv()
    skip("SOLI_KV_ALLOW_ADMIN is set for this process") if admin_enabled?()
  end

  test("keys raises") do
    assert_raises("'KEYS' is denylisted") do
      KV.keys("test:kv:*")
    end
  end

  test("flushdb raises") do
    assert_raises("'FLUSHDB' is denylisted") do
      KV.flushdb
    end
  end

  test("cmd refuses an admin verb whatever its case") do
    assert_raises("'FLUSHALL' is denylisted") do
      KV.cmd("flushall")
    end
  end
end

describe("KV admin commands with SOLI_KV_ALLOW_ADMIN=1 at launch") do
  before_each() do
    used_keys = []
    requires_solikv()
    skip("needs SOLI_KV_ALLOW_ADMIN=1 when soli starts") unless admin_enabled?()
  end

  after_each() do
    used_keys.each do |key|
      KV.delete(key)
    end
  end

  test("keys returns the keys matching a glob") do
    first = kv_key("keys:a")
    second = kv_key("keys:b")
    KV.set(first, "1")
    KV.set(second, "2")
    assert_eq(KV.keys("test:kv:keys:*").sort, [first, second])
  end

  test("flushdb wipes the database") do
    KV.set(kv_key("flush"), "doomed")
    KV.flushdb
    assert_eq(KV.dbsize, 0)
  end
end
