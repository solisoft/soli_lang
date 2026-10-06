# Literals: numbers, strings, booleans, nil, symbols, arrays and hashes — what
# each spelling evaluates to and which type it carries. The spellings are the
# subject (`=>` hashes, `1_000`, quotes): do not run `soli fmt` over this file.

describe("Literals") do
  context("integers") do
    test("evaluate to Int values") do
      assert_eq(type(42), "int")
      assert_eq(42 + 0, 42)
      assert_eq(-10 + 10, 0)
      assert_eq(-0, 0)
    end

    test("underscores separate digits and are ignored") do
      assert_eq(1_000_000, 1000000)
      assert_eq(type(1_000), "int")
    end

    test("the largest Int literal is exact") do
      assert_eq(9223372036854775807 - 9223372036854775806, 1)
    end
  end

  context("floats") do
    test("evaluate to Float values") do
      assert_eq(type(3.14), "float")
      assert_eq(3.14 * 2, 6.28)
      assert_eq(-2.5 + 2.5, 0.0)
    end

    test("a whole float keeps its Float type") do
      assert_eq(type(3.0), "float")
      assert_eq(3.0.to_s, "3")
    end
  end

  context("decimals") do
    test("a D suffix makes a Decimal") do
      assert_eq(type(19.99D), "decimal")
      assert_eq(19.99D.to_s, "19.99")
    end

    test("trailing zeros are dropped") do
      assert_eq(1.50D.to_s, "1.5")
    end
  end

  context("strings") do
    test("double and single quotes both make strings") do
      assert_eq("hello", 'hello')
      assert_eq(type('single'), "string")
      assert_eq("".length, 0)
    end

    test("escapes are processed in quoted strings") do
      assert_eq("a\tb".length, 3)
      assert_eq("x\"y", 'x"y')
      assert_eq("a\0b".length, 3)
      assert_eq("\e".length, 1)
    end

    test("unicode escapes take 4 hex digits or 1 to 6 in braces") do
      assert_eq("caf\u00e9", "café")
      assert_eq("\u{1F600}", "😀")
    end

    test("raw strings keep backslashes literally") do
      assert_eq(r"C:\n".length, 4)
      assert_eq("""a\nb""".length, 4)
      assert_eq(r"\t", "\\t")
    end

    test("interpolation evaluates the expression inside") do
      count = 2
      assert_eq("#{count} items", "2 items")
      assert_eq("#{count * 3}", "6")
    end
  end

  context("booleans and nil") do
    test("true and false are Bool") do
      assert_eq(type(true), "bool")
      assert_eq(type(false), "bool")
      assert(true)
      assert_not(false)
    end

    test("nil is the null value") do
      assert_null(nil)
      assert_eq(type(nil), "null")
      assert_eq(nil, null)
    end
  end

  context("symbols") do
    test("a colon prefix makes a Symbol") do
      assert_eq(type(:name), "symbol")
      assert_eq(:name.to_s, "name")
      assert_eq(:name, :name)
    end
  end

  context("arrays") do
    test("hold their elements in order") do
      numbers = [1, 2, 3]
      assert_eq(numbers.length, 3)
      assert_eq(numbers[0], 1)
      assert_eq(numbers[2], 3)
    end

    test("can be empty") do
      assert_eq([].length, 0)
      assert_eq(type([]), "array")
    end

    test("can mix element types") do
      mixed = [1, "two", 3.0, nil, true]
      assert_eq(mixed.map { |x| type(x) }, ["int", "string", "float", "null", "bool"])
    end

    test("nest") do
      grid = [[1, 2], [3, 4]]
      assert_eq(grid[0][0], 1)
      assert_eq(grid[1][1], 4)
    end

    test("accept a trailing comma") do
      assert_eq([1, 2,], [1, 2])
    end
  end

  context("hashes") do
    test("with quoted keys") do
      settings = {"a": 1, "b": 2}
      assert_eq(settings["a"], 1)
      assert_eq(settings["b"], 2)
    end

    test("with fat arrows") do
      settings = {"a" => 1, "b" => 2}
      assert_eq(settings, {"a": 1, "b": 2})
    end

    test("fat arrows take non-string keys") do
      names = {1 => "one", 2 => "two"}
      assert_eq(names[1], "one")
      assert_eq(names.keys, [1, 2])
    end

    test("with bare keys") do
      settings = {a: 1, b: 2}
      assert_eq(settings["a"], 1)
      assert_eq(settings["b"], 2)
    end

    test("a bare key is the string of its name, not a variable") do
      key = "dynamic"
      settings = {key: 1}
      assert_eq(settings.keys, ["key"])
      assert_eq(settings["key"], 1)
    end

    test("can be empty") do
      assert_eq({}.length, 0)
      assert_eq(type({}), "hash")
    end

    test("keep insertion order") do
      settings = {"z": 1, "a": 2, "m": 3}
      assert_eq(settings.keys, ["z", "a", "m"])
    end
  end
end
