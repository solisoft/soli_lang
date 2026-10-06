# Symbols: literals, equality, hash keys, String#to_sym and the &:method
# shorthand.

describe("symbol literals") do
  test("a symbol equals itself") do
    assert_eq(:name, :name)
  end

  test("class and type are symbol") do
    assert_eq(:name.class, "symbol")
    assert_eq(type(:name), "symbol")
  end

  test("a symbol is truthy and present") do
    assert_eq(:name ? "truthy" : "falsy", "truthy")
    assert(:name.present?)
    assert_not(:name.blank?)
  end

  test("inspect shows the colon") do
    assert_eq(:name.inspect, ":name")
    assert_eq(:hello_world.inspect, ":hello_world")
  end

  test("to_s and to_string drop the colon") do
    assert_eq(:name.to_s, "name")
    assert_eq(:hello.to_string, "hello")
  end

  test("str and interpolation keep the colon") do
    assert_eq(str(:abc), ":abc")
    assert_eq("#{:abc}", ":abc")
  end

  test("nil? is false") do
    assert_eq(:name.nil?, false)
  end

  test("may end in ? or !") do
    assert_eq(:empty?.inspect, ":empty?")
    assert_eq(:save!.inspect, ":save!")
  end

  test("is_a? names the symbol type") do
    pending("bug: Symbol#is_a? is in the method registry but raises \"symbol does not support methods\"")
    assert(:name.is_a?("symbol"))
  end
end

describe("symbol equality") do
  test("same names are equal") do
    assert_eq(:foo, :foo)
    assert(:foo == :foo)
  end

  test("different names are not") do
    assert(:name != :age)
  end

  test("a symbol never equals the string of its name") do
    assert(:name != "name")
    assert("name" != :name)
  end
end

describe("symbols as hash keys") do
  test("in a literal with colon keys") do
    person = {:name: "John", :age: 30}
    assert_eq(person[:name], "John")
    assert_eq(person[:age], 30)
  end

  # `soli fmt` rewrites `=>` to `:`; keep the arrow, it is what this tests.
  test("in a literal with fat-arrow keys") do
    point = {:x => 1, "y" => 2}
    assert_eq(point[:x], 1)
    assert_eq(point["y"], 2)
    assert_eq(point.keys, [:x, "y"])
  end

  test("a symbol key and a string key of the same name are distinct") do
    values = {:name: "sym_value"}
    values["name"] = "str_value"
    assert_eq(values[:name], "sym_value")
    assert_eq(values["name"], "str_value")
    assert_eq(values.length, 2)
  end

  test("survive to_json") do
    pending("bug: to_json drops symbol keys, {:a => 1}.to_json is \"{}\"")
    assert_eq({:a => 1}.to_json, "{\"a\":1}")
  end

  test("as array values, become strings in JSON") do
    assert_eq([:a, :b].to_json, "[\"a\",\"b\"]")
  end
end

describe("String#to_sym") do
  test("makes the symbol of that name") do
    symbol = "hello".to_sym
    assert_eq(symbol, :hello)
    assert_eq(symbol.class, "symbol")
  end

  test("round-trips through to_s") do
    assert_eq("test".to_sym.to_s, "test")
  end
end

describe("&:method shorthand") do
  test("maps with a method") do
    assert_eq([1, 2, 3].map(&:to_s), ["1", "2", "3"])
  end

  test("filters with a predicate") do
    assert_eq([1, 2, 3, 4, 5, 6].filter(&:even?), [2, 4, 6])
  end
end

describe("symbols in expressions") do
  test("a ternary returns either symbol") do
    assert_eq(true ? :yes : :no, :yes)
    assert_eq(false ? :yes : :no, :no)
  end

  test("sort orders symbols by name") do
    pending("bug: Array#sort leaves symbols in their original order ([:b, :a].sort() is [:b, :a])")
    assert_eq([:b, :a].sort(), [:a, :b])
  end
end
