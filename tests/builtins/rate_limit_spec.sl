// ============================================================================
// RateLimiter Test Suite
// ============================================================================

describe("RateLimiter", fn() {
    test("rate_limiter_from_ip creates an instance", fn() {
        let req = {"headers": {"x-forwarded-for": "192.168.1.1"}};
        let rl = rate_limiter_from_ip(req, 100);
        assert(rl != null);
    });

    test("allowed returns a boolean on first call", fn() {
        let req = {"headers": {"x-forwarded-for": "192.168.1.10"}};
        let rl = rate_limiter_from_ip(req, 100);
        let result = rl.allowed();
        assert(type(result) == "bool");
    });

    test("throttle returns an integer", fn() {
        let req = {"headers": {"x-forwarded-for": "192.168.1.11"}};
        let rl = rate_limiter_from_ip(req, 100);
        let result = rl.throttle();
        assert(type(result) == "int");
    });

    test("status returns a hash", fn() {
        let req = {"headers": {"x-forwarded-for": "192.168.1.12"}};
        let rl = rate_limiter_from_ip(req, 100);
        let result = rl.status();
        assert(type(result) == "hash");
    });

    test("status hash has expected keys", fn() {
        let req = {"headers": {"x-forwarded-for": "192.168.1.13"}};
        let rl = rate_limiter_from_ip(req, 100);
        let result = rl.status();
        assert(result.has_key("allowed"));
        assert(result.has_key("remaining"));
        assert(result.has_key("reset_in"));
        assert(result.has_key("limit"));
        assert(result.has_key("window"));
    });

    test("headers returns a hash", fn() {
        let req = {"headers": {"x-forwarded-for": "192.168.1.14"}};
        let rl = rate_limiter_from_ip(req, 100);
        let result = rl.headers();
        assert(type(result) == "hash");
    });

    test("headers hash has rate limit headers", fn() {
        let req = {"headers": {"x-forwarded-for": "192.168.1.15"}};
        let rl = rate_limiter_from_ip(req, 100);
        let result = rl.headers();
        assert(result.has_key("X-RateLimit-Limit"));
        assert(result.has_key("X-RateLimit-Remaining"));
        assert(result.has_key("X-RateLimit-Reset"));
    });

    test("RateLimiter.reset_all can be called", fn() {
        RateLimiter.reset_all();
    });

    test("RateLimiter.cleanup can be called", fn() {
        RateLimiter.cleanup();
    });
});