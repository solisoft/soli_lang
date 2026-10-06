# String builtins: length, the HTML helpers, case, whitespace, searching,
# slicing, replacing, comparing and converting. Padding (lpad, rpad, center,
# ljust, rjust) is in string_padding_spec.sl.

describe("string length") do
  test("len(), .len, .length and .size agree on ASCII") do
    assert_eq(len("hello world"), 11)
    assert_eq("hello world".len, 11)
    assert_eq("hello world".length, 11)
    assert_eq("hello world".size, 11)
  end

  test("the empty string has length 0") do
    assert_eq(len(""), 0)
    assert_eq("".length, 0)
  end

  test("len() counts characters") do
    assert_eq(len("héllo"), 5)
  end

  test(".length counts characters, like len()") do
    pending("bug: \"héllo\".length / .len / .size count bytes (6), while len(\"héllo\") counts characters (5)")
    assert_eq("héllo".length, 5)
    assert_eq("héllo".size, 5)
    assert_eq("héllo".len, 5)
  end

  test("bytesize counts UTF-8 bytes") do
    assert_eq("hello".bytesize, 5)
    assert_eq("".bytesize, 0)
    assert_eq("é".bytesize, 2)
  end
end

describe("HTML helpers") do
  test("html_escape escapes markup and quotes") do
    assert_eq(html_escape("<div>Hello</div>"), "&lt;div&gt;Hello&lt;/div&gt;")
    assert_eq(html_escape("a & b \"c\" 'd'"), "a &amp; b &quot;c&quot; &#x27;d&#x27;")
  end

  test("html_unescape decodes named and numeric entities") do
    assert_eq(html_unescape("&lt;div&gt;Hello&lt;/div&gt;"), "<div>Hello</div>")
    assert_eq(html_unescape("&amp;&quot;&#39;&#x27;"), "&\"''")
  end

  test("html_escape and html_unescape round-trip") do
    original = "<div class=\"test\">it's working</div>"
    assert_eq(html_unescape(html_escape(original)), original)
  end

  test("strip_html removes tags and keeps text") do
    assert_eq(strip_html("<p>Hello <strong>World</strong></p>"), "Hello World")
    assert_eq(strip_html("<p>a<br/>b</p>"), "ab")
    assert_eq(strip_html("no tags"), "no tags")
    assert_eq(strip_html(""), "")
  end

  test("html_entities encodes non-ASCII characters as numeric entities") do
    assert_eq("été".html_entities, "&#233;t&#233;")
    assert_eq("inscrit·e".html_entities, "inscrit&#183;e")
    assert_eq("100€".html_entities, "100&#8364;")
    assert_eq("—".html_entities, "&#8212;")
    assert_eq("😀".html_entities, "&#128512;")
  end

  test("html_entities leaves ASCII markup untouched") do
    assert_eq("<p class=\"x\">it's ok</p>".html_entities, "<p class=\"x\">it's ok</p>")
  end

  test("html_entities is idempotent") do
    once = "L'équipe €".html_entities
    assert_eq(once.html_entities, once)
  end
end

describe("case") do
  test("upcase and downcase, accents included") do
    assert_eq("hello".upcase, "HELLO")
    assert_eq("HELLO".downcase, "hello")
    assert_eq("été".upcase, "ÉTÉ")
    assert_eq("ÉTÉ".downcase, "été")
  end

  test("uppercase and lowercase are aliases") do
    assert_eq("abc".uppercase, "ABC")
    assert_eq("ABC".lowercase, "abc")
  end

  test("capitalize upcases the first letter and downcases the rest") do
    assert_eq("hello".capitalize, "Hello")
    assert_eq("HELLO".capitalize, "Hello")
    assert_eq("hELLO".capitalize, "Hello")
    assert_eq("été".capitalize, "Été")
    assert_eq("".capitalize, "")
  end

  test("swapcase") do
    assert_eq("Hello".swapcase, "hELLO")
    assert_eq("Hello World".swapcase, "hELLO wORLD")
  end
end

