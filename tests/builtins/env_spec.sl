// ============================================================================
// Environment Variables Test Suite
// ============================================================================

describe("Environment Variables", fn() {
    test("getenv() returns null for missing vars", fn() {
        assert_null(getenv("SOLI_DEFINITELY_NOT_SET_12345"));
    });

    test("hasenv() reports false for missing vars", fn() {
        assert_not(hasenv("SOLI_NONEXISTENT_VAR_12345"));
    });

    # SEC-033: setenv / unsetenv were removed. They wrapped the unsafe
    # `std::env::set_var` and let one worker thread mutate process env while
    # other workers read it. The names stay registered so existing code gets a
    # clear migration error instead of `undefined variable`.

    test("setenv() is removed and errors with SEC-033 migration message", fn() {
        let threw = false;
        let msg = "";
        try {
            setenv("SOLI_TEST_VAR", "test_value");
        } catch (e) {
            threw = true;
            msg = str(e);
        }
        assert(threw);
        assert(msg.contains("SEC-033"));
    });

    test("unsetenv() is removed and errors with SEC-033 migration message", fn() {
        let threw = false;
        let msg = "";
        try {
            unsetenv("SOLI_TEST_REMOVE");
        } catch (e) {
            threw = true;
            msg = str(e);
        }
        assert(threw);
        assert(msg.contains("SEC-033"));
    });
});
