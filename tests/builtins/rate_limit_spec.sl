// ============================================================================
// RateLimiter Class Test Suite
// ============================================================================
// Tests for RateLimiter class and rate limiting functionality
// ============================================================================

describe("RateLimiter Basic Methods", fn() {
    test("allowed() allows first request", fn() {
        RateLimiter.reset_all();
        let limiter = RateLimiter("user:123", 10, 60);
        let result = limiter.allowed();
        assert(result);
    });

    test("allowed() returns false when limit exceeded", fn() {
        RateLimiter.reset_all();
        let limiter = RateLimiter("test_exceeded", 3, 60);
        for i in 0..5 {
            let _ = limiter.allowed();
        }
        let result = limiter.allowed();
        assert_not(result);
    });

    test("allowed() with zero limit allows all", fn() {
        let limiter = RateLimiter("unlimited", 0, 60);
        let result = limiter.allowed();
        assert(result);
        let result2 = limiter.allowed();
        assert(result2);
    });

    test("allowed() different keys track separately", fn() {
        RateLimiter.reset_all();
        let limiter1 = RateLimiter("key1", 2, 60);
        let limiter2 = RateLimiter("key2", 2, 60);
        let result1 = limiter1.allowed();
        let result2 = limiter2.allowed();
        let result3 = limiter1.allowed();
        assert(result1);
        assert(result2);
        assert(result3);
    });
});

describe("RateLimiter Throttle Method", fn() {
    test("throttle() returns zero when under limit", fn() {
        RateLimiter.reset_all();
        let limiter = RateLimiter("user:456", 10, 60);
        let wait = limiter.throttle();
        assert_eq(wait, 0);
    });

    test("throttle() returns wait time when at limit", fn() {
        RateLimiter.reset_all();
        let limiter = RateLimiter("throttle_test", 3, 60);
        for i in 0..5 {
            let _ = limiter.throttle();
        }
        let wait = limiter.throttle();
        assert(wait > 0);
    });
});

describe("RateLimiter Status Method", fn() {
    test("status() returns status hash", fn() {
        RateLimiter.reset_all();
        let limiter = RateLimiter("status_test", 5, 60);
        let _ = limiter.allowed();
        let status = limiter.status();
        assert_not_null(status);
        assert_not_null(status["allowed"]);
        assert_not_null(status["remaining"]);
        assert_not_null(status["reset_in"]);
        assert_not_null(status["limit"]);
        assert_not_null(status["window"]);
    });

    test("status() shows remaining requests", fn() {
        RateLimiter.reset_all();
        let limiter = RateLimiter("remaining_test", 3, 60);
        let _ = limiter.allowed();
        let status = limiter.status();
        assert_eq(status["remaining"], 2);
    });
});

describe("RateLimiter Headers Method", fn() {
    test("headers() generates header hash", fn() {
        let limiter = RateLimiter("headers_test", 100, 60);
        let headers = limiter.headers();
        assert_not_null(headers);
        assert_eq(headers["X-RateLimit-Limit"], "100");
        assert_eq(headers["X-RateLimit-Remaining"], "100");
    });
});

describe("RateLimiter Reset Method", fn() {
    test("reset() removes specific key", fn() {
        RateLimiter.reset_all();
        let limiter = RateLimiter("to_reset", 1, 60);
        let _ = limiter.allowed();
        let before = limiter.status();
        let result = limiter.reset();
        assert(result);
        let after = limiter.status();
        assert_eq(after["remaining"], 1);
    });
});

describe("RateLimiter Static Methods", fn() {
    test("from_ip() creates instance from request", fn() {
        let req = hash();
        req["headers"] = hash();
        req["headers"]["x-forwarded-for"] = "192.168.1.1";
        req["headers"]["remote_addr"] = "127.0.0.1";
        let limiter = rate_limiter_from_ip(req, 10, 60);
        assert(limiter.allowed());
    });

    test("from_ip() uses default window", fn() {
        let req = hash();
        req["headers"] = hash();
        req["headers"]["remote_addr"] = "10.0.0.1";
        let limiter = rate_limiter_from_ip(req, 5);
        assert(limiter.allowed());
    });

    test("reset_all() clears all buckets", fn() {
        RateLimiter.reset_all();
        let limiter1 = RateLimiter("key1", 1, 60);
        let limiter2 = RateLimiter("key2", 1, 60);
        let _ = limiter1.allowed();
        let _ = limiter2.allowed();
        RateLimiter.reset_all();
        let status1 = limiter1.status();
        let status2 = limiter2.status();
        assert_eq(status1["remaining"], 1);
        assert_eq(status2["remaining"], 1);
    });

    test("cleanup() runs without error", fn() {
        let result = RateLimiter.cleanup();
        assert(result);
    });
});

describe("RateLimiter Instance Properties", fn() {
    test("instance stores key, limit, and window", fn() {
        let limiter = RateLimiter("test_key", 50, 120);
        assert_eq(limiter["key"], "test_key");
        assert_eq(limiter["limit"], 50);
        assert_eq(limiter["window"], 120);
    });
});
