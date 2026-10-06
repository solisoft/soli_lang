# Edge cases across the language: empty literals, truthiness, indexing past the
# ends, numeric limits and precision, unicode, and functions held in data.

describe("Edge cases") do
  context("empty literals") do
    test("empty braces are an empty hash, not a block") do
      result = {}
      assert_eq(type(result), "hash")
      assert_eq(result.length, 0)
    end

    test("[] and hash() are empty") do
      assert_eq([].length, 0)
      assert_eq(hash(), {})
      assert_eq(type(hash()), "hash")
    end
  end

  context("truthiness") do
    test("non-zero numbers and non-empty strings are truthy") do
      assert_eq(1 ? "t" : "f", "t")
      assert_eq(-1 ? "t" : "f", "t")
      assert_eq("non-empty" ? "t" : "f", "t")
      assert_eq("0" ? "t" : "f", "t")
      assert_eq(" " ? "t" : "f", "t")
    end

    test("a postfix if runs on a truthy condition") do
      result = ""
      result = "truthy" if 1
      assert_eq(result, "truthy")
    end

    test("0, empty string, [] and {} are falsy") do
      executed = false
      executed = true if 0
      assert_not(executed)
      executed = true if ""
      assert_not(executed)
      assert_eq([] ? "t" : "f", "f")
      assert_eq({} ? "t" : "f", "f")
    end

    test("nil and false are falsy") do
      assert_eq(nil ? "t" : "f", "f")
      assert_eq(false ? "t" : "f", "f")
    end

    test("0.0 is truthy, unlike 0") do
      assert_eq(0.0 ? "t" : "f", "t")
    end
  end

  context("indexing") do
    test("negative indexes count from the end") do
      numbers = [1, 2, 3]
      assert_eq(numbers[-1], 3)
      assert_eq(numbers[-2], 2)
      assert_eq(numbers[-3], 1)
    end

    test("an index past either end raises") do
      numbers = [1, 2, 3]
      assert_raises("Index out of bounds: 5 (length 3)") do
        numbers[5]
      end
      assert_raises("Index out of bounds: -4 (length 3)") do
        numbers[-4]
      end
    end

    test("a missing hash key reads as nil") do
      settings = {"a": 1}
      assert_null(settings["missing"])
    end
  end

  context("numbers") do
    test("nested parentheses and precedence") do
      assert_eq(((1 + 2) * (3 + 4)) + ((5 - 2) * (8 / 4)), 27)
      assert_eq(2 + 3 * 4 - 1, 13)
      assert_eq((2 + 3) * (4 - 1), 15)
    end

    test("products past 32 bits stay exact Ints") do
      large = 1000000
      assert_eq(large * large, 1000000000000)
      assert_eq(type(large * large), "int")
    end

    test("Int division truncates, Float division does not") do
      assert_eq(10 / 4, 2)
      assert_eq(10.0 / 4, 2.5)
    end

    test("floats carry binary rounding error") do
      sum = 0.1 + 0.2
      assert_eq(sum, 0.30000000000000004)
      assert_ne(sum, 0.3)
      assert(sum > 0.29 && sum < 0.31)
    end

    test("a comparison chain is not Python's: it compares a Bool with an Int") do
      x = 5
      assert(x > 0 && x < 10)
      assert_raises("Cannot compare bool and int") do
        0 < x < 10
      end
    end
  end

  context("unicode") do
    test("len() counts characters, not bytes") do
      text = "Hello 世界 🌍"
      assert_eq(len(text), 10)
      assert_eq(text.chars.length, 10)
      assert_contains(text, "世界")
    end

    test(".length agrees with len()") do
      # `"🌍".length` is 4 (bytes) on both engines while `len("🌍")` is 1;
      # builtins.md documents .len/.length/.size as aliases of len().
      pending("bug: String#length counts bytes, len() counts characters")
      assert_eq("Hello 世界 🌍".length, 10)
    end
  end

  context("functions in data") do
    test("an array of lambdas") do
      operations = [fn(x) { x + 1 }, fn(x) { x * 2 }, fn(x) { x - 3 }]
      assert_eq(operations[0](10), 11)
      assert_eq(operations[1](10), 20)
      assert_eq(operations[2](10), 7)
    end

    test("a hash of lambdas") do
      operations = {"add": fn(a, b) { a + b }, "sub": fn(a, b) { a - b }}
      assert_eq(operations["add"](5, 3), 8)
      assert_eq(operations["sub"](5, 3), 2)
    end
  end

  context("literals as receivers") do
    test("methods and len() work directly on literals") do
      assert_eq("hello".length, 5)
      assert_eq([1, 2, 3].length, 3)
      assert_eq(len("hello"), 5)
      assert_eq(len([1, 2, 3]), 3)
    end
  end

  context("redeclaring") do
    test("a second let of the same name in one scope replaces the first") do
      let value = 1
      let value = 2
      assert_eq(value, 2)
    end
  end

  context("nil") do
    test("nil is its own value, distinct from false and 0") do
      empty = nil
      assert_null(empty)
      assert_not_null(42)
      assert_ne(empty, false)
      assert_ne(empty, 0)
    end
  end
end
