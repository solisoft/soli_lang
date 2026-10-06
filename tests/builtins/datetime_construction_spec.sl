# Building a DateTime (now, utc, parse, epoch, from_unix) and reading its
# components. Components follow the value's view — local by default, UTC after
# `.utc` — so exact component checks read the UTC view to hold in every $TZ.

def pad2(number)
  number < 10 ? "0#{number}" : "#{number}"
end

describe("DateTime.now and DateTime.utc") do
  test("now returns a DateTime at the current instant") do
    moment = DateTime.now
    assert_eq(type(moment), "DateTime")
    assert_lt((moment.to_unix - datetime_now()).abs, 2)
  end

  test("utc is the same instant as now") do
    assert_lt((DateTime.utc.to_unix - DateTime.now.to_unix).abs, 2)
  end

  # `now()` used to be `timestamp() * 1_000_000_000`: every instant it made
  # carried a zero subsecond, so `millisecond()` answered 0 for all of them and
  # a duration measured between two of them was always whole seconds.
  test("now carries the subsecond that millisecond reads") do
    before = DateTime.now
    sleep(0.003)
    after = DateTime.now
    assert_ne(after.millisecond, before.millisecond)
    elapsed = Duration.between(before, after).total_seconds
    assert_gt(elapsed, 0.0029)
    assert_lt(elapsed, 0.5)
  end
end

describe("DateTime.parse") do
  test("parses an RFC 3339 instant in UTC") do
    assert_eq(DateTime.parse("2024-01-15T10:30:00Z").to_unix, 1705314600)
  end

  test("applies a timezone offset") do
    moment = DateTime.parse("2024-01-15T10:00:00+05:30")
    assert_eq(moment.to_unix, 1705293000)
    assert_eq(moment.to_iso, "2024-01-15T04:30:00+00:00")
  end

  test("keeps fractional seconds") do
    assert_eq(DateTime.parse("2024-01-15T10:30:45.123Z").millisecond, 123)
    assert_eq(DateTime.parse("2024-01-15T10:30:00.5Z").millisecond, 500)
  end

  test("parses RFC 2822") do
    assert_eq(DateTime.parse("Mon, 15 Jan 2024 10:30:00 +0000").to_unix, 1705314600)
  end

  test("reads a datetime without an offset as UTC, with a T or a space") do
    assert_eq(DateTime.parse("2024-01-15T10:30:00").to_unix, 1705314600)
    assert_eq(DateTime.parse("2024-01-15 10:30:00").to_unix, 1705314600)
  end

  test("reads a date alone as UTC midnight") do
    moment = DateTime.parse("2024-01-15")
    assert_eq(moment.to_unix, 1705276800)
    assert_eq(moment.utc.year, 2024)
    assert_eq(moment.utc.month, 1)
    assert_eq(moment.utc.day, 15)
    assert_eq(moment.utc.hour, 0)
  end

  test("raises on text that is not a date") do
    assert_raises("Invalid datetime format: nope") do
      DateTime.parse("nope")
    end
  end

  test("raises on an empty string") do
    assert_raises("Invalid datetime format") do
      DateTime.parse("")
    end
  end

  test("raises on a calendar date that does not exist") do
    assert_raises("Invalid datetime format: 2024-02-30") do
      DateTime.parse("2024-02-30")
    end
    assert_raises("Invalid datetime format") do
      DateTime.parse("2024-13-01T00:00:00Z")
    end
  end

  test("raises on a non-string") do
    assert_raises("DateTime.parse() requires string") do
      DateTime.parse([42][0])
    end
  end
end

describe("DateTime.epoch and DateTime.from_unix") do
  test("epoch is the Unix epoch") do
    epoch = DateTime.epoch
    assert_eq(epoch.to_unix, 0)
    assert_eq(epoch.to_iso, "1970-01-01T00:00:00+00:00")
    assert_eq(epoch.utc.weekday, "Thursday")
  end

  test("from_unix(0) is the epoch") do
    assert(DateTime.from_unix(0) == DateTime.epoch)
  end

  test("from_unix places a timestamp on the calendar") do
    moment = DateTime.from_unix(1704067200).utc
    assert_eq(moment.year, 2024)
    assert_eq(moment.month, 1)
    assert_eq(moment.day, 1)
    assert_eq(moment.hour, 0)
    assert_eq(moment.minute, 0)
    assert_eq(moment.second, 0)
  end

  test("round-trips through to_unix") do
    assert_eq(DateTime.from_unix(1704067200).to_unix, 1704067200)
    assert_eq(DateTime.from_unix(-100000000).to_unix, -100000000)
  end

  test("accepts timestamps before 1970 and far ahead") do
    assert_eq(DateTime.from_unix(-86400).to_iso, "1969-12-31T00:00:00+00:00")
    assert_eq(DateTime.from_unix(4102444800).to_iso, "2100-01-01T00:00:00+00:00")
  end

  test("drops the fraction of a Float timestamp") do
    assert_eq(DateTime.from_unix(1.5).to_unix, 1)
  end

  test("raises on a non-number") do
    assert_raises("DateTime.from_unix() requires number") do
      DateTime.from_unix(["1"][0])
    end
  end
