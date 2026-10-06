# Arrays: literals, indexing, operators (+, -, ==, spread), the iteration and
# Ruby-compat methods, dig/pluck/pick, set operations and the field-keyed
# family (sum_by, group_by, …) over hashes and instances.

class Member
  name: String

  new(name)
    @name = name
  end
end

class Person
  name: String
  role: String
  score: Int

  new(name: String, role: String, score: Int)
    @name = name
    @role = role
    @score = score
  end
end

const ROWS = [
  {"sku": "a", "cents": 1250, "status": "paid"},
  {"sku": "b", "cents": 900, "status": "open"},
  {"sku": "c", "cents": 350, "status": "paid"}
]

describe("Array operators") do
  context("+") do
    test("concatenates") do
      assert_eq([1, 2] + [3, 4], [1, 2, 3, 4])
    end

    test("returns a new array and leaves both operands alone") do
      left = [1, 2]
      right = [3, 4]
      combined = left + right
      combined.push(5)
      assert_eq(left, [1, 2])
      assert_eq(right, [3, 4])
    end

    test("with an empty array") do
      assert_eq([] + [1], [1])
      assert_eq([1] + [], [1])
    end
  end

  context("-") do
    test("removes matching elements") do
      assert_eq([1, 2, 3] - [1], [2, 3])
    end

    test("removes every occurrence") do
      assert_eq([1, 2, 1, 3, 1] - [1], [2, 3])
    end

    test("with no match returns an equal copy") do
      original = [1, 2, 3]
      result = original - [99]
      assert_eq(result, [1, 2, 3])
      result.push(4)
      assert_eq(original, [1, 2, 3])
    end

    test("with an empty array") do
      assert_eq([1, 2, 3] - [], [1, 2, 3])
      assert_eq([] - [1], [])
    end

    test("works on strings") do
      assert_eq(["apple", "banana", "cherry"] - ["banana"], ["apple", "cherry"])
    end

    test("compares instances by identity") do
      ann = new Member("Ann")
      bo = new Member("Bo")
      assert_eq(([ann, bo] - [ann]).map { |member| member.name }, ["Bo"])
      assert_eq(([ann, bo] - [new Member("Ann")]).length, 2)
    end
  end

  test("== compares element by element, in order") do
    assert([1, 2] == [1, 2])
    assert_not([1, 2] == [2, 1])
    assert_not([1, 2] == [1, 2, 3])
    assert([[1], ["a"]] == [[1], ["a"]])
  end

  test("* is not defined on arrays") do
    assert_raises() do
      [1, 2] * 2
    end
  end

  test("spread builds a new array from several") do
    first = [1, 2]
    second = [3, 4]
    assert_eq([...first, ...second], [1, 2, 3, 4])
    assert_eq([0, ...first, 9], [0, 1, 2, 9])
    assert_eq([...[]], [])
  end
end

describe("Array indexing") do
  test("reads by position") do
    letters = ["a", "b", "c"]
    assert_eq(letters[0], "a")
    assert_eq(letters[1], "b")
    assert_eq(letters[2], "c")
  end

  test("writes by position") do
    numbers = [1, 2, 3]
    numbers[1] = 20
    assert_eq(numbers, [1, 20, 3])
  end

  test("a negative index counts from the end") do
    numbers = [10, 20, 30]
    assert_eq(numbers[-1], 30)
    assert_eq(numbers[-2], 20)
    assert_eq(numbers[-3], 10)
  end

  test("reading past the end raises") do
    assert_raises("Index out of bounds: 5 (length 3)") do
      [1, 2, 3][5]
    end
  end

  test("writing past the end raises") do
    numbers = [1]
    assert_raises("Index out of bounds: 3 (length 1)") do
      numbers[3] = 4
    end
    assert_eq(numbers, [1])
  end

  test("holds mixed types, nil included") do
    mixed = [1, "two", true, nil]
    assert_eq(mixed[0], 1)
    assert_eq(mixed[1], "two")
    assert_eq(mixed[2], true)
    assert_null(mixed[3])
    assert_eq(mixed.length, 4)
  end

  test("indexes nested arrays") do
    grid = [[1, 2], [3, 4], [5, 6]]
    assert_eq(grid[0][1], 2)
    assert_eq(grid[2][0], 5)
  end
