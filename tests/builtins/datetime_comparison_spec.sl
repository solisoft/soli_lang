# DateTime comparison: the operators compare instants, never identity or view.

describe("DateTime comparison") do
  test("ordering operators compare by instant") do
    earlier = DateTime.from_unix(1000)
    later = DateTime.from_unix(2000)
    assert(earlier < later)
    assert(later > earlier)
    assert(earlier <= later)
    assert(later >= earlier)
    assert_not(later < earlier)
    assert_not(earlier > later)
  end

  test("equality compares by instant, not identity") do
    first = DateTime.from_unix(1700000000)
    second = DateTime.from_unix(1700000000)
    assert(first == second)
    assert_not(first != second)
  end

  test("one second apart is unequal") do
    first = DateTime.from_unix(1700000000)
    second = DateTime.from_unix(1700000001)
    assert(first != second)
    assert_not(first == second)
    assert(first < second)
    assert(first <= second)
  end

  test("is reflexive") do
    moment = DateTime.from_unix(1234567890)
    assert(moment == moment)
    assert(moment <= moment)
    assert(moment >= moment)
    assert_not(moment < moment)
    assert_not(moment > moment)
  end

  test("the same instant written with two offsets is equal") do
    assert(DateTime.parse("2024-01-15T10:30:00Z") == DateTime.parse("2024-01-15T11:30:00+01:00"))
  end

  test("the UTC and local views of one instant are equal") do
    moment = DateTime.parse("2024-01-15T10:30:00Z")
    assert(moment.utc == moment.local)
  end

  test("a DateTime is never equal to its timestamp") do
    assert_not(DateTime.from_unix(5) == 5)
  end

  test("ordering against a non-DateTime raises") do
    assert_raises("Cannot compare DateTime and int") do
      DateTime.from_unix(5) < [6][0]
    end
  end

  test("includes? finds an equal instant") do
    moments = [DateTime.from_unix(1), DateTime.from_unix(2)]
    assert(moments.includes?(DateTime.from_unix(2)))
    assert_not(moments.includes?(DateTime.from_unix(3)))
  end
end

describe("sorting DateTimes") do
  test("sort_by a timestamp orders them") do
    moments = [DateTime.from_unix(3), DateTime.from_unix(1), DateTime.from_unix(2)]
    assert_eq(moments.sort_by { |moment| moment.to_unix }.map { |moment| moment.to_unix }, [1, 2, 3])
  end

  test("sort orders them by instant") do
    pending("bug: Array#sort leaves DateTimes in their original order")
    moments = [DateTime.from_unix(3), DateTime.from_unix(1), DateTime.from_unix(2)]
    assert_eq(moments.sort().map { |moment| moment.to_unix }, [1, 2, 3])
  end

  test("min and max pick the earliest and latest") do
    pending("bug: Array#min returns the latest DateTime, not the earliest")
    moments = [DateTime.from_unix(3), DateTime.from_unix(1), DateTime.from_unix(2)]
    assert_eq(moments.min.to_unix, 1)
    assert_eq(moments.max.to_unix, 3)
  end
end
