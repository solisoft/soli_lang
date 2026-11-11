# Unit tests for app/helpers/application_helper.sl — written by `soli new`.
# Helpers are plain functions: call them directly.

describe("ApplicationHelper") do
  test("truncate_text keeps short text and cuts long text, suffix included") do
    assert_eq(truncate_text("hello", 10, "..."), "hello")
    assert_eq(truncate_text("hello world", 8, "..."), "hello...")
  end

  test("capitalize upcases the first letter only") do
    assert_eq(capitalize("hello WORLD"), "Hello WORLD")
  end

  test("link_to escapes the text") do
    assert_eq(link_to("<b>Home</b>", "/"), "<a href=\"/\">&lt;b&gt;Home&lt;/b&gt;</a>")
    assert_eq(link_to_class("Docs", "https://example.com", "btn"),
      "<a href=\"https://example.com\" class=\"btn\">Docs</a>")
  end

  test("link_to keeps relative links and refuses script URLs") do
    assert_eq(link_to("Intro", "docs/intro?page=2"), "<a href=\"docs/intro?page=2\">Intro</a>")
    assert_eq(link_to("x", "javascript:alert(1)"), "<a href=\"#\">x</a>")
  end

  test("pluralize picks the word from the count") do
    assert_eq(pluralize(1, "person", "people"), "1 person")
    assert_eq(pluralize(2, "person", "people"), "2 people")
    assert_eq(pluralize_simple(1, "post"), "1 post")
    assert_eq(pluralize_simple(3, "post"), "3 posts")
  end

  test("absolute_url ends with the path") do
    assert(absolute_url("/posts").ends_with?("/posts"))
  end
end
