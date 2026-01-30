// ============================================================================
// DateTime Extended Test Suite
// ============================================================================
// Additional tests for DateTime and Duration methods
// ============================================================================

describe("DateTime Instance Methods - Extended", fn() {
    test("millisecond() returns millisecond component", fn() {
        let dt = DateTime.from_unix(1704067200);
        let ms = dt.millisecond();
        assert(ms >= 0);
        assert(ms < 1000);
    });

    test("millisecond() is consistent with timestamp", fn() {
        let dt = DateTime.now();
        let ms = dt.millisecond();
        assert(ms >= 0);
        assert(ms < 1000);
    });

    test("weekday() returns weekday name", fn() {
        let dt = DateTime.parse("2024-01-15T10:00:00Z");
        let day = dt.weekday();
        assert_not_null(day);
        assert(day == "Monday" || day == "Tuesday" || day == "Wednesday" ||
              day == "Thursday" || day == "Friday" || day == "Saturday" || day == "Sunday");
    });

    test("weekday() is correct for known date", fn() {
        let dt = DateTime.parse("2024-01-01T00:00:00Z");
        let day = dt.weekday();
        assert_eq(day, "Monday");
    });

    test("to_iso() returns ISO 8601 formatted string", fn() {
        let dt = DateTime.parse("2024-01-15T10:30:00Z");
        let iso = dt.to_iso();
        assert_contains(iso, "2024");
        assert_contains(iso, "01");
        assert_contains(iso, "15");
        assert_contains(iso, "T");
    });

    test("to_string() returns formatted datetime string", fn() {
        let dt = DateTime.parse("2024-01-15T10:30:00Z");
        let str = dt.to_string();
        assert_not_null(str);
        assert(len(str) > 0);
    });

    test("to_string() contains date components", fn() {
        let dt = DateTime.parse("2024-03-25T15:45:30Z");
        let str = dt.to_string();
        assert_contains(str, "2024");
        assert_contains(str, "03");
        assert_contains(str, "25");
    });

    test("all datetime components together", fn() {
        let dt = DateTime.parse("2024-07-20T14:35:45Z");
        assert_eq(dt.year(), 2024);
        assert_eq(dt.month(), 7);
        assert_eq(dt.day(), 20);
        assert_eq(dt.hour(), 14);
        assert_eq(dt.minute(), 35);
        assert_eq(dt.second(), 45);
    });
});

describe("DateTime Static Methods - Extended", fn() {
    test("DateTime.epoch() returns epoch datetime", fn() {
        let epoch = DateTime.epoch();
        assert_not_null(epoch);
        assert_eq(epoch.year(), 1970);
        assert_eq(epoch.month(), 1);
        assert_eq(epoch.day(), 1);
    });

    test("DateTime.epoch() timestamp is zero", fn() {
        let epoch = DateTime.epoch();
        assert_eq(epoch.to_unix(), 0);
    });

    test("DateTime.parse() with date only", fn() {
        let dt1 = DateTime.parse("2024-01-15");
        assert_not_null(dt1);
        assert_eq(dt1.year(), 2024);
        assert_eq(dt1.month(), 1);
        assert_eq(dt1.day(), 15);
    });

    test("DateTime.parse() with timezone offset", fn() {
        let dt = DateTime.parse("2024-01-15T10:00:00+05:30");
        assert_not_null(dt);
    });
});

describe("Duration Static Methods - Extended", fn() {
    test("Duration.weeks() creates duration from weeks", fn() {
        let dur = Duration.weeks(2);
        assert_not_null(dur);
        assert_eq(dur.total_days(), 14);
    });

    test("Duration.weeks() total seconds", fn() {
        let dur = Duration.weeks(1);
        assert_eq(dur.total_seconds(), 604800.0);
    });

    test("Duration.of_weeks() alias works", fn() {
        let dur = Duration.of_weeks(3);
        assert_eq(dur.total_days(), 21);
    });
});

describe("Duration Instance Methods - Extended", fn() {
    test("to_string() returns formatted duration", fn() {
        let dur = Duration.of_seconds(3661);
        let str = dur.to_string();
        assert_not_null(str);
        assert(len(str) > 0);
    });

    test("to_string() includes time components", fn() {
        let dur = Duration.of_hours(2);
        let str = dur.to_string();
        // Duration.to_string() returns format like "7200s"
        assert_contains(str, "7200");
    });

    test("to_string() for zero duration", fn() {
        let dur = Duration.of_seconds(0);
        let str = dur.to_string();
        assert_not_null(str);
    });

    test("to_string() for large duration", fn() {
        let dur = Duration.of_days(365);
        let str = dur.to_string();
        assert_not_null(str);
        assert(len(str) > 0);
    });
});

describe("DateTime Arithmetic - Extended", fn() {
    test("add_hours() wraps across days", fn() {
        let dt = DateTime.parse("2024-01-15T20:00:00Z");
        let later = dt.add_hours(10);
        assert_eq(later.day(), 16);
        assert_eq(later.hour(), 6);
    });

    test("add_hours() wraps across months", fn() {
        let dt = DateTime.parse("2024-01-31T22:00:00Z");
        let later = dt.add_hours(5);
        assert_eq(later.month(), 2);
        assert_eq(later.day(), 1);
    });

    test("add_hours() handles leap year", fn() {
        let dt = DateTime.parse("2024-02-28T22:00:00Z");
        let later = dt.add_hours(5);
        assert_eq(later.month(), 2);
        assert_eq(later.day(), 29);
    });

    test("add_minutes() across hour boundary", fn() {
        let dt = DateTime.parse("2024-01-15T10:45:00Z");
        let later = dt.add_minutes(30);
        assert_eq(later.hour(), 11);
        assert_eq(later.minute(), 15);
    });
});
