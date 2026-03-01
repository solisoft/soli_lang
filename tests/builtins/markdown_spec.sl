// ============================================================================
// Markdown Class Test Suite
// ============================================================================

describe("Markdown.to_html()", fn() {
    test("converts headings", fn() {
        let html = Markdown.to_html("# Heading 1");
        assert_contains(html, "<h1>Heading 1</h1>");

        let html2 = Markdown.to_html("## Heading 2");
        assert_contains(html2, "<h2>Heading 2</h2>");

        let html3 = Markdown.to_html("### Heading 3");
        assert_contains(html3, "<h3>Heading 3</h3>");
    });

    test("converts bold and italic", fn() {
        let html = Markdown.to_html("**bold** and *italic*");
        assert_contains(html, "<strong>bold</strong>");
        assert_contains(html, "<em>italic</em>");
    });

    test("converts links", fn() {
        let html = Markdown.to_html("[Soli](https://example.com)");
        assert_contains(html, "<a href=\"https://example.com\">Soli</a>");
    });

    test("converts unordered lists", fn() {
        let md = "- apple\n- banana\n- cherry";
        let html = Markdown.to_html(md);
        assert_contains(html, "<ul>");
        assert_contains(html, "<li>apple</li>");
        assert_contains(html, "<li>banana</li>");
        assert_contains(html, "<li>cherry</li>");
    });

    test("converts ordered lists", fn() {
        let md = "1. first\n2. second\n3. third";
        let html = Markdown.to_html(md);
        assert_contains(html, "<ol>");
        assert_contains(html, "<li>first</li>");
    });

    test("converts code blocks", fn() {
        let md = "```\nlet x = 1\n```";
        let html = Markdown.to_html(md);
        assert_contains(html, "<code>");
        assert_contains(html, "let x = 1");
    });

    test("converts inline code", fn() {
        let html = Markdown.to_html("use `println` here");
        assert_contains(html, "<code>println</code>");
    });

    test("converts tables", fn() {
        let md = "| Name | Age |\n|------|-----|\n| Alice | 30 |";
        let html = Markdown.to_html(md);
        assert_contains(html, "<table>");
        assert_contains(html, "<th>Name</th>");
        assert_contains(html, "<td>Alice</td>");
    });

    test("converts strikethrough", fn() {
        let html = Markdown.to_html("~~removed~~");
        assert_contains(html, "<del>removed</del>");
    });

    test("converts blockquotes", fn() {
        let html = Markdown.to_html("> This is a quote");
        assert_contains(html, "<blockquote>");
        assert_contains(html, "This is a quote");
    });

    test("converts paragraphs", fn() {
        let md = "First paragraph.\n\nSecond paragraph.";
        let html = Markdown.to_html(md);
        assert_contains(html, "<p>First paragraph.</p>");
        assert_contains(html, "<p>Second paragraph.</p>");
    });

    test("handles empty string", fn() {
        let html = Markdown.to_html("");
        assert_eq(html, "");
    });

    test("converts horizontal rule", fn() {
        let html = Markdown.to_html("---");
        assert_contains(html, "<hr");
    });

    test("works with string from variable", fn() {
        let content = "# Title\n\nSome **content** from a variable.";
        let html = Markdown.to_html(content);
        assert_contains(html, "<h1>Title</h1>");
        assert_contains(html, "<strong>content</strong>");
    });

    test("works with interpolated strings", fn() {
        let name = "World";
        let html = Markdown.to_html("# Hello #{name}");
        assert_contains(html, "<h1>Hello World</h1>");
    });
});
