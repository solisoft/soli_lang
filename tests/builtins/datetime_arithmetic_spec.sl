# DateTime arithmetic (add_days, add_hours, add_minutes, subtract_days) and the
# beginning_of_* / end_of_* boundaries. Exact checks read the UTC view; the
# boundaries follow the value's view, local by default.

moment = nil

describe("DateTime arithmetic") do
  before_each() do
    moment = DateTime.parse("2024-06-15T10:30:45.123Z").utc
  end

  describe("add_days") do
    test("moves forward by whole days") do
      assert_eq(moment.add_days(1).to_iso, "2024-06-16T10:30:45.123+00:00")
      assert_eq(moment.add_days(366).to_iso, "2025-06-16T10:30:45.123+00:00")
    end

    test("moves back with a negative count") do
      assert_eq(moment.add_days(-1).to_iso, "2024-06-14T10:30:45.123+00:00")
    end

    test("returns the same instant for zero") do
      assert(moment.add_days(0) == moment)
    end

    test("crosses a month end") do
      later = DateTime.parse("2024-01-31T12:00:00Z").add_days(1).utc
      assert_eq(later.month, 2)
      assert_eq(later.day, 1)
    end

    test("raises on a non-number") do
      assert_raises("DateTime.add_days() requires number") do
        moment.add_days(["x"][0])
      end
    end
  end

  describe("subtract_days") do
    test("moves back by whole days") do
      assert_eq(moment.subtract_days(5).to_iso, "2024-06-10T10:30:45.123+00:00")
    end

    test("crosses into the previous month, leap day included") do
      earlier = DateTime.parse("2024-03-01T12:00:00Z").subtract_days(1).utc
      assert_eq(earlier.month, 2)
      assert_eq(earlier.day, 29)
    end

    test("moves forward with a negative count") do
      assert_eq(moment.subtract_days(-1).to_iso, "2024-06-16T10:30:45.123+00:00")
    end
  end

  describe("add_hours") do
    test("moves forward by hours") do
      assert_eq(moment.add_hours(5).to_iso, "2024-06-15T15:30:45.123+00:00")
    end

    test("moves back with a negative count") do
      assert_eq(moment.add_hours(-5).to_iso, "2024-06-15T05:30:45.123+00:00")
      assert_eq(moment.add_hours(-24).to_iso, "2024-06-14T10:30:45.123+00:00")
    end

    test("crosses midnight") do
      later = DateTime.parse("2024-01-15T20:00:00Z").add_hours(10).utc
      assert_eq(later.day, 16)
      assert_eq(later.hour, 6)
    end

    test("crosses a month end") do
      later = DateTime.parse("2024-01-31T22:00:00Z").add_hours(5).utc
      assert_eq(later.month, 2)
      assert_eq(later.day, 1)
      assert_eq(later.hour, 3)
    end

    test("lands on a leap day") do
      later = DateTime.parse("2024-02-28T22:00:00Z").add_hours(5).utc
      assert_eq(later.month, 2)
      assert_eq(later.day, 29)
    end

    test("raises on a non-number") do
      assert_raises("DateTime.add_hours() requires number") do
        moment.add_hours(["x"][0])
      end
    end
  end

  describe("add_minutes") do
    test("crosses an hour boundary") do
      later = DateTime.parse("2024-01-15T10:45:00Z").add_minutes(30).utc
      assert_eq(later.hour, 11)
      assert_eq(later.minute, 15)
    end

    test("moves back with a negative count") do
      earlier = DateTime.parse("2024-01-15T10:30:00Z").add_minutes(-30).utc
      assert_eq(earlier.hour, 10)
      assert_eq(earlier.minute, 0)
    end

    test("a day of minutes is a day") do
      assert(moment.add_minutes(1440) == moment.add_days(1))
    end
  end

  test("leaves the receiver unchanged") do
    original = moment.to_unix
    moment.add_days(3)
    moment.add_hours(3)
    moment.subtract_days(3)
    moment.beginning_of_day
    moment.end_of_month
    assert_eq(moment.to_unix, original)
  end

  # Regression: instances returned by add_days/add_hours carried a partial
  # method snapshot, so `dt.add_days(3).format(...)` raised
  # "Cannot access property 'format'".
  test("chained results keep every method") do
    start = DateTime.parse("2024-01-15T10:00:00Z").utc
    assert_eq(start.add_days(3).format("%Y-%m-%d"), "2024-01-18")
    assert_eq(start.add_days(1).add_days(1).add_hours(2).day, 17)
    assert_eq(start.subtract_days(1).format("%Y-%m-%d"), "2024-01-14")
    assert_eq(start.add_days(2).beginning_of_day.end_of_month.month, 1)
  end
