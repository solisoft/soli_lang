# RateLimiter: an in-memory sliding window per key. Every test starts from
# empty buckets.

describe("RateLimiter") do
  before_each() do
    RateLimiter.reset_all
  end

  after_each() do
    RateLimiter.reset_all
  end

  describe("RateLimiter(key, limit, window)") do
    test("a fresh limiter has its whole budget") do
      limiter = RateLimiter("ctor:fields", 3, 60)
      assert_eq(limiter.status, {"allowed": true, "remaining": 3, "reset_in": 0, "limit": 3, "window": 60})
    end

    test("allowed counts against the limit") do
      limiter = RateLimiter("ctor:allowed", 2, 60)
      assert(limiter.allowed)
      assert(limiter.allowed)
      assert_not(limiter.allowed)
    end

    test("status reports an exhausted bucket") do
      limiter = RateLimiter("ctor:exhausted", 1, 60)
      limiter.allowed
      status = limiter.status
      assert_eq(status["allowed"], false)
      assert_eq(status["remaining"], 0)
      assert_gt(status["reset_in"], 0)
      assert(status["reset_in"] <= 60)
    end

    test("throttle does not use up an attempt") do
      limiter = RateLimiter("ctor:throttle", 2, 60)
      assert_eq(limiter.throttle, 0)
      assert_eq(limiter.throttle, 0)
      assert_eq(limiter.status["remaining"], 2)
      assert(limiter.allowed)
      assert(limiter.allowed)
      wait = limiter.throttle
      assert_gt(wait, 0)
      assert(wait <= 60)
    end

    test("a limit of 0 never limits") do
      limiter = RateLimiter("ctor:zero", 0, 60)
      assert(limiter.allowed)
      assert(limiter.allowed)
      assert_eq(limiter.status["allowed"], true)
    end

    test("headers report limit and remaining as strings") do
      limiter = RateLimiter("ctor:headers", 100, 60)
      limiter.allowed
      headers = limiter.headers
      assert_eq(headers["X-RateLimit-Limit"], "100")
      assert_eq(headers["X-RateLimit-Remaining"], "99")
      assert_hash_has_key(headers, "X-RateLimit-Reset")
    end

    test("the reset header tells when the window resets") do
      pending("bug: the constructor stamps a `reset` field of 0, which headers() reports as X-RateLimit-Reset")
      limiter = RateLimiter("ctor:reset_header", 1, 60)
      limiter.allowed
      assert_ne(limiter.headers["X-RateLimit-Reset"], "0")
    end

    test("reset reopens this limiter's bucket") do
      pending("bug: the constructor's `reset` field (0) shadows the reset() method, so it returns 0 and resets nothing")
      limiter = RateLimiter("ctor:reset", 1, 60)
      limiter.allowed
      assert_eq(limiter.reset(), true)
      assert(limiter.allowed)
    end

    test("refuses a missing window") do
      assert_raises("RateLimiter(key, limit, window_seconds) expects 3 arguments, got 2") do
        RateLimiter("ctor:bad", 3)
      end
    end

    test("refuses a key that is not a string") do
      assert_raises("key must be a String, got int") do
        RateLimiter(1, 3, 60)
      end
    end

    test("refuses a limit that is not a non-negative Int") do
      assert_raises("limit must be an Int of 0 or more, got string") do
        RateLimiter("ctor:bad", "ten", 60)
      end
      assert_raises("limit must be an Int of 0 or more, got int") do
        RateLimiter("ctor:bad", -1, 60)
      end
    end

    test("refuses a window that is not a positive Int") do
      assert_raises("window_seconds must be an Int above 0, got string") do
        RateLimiter("ctor:bad", 3, "x")
      end
    end
  end

  describe("rate_limiter_from_ip") do
    test("keys the bucket on the TCP peer address") do
      limiter = rate_limiter_from_ip({"remote_addr": "10.55.0.1"}, 2, 60)
      assert_eq(limiter.key, "ip:10.55.0.1")
      assert_eq(limiter.status["limit"], 2)
      assert_eq(limiter.status["window"], 60)
    end

    test("ignores X-Forwarded-For when the proxy is not trusted") do
      request = {"remote_addr": "10.55.0.2", "headers": {"x-forwarded-for": "1.2.3.4"}}
      assert_eq(rate_limiter_from_ip(request, 2, 60).key, "ip:10.55.0.2")
    end

    test("defaults the window to 60 seconds") do
      assert_eq(rate_limiter_from_ip({"remote_addr": "10.55.0.3"}, 100).status["window"], 60)
    end

    test("goes false once the limit is spent") do
      limiter = rate_limiter_from_ip({"remote_addr": "10.55.0.4"}, 2, 60)
      assert_eq(limiter.allowed, true)
      assert_eq(limiter.allowed, true)
      assert_eq(limiter.allowed, false)
    end

    test("gives every address its own bucket") do
      first = rate_limiter_from_ip({"remote_addr": "10.55.0.5"}, 1, 60)
      assert_eq(first.allowed, true)
      assert_eq(first.allowed, false)
      second = rate_limiter_from_ip({"remote_addr": "10.55.0.6"}, 1, 60)
      assert_eq(second.allowed, true)
    end

    test("shares one bucket between limiters for the same address") do
      rate_limiter_from_ip({"remote_addr": "10.55.0.7"}, 1, 60).allowed
      assert_eq(rate_limiter_from_ip({"remote_addr": "10.55.0.7"}, 1, 60).allowed, false)
    end
  end

  describe("static methods") do
    test("reset_all reopens every bucket and returns true") do
      limiter = rate_limiter_from_ip({"remote_addr": "10.55.0.8"}, 1, 60)
      limiter.allowed
      assert_eq(limiter.allowed, false)
      assert_eq(RateLimiter.reset_all, true)
      assert_eq(rate_limiter_from_ip({"remote_addr": "10.55.0.8"}, 1, 60).allowed, true)
    end

    test("cleanup returns true and keeps live buckets") do
      limiter = RateLimiter("static:cleanup", 2, 60)
      limiter.allowed
      assert_eq(RateLimiter.cleanup, true)
      assert_eq(limiter.status["remaining"], 1)
    end
  end
end

describe("removed rate-limit globals") do
  test("each one returns a message pointing to RateLimiter") do
    messages = [
      rate_limit(),
      throttle(),
      rate_limit_ip(),
      rate_limit_status(),
      rate_limit_reset(),
      rate_limit_reset_all(),
      rate_limit_cleanup(),
      rate_limit_headers()
    ]
    names = [
      "rate_limit",
      "throttle",
      "rate_limit_ip",
      "rate_limit_status",
      "rate_limit_reset",
      "rate_limit_reset_all",
      "rate_limit_cleanup",
      "rate_limit_headers"
    ]
    expected = names.map { |name| "#{name}() has been removed. Use RateLimiter class instead." }
    assert_eq(messages, expected)
  end
end
