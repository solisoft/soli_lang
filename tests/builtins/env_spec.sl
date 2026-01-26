// ============================================================================
// Environment Variables Test Suite
// ============================================================================

describe("Environment Variables", fn() {
    test("setenv() and getenv() work together", fn() {
        setenv("SOLI_TEST_VAR", "test_value");
        assert_eq(getenv("SOLI_TEST_VAR"), "test_value");
    });

    test("hasenv() checks for existence", fn() {
        setenv("SOLI_TEST_EXISTS", "yes");
        assert(hasenv("SOLI_TEST_EXISTS"));
        assert_not(hasenv("SOLI_NONEXISTENT_VAR_12345"));
    });

    test("unsetenv() removes variable", fn() {
        setenv("SOLI_TEST_REMOVE", "value");
        assert(hasenv("SOLI_TEST_REMOVE"));
        unsetenv("SOLI_TEST_REMOVE");
        assert_not(hasenv("SOLI_TEST_REMOVE"));
    });

    test("getenv() returns null for missing vars", fn() {
        assert_null(getenv("SOLI_DEFINITELY_NOT_SET_12345"));
    });

    test("setenv() overwrites existing value", fn() {
        setenv("SOLI_OVERWRITE", "first");
        assert_eq(getenv("SOLI_OVERWRITE"), "first");
        setenv("SOLI_OVERWRITE", "second");
        assert_eq(getenv("SOLI_OVERWRITE"), "second");
    });
});
