// ============================================================================
// Regex Class Test Suite
// ============================================================================

describe("Regex Class", fn() {
    test("Regex.matches() checks pattern match", fn() {
        assert(Regex.matches("\\d+", "123"));
        assert(Regex.matches("[a-z]+", "hello"));
        assert_not(Regex.matches("\\d+", "hello"));
    });

    test("Regex.find() finds first match", fn() {
        let result = Regex.find("\\d+", "abc123def456");
        assert_not_null(result);
        assert_eq(result["match"], "123");
    });

    test("Regex.find_all() finds all matches", fn() {
        let results = Regex.find_all("\\d+", "abc123def456");
        assert_eq(len(results), 2);
        assert_eq(results[0]["match"], "123");
        assert_eq(results[1]["match"], "456");
    });

    test("Regex.replace() replaces first match", fn() {
        let result = Regex.replace("\\d+", "abc123def456", "X");
        assert_eq(result, "abcXdef456");
    });

    test("Regex.replace_all() replaces all matches", fn() {
        let result = Regex.replace_all("\\d+", "abc123def456", "X");
        assert_eq(result, "abcXdefX");
    });

    test("Regex.split() splits by pattern", fn() {
        let parts = Regex.split("\\s+", "hello   world  foo");
        assert_eq(len(parts), 3);
        assert_eq(parts[0], "hello");
        assert_eq(parts[1], "world");
        assert_eq(parts[2], "foo");
    });

    test("Regex.escape() escapes special characters", fn() {
        let escaped = Regex.escape("hello.world");
        assert_eq(escaped, "hello\\.world");
    });
});
