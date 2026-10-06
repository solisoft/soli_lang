# Regression coverage for the SEC-012 link_to URL safety check.
#
# The helpers live in `src/scaffold/templates/application_helper.sl`, copied
# verbatim into new apps. A spec cannot import outside its own directory, so
# they are inlined here: keep this copy in step with the template. A regression
# in the copy or in the string builtins it uses (`downcase`, `starts_with`,
# `index_of`, `substring`, `contains`) shows up here.

def _is_safe_link_url(url)
  lower = url.downcase
  return true if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
  return true if lower.starts_with("/") || lower.starts_with("#") || lower.starts_with("?")

  # No allowed scheme prefix: relative only if there is no `:` before the
  # first /?# — anything else is a custom scheme like javascript: or data:.
  cut = len(lower)
  slash_at = lower.index_of("/")
  cut = slash_at if slash_at != -1 && slash_at < cut
  query_at = lower.index_of("?")
  cut = query_at if query_at != -1 && query_at < cut
  fragment_at = lower.index_of("#")
  cut = fragment_at if fragment_at != -1 && fragment_at < cut
  !lower.substring(0, cut).contains(":")
end

def _safe_link_url(url)
  return url if _is_safe_link_url(url)

  "#"
end

def link_to(text: String, url: String) -> String
  "<a href=\"" + html_escape(_safe_link_url(url)) + "\">" + html_escape(text) + "</a>"
end

def link_to_class(text: String, url: String, css_class: String) -> String
  href = html_escape(_safe_link_url(url))
  "<a href=\"" + href + "\" class=\"" + html_escape(css_class) + "\">" + html_escape(text) + "</a>"
end

describe("link_to URL safety (SEC-012)") do
  describe("allowed") do
    test("http and https, in any case") do
      assert(_is_safe_link_url("http://example.com/x"))
      assert(_is_safe_link_url("https://example.com/x"))
      assert(_is_safe_link_url("HTTPS://example.com/x"))
    end

    test("mailto, in any case") do
      assert(_is_safe_link_url("mailto:alice@example.com"))
      assert(_is_safe_link_url("MAILTO:bob@example.com"))
    end

    test("relative paths, queries and fragments") do
      assert(_is_safe_link_url("/users/42"))
      assert(_is_safe_link_url("#section"))
      assert(_is_safe_link_url("?q=hi"))
      assert(_is_safe_link_url("posts/new"))
      assert(_is_safe_link_url("../rel"))
    end

    test("a colon after the first / ? or # is part of a relative URL") do
      assert(_is_safe_link_url("a/b:c"))
      assert(_is_safe_link_url("?a=b:c"))
      assert(_is_safe_link_url("#x:y"))
    end

    test("a percent-encoded colon is not a scheme separator") do
      assert(_is_safe_link_url("javascript%3Aalert(1)"))
    end

    test("the empty string is a relative URL") do
      assert(_is_safe_link_url(""))
    end
  end

  describe("refused") do
    test("the javascript scheme, in any case") do
      assert_not(_is_safe_link_url("javascript:alert(1)"))
      assert_not(_is_safe_link_url("JAVASCRIPT:alert(1)"))
    end

    test("a scheme hidden behind leading whitespace or a tab") do
      assert_not(_is_safe_link_url("  javascript:alert(1)"))
      assert_not(_is_safe_link_url("java\tscript:alert(1)"))
    end

    test("the data scheme") do
      assert_not(_is_safe_link_url("data:text/html,<script>alert(1)</script>"))
    end

    test("exotic schemes") do
      assert_not(_is_safe_link_url("vbscript:msgbox(1)"))
      assert_not(_is_safe_link_url("file:///etc/passwd"))
      assert_not(_is_safe_link_url("about:blank"))
    end

    test("a colon before the first slash reads as a scheme") do
      assert_not(_is_safe_link_url("posts:1/x"))
    end
  end

  describe("_safe_link_url") do
    test("maps unsafe URLs to '#'") do
      assert_eq(_safe_link_url("javascript:alert(1)"), "#")
      assert_eq(_safe_link_url("data:text/html,X"), "#")
    end

    test("passes safe URLs through unchanged") do
      assert_eq(_safe_link_url("https://example.com"), "https://example.com")
      assert_eq(_safe_link_url("/users/42"), "/users/42")
      assert_eq(_safe_link_url("posts/new"), "posts/new")
    end
  end

  describe("link_to and link_to_class") do
    test("an unsafe URL becomes href=\"#\" and the text is escaped") do
      assert_eq(link_to("Hi <b>", "javascript:alert(1)"), "<a href=\"#\">Hi &lt;b&gt;</a>")
    end

    test("a safe URL and the class are HTML-escaped") do
      html = link_to_class("A", "/x?a=1&b=2", "btn \"x\"")
      assert_eq(html, "<a href=\"/x?a=1&amp;b=2\" class=\"btn &quot;x&quot;\">A</a>")
    end
  end
end
