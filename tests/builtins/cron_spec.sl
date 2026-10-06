# Cron: the six-field expression builders (sec min hour dom month dow — the
# scheduler's `cron` crate rejects the five-field form) and the schedules kept
# in `_cron_jobs`, which persist between tests and are deleted after each one.

const SCHEDULE_NAME = "spec_nightly"

def find_schedule(name)
  Cron.list.find { |schedule| schedule["name"] == name }
end

describe("Cron expression builders") do
  describe("every") do
    test("builds minute steps") do
      assert_eq(Cron.every("1 minute"), "0 * * * * *")
      assert_eq(Cron.every("5 minutes"), "0 */5 * * * *")
      assert_eq(Cron.every("15 minutes"), "0 */15 * * * *")
    end

    test("builds hour steps") do
      assert_eq(Cron.every("1 hour"), "0 0 * * * *")
      assert_eq(Cron.every("2 hours"), "0 0 */2 * * *")
    end

    test("builds day steps") do
      assert_eq(Cron.every("1 day"), "0 0 0 */1 * *")
      assert_eq(Cron.every("3 days"), "0 0 0 */3 * *")
    end

    test("refuses less than a minute") do
      assert_raises("Cron.every() minimum granularity is 1 minute") do
        Cron.every("30 seconds")
      end
    end

    test("refuses minutes that are not a whole number of hours past 59") do
      assert_raises("Cron.every(\"90 minutes\") cannot be expressed") do
        Cron.every("90 minutes")
      end
    end

    test("refuses an unknown unit or a missing number") do
      assert_raises("unknown duration unit: fortnights") do
        Cron.every("5 fortnights")
      end
      assert_raises("invalid duration number: abc") do
        Cron.every("abc")
      end
    end
  end

  describe("daily_at") do
    test("parses HH:MM") do
      assert_eq(Cron.daily_at("00:00"), "0 0 0 * * *")
      assert_eq(Cron.daily_at("03:00"), "0 0 3 * * *")
      assert_eq(Cron.daily_at("23:45"), "0 45 23 * * *")
    end

    test("refuses a time out of range") do
      assert_raises("HH:MM out of range: 25:00") do
        Cron.daily_at("25:00")
      end
      assert_raises("HH:MM out of range: 12:60") do
        Cron.daily_at("12:60")
      end
    end

    test("refuses a time that is not HH:MM") do
      assert_raises("expected HH:MM, got 3pm") do
        Cron.daily_at("3pm")
      end
    end
  end

  test("hourly fires at the top of each hour") do
    assert_eq(Cron.hourly, "0 0 * * * *")
  end

  describe("weekly_at") do
    test("maps weekday names to cron day-of-week names") do
      assert_eq(Cron.weekly_at("monday", "09:00"), "0 0 9 * * Mon")
      assert_eq(Cron.weekly_at("sunday", "00:00"), "0 0 0 * * Sun")
      assert_eq(Cron.weekly_at("Saturday", "06:05"), "0 5 6 * * Sat")
    end

    test("accepts three-letter abbreviations") do
      assert_eq(Cron.weekly_at("fri", "17:30"), "0 30 17 * * Fri")
      assert_eq(Cron.weekly_at("tue", "06:05"), "0 5 6 * * Tue")
    end

    test("refuses an unknown weekday") do
      assert_raises("Unknown weekday: someday") do
        Cron.weekly_at("someday", "09:00")
      end
    end
  end
end

describe("Cron schedules") do
  before_each() do
    requires_solidb()
  end

  after_each() do
    Cron.delete(SCHEDULE_NAME) unless find_schedule(SCHEDULE_NAME).nil?
  end

  test("schedule stores an enabled entry and returns its name") do
    assert_eq(Cron.schedule(SCHEDULE_NAME, Cron.daily_at("03:00"), "ReportJob", {"a": 1}), SCHEDULE_NAME)
    schedule = find_schedule(SCHEDULE_NAME)
    assert_eq(schedule["cron_expression"], "0 0 3 * * *")
    assert_eq(schedule["handler"], "ReportJob")
    assert_eq(schedule["args"], {"a": 1})
    assert_eq(schedule["enabled"], true)
    assert_null(schedule["last_run_at"])
    assert_match(schedule["next_run_at"], "T03:00:00Z$")
  end

  test("schedule refuses an expression that could never fire") do
    assert_raises("invalid cron expression \"nope\"") do
      Cron.schedule(SCHEDULE_NAME, "nope", "ReportJob", {})
    end
    assert_null(find_schedule(SCHEDULE_NAME))
  end

  test("schedule is an upsert keyed by name") do
    Cron.schedule(SCHEDULE_NAME, Cron.daily_at("03:00"), "ReportJob", {})
    Cron.schedule(SCHEDULE_NAME, Cron.daily_at("04:00"), "ReportJob", {})
    names = Cron.list.map { |schedule| schedule["name"] }
    assert_eq(names, [SCHEDULE_NAME])
    assert_eq(find_schedule(SCHEDULE_NAME)["cron_expression"], "0 0 4 * * *")
  end

  test("rescheduling by name moves the next run") do
    pending("bug: Cron.schedule's upsert changes cron_expression but keeps the old next_run_at")
    Cron.schedule(SCHEDULE_NAME, Cron.daily_at("03:00"), "ReportJob", {})
    Cron.schedule(SCHEDULE_NAME, Cron.daily_at("04:00"), "ReportJob", {})
    assert_match(find_schedule(SCHEDULE_NAME)["next_run_at"], "T04:00:00Z$")
  end

  test("update changes the expression and the next run") do
    Cron.schedule(SCHEDULE_NAME, Cron.daily_at("03:00"), "ReportJob", {})
    assert_eq(Cron.update(SCHEDULE_NAME, {"cron_expression": "0 0 5 * * *"}), true)
    schedule = find_schedule(SCHEDULE_NAME)
    assert_eq(schedule["cron_expression"], "0 0 5 * * *")
    assert_match(schedule["next_run_at"], "T05:00:00Z$")
  end

  test("update refuses an invalid expression") do
    Cron.schedule(SCHEDULE_NAME, Cron.daily_at("03:00"), "ReportJob", {})
    assert_raises("Cron.update failed: invalid cron expression \"bad\"") do
      Cron.update(SCHEDULE_NAME, {"cron_expression": "bad"})
    end
    assert_eq(find_schedule(SCHEDULE_NAME)["cron_expression"], "0 0 3 * * *")
  end

  test("delete removes the schedule") do
    Cron.schedule(SCHEDULE_NAME, Cron.daily_at("03:00"), "ReportJob", {})
    assert_eq(Cron.delete(SCHEDULE_NAME), true)
    assert_null(find_schedule(SCHEDULE_NAME))
  end

  test("delete raises for an unknown name") do
    assert_raises("Cron.delete failed: HTTP 404") do
      Cron.delete("spec_no_such_schedule")
    end
  end
end
