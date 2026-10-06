# Array and Hash builtin methods, to_string, and len(). String methods are in
# strings_spec.sl, Base64 in base64_spec.sl; the array and hash *syntax* is
# covered by tests/language/arrays_spec.sl and hashes_spec.sl.

describe("Array") do
  describe("to_string and len") do
    test("to_string renders the elements") do
      assert_eq([1, 2, 3].to_string, "[1, 2, 3]")
      assert_eq(["a", nil, true].to_string, "[a, null, true]")
      assert_eq([].to_string, "[]")
    end

    test("length and len() count elements") do
      assert_eq([1, 2].length, 2)
      assert_eq(len([1, 2]), 2)
      assert_eq([].length, 0)
    end
  end

  describe("reading") do
    test("get reads an index, negative from the end") do
      letters = ["first", "second", "third"]
      assert_eq(letters.get(0), "first")
      assert_eq(letters.get(-1), "third")
      assert_eq(letters.get(-2), "second")
    end

    test("get past the end is nil") do
      assert_null([1].get(5))
    end

    test("first and last") do
      assert_eq([1, 2, 3].first, 1)
      assert_eq([1, 2, 3].last, 3)
    end

    test("first and last of an empty array are nil") do
      assert_null([].first)
      assert_null([].last)
    end

    test("empty?") do
      assert([].empty?)
      assert_not([1, 2, 3].empty?)
    end

    test("includes? and its alias include?") do
      assert([1, 2, 3].includes?(2))
      assert_not([1, 2, 3].includes?(5))
      assert([1, 2].include?(2))
      assert_not([1, 2].include?(3))
    end

    test("find returns the first match") do
      assert_eq([1, 2, 3, 4, 5].find { |x| x > 3 }, 4)
    end

    test("find returns nil without a match") do
      assert_null([1, 2, 3].find { |x| x > 10 })
    end

    test("any? and all?") do
      assert([1, 2, 3, 4].any? { |x| x > 3 })
      assert_not([1, 2, 3, 4].any? { |x| x > 10 })
      assert([2, 4, 6].all? { |x| x % 2 == 0 })
      assert_not([1, 2, 3].all? { |x| x > 2 })
    end

    test("any? is false and all? true on an empty array") do
      assert_not([].any? { |x| x })
      assert([].all? { |x| false })
    end
  end

  describe("mutating") do
    test("push appends") do
      numbers = []
      numbers.push(42)
      assert_eq(numbers, [42])
    end

    test("pop removes and returns the last element") do
      numbers = [1, 2]
      assert_eq(numbers.pop, 2)
      assert_eq(numbers, [1])
    end

    test("pop on an empty array is nil") do
      assert_null([].pop)
    end

    test("clear empties the array") do
      numbers = [1, 2, 3]
      numbers.clear
      assert_eq(numbers, [])
    end
  end

  describe("concat") do
    test("appends another array in place") do
      numbers = [1, 2]
      numbers.concat([3, 4])
      assert_eq(numbers, [1, 2, 3, 4])
    end

    test("returns the receiver itself") do
      numbers = [1, 2]
      result = numbers.concat([3])
      numbers.push(99)
      assert_eq(result, [1, 2, 3, 99])
    end

    test("accepts several arrays") do
      numbers = [1]
      numbers.concat([2, 3], [4, 5])
      assert_eq(numbers, [1, 2, 3, 4, 5])
    end

    test("an empty array changes nothing") do
      numbers = [1, 2, 3]
      numbers.concat([])
      assert_eq(numbers, [1, 2, 3])
    end

    test("leaves the argument unchanged") do
      numbers = [1, 2]
      more = [3, 4]
      numbers.concat(more)
      assert_eq(more, [3, 4])
    end

    test("raises on a non-array") do
      assert_raises("Array.concat() argument must be an Array") do
        [1, 2].concat([42][0])
      end
    end
  end

  describe("deriving") do
    test("reverse") do
      assert_eq([1, 2, 3].reverse, [3, 2, 1])
    end

    test("uniq keeps the first of each value") do
      assert_eq([1, 2, 2, 3, 3, 3, "a", "a"].uniq, [1, 2, 3, "a"])
    end

    test("take and drop") do
      assert_eq([1, 2, 3, 4, 5].take(3), [1, 2, 3])
      assert_eq([1, 2, 3, 4, 5].drop(2), [3, 4, 5])
    end

    test("take and drop past the end") do
      assert_eq([1, 2, 3].take(5), [1, 2, 3])
      assert_eq([1, 2, 3].drop(5), [])
      assert_eq([1, 2, 3].take(0), [])
    end

    test("zip pairs elements, stopping at the shorter array") do
      assert_eq([1, 2, 3].zip([4, 5, 6]), [[1, 4], [2, 5], [3, 6]])
      assert_eq([1, 2].zip([3]), [[1, 3]])
    end

    test("compact drops nils") do
      assert_eq([1, nil, 2, nil, 3].compact, [1, 2, 3])
    end

    test("flatten flattens every level, or a given depth") do
      assert_eq([1, [2, 3], [4, [5, 6]]].flatten, [1, 2, 3, 4, 5, 6])
      assert_eq([1, [2, [3]]].flatten(1), [1, 2, [3]])
    end

    test("join") do
      assert_eq([1, 2, 3].join("-"), "1-2-3")
    end
  end

  describe("aggregating") do
    test("sum") do
      assert_eq([1, 2, 3, 4, 5].sum, 15)
      assert_eq([1.5, 2].sum, 3.5)
      assert_eq([].sum, 0)
    end

    test("sum of Ints is an Int") do
      pending("bug: [1, 2].sum is a Float")
      assert_eq(type([1, 2].sum), "int")
    end

    test("min and max") do
      assert_eq([3, 1, 4, 1, 5].min, 1)
      assert_eq([3, 1, 4, 1, 5].max, 5)
    end

    test("fold folds from an initial value") do
      assert_eq([1, 2, 3].fold(fn(acc, x) { acc + x }, 10), 16)
      assert_eq([].fold(fn(acc, x) { acc + x }, 0), 0)
    end

    test("reduce with and without an initial value") do
      assert_eq([1, 2, 3].reduce(fn(acc, x) { acc + x }, 0), 6)
      assert_eq([1, 2, 3].reduce(fn(acc, x) { acc * x }), 6)
    end
  end

  describe("sorting") do
    test("sort numbers, Int and Float mixed") do
      assert_eq([3, 1, 4, 1, 5].sort(), [1, 1, 3, 4, 5])
      assert_eq([2.5, 1, 3].sort(), [1, 2.5, 3])
    end

    test("sort strings") do
      assert_eq(["banana", "apple", "cherry"].sort(), ["apple", "banana", "cherry"])
    end

    test("sort_by a hash key") do
      people = [{"name": "Charlie"}, {"name": "Alice"}, {"name": "Bob"}]
      assert_eq(people.sort_by("name").map { |person| person["name"] }, ["Alice", "Bob", "Charlie"])
    end

    test("sort_by a block") do
      assert_eq(["ccc", "a", "bb"].sort_by { |word| word.length }, ["a", "bb", "ccc"])
    end
  end

  describe("randomness") do
    test("sample returns an element") do
      numbers = [1, 2, 3, 4, 5]
      assert(numbers.includes?(numbers.sample))
    end

    test("sample of an empty array is nil") do
      assert_null([].sample)
    end

    test("shuffle keeps the same elements") do
      assert_eq([1, 2, 3, 4, 5].shuffle.sort(), [1, 2, 3, 4, 5])
    end
  end

  test("nil.to_a is an empty array") do
    assert_eq(nil.to_a, [])
    assert_eq(nil.to_array, [])
  end
