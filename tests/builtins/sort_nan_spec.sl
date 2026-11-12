# A NaN in a sort (#627): numbers keep their order and every NaN goes last.
# With NaN equal to everything, the order was not transitive and a sort met
# by one returned its input in no particular order.

describe("sorting with NaN") do
  test("sort puts NaN after every number") do
    nan = "nan".to_f
    sorted = [nan, 3.0, 1, nan, 2.5, 0.5].sort
    assert_eq(sorted.take(4), [0.5, 1, 2.5, 3.0])
    assert(sorted[4].nan?)
    assert(sorted[5].nan?)
  end

  test("sort_by on a field with NaN") do
    nan = "nan".to_f
    rows = [{"s": 2.0}, {"s": nan}, {"s": 1.0}, {"s": nil}]
    scores = rows.sort_by { |row| row["s"] }.map { |row| row["s"] }
    assert_eq(scores[0], nil)
    assert_eq(scores[1], 1.0)
    assert_eq(scores[2], 2.0)
    assert(scores[3].nan?)
  end
end