end

describe("Core array methods") do
  test("push appends and returns the array; pop removes the last") do
    stack = [1, 2]
    assert_eq(stack.push(3), [1, 2, 3])
    assert_eq(stack.pop, 3)
    assert_eq(stack, [1, 2])
  end

  test("pop on an empty array is nil") do
    assert_null([].pop)
  end

  test("first and last, nil when empty") do
    assert_eq([3, 1, 2].first, 3)
    assert_eq([3, 1, 2].last, 2)
    assert_null([].first)
    assert_null([].last)
  end

  test("sort, reverse, uniq, compact and flatten") do
    assert_eq([3, 1, 2].sort, [1, 2, 3])
    assert_eq([3, 1, 2].reverse, [2, 1, 3])
    assert_eq([1, 1, 2].uniq, [1, 2])
    assert_eq([1, nil, 2].compact, [1, 2])
    assert_eq([1, [2, [3]]].flatten, [1, 2, 3])
  end

  test("sum, min and max; empty gives 0 or nil") do
    assert_eq([1, 2, 3].sum, 6)
    assert_eq([].sum, 0)
    assert_eq([3, 1, 2].min, 1)
    assert_eq([3, 1, 2].max, 3)
    assert_null([].max)
  end

  test("join, take, drop and zip") do
    assert_eq(["a", "b"].join(", "), "a, b")
    assert_eq([1, 2, 3].take(2), [1, 2])
    assert_eq([1, 2, 3].drop(2), [3])
    assert_eq([1, 2].zip(["a", "b"]), [[1, "a"], [2, "b"]])
  end

  test("length and size, includes? and empty?") do
    assert_eq([1, 2, 3].length, 3)
    assert_eq([1, 2, 3].size, 3)
    assert_eq([].size, 0)
    assert([1, 2].includes?(2))
    assert_not([1, 2].includes?(3))
    assert([].empty?)
  end
end

describe("Iteration methods") do
  test("map") do
    assert_eq([1, 2, 3].map { |x| x * 2 }, [2, 4, 6])
    assert_eq([].map { |x| x * 2 }, [])
  end

  test("filter") do
    assert_eq([1, 2, 3, 4, 5].filter { |x| x % 2 == 0 }, [2, 4])
    assert_eq([1, 3].filter { |x| x % 2 == 0 }, [])
  end

  test("each visits every element in order") do
    joined = ""
    ["a", "b", "c"].each do |letter|
      joined = joined + letter
    end
    assert_eq(joined, "abc")
  end

  test("each_with_index passes the element and its index") do
    joined = ""
    ["a", "b", "c"].each_with_index do |letter, i|
      joined = joined + str(i) + letter
    end
    assert_eq(joined, "0a1b2c")
  end

  test("each_with_index returns the array") do
    assert_eq([10, 20, 30].each_with_index { |x, i| x * i }, [10, 20, 30])
  end

  test("each_with_index without a block answers the [item, index] pairs") do
    assert_eq(["a", "b"].each_with_index, [["a", 0], ["b", 1]])
    assert_eq(["a", "b"].each_with_index.map { |letter, i| "#{i}#{letter}" }, ["0a", "1b"])
  end

  test("a block with two parameters destructures an array element") do
    assert_eq([["a", 1], ["b", 2]].map { |name, n| "#{name}#{n}" }, ["a1", "b2"])
    assert_eq([[1, 2], [3, 4]].filter { |a, b| a + b > 4 }, [[3, 4]])
    assert_eq([["a"]].map { |name, n| n.nil? }, [true])
  end

  test("a one-parameter block still gets the element whole") do
    assert_eq([[1, 2]].map { |pair| pair.length }, [2])
  end

  test("reduce folds with an initial value") do
    assert_eq([1, 2, 3, 4].reduce(fn(acc, x) { acc + x }, 0), 10)
    assert_eq([].reduce(fn(acc, x) { acc + x }, 7), 7)
  end
