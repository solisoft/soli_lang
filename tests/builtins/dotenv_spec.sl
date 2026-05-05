// ============================================================================
// Dotenv Test Suite
// ============================================================================
//
// SEC-033: `dotenv` / `dotenv!` were removed as runtime builtins. `.env` and
// `.env.{APP_ENV}` are auto-loaded once at single-threaded server boot, so
// runtime mutation of process env is no longer needed. The names stay
// registered so existing code gets a clear migration error.

describe("Dotenv", fn() {
    test("dotenv() is removed and errors with SEC-033 migration message", fn() {
        let threw = false;
        let msg = "";
        try {
            dotenv("tests/fixtures/.env.test");
        } catch (e) {
            threw = true;
            msg = str(e);
        }
        assert(threw);
        assert(msg.contains("SEC-033"));
    });
});
