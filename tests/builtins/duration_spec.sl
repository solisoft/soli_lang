# Duration: the factories, unit conversions, Duration.between, to_string and
# humanize. A Duration stores seconds; every total_* is a Float.

describe("Duration factories") do
  test("seconds, minutes, hours, days and weeks store seconds") do
    assert_eq(Duration.seconds(120).total_seconds, 120)
    assert_eq(Duration.minutes(5).total_seconds, 300)
    assert_eq(Duration.hours(2).total_seconds, 7200)
    assert_eq(Duration.days(1).total_seconds, 86400)
    assert_eq(Duration.weeks(1).total_seconds, 604800)
  end

  test("the of_* forms are aliases") do
    assert_eq(Duration.of_seconds(120).total_seconds, 120)
    assert_eq(Duration.of_minutes(5).total_seconds, 300)
    assert_eq(Duration.of_hours(2).total_seconds, 7200)
    assert_eq(Duration.of_days(1).total_seconds, 86400)
    assert_eq(Duration.of_weeks(3).total_days, 21)
  end

  test("make a Duration") do
    assert_eq(type(Duration.seconds(120)), "Duration")
    assert_eq(type(Duration.of_weeks(1)), "Duration")
  end

  test("accept a fraction") do
    assert_eq(Duration.of_seconds(1.5).total_seconds, 1.5)
    assert_eq(Duration.of_minutes(1.5).total_seconds, 90)
  end

  test("accept a negative length") do
    assert_eq(Duration.seconds(-60).total_minutes, -1)
  end

  test("total_seconds is a Float") do
    assert_eq(type(Duration.minutes(5).total_seconds), "float")
  end

  test("raise on a non-number") do
    assert_raises("Duration.of_seconds() requires number") do
      Duration.seconds(["x"][0])
    end
  end
end

describe("Duration conversions") do
  test("total_minutes") do
    assert_eq(Duration.of_seconds(120).total_minutes, 2)
    assert_eq(Duration.of_hours(2).total_minutes, 120)
    assert_eq(Duration.seconds(90).total_minutes, 1.5)
  end

  test("total_hours") do
    assert_eq(Duration.of_hours(2).total_hours, 2)
    assert_eq(Duration.of_days(1).total_hours, 24)
    assert_eq(Duration.seconds(90).total_hours, 0.025)
  end

  test("total_days") do
    assert_eq(Duration.of_days(1).total_days, 1)
    assert_eq(Duration.weeks(2).total_days, 14)
  end

  test("a zero duration is zero in every unit") do
    zero = Duration.seconds(0)
    assert_eq(zero.total_seconds, 0)
    assert_eq(zero.total_minutes, 0)
    assert_eq(zero.total_hours, 0)
    assert_eq(zero.total_days, 0)
  end
end

describe("Duration.between") do
  test("measures from the first DateTime to the second") do
    start = DateTime.parse("2024-01-15T10:00:00Z")
    finish = DateTime.parse("2024-01-15T11:30:00Z")
    duration = Duration.between(start, finish)
    assert_eq(type(duration), "Duration")
    assert_eq(duration.total_minutes, 90)
  end

  # Regression: `_ts` is nanoseconds — between used to store the raw nano
  # difference as seconds, so 1 hour came back as ~1e9 hours.
  test("returns the magnitude in seconds") do
    start = DateTime.parse("2024-01-15T10:00:00Z")
    finish = DateTime.parse("2024-01-15T11:00:00Z")
    duration = Duration.between(start, finish)
    assert_eq(duration.total_seconds, 3600)
    assert_eq(duration.total_hours, 1)
  end

  test("is negative when the second DateTime is earlier") do
    start = DateTime.parse("2024-01-15T10:00:00Z")
    finish = DateTime.parse("2024-01-15T11:30:00Z")
    assert_eq(Duration.between(finish, start).total_minutes, -90)
  end

  test("keeps sub-second precision") do
    start = DateTime.parse("2024-01-15T10:00:00.250Z")
    finish = DateTime.parse("2024-01-15T10:00:01Z")
    assert_eq(Duration.between(start, finish).total_seconds, 0.75)
  end

  test("is zero between a DateTime and itself") do
    moment = DateTime.parse("2024-01-15T10:00:00Z")
    assert_eq(Duration.between(moment, moment).total_seconds, 0)
  end

  test("raises unless both arguments are DateTimes") do
    assert_raises("Duration.between() requires DateTime") do
      Duration.between(DateTime.epoch, [5][0])
    end
  end
end

describe("Duration#to_string") do
  test("is the length in seconds") do
    assert_eq(Duration.of_seconds(3661).to_string, "3661s")
    assert_eq(Duration.of_hours(2).to_string, "7200s")
    assert_eq(Duration.of_days(365).to_string, "31536000s")
  end

  test("zero, negative and fractional lengths") do
    assert_eq(Duration.of_seconds(0).to_string, "0s")
    assert_eq(Duration.seconds(-90).to_string, "-90s")
    assert_eq(Duration.seconds(1.5).to_string, "1.5s")
  end

  test("interpolation shows the inspect form") do
    assert_eq("#{Duration.seconds(5)}", "<Duration seconds: 5>")
  end
end

# humanize describes magnitude only — never appends " ago". Relative past
# phrasing is `time_ago(...)`, not Duration.humanize.
describe("Duration#humanize") do
  test("combines the two largest units") do
    assert_eq(Duration.seconds(3661).humanize("en"), "1 hour 1 minute")
    assert_eq(Duration.seconds(90).humanize("en"), "1 minute 30 seconds")
    assert_eq(Duration.seconds(90061).humanize("en"), "1 day 1 hour")
  end

  test("names a single unit when the rest is zero") do
    assert_eq(Duration.minutes(5).humanize("en"), "5 minutes")
    assert_eq(Duration.seconds(7200).humanize("en"), "2 hours")
  end

  test("singular and zero") do
    assert_eq(Duration.seconds(1).humanize("en"), "1 second")
    assert_eq(Duration.seconds(0).humanize("en"), "0 seconds")
  end

  test("counts weeks and years in days") do
    assert_eq(Duration.weeks(3).humanize("en"), "21 days")
    assert_eq(Duration.days(400).humanize("en"), "400 days")
  end

  test("uses the absolute magnitude of a negative interval") do
    earlier = DateTime.parse("2024-01-15T10:00:00Z")
    later = DateTime.parse("2024-01-15T11:00:00Z")
    past = Duration.between(later, earlier)
    assert_eq(past.total_seconds, -3600)
    assert_eq(past.humanize("en"), "1 hour")
  end

  test("defaults to the current locale") do
    assert_eq(Duration.seconds(3661).humanize(), "1 hour 1 minute")
  end

  test("falls back to English for a locale without duration translations") do
    assert_eq(Duration.seconds(59).humanize("fr"), "59 seconds")
    assert_eq(Duration.days(2).humanize("xx"), "2 days")
  end

  test("raises on a non-string locale") do
    assert_raises("Duration.humanize() locale must be a string") do
      Duration.seconds(1).humanize([5][0])
    end
  end
end

describe("Duration equality") do
  test("two Durations of the same length are equal") do
    pending("bug: == compares Durations by identity, so Duration.seconds(120) != Duration.seconds(120)")
    assert(Duration.seconds(120) == Duration.of_seconds(120))
  end
end
