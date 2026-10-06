# HTML helpers: html_escape, html_unescape, strip_html and sanitize_html.

describe("html_escape") do
  test("escapes angle brackets") do
    assert_eq(html_escape("<div>"), "&lt;div&gt;")
  end

  test("escapes the ampersand and both quote kinds") do
    assert_eq(html_escape("a & b"), "a &amp; b")
    assert_eq(html_escape("\"'"), "&quot;&#x27;")
  end

  test("returns a string without special characters unchanged") do
    assert_eq(html_escape("hello world"), "hello world")
    assert_eq(html_escape(""), "")
  end

  test("converts a number to a string") do
    assert_eq(html_escape(123), "123")
  end

  test("renders nil as the empty string") do
    pending("bug: html_escape(nil) returns the string \"null\"")
    assert_eq(html_escape(nil), "")
  end
end

describe("html_unescape") do
  test("unescapes the basic entities") do
    assert_eq(html_unescape("&lt;div&gt;"), "<div>")
    assert_eq(html_unescape("&quot;&#39;&#x27;"), "\"''")
  end

  test("unescapes one level only") do
    assert_eq(html_unescape("&amp;amp;"), "&amp;")
  end

  test("round-trips html_escape") do
    original = "<a href=\"x\">Tom & 'Jerry'</a>"
    assert_eq(html_unescape(html_escape(original)), original)
  end

  test("expects a string") do
    assert_raises("html_unescape expects string, got int") do
      html_unescape(123)
    end
  end
end

describe("strip_html") do
  test("removes simple tags") do
    assert_eq(strip_html("<div>hello</div>"), "hello")
  end

  test("removes nested tags") do
    assert_eq(strip_html("<p><strong>bold</strong> text</p>"), "bold text")
  end

  test("removes self-closing tags") do
    assert_eq(strip_html("line1<br/>line2"), "line1line2")
  end

  test("handles an unclosed tag at the end") do
    assert_eq(strip_html("<div>hello"), "hello")
  end

  test("preserves text outside tags") do
    assert_eq(strip_html("before<div>inside</div>after"), "beforeinsideafter")
  end

  test("removes comments and leaves entities alone") do
    assert_eq(strip_html("<!-- c -->x"), "x")
    assert_eq(strip_html("&lt;p&gt;"), "&lt;p&gt;")
  end

  test("handles the empty string") do
    assert_eq(strip_html(""), "")
  end

  test("keeps a bare less-than sign in text") do
    pending("bug: strip_html(\"1 < 2 and 3 > 2\") returns \"1  2\" — a bare < is read as a tag")
    assert_eq(strip_html("1 < 2 and 3 > 2"), "1 < 2 and 3 > 2")
  end

  test("expects a string") do
    assert_raises("strip_html expects string, got int") do
      strip_html(123)
    end
  end
end

describe("sanitize_html") do
  test("keeps safe formatting tags") do
    assert_eq(sanitize_html("<p>hello</p>"), "<p>hello</p>")
    assert_eq(sanitize_html("<b>b</b><i>i</i><div>d</div>"), "<b>b</b><i>i</i><div>d</div>")
  end

  test("removes script elements with their content") do
    assert_eq(sanitize_html("<script>alert(1)</script>content"), "content")
  end

  test("removes event handlers, javascript: links and unsafe images") do
    unsafe_link = "<a href=\"javascript:alert(1)\" onclick=\"x()\">l</a>"
    assert_eq(sanitize_html(unsafe_link), "<a rel=\"noopener noreferrer\">l</a>")
    assert_eq(sanitize_html("<img src=x onerror=alert(1)>"), "")
  end

  test("keeps a safe link and adds rel=noopener noreferrer") do
    assert_eq(
      sanitize_html("<a href=\"https://x.example\">x</a>"),
      "<a href=\"https://x.example\" rel=\"noopener noreferrer\">x</a>"
    )
  end

  test("expects a string") do
    assert_raises("sanitize_html expects string, got int") do
      sanitize_html(123)
    end
  end
end