end

describe("index_of") do
  test("finds the first matching element") do
    letters = ["a", "b", "c", "b"]
    assert_eq(letters.index_of("b"), 1)
    assert_eq(letters.index_of("a"), 0)
    assert_eq(letters.index_of("c"), 2)
  end

  test("is -1 when not found") do
    assert_eq([1, 2, 3].index_of(99), -1)
  end

  test("is -1 on an empty array") do
    assert_eq([].index_of(1), -1)
  end
end

describe("slice") do
  test("with a start and an end (exclusive)") do
    assert_eq([1, 2, 3, 4, 5].slice(1, 3), [2, 3])
  end

  test("with a negative start") do
    assert_eq([1, 2, 3, 4, 5].slice(-2), [4, 5])
  end

  test("with a negative end") do
    assert_eq([1, 2, 3, 4, 5].slice(1, -1), [2, 3, 4])
  end

  test("starting past the end is empty") do
    assert_eq([1, 2, 3].slice(5), [])
  end

  test("does not mutate the original") do
    numbers = [1, 2, 3, 4, 5]
    numbers.slice(1, 3)
    assert_eq(numbers, [1, 2, 3, 4, 5])
  end

  test("with no arguments returns a copy") do
    original = [1, 2, 3]
    copy = original.slice()
    assert_eq(copy, [1, 2, 3])
    copy.push(4)
    assert_eq(original, [1, 2, 3])
  end
end

describe("Ruby-compat methods") do
  test("size is an alias for length") do
    assert_eq([1, 2, 3].size, [1, 2, 3].length)
  end

  test("delete removes every matching element") do
    assert_eq([1, 2, 3, 2].delete(2), [1, 3])
  end

  test("delete returns nil if not found") do
    assert_null([1, 2].delete(99))
  end

  test("delete_at removes at an index") do
    assert_eq([1, 2, 3].delete_at(1), [1, 3])
  end

  test("delete_at with a negative index") do
    assert_eq([1, 2, 3].delete_at(-1), [1, 2])
  end

  test("delete_at returns nil out of bounds") do
    assert_null([1].delete_at(5))
  end

  test("shift drops the first element") do
    assert_eq([1, 2, 3].shift, [2, 3])
  end

  test("shift on an empty array is nil") do
    assert_null([].shift)
  end

  test("unshift prepends") do
    assert_eq([1, 2].unshift(0), [0, 1, 2])
  end

  test("insert at an index") do
    assert_eq([1, 3].insert(1, 2), [1, 2, 3])
  end

  test("insert several values") do
    assert_eq([1, 4].insert(1, 2, 3), [1, 2, 3, 4])
  end

  test("rotate by one by default") do
    assert_eq([1, 2, 3].rotate, [2, 3, 1])
  end

  test("rotate by an explicit count") do
    assert_eq([1, 2, 3].rotate(2), [3, 1, 2])
  end

  test("rotate by a negative count") do
    assert_eq([1, 2, 3].rotate(-1), [3, 1, 2])
  end

  test("reject is the opposite of filter") do
    assert_eq([1, 2, 3, 4].reject { |x| x % 2 == 0 }, [1, 3])
  end

  test("none? is true when nothing matches") do
    assert([1, 2].none? { |x| x > 10 })
  end

  test("none? is false when anything matches") do
    assert_not([1, 2].none? { |x| x == 2 })
  end

  test("one? is true when exactly one matches") do
    assert([1, 2, 3].one? { |x| x == 2 })
  end

  test("one? is false when several match") do
    assert_not([1, 2, 2].one? { |x| x == 2 })
  end

  test("values_at returns the selected indices") do
    assert_eq([10, 20, 30, 40].values_at(0, 2), [10, 30])
  end

  test("values_at with a negative index") do
    assert_eq([10, 20, 30].values_at(0, -1), [10, 30])
  end

  test("count with no argument is the length") do
    assert_eq([1, 2, 3].count, 3)
  end

  test("count with a value") do
    assert_eq([1, 2, 2, 3].count(2), 2)
  end

  test("count with a block") do
    assert_eq([1, 2, 3, 4].count { |x| x % 2 == 0 }, 2)
  end