describe("camelize") do
  test("snake_case to camelCase by default") do
    assert_eq("foo_bar".camelize, "fooBar")
    assert_eq("foo_bar_baz".camelize, "fooBarBaz")
  end

  test("kebab-case to camelCase") do
    assert_eq("foo-bar".camelize, "fooBar")
    assert_eq("foo-bar-baz".camelize, "fooBarBaz")
  end

  test("camelize(true) is PascalCase") do
    assert_eq("foo_bar".camelize(true), "FooBar")
    assert_eq("foo-bar-baz".camelize(true), "FooBarBaz")
  end

  test("idempotent on camelized input") do
    assert_eq("fooBar".camelize, "fooBar")
    assert_eq("FooBar".camelize(true), "FooBar")
  end

  test("lowercases the first letter by default") do
    assert_eq("FooBar".camelize, "fooBar")
  end

  test("empty string and single word") do
    assert_eq("".camelize, "")
    assert_eq("".camelize(true), "")
    assert_eq("foo".camelize, "foo")
    assert_eq("foo".camelize(true), "Foo")
  end

  test("leading, trailing and doubled separators") do
    assert_eq("_foo_bar".camelize, "fooBar")
    assert_eq("foo_bar_".camelize, "fooBar")
    assert_eq("foo__bar".camelize, "fooBar")
    assert_eq("--foo--bar--".camelize(true), "FooBar")
  end

  test("mixed snake and kebab separators") do
    assert_eq("foo_bar-baz".camelize, "fooBarBaz")
  end

  test("spaces are not separators") do
    assert_eq("foo bar".camelize, "foo bar")
  end
end

describe("slugify") do
  test("lowercases and joins words with dashes") do
    assert_eq("Hello World".slugify, "hello-world")
    assert_eq("Hello, World! Été 2024".slugify, "hello-world-ete-2024")
  end

  test("strips accents and punctuation") do
    assert_eq("Ça va?".slugify, "ca-va")
    assert_eq("C++ & Rust!".slugify, "c-rust")
  end

  test("trims and collapses dashes") do
    assert_eq("  --a  b--  ".slugify, "a-b")
  end

  test("a slug stays as it is; empty stays empty") do
    assert_eq("already-slug".slugify, "already-slug")
    assert_eq("".slugify, "")
  end
end

describe("whitespace") do
  test("trim and its alias strip remove both ends") do
    assert_eq("  hello  ".trim, "hello")
    assert_eq(" \t hi \n".trim, "hi")
    assert_eq(" \t hi \n".strip, "hi")
    assert_eq("".trim, "")
  end

  test("lstrip and rstrip remove one end") do
    assert_eq("  hello  ".lstrip, "hello  ")
    assert_eq("  hello  ".rstrip, "  hello")
  end

  test("chomp removes one trailing line ending") do
    assert_eq("hello\n".chomp, "hello")
    assert_eq("hello\r".chomp, "hello")
    assert_eq("hello\n\n".chomp, "hello\n")
    assert_eq("hello".chomp, "hello")
  end

  test("chomp removes a CRLF line ending whole") do
    pending("bug: \"hello\\r\\n\".chomp is \"hello\\r\"; it strips the \\n and leaves the \\r")
    assert_eq("hello\r\n".chomp, "hello")
  end

  test("chop removes the last character") do
    assert_eq("hello".chop, "hell")
    assert_eq("hello\n".chop, "hello")
    assert_eq("a".chop, "")
    assert_eq("".chop, "")
  end

  test("squeeze collapses runs") do
    assert_eq("aaabbbccc".squeeze, "abc")
    assert_eq("aaabbbccc".squeeze("a"), "abbbccc")
    assert_eq("aabbcc  dd".squeeze(" "), "aabbcc dd")
    assert_eq("abc".squeeze, "abc")
  end
end

describe("searching") do
  test("contains, includes? and include? find a substring") do
    assert("hello world".contains("world"))
    assert_not("hello world".contains("foo"))
    assert("hello".includes?("ell"))
    assert("hello".include?("ell"))
    assert_not("hello".include?("xyz"))
  end

  test("searching is case-sensitive") do
    assert_not("Hello".contains("hello"))
  end

  test("every string contains the empty string") do
    assert("abc".includes?(""))
  end

  test("starts_with and ends_with, with or without ?") do
    assert("hello world".starts_with("hello"))
    assert_not("hello world".starts_with("world"))
    assert("hello world".ends_with("world"))
    assert_not("hello world".ends_with("hello"))
    assert("hello".starts_with?("he"))
    assert("hello".ends_with?("lo"))
    assert_not("".starts_with("a"))
  end

  test("index_of is the position of the first match, or -1") do
    assert_eq("hello world".index_of("world"), 6)
    assert_eq("hello world".index_of("hello"), 0)
    assert_eq("hello world".index_of("foo"), -1)
    assert_eq("".index_of("a"), -1)
  end

  test("count counts non-overlapping occurrences") do
    assert_eq("hello world hello".count("hello"), 2)
    assert_eq("aaa".count("a"), 3)
    assert_eq("aaaa".count("aa"), 2)
    assert_eq("abc".count("z"), 0)
  end

  test("scan returns every regex match") do
    assert_eq("foo123bar456".scan("[0-9]+"), ["123", "456"])
    assert_eq("a1b2".scan("([a-z])(\\d)"), ["a1", "b2"])
  end

  test("scan without a match is empty") do
    assert_eq("hello".scan("[0-9]+"), [])
  end
end

