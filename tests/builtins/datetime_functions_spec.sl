# What freeze_time / travel_to / unfreeze_time return and which strings they
# read (basics in freeze_time_spec.sl), the `.utc` view through formatting and
# arithmetic, and the `datetime_*` view helpers, which exist only in templates.

# 2024-01-15T10:30:00Z
const FROZEN_AT = 1705314600

VIEW_HELPERS = ["datetime_format", "datetime_parse", "datetime_add_days", "datetime_add_hours", "datetime_diff"]

# 23:30 UTC: a local zone east of UTC already reads the next day.
late_evening = nil

describe("view-only datetime helpers") do
  test("are not defined in plain code") do
    VIEW_HELPERS.each do |name|
      assert_not(defined(name), "#{name} should be a template helper only")
    end
  end
end

describe("freeze_time") do
  after_each() do
    unfreeze_time()
  end

  test("returns the pinned timestamp") do
    assert_eq(freeze_time(FROZEN_AT), FROZEN_AT)
    assert_eq(datetime_now(), FROZEN_AT)
  end

  test("returns the timestamp it parsed from a string") do
    assert_eq(freeze_time("2024-01-15T10:30:00Z"), FROZEN_AT)
  end

  test("reads a bare date as midnight UTC") do
    assert_eq(freeze_time("2024-01-15"), 1705276800)
    assert_eq(datetime_now(), 1705276800)
  end

  test("accepts a space-separated date and time") do
    assert_eq(freeze_time("2024-01-15 10:30:00"), FROZEN_AT)
  end

  test("refuses a float") do
    assert_raises("freeze_time() expects timestamp (int) or date string, got float") do
      freeze_time(1.5)
    end
  end
end

describe("travel_to") do
  after_each() do
    unfreeze_time()
  end

  test("returns the pinned timestamp") do
    assert_eq(travel_to(FROZEN_AT), FROZEN_AT)
  end

  test("parses a date string") do
    assert_eq(travel_to("2024-06-15"), 1718409600)
    assert_eq(datetime_now(), 1718409600)
  end

  test("refuses nil and unparseable strings") do
    assert_raises("travel_to() expects timestamp (int) or date string, got null") do
      travel_to(nil)
    end
    assert_raises("travel_to(): invalid date string \"nope\"") do
      travel_to("nope")
    end
  end
end

describe("unfreeze_time") do
  test("returns nil") do
    freeze_time(FROZEN_AT)
    assert_null(unfreeze_time())
    assert_gt(datetime_now(), FROZEN_AT)
  end
end

describe("DateTime .utc view") do
  before_each() do
    late_evening = DateTime.parse("2024-01-15T23:30:00Z")
  end

  test("reads the UTC weekday") do
    assert_eq(late_evening.utc.weekday, "Monday")
  end

  test("to_iso is always UTC, whatever the view") do
    assert_eq(late_evening.utc.to_iso, "2024-01-15T23:30:00+00:00")
    assert_eq(late_evening.local.to_iso, "2024-01-15T23:30:00+00:00")
  end

  test("renders and formats in UTC") do
    assert_eq(late_evening.utc.to_s, "2024-01-15 23:30:00")
    assert_eq("#{late_evening.utc}", "2024-01-15 23:30:00")
    assert_eq(late_evening.utc.format("%Y-%m-%d %H:%M"), "2024-01-15 23:30")
  end

  test("survives arithmetic") do
    assert_eq(late_evening.utc.add_hours(1).hour, 0)
    assert_eq(late_evening.utc.add_hours(1).day, 16)
    assert_eq(late_evening.utc.beginning_of_day.to_iso, "2024-01-15T00:00:00+00:00")
  end

  test("DateTime.local is an instance method, not a static constructor") do
    assert_raises("Cannot access property 'local' on DateTime") do
      DateTime.local
    end
  end
end