end

describe("Array#dig") do
  test("with an integer index") do
    numbers = [10, 20, 30]
    assert_eq(numbers.dig(0), 10)
    assert_eq(numbers.dig(2), 30)
  end

  test("with a negative index") do
    numbers = [10, 20, 30]
    assert_eq(numbers.dig(-1), 30)
    assert_eq(numbers.dig(-2), 20)
    assert_eq(numbers.dig(-3), 10)
  end

  test("is nil out of bounds, instead of raising") do
    numbers = [1, 2, 3]
    assert_null(numbers.dig(5))
    assert_null(numbers.dig(-10))
  end

  test("into nested arrays") do
    nested = [[1, 2], [3, [4, 5]]]
    assert_eq(nested.dig(0, 1), 2)
    assert_eq(nested.dig(1, 1, 0), 4)
  end

  test("into hashes inside the array") do
    users = [{"user": {"name": "Alice"}}, {"user": {"name": "Bob"}}]
    assert_eq(users.dig(0, "user", "name"), "Alice")
    assert_eq(users.dig(1, "user", "name"), "Bob")
  end

  test("along a mixed array and hash path") do
    data = [{"items": [10, {"price": 99}]}]
    assert_eq(data.dig(0, "items", 1, "price"), 99)
  end

  test("stops at the first miss and returns nil") do
    nested = [[1, 2], [3, 4]]
    assert_null(nested.dig(0, 5))
    assert_null(nested.dig(9, 0))
  end

  test("needs at least one key, as Hash#dig does") do
    assert_raises("Wrong number of arguments: expected 1, got 0") do
      [1, 2].dig()
    end
  end
end

describe("Array#pluck") do
  test("one field from an array of hashes") do
    posts = [
      {"id": 1, "title": "Hello"},
      {"id": 2, "title": "World"},
      {"id": 3, "title": "Soli"}
    ]
    assert_eq(posts.pluck("title"), ["Hello", "World", "Soli"])
    assert_eq(posts.pluck("id"), [1, 2, 3])
  end

  test("nil for a missing key") do
    users = [{"name": "Alice"}, {"name": "Bob", "email": "bob@example.com"}]
    assert_eq(users.pluck("email"), [nil, "bob@example.com"])
  end

  test("several fields give an array of rows") do
    data = [{"name": "Alice", "age": 30}, {"name": "Bob", "age": 25}]
    assert_eq(data.pluck("name", "age"), [["Alice", 30], ["Bob", 25]])
  end

  test("integer indices on arrays of arrays") do
    rows = [[10, "foo"], [20, "bar"], [30, "baz"]]
    assert_eq(rows.pluck(0), [10, 20, 30])
    assert_eq(rows.pluck(1), ["foo", "bar", "baz"])
  end

  test("several indices give rows as arrays") do
    assert_eq([[1, 2, 3], [4, 5, 6]].pluck(0, 2), [[1, 3], [4, 6]])
  end

  test("an empty array plucks to an empty array") do
    assert_eq([].pluck("anything"), [])
  end

  test("elements that are neither hash nor array give nil") do
    assert_eq([{"x": 1}, "not a hash", 42, nil].pluck("x"), [1, nil, nil, nil])
  end
end

describe("Array#pick") do
  test("one field from the first element") do
    posts = [{"id": 1, "title": "Hello"}, {"id": 2, "title": "World"}]
    assert_eq(posts.pick("title"), "Hello")
    assert_eq(posts.pick("id"), 1)
  end

  test("nil on an empty array") do
    assert_null([].pick("title"))
  end

  test("nil when the first element lacks the key, even if a later one has it") do
    users = [{"name": "Alice"}, {"name": "Bob", "email": "bob@x.com"}]
    assert_null(users.pick("email"))
  end

  test("several fields give the first element's values") do
    rows = [{"name": "Alice", "age": 30, "active": true}, {"name": "Bob", "age": 25}]
    assert_eq(rows.pick("name", "age"), ["Alice", 30])
  end

  test("integer indices on arrays of arrays") do
    rows = [[10, "foo"], [20, "bar"]]
    assert_eq(rows.pick(1), "foo")
    assert_eq(rows.pick(0, 1), [10, "foo"])
  end

  test("nil when the first element is not a hash") do
    assert_null(["string", {"x": 1}].pick("x"))
  end
