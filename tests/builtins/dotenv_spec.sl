// ============================================================================
// Dotenv Test Suite
// ============================================================================

describe("Dotenv", fn() {
    test("dotenv loads env file", fn() {
        let count = dotenv("tests/fixtures/.env.test");
        assert(count >= 2);
    });

    test("dotenv returns count of loaded variables", fn() {
        let count = dotenv("tests/fixtures/.env.test");
        assert(count > 0);
    });

    test("dotenv loads variables into environment", fn() {
        dotenv("tests/fixtures/.env.test");
        # The variables should be available via getenv
    });
});
