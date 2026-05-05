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

    test("System.run accepts argv array", fn() {
        let result = System.run_sync(["echo", "from", "array"]);
        assert_eq(result["exit_code"], 0);
        assert(result["stdout"].contains("from array"));
    });

    test("argv array passes args verbatim (no shell)", fn() {
        # `*` is a literal arg, not a glob — the shell would expand it.
        let result = System.run_sync(["echo", "*"]);
        assert_eq(result["exit_code"], 0);
        assert_eq(result["stdout"].trim(), "*");
    });

    test("System.run rejects shell metacharacters in string form", fn() {
        let threw = false;
        try {
            System.run_sync("echo hi > /tmp/should-not-exist.txt");
        } catch (e) {
            threw = true;
        }
        assert(threw);
    });

    test("System.shell_sync runs through sh -c", fn() {
        let result = System.shell_sync("echo one | tr a-z A-Z");
        assert_eq(result["exit_code"], 0);
        assert(result["stdout"].contains("ONE"));
    });

    test("System.shell returns a future", fn() {
        let future = System.shell("echo shellfut");
        assert(future != null);
    });
});
