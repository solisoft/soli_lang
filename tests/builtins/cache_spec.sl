# The Cache class: JSON values in SoliKV under a prefix scoped to the test
# database, so Cache.clear here only wipes this spec's entries.

describe("Cache") do
  before_each() do
    requires_solikv()
    Cache.clear
  end

  after_each() do
    Cache.clear
  end

  describe("set and get") do
    test("set returns nil and get reads the value back") do
      assert_null(Cache.set("greeting", "hello"))
      assert_eq(Cache.get("greeting"), "hello")
    end

    test("get returns nil for a missing key") do
      assert_null(Cache.get("missing"))
    end

    test("keeps the type of scalars") do
      Cache.set("int", 123)
      Cache.set("float", 1.5)
      Cache.set("flag", false)
      assert_eq(Cache.get("int"), 123)
      assert_eq(Cache.get("float"), 1.5)
      assert_eq(Cache.get("flag"), false)
    end

    test("stores arrays") do
      Cache.set("array", [1, 2, 3, 4, 5])
      assert_eq(Cache.get("array"), [1, 2, 3, 4, 5])
    end

    test("stores hashes") do
      Cache.set("hash", {"name": "test", "value": 42})
      assert_eq(Cache.get("hash"), {"name": "test", "value": 42})
    end

    test("stores nested structures") do
      Cache.set("nested", {"items": [1, 2, 3], "meta": {"count": 3}})
      cached = Cache.get("nested")
      assert_eq(cached["items"], [1, 2, 3])
      assert_eq(cached["meta"]["count"], 3)
    end

    test("a cached nil is a present key that reads as nil") do
      Cache.set("nothing", nil)
      assert(Cache.has("nothing"))
      assert_null(Cache.get("nothing"))
    end

    test("set refuses a missing value") do
      assert_raises("Cache.set() expects 2-3 arguments, got 1") do
        Cache.set("only_a_key")
      end
    end

    test("get refuses a non-string key") do
      assert_raises("Cache.get() expects string key, got int") do
        Cache.get(1)
      end
    end
  end

  describe("delete and has") do
    test("delete removes the key and returns true") do
      Cache.set("doomed", "value")
      assert(Cache.delete("doomed"))
      assert_null(Cache.get("doomed"))
      assert_not(Cache.has("doomed"))
    end

    test("delete returns false for a missing key") do
      assert_not(Cache.delete("missing"))
    end

    test("has checks existence, including a cached false") do
      Cache.set("present", "value")
      Cache.set("falsey", false)
      assert(Cache.has("present"))
      assert(Cache.has("falsey"))
      assert_not(Cache.has("missing"))
    end
  end

  describe("clear, keys and size") do
    test("clear removes every entry and returns nil") do
      Cache.set("first", "v1")
      Cache.set("second", "v2")
      assert_null(Cache.clear)
      assert_null(Cache.get("first"))
      assert_null(Cache.get("second"))
      assert_eq(Cache.size, 0)
    end

    test("clear leaves keys outside the cache prefix alone") do
      KV.set("test:cache:outside", "keep")
      Cache.clear
      assert_eq(KV.get("test:cache:outside"), "keep")
      KV.delete("test:cache:outside")
    end

    test("keys lists cache keys without their prefix") do
      Cache.set("key1", "a")
      Cache.set("key2", "b")
      assert_eq(Cache.keys.sort, ["key1", "key2"])
    end

    test("keys and size are empty on an empty cache") do
      assert_eq(Cache.keys, [])
      assert_eq(Cache.size, 0)
    end

    test("size counts the entries") do
      Cache.set("first", "value")
      Cache.set("second", "value")
      assert_eq(Cache.size, 2)
    end
  end

  describe("TTL") do
    test("set defaults to a one-hour TTL") do
      Cache.set("default_ttl", "value")
      assert_eq(Cache.ttl("default_ttl"), 3600)
    end

    test("set takes a TTL in seconds") do
      Cache.set("short_ttl", "value", 60)
      ttl = Cache.ttl("short_ttl")
      assert_gt(ttl, 0)
      assert(ttl <= 60)
    end

    test("ttl returns nil for a missing key") do
      assert_null(Cache.ttl("missing"))
    end

    test("touch replaces the TTL of an existing key") do
      Cache.set("touched", "value")
      assert(Cache.touch("touched", 7200))
      assert_eq(Cache.ttl("touched"), 7200)
      assert_eq(Cache.get("touched"), "value")
    end

    test("touch returns false for a missing key") do
      assert_not(Cache.touch("missing", 10))
    end

    test("clear_expired is a no-op that keeps live entries") do
      Cache.set("alive", "value")
      assert_null(Cache.clear_expired)
      assert_eq(Cache.get("alive"), "value")
    end
  end

  describe("fetch") do
    test("runs the block on a miss and caches its result") do
      result = Cache.fetch("computed") do
        "computed_value"
      end
      assert_eq(result, "computed_value")
      assert_eq(Cache.get("computed"), "computed_value")
      assert_eq(Cache.ttl("computed"), 3600)
    end

    test("returns the cached value on a hit without running the block") do
      Cache.set("hit", "original")
      calls = 0
      result = Cache.fetch("hit") do
        calls = calls + 1
        "new_value"
      end
      assert_eq(result, "original")
      assert_eq(calls, 0)
    end

    test("takes a TTL before the block") do
      result = Cache.fetch("with_ttl", 600) do
        "ttl_value"
      end
      assert_eq(result, "ttl_value")
      assert_eq(Cache.ttl("with_ttl"), 600)
    end

    test("returns and stores a nil result") do
      result = Cache.fetch("nil_result") do
        nil
      end
      assert_null(result)
      assert(Cache.has("nil_result"))
    end

    test("a cached nil is a hit for the next fetch") do
      pending("bug: Cache.fetch re-runs the block when the cached value is nil, though Cache.has reports the key")
      calls = 0
      Cache.fetch("nil_hit") do
        calls = calls + 1
        nil
      end
      second = Cache.fetch("nil_hit") do
        calls = calls + 1
        "recomputed"
      end
      assert_null(second)
      assert_eq(calls, 1)
    end

    test("without a block returns nil on a miss and the value on a hit") do
      assert_null(Cache.fetch("missing"))
      Cache.set("existing", "hello")
      assert_eq(Cache.fetch("existing"), "hello")
    end

    test("refuses a non-string key") do
      assert_raises("Cache.fetch() expects string key") do
        Cache.fetch(1)
      end
    end
  end

  describe("configure") do
    test("refuses a call without a host") do
      assert_raises("Cache.configure() expects 1-2 arguments, got 0") do
        Cache.configure()
      end
    end

    test("refuses a host that is not a string") do
      assert_raises("Cache.configure() expects string host, got int") do
        Cache.configure(6380)
      end
    end
  end
end
