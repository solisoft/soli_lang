describe("Trust Proxy", fn() {
    after_each(fn() {
        disable_trust_proxy();
    });

    test("trust proxy defaults to disabled", fn() {
        assert(!trust_proxy_enabled());
    });

    test("enable_trust_proxy returns true", fn() {
        let result = enable_trust_proxy();
        assert_eq(result, true);
    });

    test("enable_trust_proxy makes enabled return true", fn() {
        enable_trust_proxy();
        assert(trust_proxy_enabled());
    });

    test("disable_trust_proxy makes enabled return false", fn() {
        enable_trust_proxy();
        disable_trust_proxy();
        assert(!trust_proxy_enabled());
    });

    test("disable_trust_proxy returns true", fn() {
        let result = disable_trust_proxy();
        assert_eq(result, true);
    });
});

describe("Body Limit", fn() {
    after_each(fn() {
        set_max_body_size(8 * 1024 * 1024);
    });

    test("max_body_size returns default", fn() {
        let size = max_body_size();
        assert_eq(size, 8 * 1024 * 1024);
    });

    test("set_max_body_size sets and returns the value", fn() {
        let result = set_max_body_size(1024);
        assert_eq(result, 1024);
        assert_eq(max_body_size(), 1024);
    });

    test("set_max_body_size with zero", fn() {
        set_max_body_size(0);
        assert_eq(max_body_size(), 0);
    });

    test("set_max_body_size with large value", fn() {
        set_max_body_size(64 * 1024 * 1024);
        assert_eq(max_body_size(), 64 * 1024 * 1024);
    });

    test("set_max_body_size with string throws", fn() {
        let caught = false;
        try {
            set_max_body_size("big");
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("set_max_body_size with negative throws", fn() {
        let caught = false;
        try {
            set_max_body_size(-1);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("set_max_body_size with float throws", fn() {
        let caught = false;
        try {
            set_max_body_size(1024.5);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });
});
