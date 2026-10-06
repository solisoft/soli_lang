# Edge cases of KV.append, KV.touch and KV.type beyond the basics in
# tests/builtins/kv_spec.sl. Every key lives under "test:extra:" and is
# deleted after each test.

KEY_SUFFIXES = ["text", "empty", "list", "counter", "ttl", "gone", "bits", "hll", "a", "b"]

def extra_key(name)
  "test:extra:#{name}"
end

describe("KV extra commands") do
  before_each() do
    requires_solikv()
  end

  after_each() do
    KEY_SUFFIXES.each do |suffix|
      KV.delete(extra_key(suffix))
    end
  end

  context("append") do
    test("appending an empty string to a missing key creates it empty") do
      key = extra_key("empty")
      assert_eq(KV.append(key, ""), 0)
      assert(KV.exists(key))
      assert_eq(KV.get(key), "")
    end

    test("counts bytes, not characters") do
      key = extra_key("text")
      assert_eq(KV.append(key, "é"), 2)
      assert_eq(KV.strlen(key), 2)
      assert_eq(KV.get(key), "é")
    end

    test("stringifies a non-string value") do
      key = extra_key("text")
      KV.set(key, "id-")
      assert_eq(KV.append(key, 42), 5)
      assert_eq(KV.get(key), "id-42")
    end

    test("concatenates onto a counter, which stays a counter") do
      key = extra_key("counter")
      KV.incr(key)
      assert_eq(KV.append(key, "5"), 2)
      assert_eq(KV.get(key), "15")
      assert_eq(KV.incr(key), 16)
    end

    test("keeps the key's TTL") do
      key = extra_key("ttl")
      KV.set(key, "v", 100)
      KV.append(key, "w")
      assert_eq(KV.get(key), "vw")
      assert_gt(KV.ttl(key), 90)
    end

    test("refuses a key holding a list") do
      key = extra_key("list")
      KV.rpush(key, "x")
      assert_raises("WRONGTYPE Operation against a key holding the wrong kind of value") do
        KV.append(key, "y")
      end
      assert_eq(KV.lrange(key, 0, -1), ["x"])
    end

    test("needs a value") do
      assert_raises("Wrong number of arguments: expected 2, got 1") do
        KV.append(extra_key("text"))
      end
    end
  end

  context("touch") do
    test("answers 0 when no key exists") do
      assert_eq(KV.touch(extra_key("a"), extra_key("b")), 0)
      assert_not(KV.exists(extra_key("a")))
    end

    test("counts a key named twice twice") do
      key = extra_key("a")
      KV.set(key, "1")
      assert_eq(KV.touch(key, key), 2)
    end

    test("leaves the TTL alone") do
      key = extra_key("ttl")
      KV.set(key, "v", 100)
      assert_eq(KV.touch(key), 1)
      assert_gt(KV.ttl(key), 90)
      assert_eq(KV.get(key), "v")
    end

    test("needs at least one key") do
      assert_raises("KV.touch() expects at least 1 argument (key, ...keys)") do
        KV.touch()
      end
    end
  end

  context("type") do
    test("a counter is a string") do
      key = extra_key("counter")
      KV.incr(key)
      assert_eq(KV.type(key), "string")
    end

    test("a bitmap and a HyperLogLog are strings") do
      KV.setbit(extra_key("bits"), 7, 1)
      KV.pfadd(extra_key("hll"), "visitor")
      assert_eq(KV.type(extra_key("bits")), "string")
      assert_eq(KV.type(extra_key("hll")), "string")
    end

    test("a deleted key is none") do
      key = extra_key("gone")
      KV.set(key, "v")
      KV.delete(key)
      assert_eq(KV.type(key), "none")
    end

    test("needs a key") do
      assert_raises("Wrong number of arguments: expected 1, got 0") do
        KV.type()
      end
    end
  end
end