end

describe("DateTime boundaries") do
  before_each() do
    moment = DateTime.parse("2024-06-15T10:30:45.123Z").utc
  end

  test("beginning_of_minute and end_of_minute") do
    assert_eq(moment.beginning_of_minute.to_iso, "2024-06-15T10:30:00+00:00")
    assert_eq(moment.end_of_minute.to_iso, "2024-06-15T10:30:59.999+00:00")
  end

  test("beginning_of_hour and end_of_hour") do
    assert_eq(moment.beginning_of_hour.to_iso, "2024-06-15T10:00:00+00:00")
    assert_eq(moment.end_of_hour.to_iso, "2024-06-15T10:59:59.999+00:00")
  end

  test("beginning_of_day and end_of_day") do
    assert_eq(moment.beginning_of_day.to_iso, "2024-06-15T00:00:00+00:00")
    assert_eq(moment.end_of_day.to_iso, "2024-06-15T23:59:59.999+00:00")
    assert_eq(moment.end_of_day.millisecond, 999)
  end

  test("beginning_of_month and end_of_month") do
    assert_eq(moment.beginning_of_month.to_iso, "2024-06-01T00:00:00+00:00")
    assert_eq(moment.end_of_month.to_iso, "2024-06-30T23:59:59.999+00:00")
  end

  test("beginning_of_year and end_of_year") do
    assert_eq(moment.beginning_of_year.to_iso, "2024-01-01T00:00:00+00:00")
    assert_eq(moment.end_of_year.to_iso, "2024-12-31T23:59:59.999+00:00")
  end

  test("every boundary brackets the instant") do
    pairs = [
      [moment.beginning_of_minute, moment.end_of_minute],
      [moment.beginning_of_hour, moment.end_of_hour],
      [moment.beginning_of_day, moment.end_of_day],
      [moment.beginning_of_month, moment.end_of_month],
      [moment.beginning_of_year, moment.end_of_year]
    ]
    pairs.each do |pair|
      assert(pair[0] <= moment)
      assert(moment <= pair[1])
    end
  end

  test("end_of_month knows December and February") do
    assert_eq(DateTime.parse("2024-12-15T10:30:45Z").utc.end_of_month.day, 31)
    assert_eq(DateTime.parse("2024-02-15T10:30:45Z").utc.end_of_month.day, 29)
    assert_eq(DateTime.parse("2023-02-15T10:30:45Z").utc.end_of_month.day, 28)
    assert_eq(DateTime.parse("2023-02-15T10:30:45Z").utc.end_of_month.month, 2)
  end

  test("an instant already on the boundary stays put") do
    midnight = DateTime.parse("2024-06-15T00:00:00Z").utc
    assert(midnight.beginning_of_day == midnight)
    assert(midnight.beginning_of_month.beginning_of_month == midnight.beginning_of_month)
  end

  test("follow the local wall clock by default") do
    local = DateTime.parse("2024-06-15T12:00:00Z")
    start = local.beginning_of_day
    finish = local.end_of_day
    assert_eq(start.hour, 0)
    assert_eq(start.minute, 0)
    assert_eq(start.day, local.day)
    assert_eq(finish.hour, 23)
    assert_eq(finish.second, 59)
    assert_eq(finish.day, local.day)
    assert_eq(local.beginning_of_month.day, 1)
  end
end
