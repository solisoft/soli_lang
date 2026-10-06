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

  test("to_iso is RFC 3339 in UTC, whatever the view") do
    assert_eq(moment.to_iso, "2024-01-15T10:30:00+00:00")
    assert_eq(moment.utc.to_iso, "2024-01-15T10:30:00+00:00")
    assert_eq(moment.local.to_iso, "2024-01-15T10:30:00+00:00")
  end

  test("to_iso keeps milliseconds") do
    assert_eq(DateTime.parse("2024-01-15T10:30:00.123Z").to_iso, "2024-01-15T10:30:00.123+00:00")
  end

  test("to_iso normalises an offset to UTC") do
    assert_eq(DateTime.parse("2024-01-15T12:30:00+02:00").to_iso, "2024-01-15T10:30:00+00:00")
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

  test("JSON renders it as to_iso") do
    assert_eq({"at": moment}.to_json, "{\"at\":\"2024-01-15T10:30:00+00:00\"}")
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

  test("uses the local view by default") do
    local = DateTime.parse("2024-01-15T10:30:05Z")
    assert_eq(local.format("%H"), local.local.format("%H"))
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
