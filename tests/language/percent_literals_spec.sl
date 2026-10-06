# Percent literal arrays: %w[] strings, %i[] symbols, %n[] numbers.
# Elements are split on any whitespace; nothing inside is escaped or
# interpolated. The literal syntax is the subject here — keep it written
# as %w[…], not as the equivalent plain array.

def get_tags
  %w[ruby javascript python]
end

const ENVIRONMENTS = %w[development staging production]

describe("%w[] string arrays") do
  test("splits words into strings") do
    words = %w[demo test]
    assert_eq(words, ["demo", "test"])
    assert_eq(%w[foo bar baz], ["foo", "bar", "baz"])
  end

  test("an empty literal is an empty array") do
    assert_eq(%w[], [])
  end

  test("a single word") do
    assert_eq(%w[hello], ["hello"])
  end

  test("runs of spaces separate like one") do
    assert_eq(%w[one  two   three], ["one", "two", "three"])
  end

  test("tabs separate words") do
    assert_eq(%w[one	two	three], ["one", "two", "three"])
  end

  test("spans several lines") do
    words = %w[
      one
      two
      three
    ]
    assert_eq(words, ["one", "two", "three"])
  end

  test("keeps underscores, digits, quotes and commas inside a word") do
    assert_eq(%w[hello_world foo_bar], ["hello_world", "foo_bar"])
    assert_eq(%w[test1 test2 test3], ["test1", "test2", "test3"])
    assert_eq(%w[a,b c], ["a,b", "c"])
    assert_eq(%w[it's "q"], ["it's", "\"q\""])
  end

  test("does not interpolate") do
    assert_eq(%w[#{x} y], ["#" + "{x}", "y"])
  end

  test("keeps non-ASCII words whole") do
    assert_eq(%w[é ü], ["é", "ü"])
  end

  test("works with array methods") do
    assert_eq(%w[hello world].map { |word| word.upcase }, ["HELLO", "WORLD"])
    assert_eq(%w[hello world foo bar].filter { |word| word.length > 3 }, ["hello", "world"])
  end

  test("spreads into another array") do
    assert_eq([...%w[one two], "three"], ["one", "two", "three"])
  end
end

describe("%i[] symbol arrays") do
  test("splits words into symbols") do
    assert_eq(%i[demo test], [:demo, :test])
    assert_eq(%i[get post put delete], [:get, :post, :put, :delete])
  end

  test("an empty literal is an empty array") do
    assert_eq(%i[], [])
  end

  test("a single word") do
    assert_eq(%i[hello], [:hello])
  end

  test("spans several lines") do
    permissions = %i[
      read
      write
      execute
    ]
    assert_eq(permissions, [:read, :write, :execute])
  end

  test("elements are symbols, not strings") do
    names = %i[foo bar]
    assert_eq(names.map { |name| name.class }, ["symbol", "symbol"])
    assert_ne(names[0], "foo")
  end

  test("symbols work as hash keys") do
    fields = {}
    %i[name email phone].each do |key|
      fields[key] = "value"
    end
    assert_eq(fields[:name], "value")
    assert_eq(fields[:email], "value")
    assert_eq(fields[:phone], "value")
    assert_eq(fields.length, 3)
  end
end

describe("%n[] number arrays") do
  test("integers") do
    assert_eq(%n[1 2 3], [1, 2, 3])
    assert_eq(%n[1 2 3].map { |n| n.class }, ["int", "int", "int"])
  end

  test("floats") do
    assert_eq(%n[1.5 2.5 3.5], [1.5, 2.5, 3.5])
    assert_eq(%n[1.5 2.5].map { |n| n.class }, ["float", "float"])
  end

  test("integers and floats mixed keep their own types") do
    mixed = %n[1 2.5 3]
    assert_eq(mixed, [1, 2.5, 3])
    assert_eq(mixed.map { |n| n.class }, ["int", "float", "int"])
  end

  test("exponent notation is a float") do
    assert_eq(%n[1e3], [1000.0])
    assert_eq(%n[1e3][0].class, "float")
  end

  test("negative numbers and zero") do
    assert_eq(%n[-5 0 5], [-5, 0, 5])
  end

  test("an empty literal is an empty array") do
    assert_eq(%n[], [])
  end

  test("a single number") do
    assert_eq(%n[42], [42])
  end

  test("spans several lines") do
    numbers = %n[
      10
      20
      30
    ]
    assert_eq(numbers, [10, 20, 30])
  end

  test("a D suffix makes a decimal, normalized like a decimal literal") do
    decimals = %n[1.5D 2.5D 3D]
    assert_eq(decimals.map { |d| d.class }, ["decimal", "decimal", "decimal"])
    assert_eq(decimals.map { |d| str(d) }, ["1.5", "2.5", "3.00"])
  end

  test("ints, floats and decimals in one literal") do
    mixed = %n[1 2.5 3.5D]
    assert_eq(mixed.map { |n| n.class }, ["int", "float", "decimal"])
    assert_eq(str(mixed[2]), "3.5")
  end

  test("decimals from the literal do exact arithmetic") do
    decimals = %n[0.1D 0.2D]
    assert_eq(str(decimals[0] + decimals[1]), "0.3")
  end

  test("underscores separate digits as in a number literal") do
    pending("bug: %n[1_000 2] gives [0, 2] — an element that does not parse silently becomes 0")
    assert_eq(%n[1_000 2], [1000, 2])
  end

  test("works with array methods and arithmetic") do
    assert_eq(%n[1 2 3 4 5].reduce(fn(acc, x) acc + x, 0), 15)
    tens = %n[10 20 30]
    assert_eq(tens[0] + tens[1], 30)
    assert_eq(tens[2] - tens[0], 20)
  end

  test("% followed by a space is still the modulo operator") do
    assert_eq(10 % 3, 1)
    remainder = 10
    remainder %= 4
    assert_eq(remainder, 2)
  end
end

describe("Percent literals in context") do
  test("as a function's return value") do
    assert_eq(get_tags(), ["ruby", "javascript", "python"])
  end

  test("in a conditional expression") do
    environment = "staging"
    allowed = %w[dev staging prod]
    assert_eq(allowed.includes?(environment) ? "yes" : "no", "yes")
    assert_eq(allowed.includes?("qa") ? "yes" : "no", "no")
  end

  test("as a constant") do
    assert_eq(ENVIRONMENTS, ["development", "staging", "production"])
  end

  test("nested inside a regular array") do
    assert_eq([%w[a b], %w[c d]], [["a", "b"], ["c", "d"]])
  end
end
