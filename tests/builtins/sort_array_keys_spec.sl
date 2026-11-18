# Arrays order element by element, then shorter first (Ruby's <=>): a
# composite sort key used to compare as equal and keep the input order.

rows = [{"p": 1, "y": 5}, {"p": 3, "y": 90}, {"p": 3, "y": 10}, {"p": 2, "y": 1}]

describe("Array keys and array elements") do
  test("sort_by with a tuple key sorts by each part in turn") do
    keys = rows.sort_by { |r| [r["p"], r["y"]] }.map { |r| "#{r["p"]}:#{r["y"]}" }
    assert_eq(keys, ["1:5", "2:1", "3:10", "3:90"])
  end

  test("a negated first part sorts that part descending") do
    keys = rows.sort_by { |r| [0 - r["p"], r["y"]] }.map { |r| "#{r["p"]}:#{r["y"]}" }
    assert_eq(keys, ["3:10", "3:90", "2:1", "1:5"])
  end

  test("sort orders arrays, a prefix before its extension") do
    assert_eq([[2, 1], [1, 9], [1, 2, 0], [1, 2]].sort, [[1, 2], [1, 2, 0], [1, 9], [2, 1]])
  end

  test("min and max compare arrays") do
    pairs = [["b", 1], ["a", 2]]
    assert_eq(pairs.min, ["a", 2])
    assert_eq(pairs.max, ["b", 1])
  end
end