end

describe("Hash") do
  describe("to_string and len") do
    test("to_string renders the entries") do
      assert_eq({"a": 1, "b": 2}.to_string, "{a => 1, b => 2}")
      assert_eq({}.to_string, "{}")
    end

    test("length and len() count entries") do
      assert_eq({"a": 1, "b": 2}.length, 2)
      assert_eq(len({"a": 1, "b": 2}), 2)
    end

    test("len() raises on a number") do
      assert_raises("len() expects array, string, or hash, got int") do
        len([5][0])
      end
    end
  end

  describe("reading") do
    test("get reads a key, nil when missing") do
      assert_eq({"name": "test"}.get("name"), "test")
      assert_null({"a": 1}.get("missing"))
    end

    test("has_key") do
      flags = {"exists": true}
      assert(flags.has_key("exists"))
      assert_not(flags.has_key("missing"))
    end

    test("keys, values and entries keep insertion order") do
      letters = {"a": 1, "b": 2}
      assert_eq(letters.keys, ["a", "b"])
      assert_eq(letters.values, [1, 2])
      assert_eq(letters.entries, [["a", 1], ["b", 2]])
    end

    test("empty?") do
      assert({}.empty?)
      assert_not({"a": 1}.empty?)
    end

    test("fetch returns the value or the default") do
      assert_eq({"a": 1}.fetch("a"), 1)
      assert_eq({"a": 1}.fetch("missing", "default"), "default")
    end

    test("fetch raises on a missing key without a default") do
      assert_raises("key not found: missing") do
        {"a": 1}.fetch("missing")
      end
    end
  end

  describe("mutating") do
    test("set writes a key") do
      values = {}
      values.set("key", "value")
      assert_eq(values, {"key": "value"})
    end

    test("delete removes a key and returns its value") do
      letters = {"a": 1, "b": 2}
      assert_eq(letters.delete("a"), 1)
      assert_eq(letters, {"b": 2})
    end

    test("delete of a missing key is nil") do
      assert_null({"a": 1}.delete("x"))
    end

    test("clear empties the hash") do
      letters = {"a": 1, "b": 2}
      letters.clear
      assert_eq(letters, {})
    end
  end

  describe("iterating") do
    test("each yields key and value in order") do
      seen = []
      {"a": 1, "b": 2}.each do |key, value|
        seen.push("#{key}=#{value}")
      end
      assert_eq(seen, ["a=1", "b=2"])
    end

    test("map builds a hash from [key, value] pairs") do
      assert_eq({"a": 1, "b": 2}.map { |key, value| [key, value * 10] }, {"a": 10, "b": 20})
    end

    test("filter and select keep matching entries") do
      letters = {"a": 1, "b": 2, "c": 3}
      assert_eq(letters.filter { |key, value| value > 1 }, {"b": 2, "c": 3})
      assert_eq(letters.select { |key, value| value >= 2 }, {"b": 2, "c": 3})
    end

    test("reject drops matching entries") do
      assert_eq({"a": 1, "b": 2, "c": 3}.reject { |key, value| value >= 2 }, {"a": 1})
    end
  end

  describe("deriving") do
    test("merge, the argument winning on a shared key") do
      assert_eq({"a": 1}.merge({"b": 2}), {"a": 1, "b": 2})
      assert_eq({"a": 1, "b": 1}.merge({"b": 2}), {"a": 1, "b": 2})
    end

    test("invert swaps keys and values") do
      assert_eq({"a": 1, "b": 2}.invert, {1: "a", 2: "b"})
    end

    test("transform_values and transform_keys") do
      assert_eq({"a": 1, "b": 2}.transform_values { |value| value * 10 }, {"a": 10, "b": 20})
      assert_eq({"hello": 1, "world": 2}.transform_keys { |key| key.upcase }, {"HELLO": 1, "WORLD": 2})
    end

    test("slice keeps the given keys that exist") do
      assert_eq({"a": 1, "b": 2, "c": 3}.slice(["a", "c", "z"]), {"a": 1, "c": 3})
    end

    test("except drops the given keys") do
      assert_eq({"a": 1, "b": 2, "c": 3}.except(["b", "z"]), {"a": 1, "c": 3})
    end

    test("compact drops nil values") do
      assert_eq({"a": 1, "b": nil, "c": 3}.compact, {"a": 1, "c": 3})
    end
  end
end
