// ============================================================================
// String Functions Test Suite (HTML functions only)
// ============================================================================

describe("String Length", fn() {
    test("len() returns string length", fn() {
        assert_eq(len("hello"), 5);
        assert_eq(len(""), 0);
        assert_eq(len("hello world"), 11);
    });

    test(".len() instance method returns string length", fn() {
        assert_eq("hello".len(), 5);
        assert_eq("".len(), 0);
        assert_eq("hello world".len(), 11);
    });

    test(".len() and .length() return same value", fn() {
        let s = "test string";
        assert_eq(s.len(), s.length());
    });

    test(".len() on variable", fn() {
        let s = "abcdef";
        assert_eq(s.len(), 6);
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

// ============================================================================
// String Instance Methods
// ============================================================================

describe("String.chomp", fn() {
    test("removes trailing newline", fn() {
        assert_eq("hello\n".chomp(), "hello");
    });
    test("removes trailing carriage return", fn() {
        assert_eq("hello\r".chomp(), "hello");
    });
    test("no change when no trailing newline", fn() {
        assert_eq("hello".chomp(), "hello");
    });
});

describe("String.lstrip and String.rstrip", fn() {
    test("lstrip removes leading whitespace", fn() {
        assert_eq("  hello".lstrip(), "hello");
        assert_eq("  hello  ".lstrip(), "hello  ");
    });
    test("rstrip removes trailing whitespace", fn() {
        assert_eq("hello  ".rstrip(), "hello");
        assert_eq("  hello  ".rstrip(), "  hello");
    });
});

describe("String.squeeze", fn() {
    test("squeezes all repeated characters", fn() {
        assert_eq("aaabbbccc".squeeze(), "abc");
    });
    test("squeezes specific characters", fn() {
        assert_eq("aaabbbccc".squeeze("a"), "abbbccc");
    });
    test("no change when no repeats", fn() {
        assert_eq("abc".squeeze(), "abc");
    });
});

describe("String.count", fn() {
    test("counts occurrences of substring", fn() {
        assert_eq("hello world hello".count("hello"), 2);
        assert_eq("aaa".count("a"), 3);
        assert_eq("abc".count("z"), 0);
    });
});

describe("String.gsub and String.sub", fn() {
    test("gsub replaces all matches", fn() {
        assert_eq("hello world".gsub("o", "0"), "hell0 w0rld");
    });
    test("gsub with regex pattern", fn() {
        assert_eq("foo123bar456".gsub("[0-9]+", "#"), "foo#bar#");
    });
    test("gsub with limit", fn() {
        assert_eq("aaa".gsub("a", "b", 2), "bba");
    });
    test("sub replaces first match only", fn() {
        assert_eq("hello hello".sub("hello", "hi"), "hi hello");
    });
});

describe("String.scan", fn() {
    test("scan returns all matches", fn() {
        let result = "foo123bar456".scan("[0-9]+");
        assert_eq(result.length(), 2);
        assert_eq(result.get(0), "123");
        assert_eq(result.get(1), "456");
    });
    test("scan with no matches returns empty array", fn() {
        let result = "hello".scan("[0-9]+");
        assert_eq(result.length(), 0);
    });
});

describe("String.tr", fn() {
    test("transliterates characters", fn() {
        assert_eq("hello".tr("aeiou", "AEIOU"), "hEllO");
    });
    test("leaves unmatched characters", fn() {
        assert_eq("abc".tr("a", "x"), "xbc");
    });
});

describe("String.center, String.ljust, String.rjust", fn() {
    test("center pads both sides", fn() {
        assert_eq("hi".center(6), "  hi  ");
    });
    test("center with custom pad char", fn() {
        assert_eq("hi".center(6, "-"), "--hi--");
    });
    test("center returns original if wider", fn() {
        assert_eq("hello".center(3), "hello");
    });
    test("ljust pads right side", fn() {
        assert_eq("hi".ljust(5), "hi   ");
    });
    test("ljust with custom pad char", fn() {
        assert_eq("hi".ljust(5, "."), "hi...");
    });
    test("rjust pads left side", fn() {
        assert_eq("hi".rjust(5), "   hi");
    });
    test("rjust with custom pad char", fn() {
        assert_eq("hi".rjust(5, "0"), "000hi");
    });
});

describe("String.ord", fn() {
    test("returns Unicode code point of first char", fn() {
        assert_eq("A".ord(), 65);
        assert_eq("a".ord(), 97);
        assert_eq("0".ord(), 48);
    });
});

describe("String.bytes and String.chars", fn() {
    test("bytes returns array of byte values", fn() {
        let b = "AB".bytes();
        assert_eq(b.length(), 2);
        assert_eq(b.get(0), 65);
        assert_eq(b.get(1), 66);
    });
    test("chars returns array of characters", fn() {
        let c = "hello".chars();
        assert_eq(c.length(), 5);
        assert_eq(c.get(0), "h");
        assert_eq(c.get(4), "o");
    });
});

describe("String.lines", fn() {
    test("splits string into lines", fn() {
        let l = "line1\nline2\nline3".lines();
        assert_eq(l.length(), 3);
        assert_eq(l.get(0), "line1");
        assert_eq(l.get(2), "line3");
    });
});

describe("String.bytesize", fn() {
    test("returns byte length of string", fn() {
        assert_eq("hello".bytesize(), 5);
        assert_eq("".bytesize(), 0);
    });
});

describe("String.capitalize", fn() {
    test("capitalizes first letter", fn() {
        assert_eq("hello".capitalize(), "Hello");
        assert_eq("HELLO".capitalize(), "Hello");
        assert_eq("hELLO".capitalize(), "Hello");
    });
    test("empty string returns empty", fn() {
        assert_eq("".capitalize(), "");
    });
});

describe("String.swapcase", fn() {
    test("swaps case of all characters", fn() {
        assert_eq("Hello".swapcase(), "hELLO");
        assert_eq("ABC".swapcase(), "abc");
    });
});

describe("String.insert", fn() {
    test("inserts string at index", fn() {
        assert_eq("hello".insert(0, "X"), "Xhello");
        assert_eq("hello".insert(5, "!"), "hello!");
        assert_eq("hello".insert(2, "--"), "he--llo");
    });
});

describe("String.delete", fn() {
    test("removes all occurrences of substring", fn() {
        assert_eq("hello world".delete("l"), "heo word");
        assert_eq("aabbcc".delete("b"), "aacc");
    });
});

describe("String.delete_prefix and String.delete_suffix", fn() {
    test("delete_prefix removes matching prefix", fn() {
        assert_eq("hello world".delete_prefix("hello "), "world");
    });
    test("delete_prefix returns original if no match", fn() {
        assert_eq("hello".delete_prefix("xyz"), "hello");
    });
    test("delete_suffix removes matching suffix", fn() {
        assert_eq("hello.txt".delete_suffix(".txt"), "hello");
    });
    test("delete_suffix returns original if no match", fn() {
        assert_eq("hello".delete_suffix("xyz"), "hello");
    });
});

describe("String.partition and String.rpartition", fn() {
    test("partition splits at first occurrence", fn() {
        let result = "hello-world-test".partition("-");
        assert_eq(result.get(0), "hello");
        assert_eq(result.get(1), "-");
        assert_eq(result.get(2), "world-test");
    });
    test("partition with no match returns original", fn() {
        let result = "hello".partition("-");
        assert_eq(result.get(0), "hello");
        assert_eq(result.get(1), "");
        assert_eq(result.get(2), "");
    });
    test("rpartition splits at last occurrence", fn() {
        let result = "hello-world-test".rpartition("-");
        assert_eq(result.get(0), "hello-world");
        assert_eq(result.get(1), "-");
        assert_eq(result.get(2), "test");
    });
    test("rpartition with no match", fn() {
        let result = "hello".rpartition("-");
        assert_eq(result.get(0), "");
        assert_eq(result.get(1), "");
        assert_eq(result.get(2), "hello");
    });
});

describe("String.reverse", fn() {
    test("reverses the string", fn() {
        assert_eq("hello".reverse(), "olleh");
        assert_eq("abc".reverse(), "cba");
        assert_eq("".reverse(), "");
    });
});

describe("String.hex and String.oct", fn() {
    test("hex parses hexadecimal string", fn() {
        assert_eq("ff".hex(), 255);
        assert_eq("10".hex(), 16);
    });
    test("oct parses octal string", fn() {
        assert_eq("77".oct(), 63);
        assert_eq("10".oct(), 8);
    });
});

describe("String.truncate", fn() {
    test("truncates long string with ellipsis", fn() {
        assert_eq("hello world".truncate(8), "hello...");
    });
    test("no truncation when shorter than limit", fn() {
        assert_eq("hi".truncate(10), "hi");
    });
    test("truncate with custom suffix", fn() {
        assert_eq("hello world".truncate(8, "~"), "hello w~");
    });
});
