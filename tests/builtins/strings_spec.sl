// ============================================================================
// String Functions Test Suite
// ============================================================================

describe("String Length", fn() {
    test("len() returns string length", fn() {
        assert_eq(len("hello"), 5);
        assert_eq(len(""), 0);
        assert_eq(len("hello world"), 11);
    });

    test("len() with unicode characters", fn() {
        assert_eq(len("hello"), 5);
        assert_eq(len("世界"), 2);
        assert_eq(len("🎉"), 1);
    });

    test("length() method returns string length", fn() {
        assert_eq("hello".length(), 5);
        assert_eq("".length(), 0);
    });
});

describe("String Case Conversion", fn() {
    test("upcase() converts to uppercase", fn() {
        assert_eq(upcase("hello"), "HELLO");
        assert_eq(upcase("Hello World"), "HELLO WORLD");
    });

    test("downcase() converts to lowercase", fn() {
        assert_eq(downcase("HELLO"), "hello");
        assert_eq(downcase("Hello World"), "hello world");
    });

    test("upcase() method", fn() {
        assert_eq("hello".upcase(), "HELLO");
        assert_eq("Hello".upcase(), "HELLO");
    });

    test("downcase() method", fn() {
        assert_eq("HELLO".downcase(), "hello");
        assert_eq("Hello".downcase(), "hello");
    });
});

describe("String Trimming", fn() {
    test("trim() removes whitespace from both ends", fn() {
        assert_eq(trim("  hello  "), "hello");
        assert_eq(trim("\n\thello\t\n"), "hello");
        assert_eq(trim("  hello world  "), "hello world");
    });

    test("trim() with only whitespace", fn() {
        assert_eq(trim("   "), "");
    });

    test("trim() method", fn() {
        assert_eq("  hello  ".trim(), "hello");
        assert_eq("\n\ttext\t\n".trim(), "text");
    });
});

describe("String Searching", fn() {
    test("contains() checks for substring", fn() {
        assert(contains("hello world", "world"));
        assert(contains("hello world", "hello"));
        assert_not(contains("hello world", "foo"));
    });

    test("contains() with empty string", fn() {
        assert(contains("hello", ""));
        assert(contains("", ""));
    });

    test("index_of() finds substring position", fn() {
        assert_eq(index_of("hello world", "world"), 6);
        assert_eq(index_of("hello world", "hello"), 0);
        assert_eq(index_of("hello world", "foo"), -1);
    });

    test("index_of() returns -1 when not found", fn() {
        assert_eq(index_of("hello", "xyz"), -1);
        assert_eq(index_of("", "test"), -1);
    });

    test("starts_with?() method", fn() {
        assert("hello world".starts_with?("hello"));
        assert_not("hello world".starts_with?("world"));
        assert("".starts_with?(""));
    });

    test("ends_with?() method", fn() {
        assert("hello world".ends_with?("world"));
        assert_not("hello world".ends_with?("hello"));
        assert("".ends_with?(""));
    });

    test("contains?() method", fn() {
        assert("hello world".contains?("world"));
        assert_not("hello world".contains?("xyz"));
    });
});

describe("String Substring Operations", fn() {
    test("substring() extracts part of string", fn() {
        assert_eq(substring("hello world", 0, 5), "hello");
        assert_eq(substring("hello world", 6, 11), "world");
    });

    test("substring() with various ranges", fn() {
        assert_eq(substring("hello", 0, 3), "hel");
        assert_eq(substring("hello", 2, 5), "llo");
    });

    test("substring() with out of bounds", fn() {
        assert_eq(substring("hello", 0, 100), "hello");
        assert_eq(substring("hello", 10, 5), "");
    });

    test("substring() method", fn() {
        assert_eq("hello world".substring(0, 5), "hello");
        assert_eq("hello world".substring(6, 11), "world");
    });
});

