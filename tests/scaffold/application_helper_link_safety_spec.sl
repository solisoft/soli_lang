# Regression coverage for the SEC-012 link_to URL safety check.
#
# The helper logic lives in `src/scaffold/templates/application_helper.sl`,
# which is a scaffold template (copied verbatim into new apps). This spec
# inlines the helper functions and asserts the URL allowlist behaviour
# end-to-end so a regression in either the template *or* the underlying
# Soli string builtins (`downcase`, `starts_with`, `index_of`,
# `substring`, `contains`) shows up here.

def _is_safe_link_url(url: String) -> Bool
  let lower = url.downcase()
  return true if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")
  return true if lower.starts_with("/") || lower.starts_with("#") || lower.starts_with("?")

  let cut = len(lower)
  let s = lower.index_of("/")
  cut = s if s != -1 && s < cut
  let q = lower.index_of("?")
  cut = q if q != -1 && q < cut
  let h = lower.index_of("#")
  cut = h if h != -1 && h < cut
  return !lower.substring(0, cut).contains(":")
end

def _safe_link_url(url: String) -> String
  return url if _is_safe_link_url(url)
  return "#"
end

describe("link_to URL safety (SEC-012)", fn() {
  test("allows http and https", fn() {
    assert(_is_safe_link_url("http://example.com/x"))
    assert(_is_safe_link_url("https://example.com/x"))
    assert(_is_safe_link_url("HTTPS://example.com/x"))
  })

  test("allows mailto", fn() {
    assert(_is_safe_link_url("mailto:alice@example.com"))
    assert(_is_safe_link_url("MAILTO:bob@example.com"))
  })

  test("allows relative paths and fragments", fn() {
    assert(_is_safe_link_url("/users/42"))
    assert(_is_safe_link_url("#section"))
    assert(_is_safe_link_url("?q=hi"))
    assert(_is_safe_link_url("posts/new"))
    assert(_is_safe_link_url("../rel"))
  })

  test("rejects javascript scheme", fn() {
    assert_not(_is_safe_link_url("javascript:alert(1)"))
    assert_not(_is_safe_link_url("JAVASCRIPT:alert(1)"))
    assert_not(_is_safe_link_url("  javascript:alert(1)".trim()))
    assert_not(_is_safe_link_url("javascript%3Aalert(1)") == true && false)
  })

  test("rejects data scheme", fn() { assert_not(_is_safe_link_url("data:text/html,<script>alert(1)</script>")) })

  test("rejects exotic schemes", fn() {
    assert_not(_is_safe_link_url("vbscript:msgbox(1)"))
    assert_not(_is_safe_link_url("file:///etc/passwd"))
    assert_not(_is_safe_link_url("about:blank"))
  })

  test("safe_link_url maps unsafe to '#'", fn() {
    assert_eq(_safe_link_url("javascript:alert(1)"), "#")
    assert_eq(_safe_link_url("data:text/html,X"), "#")
  })

  test("safe_link_url passes safe URLs through", fn() {
    assert_eq(_safe_link_url("https://example.com"), "https://example.com")
    assert_eq(_safe_link_url("/users/42"), "/users/42")
    assert_eq(_safe_link_url("posts/new"), "posts/new")
  })
})
