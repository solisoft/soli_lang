# Array and hash comprehensions: `[expr for x in xs if cond]` and
# `{key: value for x in xs if cond}`.

describe("Array comprehensions") do
  test("map every element") do
    assert_eq([x * x for x in range(1, 5)], [1, 4, 9, 16])
  end

  test("iterate over an array literal or a variable") do
    numbers = [10, 20, 30]
    assert_eq([x * 2 for x in numbers], [20, 40, 60])
  end

  test("iterate over a range") do
    assert_eq([x + 1 for x in 1..4], [2, 3, 4])
  end

  test("filter with if") do
    assert_eq([x for x in range(1, 10) if x % 2 == 0], [2, 4, 6, 8])
  end

  test("the condition can read the enclosing scope") do
    limit = 2
    assert_eq([x for x in 1..6 if x > limit], [3, 4, 5])
  end

  test("call methods on each element") do
    words = ["hello", "world"]
    assert_eq([word.upcase for word in words], ["HELLO", "WORLD"])
  end

  test("can build an array per element") do
    assert_eq([[x, x * 2] for x in 1..4], [[1, 2], [2, 4], [3, 6]])
  end

  test("iterate over nested arrays element by element") do
    groups = [[1, 2], [3]]
    assert_eq([group.length for group in groups], [2, 1])
  end

  test("read fields of hash elements") do
    rows = [{"v": 1}, {"v": 2}, {"v": 3}]
    assert_eq([row["v"] for row in rows], [1, 2, 3])
  end

  test("over an empty array, or with nothing passing the filter, give []") do
    assert_eq([x for x in []], [])
    assert_eq([x for x in [1, 2, 3] if x > 5], [])
  end

  test("the loop variable does not leak into the enclosing scope") do
    x = 100
    incremented = [x + 1 for x in [1, 2]]
    assert_eq(incremented, [2, 3])
    assert_eq(x, 100)
  end

  test("refuse a value that is not an array") do
    assert_raises("expected array") do
      [key for key in {"a": 1}]
    end
    assert_raises("expected array") do
      [n for n in 5]
    end
  end
end

describe("Hash comprehensions") do
  test("build a hash from an array of hashes") do
    users = [{"name": "Alice", "age": 30}, {"name": "Bob", "age": 25}]
    ages = {user["name"]: user["age"] for user in users}
    assert_eq(ages, {"Alice": 30, "Bob": 25})
  end

  test("keys can be any value, not only strings") do
    items = [{"id": 1, "value": "a"}, {"id": 2, "value": "b"}]
    mapped = {item["id"]: item["value"] for item in items}
    assert_eq(mapped[1], "a")
    assert_eq(mapped[2], "b")
    assert_eq(mapped.length, 2)
  end

  test("filter with if") do
    people = [
      {"name": "Alice", "active": true},
      {"name": "Bob", "active": false},
      {"name": "Carol", "active": true}
    ]
    active = {person["name"]: person["active"] for person in people if person["active"]}
    assert_eq(active, {"Alice": true, "Carol": true})
  end

  test("compute keys from expressions") do
    items = [{"prefix": "key", "value": 1}, {"prefix": "key", "value": 2}]
    mapped = {item["prefix"] + "_" + str(item["value"]): item["value"] for item in items}
    assert_eq(mapped, {"key_1": 1, "key_2": 2})
  end

  test("compute values from expressions") do
    rows = [{"n": 1}, {"n": 2}, {"n": 3}]
    squares = {row["n"]: row["n"] * row["n"] for row in rows}
    assert_eq(squares[1], 1)
    assert_eq(squares[2], 4)
    assert_eq(squares[3], 9)
  end

  test("a bare name key is the string of that name, as in a hash literal") do
    lengths = {word: word.length for word in ["ab", "c"]}
    assert_eq(lengths, {"word": 1})
    by_word = {(word): word.length for word in ["ab", "c"]}
    assert_eq(by_word, {"ab": 2, "c": 1})
  end

  test("a later element overwrites an earlier one with the same key") do
    by_parity = {n % 2: n for n in 1..5}
    assert_eq(by_parity[1], 3)
    assert_eq(by_parity[0], 4)
  end
end