describe("String Split and Join", fn() {
    test("split() splits string into array", fn() {
        let parts = split("a,b,c", ",");
        assert_eq(len(parts), 3);
        assert_eq(parts[0], "a");
        assert_eq(parts[1], "b");
        assert_eq(parts[2], "c");
    });

    test("split() with different delimiters", fn() {
        let parts = split("a b c", " ");
        assert_eq(len(parts), 3);
        assert_eq(parts[0], "a");
    });

    test("split() with empty result", fn() {
        let parts = split("a,b,c", "|");
        assert_eq(len(parts), 1);
    });

    test("split() method", fn() {
        let parts = "a,b,c".split(",");
        assert_eq(len(parts), 3);
    });

    test("join() joins array into string", fn() {
        assert_eq(join(["a", "b", "c"], ","), "a,b,c");
        assert_eq(join(["hello", "world"], " "), "hello world");
    });

    test("join() with empty array", fn() {
        assert_eq(join([], ","), "");
    });

    test("join() with single element", fn() {
        assert_eq(join(["only"], ","), "only");
    });
});

describe("String Replace", fn() {
    test("replace() replaces all occurrences", fn() {
        assert_eq(replace("hello world", "world", "soli"), "hello soli");
        assert_eq(replace("aaaa", "a", "b"), "bbbb");
    });

    test("replace() with empty string", fn() {
        assert_eq(replace("hello", "", "-"), "-h-e-l-l-o-");
    });

    test("replace() method", fn() {
        assert_eq("hello world".replace("world", "soli"), "hello soli");
        assert_eq("foo bar foo".replace("foo", "baz"), "baz bar baz");
    });

    test("replace() when pattern not found", fn() {
        assert_eq(replace("hello", "xyz", "abc"), "hello");
    });
});

describe("String Padding", fn() {
    test("lpad() pads string on the left", fn() {
        assert_eq(lpad("hello", 10), "     hello");
        assert_eq(lpad("hi", 5, "*"), "***hi");
    });

    test("lpad() with custom pad character", fn() {
        assert_eq(lpad("test", 10, "-"), "------test");
        assert_eq(lpad("a", 3, "0"), "00a");
    });

    test("lpad() when string is longer than width", fn() {
        assert_eq(lpad("hello", 3), "hello");
    });

    test("rpad() pads string on the right", fn() {
        assert_eq(rpad("hello", 10), "hello     ");
        assert_eq(rpad("hi", 5, "*"), "hi***");
    });

    test("rpad() with custom pad character", fn() {
        assert_eq(rpad("test", 10, "-"), "test------");
        assert_eq(rpad("a", 3, "0"), "a00");
    });

    test("rpad() when string is longer than width", fn() {
        assert_eq(rpad("hello", 3), "hello");
    });
});

describe("HTML String Functions", fn() {
    test("html_escape() escapes HTML characters", fn() {
        assert_eq(html_escape("<div>"), "&lt;div&gt;");
        assert_eq(html_escape("a & b"), "a &amp; b");
        assert_eq(html_escape("\"quoted\""), "&quot;quoted&quot;");
        assert_eq(html_escape("<script>alert('xss')</script>"), "&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;");
    });

    test("html_unescape() unescapes HTML entities", fn() {
        assert_eq(html_unescape("&lt;div&gt;"), "<div>");
        assert_eq(html_unescape("a &amp; b"), "a & b");
        assert_eq(html_unescape("&quot;quoted&quot;"), "\"quoted\"");
    });
});

describe("String Edge Cases", fn() {
    test("empty string operations", fn() {
        assert_eq(len(""), 0);
        assert_eq(trim(""), "");
        assert_eq(split("", ","), [""]);
    });

    test("single character string", fn() {
        assert_eq(len("a"), 1);
        assert_eq("a".upcase(), "A");
        assert_eq("A".downcase(), "a");
    });

    test("whitespace only strings", fn() {
        assert_eq(len("   "), 3);
        assert_eq(trim("   "), "");
    });

    test("special characters", fn() {
        assert_eq(len("hello\nworld"), 11);
        assert_eq(len("hello\tworld"), 11);
    });
});
