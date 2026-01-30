// ============================================================================
// String Functions Test Suite (HTML functions only)
// ============================================================================

describe("String Length", fn() {
    test("len() returns string length", fn() {
        assert_eq(len("hello"), 5);
        assert_eq(len(""), 0);
        assert_eq(len("hello world"), 11);
    });
});

describe("HTML String Functions", fn() {
    test("html_escape() escapes HTML characters", fn() {
        let result = html_escape("<div>Hello</div>");
        assert_eq(result, "&lt;div&gt;Hello&lt;/div&gt;");
    });

    test("html_unescape() unescapes HTML entities", fn() {
        let result = html_unescape("&lt;div&gt;Hello&lt;/div&gt;");
        assert_eq(result, "<div>Hello</div>");
    });

    test("html_escape and html_unescape round-trip", fn() {
        let original = "<div class=\"test\">it's working</div>";
        let escaped = html_escape(original);
        let unescaped = html_unescape(escaped);
        assert_eq(unescaped, original);
    });
});

describe("HTML Sanitization", fn() {
    test("strip_html() removes all HTML tags", fn() {
        let html = "<p>Hello <strong>World</strong></p>";
        let text = strip_html(html);
        assert_eq(text, "Hello World");
    });

    test("strip_html() with empty string", fn() {
        assert_eq(strip_html(""), "");
    });
});

describe("String Edge Cases", fn() {
    test("empty string operations", fn() {
        assert_eq(len(""), 0);
        assert_eq(strip_html(""), "");
    });

    test("single character string", fn() {
        assert_eq(len("a"), 1);
    });
});