end

describe("component accessors") do
  test("read every component in the UTC view") do
    moment = DateTime.parse("2024-07-20T14:35:45.678Z").utc
    assert_eq(moment.year, 2024)
    assert_eq(moment.month, 7)
    assert_eq(moment.day, 20)
    assert_eq(moment.hour, 14)
    assert_eq(moment.minute, 35)
    assert_eq(moment.second, 45)
    assert_eq(moment.millisecond, 678)
    assert_eq(moment.weekday, "Saturday")
  end

  test("weekday names each day of a known week") do
    names = (1..8).map { |day| DateTime.parse("2024-01-0#{day}T12:00:00Z").utc.weekday }
    assert_eq(names, ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"])
  end

  test("millisecond is zero on a whole-second instant") do
    assert_eq(DateTime.from_unix(1704067200).millisecond, 0)
  end

  # The bug this guards: hour/minute were read in UTC while day was local.
  test("the default view agrees with format on every component") do
    moment = DateTime.parse("2026-01-01T23:30:00Z")
    assert_eq(moment.format("%Y"), "#{moment.year}")
    assert_eq(moment.format("%m"), pad2(moment.month))
    assert_eq(moment.format("%d"), pad2(moment.day))
    assert_eq(moment.format("%H"), pad2(moment.hour))
    assert_eq(moment.format("%M"), pad2(moment.minute))
    assert_eq(moment.format("%S"), pad2(moment.second))
  end

  test("the default view is the local view") do
    moment = DateTime.parse("2026-01-01T23:30:00Z")
    assert_eq(moment.hour, moment.local.hour)
    assert_eq(moment.day, moment.local.day)
  end

  test("utc and local change the view, not the instant") do
    moment = DateTime.parse("2026-01-01T23:30:00Z")
    assert_eq(moment.utc.hour, 23)
    assert_eq(moment.utc.minute, 30)
    assert_eq(moment.utc.day, 1)
    assert_eq(moment.utc.to_unix, moment.to_unix)
    assert(moment == moment.utc)
    assert(moment.utc.local == moment)
  end
end

# Splitting the epoch nanoseconds by hand — `t / 1e9` and `(t % 1e9) as u32` —
# wrapped for negative instants: the remainder is negative before 1970, and
# casting it to u32 gave ~4.29e9 nanoseconds, so every accessor raised
# "Invalid timestamp". Only sub-second instants showed it: a whole-second
# instant has a zero remainder.
describe("DateTime before 1970") do
  test("reads a pre-1970 instant carrying milliseconds") do
    moment = DateTime.parse("1969-07-20T20:17:00.500Z").utc
    assert_eq(moment.year, 1969)
    assert_eq(moment.month, 7)
    assert_eq(moment.day, 20)
    assert_eq(moment.millisecond, 500)
    assert_eq(moment.to_unix, -14182979)
  end

  test("reads a whole-second pre-1970 instant") do
    moment = DateTime.parse("1969-07-20T20:17:00Z").utc
    assert_eq(moment.year, 1969)
    assert_eq(moment.hour, 20)
  end

  test("reads a long-past instant with milliseconds") do
    moment = DateTime.parse("1900-06-15T12:30:45.123Z").utc
    assert_eq(moment.year, 1900)
    assert_eq(moment.month, 6)
    assert_eq(moment.second, 45)
    assert_eq(moment.millisecond, 123)
  end

  test("places a negative timestamp on the calendar") do
    assert_eq(DateTime.from_unix(-100000000).utc.year, 1966)
    assert_eq(DateTime.from_unix(-100000000).to_iso, "1966-10-31T14:13:20+00:00")
  end

  test("formats a pre-1970 instant with milliseconds") do
    moment = DateTime.parse("1969-07-20T20:17:00.500Z").utc
    assert_eq(moment.format("%Y-%m-%d %H:%M:%S"), "1969-07-20 20:17:00")
  end
end