describe("slicing") do
  test("substring takes a start and an end index") do
    assert_eq("hello world".substring(0, 5), "hello")
    assert_eq("hello world".substring(6, 11), "world")
  end

  test("substring is empty when the range is empty or past the end") do
    assert_eq("hello".substring(3, 1), "")
    assert_eq("hello".substring(5, 9), "")
  end

  test("substring counts characters, not bytes") do
    assert_eq("ab↩cd".substring(0, 3), "ab↩")
    assert_eq("ab↩cd".substring(2, 3), "↩")
    assert_eq("ab↩cd".substring(0, 100), "ab↩cd")
  end

  test("indexing reads one character") do
    assert_eq("hello"[0], "h")
    assert_eq("hello"[-1], "o")
  end

  test("indexing past the end raises") do
    assert_raises("Index out of bounds: 10 (length 5)") do
      "hello"[10]
    end
  end

  test("chars, bytes and lines") do
    assert_eq("héllo".chars, ["h", "é", "l", "l", "o"])
    assert_eq("AB".bytes, [65, 66])
    assert_eq("é".bytes, [195, 169])
    assert_eq("line1\nline2\nline3".lines, ["line1", "line2", "line3"])
  end

  test("lines drops a trailing newline and handles CRLF") do
    assert_eq("a\nb\n".lines, ["a", "b"])
    assert_eq("a\r\nb".lines, ["a", "b"])
  end

  test("chars, bytes and lines of the empty string are empty") do
    assert_eq("".chars, [])
    assert_eq("".bytes, [])
    assert_eq("".lines, [])
  end

  test("split on a separator") do
    assert_eq("a,b,c".split(","), ["a", "b", "c"])
    assert_eq("a,,b".split(","), ["a", "", "b"])
    assert_eq("".split(","), [""])
  end

  test("split on a space keeps empty fields") do
    assert_eq("a b  c".split(" "), ["a", "b", "", "c"])
  end

  test("split on the empty string gives the characters") do
    pending("bug: \"abc\".split(\"\") is [\"\", \"a\", \"b\", \"c\", \"\"], with empty strings at both ends")
    assert_eq("abc".split(""), ["a", "b", "c"])
  end

  test("partition splits at the first occurrence") do
    assert_eq("hello-world-test".partition("-"), ["hello", "-", "world-test"])
    assert_eq("hello".partition("-"), ["hello", "", ""])
  end

  test("rpartition splits at the last occurrence") do
    assert_eq("hello-world-test".rpartition("-"), ["hello-world", "-", "test"])
    assert_eq("hello".rpartition("-"), ["", "", "hello"])
  end

  test("reverse, accents included") do
    assert_eq("hello".reverse, "olleh")
    assert_eq("héllo".reverse, "olléh")
    assert_eq("".reverse, "")
  end

  test("chr is the first character") do
    assert_eq("ABC".chr, "A")
    assert_eq("éa".chr, "é")
    assert_eq("".chr, "")
  end

  test("ord is the code point of the first character") do
    assert_eq("A".ord, 65)
    assert_eq("a".ord, 97)
    assert_eq("0".ord, 48)
    assert_eq("é".ord, 233)
    assert_eq("😀".ord, 128512)
  end

  test("ord raises on the empty string") do
    assert_raises("ord on empty string") do
      "".ord
    end
  end
end

describe("building") do
  test("insert at an index") do
    assert_eq("hello".insert(0, "X"), "Xhello")
    assert_eq("hello".insert(5, "!"), "hello!")
    assert_eq("hello".insert(2, "--"), "he--llo")
  end

  test("insert raises outside the string") do
    assert_raises("insert index out of bounds") do
      "hello".insert(10, "X")
    end
    assert_raises("insert expects a non-negative integer index") do
      "hello".insert(-1, "X")
    end
  end

  test("prepend returns a new string") do
    original = "world"
    assert_eq(original.prepend("hello "), "hello world")
    assert_eq(original, "world")
  end

  test("* repeats") do
    assert_eq("ab" * 3, "ababab")
  end

  test("delete removes every occurrence") do
    assert_eq("hello world".delete("l"), "heo word")
    assert_eq("aabbcc".delete("b"), "aacc")
    assert_eq("hello".delete("ll"), "heo")
  end

  test("delete_prefix and delete_suffix") do
    assert_eq("hello world".delete_prefix("hello "), "world")
    assert_eq("hello".delete_prefix("xyz"), "hello")
    assert_eq("hello.txt".delete_suffix(".txt"), "hello")
    assert_eq("hello".delete_suffix("xyz"), "hello")
  end

  test("truncate keeps the result within the limit, suffix included") do
    assert_eq("hello world".truncate(8), "hello...")
    assert_eq("hello!".truncate(5), "he...")
    assert_eq("hello".truncate(5), "hello")
    assert_eq("hi".truncate(10), "hi")
  end

  test("truncate with a custom suffix") do
    assert_eq("hello world".truncate(8, "~"), "hello w~")
  end
