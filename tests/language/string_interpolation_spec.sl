# String literals: `#{…}` interpolation, escape sequences, and raw `r"…"`
# strings. Triple-quoted strings live in multiline_strings_spec.sl.

class Point
  x: Int
  y: Int

  new(x: Int, y: Int)
    @x = x
    @y = y
  end

  def to_s
    "(#{@x}, #{@y})"
  end
end

describe("String interpolation") do
  context("expressions") do
    test("a variable") do
      name = "World"
      assert_eq("Hello #{name}!", "Hello World!")
    end

    test("arithmetic") do
      first = 2
      second = 3
      assert_eq("Sum is #{first + second}", "Sum is 5")
      assert_eq("Double is #{first * 10}", "Double is 20")
    end

    test("several interpolations, adjacent or separated") do
      first = "John"
      last = "Doe"
      assert_eq("#{first} #{last}", "John Doe")
      assert_eq("#{first}#{last}", "JohnDoe")
      assert_eq("#{1}#{2}", "12")
    end

    test("a method call") do
      text = "hello"
      assert_eq("Uppercase: #{text.upcase}", "Uppercase: HELLO")
    end

    test("an array index") do
      names = ["Alice", "Bob"]
      assert_eq("First: #{names[0]}", "First: Alice")
    end

    test("a hash lookup with a quoted key") do
      person = {"name": "Charlie"}
      assert_eq("Name: #{person["name"]}", "Name: Charlie")
    end

    test("a ternary with string branches") do
      assert_eq("#{1 > 0 ? "yes" : "no"}", "yes")
    end

    test("a block call") do
      assert_eq("#{[1, 2, 3].map { |x| x * 2 }.join(",")}", "2,4,6")
    end
  end

  context("value formatting") do
    test("nil, booleans, floats, arrays and hashes") do
      assert_eq("#{nil}|#{true}|#{false}|#{3.5}", "null|true|false|3.5")
      assert_eq("#{[1, 2]}", "[1, 2]")
      assert_eq("#{{"a": 1}}", "{a => 1}")
      assert_eq("#{{}}", "{}")
    end

    test("an empty interpolation result leaves nothing behind") do
      empty = ""
      assert_eq("[#{empty}]", "[]")
    end

    test("an instance renders through its to_s") do
      pending("bug: interpolation and str() ignore a user-defined to_s and print the inspect form")
      assert_eq("p=#{new Point(1, 2)}", "p=(1, 2)")
    end
  end

  context("text that is not an interpolation") do
    test("braces without a leading # and a # without braces stay literal") do
      assert_eq("{not interp} #1", "{not interp} #1")
      assert_eq("price: $#{5}", "price: $5")
    end

    test("single quotes interpolate too") do
      assert_eq('single #{1 + 1}', "single 2")
    end
  end
end

describe("Escape sequences") do
  test("\\n, \\t and \\r are one character each") do
    assert_eq("a\nb".length, 3)
    assert_eq("a\tb".chars, ["a", "\t", "b"])
    assert_eq("a\rb".length, 3)
    assert_eq("a\tb".split("\t"), ["a", "b"])
  end

  test("\\\\ and \\\" escape the backslash and the quote") do
    assert_eq("\\".length, 1)
    assert_eq("\"".length, 1)
    assert_eq("say \"hi\"", 'say "hi"')
  end

  test("\\' escapes a single quote in either quote style") do
    assert_eq("it\'s", "it's")
    assert_eq('it\'s', "it's")
  end

  test("\\0 is the NUL byte") do
    assert_eq("\0".length, 1)
    assert_eq("\0".bytes, [0])
  end

  test("\\e is ESC, for ANSI sequences") do
    assert_eq("\e".bytes, [27])
    assert_eq("\e[1m".length, 4)
  end

  test("\\u00e9 is a four-digit Unicode escape") do
    assert_eq("\u00e9", "é")
    assert_eq("\u00e9".chars.length, 1)
    assert_eq("\u00e9".length, 2)
  end

  test("\\u{1F600} is a braced Unicode escape beyond the BMP") do
    assert_eq("\u{1F600}", "😀")
    assert_eq("\u{1F600}".chars.length, 1)
    assert_eq("\u{1F600}".length, 4)
  end

  test("an escaped backslash before an interpolation keeps both") do
    assert_eq("a\\#{1}", "a\\1")
  end
end

describe("Raw strings") do
  test("r\"…\" keeps backslashes literally") do
    path = r"C:\Users\name"
    assert_eq(path.length, 13)
    assert_eq(path, "C:\\Users\\name")
  end

  test("r'…' is raw too") do
    assert_eq(r'a\nb', "a\\nb")
    assert_eq(r'a\nb'.length, 4)
  end

  test("a raw string does not interpolate") do
    name = "x"
    raw = r"#{name}\t"
    assert_eq(raw, "#" + "{name}" + "\\t")
    assert_eq(raw.length, 9)
  end

  test("an empty raw string equals an empty string") do
    assert_eq(r"", "")
  end
end
