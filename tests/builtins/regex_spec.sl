# The Regex class: matches, find, find_all, capture, replace, replace_all,
# split and escape. Patterns are strings, so a backslash is written twice.

describe("Regex.matches") do
  test("is true when the pattern matches anywhere") do
    assert(Regex.matches("\\d+", "123"))
    assert(Regex.matches("[a-z]+", "hello"))
    assert_not(Regex.matches("\\d+", "hello"))
  end

  test("honours anchors") do
    assert(Regex.matches("^abc$", "abc"))
    assert_not(Regex.matches("^abc$", "xabc"))
  end

  test("honours an inline case-insensitive flag") do
    assert(Regex.matches("(?i)hello", "HELLO"))
  end

  test("raises on an invalid pattern") do
    assert_raises("unclosed group") do
      Regex.matches("(", "x")
    end
  end

  test("compiles a pattern with two \\w groups") do
    pending("bug: \"(\\\\w+)@(\\\\w+)\" fails with \"Compiled regex exceeds size limit of 100000 bytes\"")
    assert(Regex.matches("(\\w+)@(\\w+)", "a@b"))
  end
end

describe("Regex.find and Regex.find_all") do
  test("find returns the first match with its byte offsets") do
    assert_eq(Regex.find("\\d+", "abc123def456"), {"match": "123", "start": 3, "end": 6})
  end

  test("find returns nil without a match") do
    assert_null(Regex.find("\\d+", "abc"))
  end

  test("offsets are in bytes") do
    assert_eq(Regex.find("é+", "caféé"), {"match": "éé", "start": 3, "end": 7})
  end

  test("find_all returns every match") do
    assert_eq(Regex.find_all("\\d+", "abc123def456"), [
      {"match": "123", "start": 3, "end": 6},
      {"match": "456", "start": 9, "end": 12}
    ])
  end

  test("find_all returns an empty array without a match") do
    assert_eq(Regex.find_all("\\d+", "abc"), [])
  end
end

describe("Regex.capture") do
  test("adds each named group to the match") do
    result = Regex.capture("(?P<year>\\d+)-(?P<month>\\d+)", "date: 2024-01")
    assert_eq(result, {"match": "2024-01", "start": 6, "end": 13, "year": "2024", "month": "01"})
  end

  test("accepts the (?<name>) form") do
    assert_eq(Regex.capture("(?<year>\\d+)", "date: 2024")["year"], "2024")
  end

  test("unnamed groups add nothing") do
    assert_eq(Regex.capture("(\\d+)-(\\d+)", "date: 2024-01"), {"match": "2024-01", "start": 6, "end": 13})
  end

  test("returns nil without a match") do
    assert_null(Regex.capture("\\d+", "no numbers here"))
  end
end

describe("Regex.replace and Regex.replace_all") do
  test("replace changes the first match") do
    assert_eq(Regex.replace("\\d+", "abc123def456", "X"), "abcXdef456")
  end

  test("replace_all changes every match") do
    assert_eq(Regex.replace_all("\\d+", "abc123def456", "X"), "abcXdefX")
  end

  test("the replacement can refer to groups") do
    assert_eq(Regex.replace_all("([a-z]+)@([a-z]+)", "a@b c@d", "$2@$1"), "b@a d@c")
    assert_eq(Regex.replace("(?P<user>[a-z]+)@", "a@b", "${user}#"), "a#b")
  end

  test("no match leaves the string unchanged") do
    assert_eq(Regex.replace_all("\\d+", "abc", "X"), "abc")
  end
end

describe("Regex.split") do
  test("splits on every match") do
    assert_eq(Regex.split("\\s+", "hello   world  foo"), ["hello", "world", "foo"])
  end

  test("returns the whole string without a match") do
    assert_eq(Regex.split(",", "abc"), ["abc"])
  end

  test("an empty string splits into one empty string") do
    assert_eq(Regex.split(",", ""), [""])
  end
end

describe("Regex.escape") do
  test("escapes every metacharacter") do
    assert_eq(Regex.escape("hello.world"), "hello\\.world")
    assert_eq(Regex.escape("a.b*c?(d)[e]"), "a\\.b\\*c\\?\\(d\\)\\[e\\]")
  end

  test("an escaped pattern matches the text literally") do
    pattern = Regex.escape("1+1=2")
    assert(Regex.matches(pattern, "is 1+1=2?"))
    assert_not(Regex.matches(pattern, "11=2"))
  end
end
