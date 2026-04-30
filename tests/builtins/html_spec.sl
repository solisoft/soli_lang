// ============================================================================
// HTML Functions Test Suite
// ============================================================================

describe("html_escape", fn() {
    test("escapes less than and greater than", fn() {
        let result = html_escape("<div>");
        assert(result != "<div>");
        assert(result.contains("&lt;"));
        assert(result.contains("&gt;"));
    });

    test("escapes ampersand", fn() {
        let result = html_escape("a & b");
        assert(result.contains("&amp;"));
    });

    test("returns string unchanged if no special chars", fn() {
        assert_eq(html_escape("hello world"), "hello world");
    });

    test("html_escape with number converts to string", fn() {
        let result = html_escape(123);
        assert(result == "123");
    });
});

describe("html_unescape", fn() {
    test("unescapes basic entities", fn() {
        let result = html_unescape("&lt;div&gt;");
        assert(result != "&lt;div&gt;");
    });

    test("html_unescape expects string", fn() {
        let failed = false;
        try {
            html_unescape(123);
        } catch e {
            failed = true;
        }
        assert(failed);
    });
});

describe("strip_html", fn() {
    test("removes simple HTML tags", fn() {
        assert_eq(strip_html("<div>hello</div>"), "hello");
    });

    test("removes nested tags", fn() {
        assert_eq(strip_html("<p><strong>bold</strong> text</p>"), "bold text");
    });

    test("removes self-closing tags", fn() {
        assert_eq(strip_html("line1<br/>line2"), "line1line2");
    });

    test("handles unclosed tag at end", fn() {
        assert_eq(strip_html("<div>hello"), "hello");
    });

    test("preserves text outside tags", fn() {
        assert_eq(strip_html("before<div>inside</div>after"), "beforeinsideafter");
    });

    test("handles empty string", fn() {
        assert_eq(strip_html(""), "");
    });

    test("strip_html expects string", fn() {
        let failed = false;
        try {
            strip_html(123);
        } catch e {
            failed = true;
        }
        assert(failed);
    });
});

describe("sanitize_html", fn() {
    test("keeps safe paragraph tag", fn() {
        let result = sanitize_html("<p>hello</p>");
        assert(result.contains("hello"));
    });

    test("removes script tags", fn() {
        let result = sanitize_html("<script>alert(1)</script>content");
        assert(result != "<script>alert(1)</script>content");
    });

    test("sanitize_html expects string", fn() {
        let failed = false;
        try {
            sanitize_html(123);
        } catch e {
            failed = true;
        }
        assert(failed);
    });
});