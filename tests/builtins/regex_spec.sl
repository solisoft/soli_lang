// ============================================================================
// Regex Functions Test Suite
// ============================================================================

describe("Regex Functions", fn() {
    test("regex_match() checks pattern match", fn() {
        assert(regex_match("\\d+", "123"));
        assert(regex_match("[a-z]+", "hello"));
        assert_not(regex_match("\\d+", "hello"));
    });

    test("regex_find() finds first match", fn() {
        let result = regex_find("\\d+", "abc123def456");
        assert_not_null(result);
        assert_eq(result["match"], "123");
    });

    test("regex_find_all() finds all matches", fn() {
        let results = regex_find_all("\\d+", "abc123def456");
        assert_eq(len(results), 2);
        assert_eq(results[0]["match"], "123");
        assert_eq(results[1]["match"], "456");
    });

    test("regex_replace() replaces first match", fn() {
        let result = regex_replace("\\d+", "abc123def456", "X");
        assert_eq(result, "abcXdef456");
    });

    test("regex_replace_all() replaces all matches", fn() {
        let result = regex_replace_all("\\d+", "abc123def456", "X");
        assert_eq(result, "abcXdefX");
    });

    test("regex_split() splits by pattern", fn() {
        let parts = regex_split("\\s+", "hello   world  foo");
        assert_eq(len(parts), 3);
        assert_eq(parts[0], "hello");
        assert_eq(parts[1], "world");
        assert_eq(parts[2], "foo");
    });

    test("regex_escape() escapes special characters", fn() {
        let escaped = regex_escape("hello.world");
        assert_eq(escaped, "hello\\.world");
    });
});