end

describe("replacing") do
  test("replace changes every occurrence of a literal") do
    assert_eq("hello world".replace("world", "soli"), "hello soli")
    assert_eq("a.b.c".replace(".", "-"), "a-b-c")
  end

  test("gsub replaces every regex match") do
    assert_eq("hello world".gsub("o", "0"), "hell0 w0rld")
    assert_eq("foo123bar456".gsub("[0-9]+", "#"), "foo#bar#")
    assert_eq("a.b.c".gsub(".", "-"), "-----")
  end

  test("gsub with a limit") do
    assert_eq("aaa".gsub("a", "b", 2), "bba")
  end

  test("replace_all is gsub") do
    assert_eq("hello world".replace_all("o", "0"), "hell0 w0rld")
    assert_eq("foo123bar".replace_all("[0-9]+", "#"), "foo#bar")
    assert_eq("aaa".replace_all("a", "b", 2), "bba")
  end

  test("sub replaces the first match only") do
    assert_eq("hello hello".sub("hello", "hi"), "hi hello")
    assert_eq("abc".sub("x", "y"), "abc")
  end

  test("tr maps characters one to one") do
    assert_eq("hello".tr("aeiou", "AEIOU"), "hEllO")
    assert_eq("abc".tr("a", "x"), "xbc")
  end
end

describe("comparing") do
  test("casecmp orders ignoring case") do
    assert_eq("hello".casecmp("HELLO"), 0)
    assert_eq("Hello".casecmp("hello"), 0)
    assert_eq("apple".casecmp("BANANA"), -1)
    assert_eq("banana".casecmp("APPLE"), 1)
    assert_eq("a".casecmp("ab"), -1)
    assert_eq("É".casecmp("é"), 0)
  end

  test("casecmp? compares ignoring case") do
    assert("hello".casecmp?("HELLO"))
    assert("Hello".casecmp?("hello"))
    assert_not("hello".casecmp?("world"))
    assert_not("hello".casecmp?("hell"))
  end

  test("< and == compare by code point") do
    assert("a" < "b")
    assert("B" < "a")
    assert("abc" == "abc")
  end

  test("empty?, blank? and present?") do
    assert("".empty?)
    assert_not(" ".empty?)
    assert(" ".blank?)
    assert("a".present?)
  end

  test("ascii_only?") do
    assert("hello".ascii_only?)
    assert("".ascii_only?)
    assert_not("café".ascii_only?)
    assert_not("你好".ascii_only?)
  end
end

describe("converting") do
  test("hex and oct parse a number") do
    assert_eq("ff".hex, 255)
    assert_eq("FF".hex, 255)
    assert_eq("10".hex, 16)
    assert_eq("77".oct, 63)
    assert_eq("10".oct, 8)
  end

  test("hex and oct raise on a bad digit") do
    assert_raises("invalid hex") do
      "zz".hex
    end
    assert_raises("invalid octal") do
      "9".oct
    end
  end

  test("parse_json returns any JSON value") do
    assert_eq("{\"a\": [1, 2]}".parse_json, {"a": [1, 2]})
    assert_eq("[1,2]".parse_json, [1, 2])
    assert_eq("42".parse_json, 42)
    assert_eq("\"s\"".parse_json, "s")
  end

  test("parse_json returns an empty hash on invalid JSON") do
    assert_eq("{nope".parse_json, {})
  end

  test("to_h returns a hash, or nil when the JSON is not an object") do
    assert_eq("{\"a\":1}".to_h, {"a": 1})
    assert_null("[1]".to_h)
    assert_null("nope".to_h)
  end

  test("to_s is the string itself") do
    assert_eq("x".to_s, "x")
  end
end

describe("succ and next") do
  test("increment letters, carrying") do
    assert_eq("a".succ, "b")
    assert_eq("z".succ, "aa")
    assert_eq("A".succ, "B")
    assert_eq("Z".succ, "AA")
    assert_eq("az".succ, "ba")
    assert_eq("Zz".succ, "AAa")
  end

  test("increment digits") do
    assert_eq("0".succ, "1")
    assert_eq("9".succ, "10")
    assert_eq("99".succ, "100")
  end

  test("increment alphanumeric runs") do
    assert_eq("aa".succ, "ab")
    assert_eq("zz".succ, "aaa")
    assert_eq("a9".succ, "b0")
  end

  test("next is succ") do
    assert_eq("a".next, "b")
    assert_eq("9".next, "10")
  end

  test("the empty string stays empty") do
    assert_eq("".succ, "")
  end
end
