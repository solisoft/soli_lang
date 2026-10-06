# Markdown: to_html (trusted input), to_safe_html (untrusted input),
# to_text (plain text) and to_spans (inline runs for the PDF renderer).

describe("Markdown") do
  describe("to_html") do
    test("converts headings by level") do
      assert_eq(Markdown.to_html("# Heading 1"), "<h1>Heading 1</h1>\n")
      assert_eq(Markdown.to_html("## Heading 2"), "<h2>Heading 2</h2>\n")
      assert_eq(Markdown.to_html("### Heading 3"), "<h3>Heading 3</h3>\n")
    end

    test("converts bold and italic") do
      assert_eq(Markdown.to_html("**bold** and *italic*"), "<p><strong>bold</strong> and <em>italic</em></p>\n")
    end

    test("converts links") do
      assert_eq(Markdown.to_html("[Soli](https://example.com)"), "<p><a href=\"https://example.com\">Soli</a></p>\n")
    end

    test("converts unordered lists") do
      html = Markdown.to_html("- apple\n- banana\n- cherry")
      assert_eq(html, "<ul>\n<li>apple</li>\n<li>banana</li>\n<li>cherry</li>\n</ul>\n")
    end

    test("converts ordered lists") do
      assert_eq(Markdown.to_html("1. first\n2. second"), "<ol>\n<li>first</li>\n<li>second</li>\n</ol>\n")
    end

    test("converts fenced code blocks") do
      assert_eq(Markdown.to_html("```\nx = 1\n```"), "<pre><code>x = 1\n</code></pre>\n")
    end

    test("tags a fenced code block with its language") do
      assert_eq(Markdown.to_html("```ruby\nx = 1\n```"), "<pre><code class=\"language-ruby\">x = 1\n</code></pre>\n")
    end

    test("converts inline code") do
      assert_eq(Markdown.to_html("use `println` here"), "<p>use <code>println</code> here</p>\n")
    end

    test("converts tables") do
      html = Markdown.to_html("| Name | Age |\n|------|-----|\n| Alice | 30 |")
      assert_contains(html, "<thead><tr><th>Name</th><th>Age</th></tr></thead>")
      assert_contains(html, "<tr><td>Alice</td><td>30</td></tr>")
    end

    test("converts strikethrough") do
      assert_eq(Markdown.to_html("~~removed~~"), "<p><del>removed</del></p>\n")
    end

    test("converts blockquotes") do
      assert_eq(Markdown.to_html("> This is a quote"), "<blockquote>\n<p>This is a quote</p>\n</blockquote>\n")
    end

    test("splits paragraphs on blank lines") do
      assert_eq(Markdown.to_html("First.\n\nSecond."), "<p>First.</p>\n<p>Second.</p>\n")
    end

    test("converts a horizontal rule") do
      assert_eq(Markdown.to_html("---"), "<hr />\n")
    end

    test("returns an empty string for empty input") do
      assert_eq(Markdown.to_html(""), "")
    end

    test("works on an interpolated string") do
      name = "World"
      assert_eq(Markdown.to_html("# Hello #{name}"), "<h1>Hello World</h1>\n")
    end

    test("passes raw HTML and javascript: links through — it is for trusted input") do
      assert_eq(Markdown.to_html("<b>raw</b>"), "<p><b>raw</b></p>\n")
      assert_contains(Markdown.to_html("[x](javascript:alert(1))"), "href=\"javascript:alert(1)\"")
    end
  end

  describe("to_safe_html") do
    test("escapes raw HTML") do
      html = Markdown.to_safe_html("Hello <script>alert(1)</script>")
      assert_eq(html, "<p>Hello &lt;script&gt;alert(1)&lt;/script&gt;</p>\n")
    end

    test("neutralizes javascript: links") do
      assert_eq(Markdown.to_safe_html("[click](javascript:alert(1))"), "<p><a href=\"#\">click</a></p>\n")
    end

    test("keeps markdown formatting and safe links") do
      html = Markdown.to_safe_html("**ok** [docs](https://example.com)")
      assert_eq(html, "<p><strong>ok</strong> <a href=\"https://example.com\">docs</a></p>\n")
    end
  end

  describe("to_text") do
    test("strips the markup and keeps the structure") do
      text = Markdown.to_text("# Title\n\nSome **bold** and `code`\n\n- a\n- b")
      assert_eq(text, "Title\n\nSome bold and code\n\n- a\n- b")
    end

    test("keeps a link's target after its text") do
      assert_eq(Markdown.to_text("see [the docs](https://example.com)"), "see the docs <https://example.com>")
    end

    test("drops HTML tags and decodes entities") do
      assert_eq(Markdown.to_text("<b>raw</b> &amp; text"), "raw & text")
    end

    test("returns an empty string for empty input") do
      assert_eq(Markdown.to_text(""), "")
    end

    test("refuses something that is not a string") do
      assert_raises("Markdown.to_text() expects string, got int") do
        Markdown.to_text(5)
      end
    end
  end

  describe("to_spans") do
    test("maps inline markdown onto PDF span hashes") do
      spans = Markdown.to_spans("plain **bold** *italic* `code`")
      assert_eq(spans, [
        {"text": "plain "},
        {"text": "bold", "fontWeight": "bold"},
        {"text": " "},
        {"text": "italic", "italic": true},
        {"text": " "},
        {"text": "code", "mono": true}
      ])
    end

    test("only sets the keys a span needs") do
      spans = Markdown.to_spans("**bold**")
      assert_null(spans[0]["italic"])
      assert_null(spans[0]["mono"])
    end

    test("attaches link targets") do
      assert_eq(Markdown.to_spans("[docs](https://example.com)"), [{"text": "docs", "link": "https://example.com"}])
    end

    test("returns no spans for empty input") do
      assert_eq(Markdown.to_spans(""), [])
    end
  end
end
