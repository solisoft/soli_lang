// ============================================================================
// String Functions Test Suite
// ============================================================================

describe("String Functions", fn() {
    test("len() returns string length", fn() {
        assert_eq(len("hello"), 5);
        assert_eq(len(""), 0);
        assert_eq(len("hello world"), 11);
    });

    test("contains() checks for substring", fn() {
        assert(contains("hello world", "world"));
        assert(contains("hello world", "hello"));
        assert_not(contains("hello world", "foo"));
    });

    test("index_of() finds substring position", fn() {
        assert_eq(index_of("hello world", "world"), 6);
        assert_eq(index_of("hello world", "hello"), 0);
        assert_eq(index_of("hello world", "foo"), -1);
    });

    test("substring() extracts part of string", fn() {
        assert_eq(substring("hello world", 0, 5), "hello");
        assert_eq(substring("hello world", 6, 11), "world");
    });

    test("upcase() converts to uppercase", fn() {
        assert_eq(upcase("hello"), "HELLO");
        assert_eq(upcase("Hello World"), "HELLO WORLD");
    });

    test("downcase() converts to lowercase", fn() {
        assert_eq(downcase("HELLO"), "hello");
        assert_eq(downcase("Hello World"), "hello world");
    });

    test("trim() removes whitespace", fn() {
        assert_eq(trim("  hello  "), "hello");
        assert_eq(trim("\n\thello\t\n"), "hello");
    });

    test("split() splits string into array", fn() {
        let parts = split("a,b,c", ",");
        assert_eq(len(parts), 3);
        assert_eq(parts[0], "a");
        assert_eq(parts[1], "b");
        assert_eq(parts[2], "c");
    });

    test("join() joins array into string", fn() {
        assert_eq(join(["a", "b", "c"], ","), "a,b,c");
        assert_eq(join(["hello", "world"], " "), "hello world");
    });

    test("html_escape() escapes HTML characters", fn() {
        assert_eq(html_escape("<div>"), "&lt;div&gt;");
        assert_eq(html_escape("a & b"), "a &amp; b");
        assert_eq(html_escape("\"quoted\""), "&quot;quoted&quot;");
    });

    test("html_unescape() unescapes HTML entities", fn() {
        assert_eq(html_unescape("&lt;div&gt;"), "<div>");
        assert_eq(html_unescape("a &amp; b"), "a & b");
    });
});
