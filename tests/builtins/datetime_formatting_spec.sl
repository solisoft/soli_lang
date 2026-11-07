# Turning a DateTime into text: to_unix, to_iso, to_string, format, string
# interpolation and JSON. Localized month and day names are in
# datetime_locale_spec.sl.

moment = nil

describe("DateTime serialisation") do
  before_each() do
    moment = DateTime.parse("2024-01-15T10:30:00Z")
  end

  test("to_unix is the epoch second") do
    assert_eq(moment.to_unix, 1705314600)
  end

  # #673: to_iso printed UTC while format/hour read the server's zone.
  test("to_iso is RFC 3339 in the value's own zone, like format") do
    assert_eq(moment.to_iso, "2024-01-15T10:30:00+00:00")
    assert_eq(moment.utc.to_iso, "2024-01-15T10:30:00+00:00")
    local = moment.local
    assert_eq(local.to_iso.substring(0, 19), local.format("%Y-%m-%dT%H:%M:%S"))
    assert_eq(local.to_iso.substring(19, 25), local.format("%:z"))
  end

  test("to_iso keeps milliseconds") do
    assert_eq(DateTime.parse("2024-01-15T10:30:00.123Z").to_iso, "2024-01-15T10:30:00.123+00:00")
  end

  test("to_iso keeps a parsed offset; utc normalises it") do
    stamped = DateTime.parse("2024-01-15T12:30:00+02:00")
    assert_eq(stamped.to_iso, "2024-01-15T12:30:00+02:00")
    assert_eq(stamped.hour, 12)
    assert_eq(stamped.format("%H:%M %z"), "12:30 +0200")
    assert_eq(stamped.utc.to_iso, "2024-01-15T10:30:00+00:00")
    assert_eq(DateTime.parse("2024-01-15T05:30:00-05:00").to_iso, "2024-01-15T05:30:00-05:00")
  end

  test("text without an offset, and from_unix, read in UTC") do
    assert_eq(DateTime.parse("2026-03-02").format("%Y-%m-%d %H:%M"), "2026-03-02 00:00")
    assert_eq(DateTime.parse("2026-03-02 23:30:00").day, 2)
    epoch = DateTime.from_unix(0)
    assert_eq(epoch.to_iso + " " + epoch.format("%H:%M"), "1970-01-01T00:00:00+00:00 00:00")
  end

  test("to_string is the wall clock of the view") do
    assert_eq(moment.utc.to_string, "2024-01-15 10:30:00")
    assert_eq(moment.to_string, moment.format("%Y-%m-%d %H:%M:%S"))
  end

  test("str and interpolation render like to_string") do
    utc = moment.utc
    assert_eq(str(utc), "2024-01-15 10:30:00")
    assert_eq("at #{utc}", "at 2024-01-15 10:30:00")
  end

  test("the string form matches format() before 1970, on leap days and in both views") do
    [-1, -86401, 0, 951782399, 951782400, 4107542399, -2203891200].each do |seconds|
      instant = DateTime.from_unix(seconds)
      [instant.utc, instant.local].each do |view|
        expected = view.format("%Y-%m-%d %H:%M:%S")
        assert_eq(view.to_s, expected)
        assert_eq(str(view), expected)
        assert_eq("[#{view}]", "[#{expected}]")
      end
    end
    assert_eq(DateTime.from_unix(-1).utc.to_s, "1969-12-31 23:59:59")
    assert_eq(DateTime.from_unix(951782400).utc.to_s, "2000-02-29 00:00:00")
    assert_eq(DateTime.from_unix(4107542399).utc.to_s, "2100-02-28 23:59:59")
  end

  test("JSON renders the UTC instant, whatever the zone") do
    assert_eq({"at": moment}.to_json, "{\"at\":\"2024-01-15T10:30:00+00:00\"}")
    stamped = DateTime.parse("2024-01-15T12:30:00+02:00")
    assert_eq({"at": stamped}.to_json, "{\"at\":\"2024-01-15T10:30:00+00:00\"}")
  end
end

describe("DateTime#format") do
  before_each() do
    moment = DateTime.parse("2024-01-15T10:30:05Z").utc
  end

  test("numeric date and time specifiers") do
    assert_eq(moment.format("%Y-%m-%d %H:%M:%S"), "2024-01-15 10:30:05")
    assert_eq(moment.format("%d/%m/%Y"), "15/01/2024")
  end

  test("month and day names default to English") do
    assert_eq(moment.format("%B %d, %Y"), "January 15, 2024")
    assert_eq(moment.format("%A %a %b"), "Monday Mon Jan")
  end

  test("day of year and a literal percent") do
    assert_eq(moment.format("%j %%"), "015 %")
  end

  test("text without specifiers passes through") do
    assert_eq(moment.format("plain"), "plain")
    assert_eq(moment.format(""), "")
  end

  test("uses the value's own zone") do
    assert_eq(DateTime.parse("2024-01-15T10:30:05Z").format("%H"), "10")
    assert_eq(DateTime.parse("2024-01-15T10:30:05+03:00").format("%H"), "10")
  end

  test("raises without a pattern") do
    assert_raises("DateTime.format() expects 1-2 arguments, got 0") do
      moment.format()
    end
  end

  test("raises on a non-string pattern") do
    assert_raises("DateTime.format() requires format string") do
      moment.format([42][0])
    end
  end
end
