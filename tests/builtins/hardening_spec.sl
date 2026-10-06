# Server hardening switches: trust-proxy mode and the request body size limit.

const DEFAULT_MAX_BODY = 8 * 1024 * 1024

describe("trust proxy") do
  after_each() do
    disable_trust_proxy()
  end

  test("is disabled by default") do
    assert_not(trust_proxy_enabled())
  end

  test("enable_trust_proxy returns true and turns it on") do
    assert_eq(enable_trust_proxy(), true)
    assert(trust_proxy_enabled())
  end

  test("enabling twice keeps it on") do
    enable_trust_proxy()
    assert_eq(enable_trust_proxy(), true)
    assert(trust_proxy_enabled())
  end

  test("disable_trust_proxy returns true and turns it off") do
    enable_trust_proxy()
    assert_eq(disable_trust_proxy(), true)
    assert_not(trust_proxy_enabled())
  end

  test("disabling when already off is a no-op that returns true") do
    assert_eq(disable_trust_proxy(), true)
    assert_not(trust_proxy_enabled())
  end
end

describe("body limit") do
  after_each() do
    set_max_body_size(DEFAULT_MAX_BODY)
  end

  test("max_body_size defaults to 8 MiB") do
    assert_eq(max_body_size(), DEFAULT_MAX_BODY)
  end

  test("set_max_body_size sets and returns the value") do
    assert_eq(set_max_body_size(1024), 1024)
    assert_eq(max_body_size(), 1024)
  end

  test("accepts zero") do
    assert_eq(set_max_body_size(0), 0)
    assert_eq(max_body_size(), 0)
  end

  test("accepts a large value") do
    set_max_body_size(64 * 1024 * 1024)
    assert_eq(max_body_size(), 64 * 1024 * 1024)
  end

  test("refuses a string") do
    assert_raises("set_max_body_size expects Int, got string") do
      set_max_body_size("big")
    end
  end

  test("refuses a negative size") do
    assert_raises("set_max_body_size: bytes must be non-negative") do
      set_max_body_size(-1)
    end
  end

  test("refuses a float") do
    assert_raises("set_max_body_size expects Int, got float") do
      set_max_body_size(1024.5)
    end
  end

  test("a refused value leaves the limit unchanged") do
    set_max_body_size(2048)
    assert_raises() do
      set_max_body_size(-1)
    end
    assert_eq(max_body_size(), 2048)
  end
end
