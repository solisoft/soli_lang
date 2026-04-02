// ============================================================================
// System Test Suite
// ============================================================================

describe("System", fn() {
    test("System.run returns a future", fn() {
        let future = System.run("echo hello");
        assert(future != null);
    });

    test("System.run_sync returns result hash", fn() {
        let result = System.run_sync("echo hello");
        assert(result != null);
        assert_eq(result["exit_code"], 0);
    });

    test("run_sync captures stdout", fn() {
        let result = System.run_sync("echo hello");
        assert(result["stdout"].contains("hello"));
    });
});