end

describe("Set operations") do
  context("intersection") do
    test("keeps shared elements in receiver order") do
      assert_eq([1, 2, 3].intersection([2, 3, 4]), [2, 3])
    end

    test("deduplicates") do
      assert_eq([1, 1, 2, 2, 3].intersection([1, 2]), [1, 2])
    end

    test("with no overlap is empty") do
      assert_eq([1, 2, 3].intersection([4, 5, 6]), [])
    end

    test("with an empty receiver is empty") do
      assert_eq([].intersection([1, 2, 3]), [])
    end

    test("with an empty argument is empty") do
      assert_eq([1, 2, 3].intersection([]), [])
    end

    test("works on strings") do
      assert_eq(["a", "b", "c"].intersection(["b", "c", "d"]), ["b", "c"])
    end
  end

  context("union") do
    test("keeps everything, receiver first, deduplicated") do
      assert_eq([1, 2, 3].union([2, 3, 4]), [1, 2, 3, 4])
    end

    test("deduplicates within the receiver and across") do
      assert_eq([1, 1, 2].union([2, 3]), [1, 2, 3])
    end

    test("with an empty argument is the deduplicated receiver") do
      assert_eq([1, 1, 2, 3].union([]), [1, 2, 3])
    end

    test("with an empty receiver is the deduplicated argument") do
      assert_eq([].union([1, 2, 2, 3]), [1, 2, 3])
    end

    test("of two empty arrays is empty") do
      assert_eq([].union([]), [])
    end
  end

  context("difference") do
    test("keeps receiver elements not in the argument") do
      assert_eq([1, 2, 3].difference([2, 3]), [1])
    end

    test("deduplicates, unlike -") do
      assert_eq([1, 1, 2, 2, 3].difference([3]), [1, 2])
      assert_eq([1, 1, 2, 2, 3] - [3], [1, 1, 2, 2])
    end

    test("with no overlap is the deduplicated receiver") do
      assert_eq([1, 1, 2].difference([3, 4]), [1, 2])
    end

    test("is empty when the argument holds everything") do
      assert_eq([1, 2, 3].difference([1, 2, 3]), [])
    end

    test("with an empty argument is the deduplicated receiver") do
      assert_eq([1, 1, 2, 3].difference([]), [1, 2, 3])
    end

    test("with an empty receiver is empty") do
      assert_eq([].difference([1, 2]), [])
    end
  end

  test("leave both operands unchanged") do
    left = [1, 2, 3]
    right = [2, 3, 4]
    left.intersection(right)
    left.union(right)
    left.difference(right)
    assert_eq(left, [1, 2, 3])
    assert_eq(right, [2, 3, 4])
  end
end

