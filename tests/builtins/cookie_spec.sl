// ============================================================================
// Cookie Test Suite
// ============================================================================

// These are smoke tests for cookie builtins.
// Full integration tests (Cookie header parsing, Set-Cookie emission) are
// in tests/server_e2e_test.rs.

describe("Cookie Functions", fn() {
    test("set_cookie exists and can be called", fn() {
        set_cookie("test_name", "test_value");
        assert(true);
    });

    test("read_cookie returns null for an absent cookie", fn() {
        assert_null(read_cookie("no_such_cookie"));
        assert_null(read_cookie("no_such_cookie", {"encrypted": true}));
    });

    test("read_cookie rejects unknown options", fn() {
        let error = null;
        try {
            read_cookie("x", {"encrytped": true});
        } catch e {
            error = e;
        }
        assert(str(error).includes?("unknown option"));
    });
});

describe("Signed/Encrypted Cookie Jar", fn() {
    before_all(fn() {
        session_configure({"secret": "spec-secret-0123456789abcdef-0123"});
    });

    test("encrypted cookie round-trips structured values", fn() {
        set_cookie("jar_prefs", {"theme": "dark", "cols": [1, 2]}, {"encrypted": true});
        let prefs = read_cookie("jar_prefs", {"encrypted": true});
        assert_eq(prefs["theme"], "dark");
        assert_eq(prefs["cols"], [1, 2]);
    });

    test("signed cookie round-trips and is not readable as encrypted", fn() {
        set_cookie("jar_uid", 42, {"signed": true});
        assert_eq(read_cookie("jar_uid", {"signed": true}), 42);
        // Wrong mode reads as absent, never as a decoded value.
        assert_null(read_cookie("jar_uid", {"encrypted": true}));
    });

    test("plain read of a sealed cookie sees the opaque wire value", fn() {
        set_cookie("jar_raw", "hello", {"encrypted": true});
        let raw = read_cookie("jar_raw");
        assert(raw.starts_with("enc.v1."));
        assert(!raw.includes?("hello"));
    });

    test("encrypted and signed options are mutually exclusive", fn() {
        let error = null;
        try {
            set_cookie("jar_bad", 1, {"encrypted": true, "signed": true});
        } catch e {
            error = e;
        }
        assert(str(error).includes?("mutually exclusive"));
    });
});