describe("Field-keyed array methods") do
  test("sum_by keeps integers integral and skips missing fields") do
    assert_eq(ROWS.sum_by("cents"), 2500)
    assert_eq(ROWS.sum_by("nope"), 0)
  end

  test("sum_by promotes to float only once a float appears") do
    assert_eq([{"n": 1.5}, {"n": 2}].sum_by("n"), 3.5)
  end

  test("avg is a ratio, never integer division") do
    assert_eq([2, 3].avg(), 2.5)
    assert_eq([{"n": 90}, {"n": 95}].avg_by("n"), 92.5)
  end

  test("averaging nothing is nil, not a misleading zero") do
    assert_null([].avg())
    assert_null(ROWS.avg_by("nope"))
    assert_eq([0].avg(), 0)
  end

  test("avg and tally are called without parentheses, like sum") do
    pending("bug: `[2, 3].avg` and `[1, 1].tally` without () return the bound method, on both engines")
    assert_eq([2, 3].avg, 2.5)
    assert_eq([1, 1].tally, {1: 2})
  end

  test("group_by preserves first-seen key order and group order") do
    assert_eq(ROWS.group_by("status").keys, ["paid", "open"])
    assert_eq(ROWS.group_by("status")["paid"].pluck("sku"), ["a", "c"])
  end

  test("index_by is last-write-wins") do
    assert_eq(ROWS.index_by("status")["paid"]["sku"], "c")
    assert_eq(ROWS.index_by("status")["open"]["sku"], "b")
  end

  test("count_by and tally total the input") do
    assert_eq(ROWS.count_by("status"), {"paid": 2, "open": 1})
    assert_eq([1, 2, 2, 3].tally()[2], 2)
  end

  test("filter_by and find_by select by field value") do
    assert_eq(ROWS.filter_by("status", "paid").pluck("sku"), ["a", "c"])
    assert_eq(ROWS.find_by("status", "open")["sku"], "b")
    assert_null(ROWS.find_by("status", "void"))
  end

  test("uniq_by keeps the first of each group") do
    assert_eq(ROWS.uniq_by("status").pluck("sku"), ["a", "b"])
  end

  test("max_by and min_by return the record, not the value") do
    assert_eq(ROWS.max_by("cents")["sku"], "a")
    assert_eq(ROWS.min_by("cents")["sku"], "c")
  end

  test("extremes skip records missing the field instead of ranking nil") do
    with_gap = [{"sku": "ghost"}, {"sku": "a", "cents": 900}]
    assert_eq(with_gap.min_by("cents")["sku"], "a")
    assert_null([{"sku": "ghost"}].max_by("cents"))
  end

  test("a record missing the grouping field lands under nil") do
    mixed = [{"s": "x"}, {"other": 1}]
    assert_eq(mixed.count_by("s")[nil], 1)
    assert_eq(mixed.count_by("s")["x"], 1)
  end

  # Rows from the ORM are instances, not hashes — if the shared field
  # accessor skipped them these would all silently return empty results.
  test("the whole family reads instance fields, as ORM rows are instances") do
    people = [new Person("Ann", "admin", 5), new Person("Bo", "member", 9)]
    assert_eq(people.pluck("name"), ["Ann", "Bo"])
    assert_eq(people.filter_by("role", "admin").map { |person| person.name }, ["Ann"])
    assert_eq(people.find_by("role", "member").name, "Bo")
    assert_eq(people.max_by("score").name, "Bo")
    assert_eq(people.sum_by("score"), 14)
    assert_eq(people.avg_by("score"), 7)
    assert_eq(people.uniq_by("role").length, 2)
  end

  test("a closure where a field name belongs raises instead of returning nothing") do
    assert_raises("`max_by` takes a field name as a string, not a function") do
      ROWS.max_by(fn(row) { row["cents"] })
    end
  end
end

# These run correctly on both engines but were once rejected by `soli check`,
# which declared fewer parameters than the implementations accept.
describe("Optional arguments the type checker used to reject") do
  test("hash get and fetch take a default") do
    config = {"port": 8080}
    assert_eq(config.get("port", 3000), 8080)
    assert_eq(config.get("missing", 3000), 3000)
    assert_eq(config.fetch("missing", 42), 42)
  end

  test("padding helpers take a pad string") do
    assert_eq("abc".center(9, "*"), "***abc***")
    assert_eq("abc".ljust(6, "*"), "abc***")
    assert_eq("abc".rjust(6, "*"), "***abc")
    assert_eq("abc".lpad(6, "0"), "000abc")
  end

  test("padding helpers still default to spaces") do
    assert_eq("abc".center(7), "  abc  ")
    assert_eq("abc".ljust(5), "abc  ")
  end

  test("string search helpers take their needle") do
    assert_eq("banana".count("a"), 3)
    assert_eq("banana".index_of("n"), 2)
    assert_eq("banana".scan("a"), ["a", "a", "a"])
    assert_eq("a,b".partition(","), ["a", ",", "b"])
    assert_eq("a,b".rpartition(","), ["a", ",", "b"])
  end
end
