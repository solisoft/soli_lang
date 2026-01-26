// ============================================================================
// DateTime Extended Test Suite
// ============================================================================
// Additional tests for DateTime and Duration methods not covered in datetime_spec.sl
// ============================================================================

describe("DateTime Instance Methods - Extended", fn() {
    test("millisecond() returns millisecond component", fn() {
        let dt = DateTime.from_unix(1704067200000);
        let ms = dt.millisecond();
        assert(ms >= 0);
        assert(ms < 1000);
    });

    test("millisecond() is consistent with timestamp", fn() {
        let dt = DateTime.now();
        let ms = dt.millisecond();
        assert_eq(ms, dt.timestamp() % 1000000000 / 1000000);
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

    test("to_iso() includes time with Z suffix", fn() {
        let dt = DateTime.parse("2024-06-15T14:30:00Z");
        let iso = dt.to_iso();
        assert_contains(iso, "Z");
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
        let dt = DateTime.parse("2024-07-20T14:35:45.123Z");
        assert_eq(dt.year(), 2024);
        assert_eq(dt.month(), 7);
        assert_eq(dt.day(), 20);
        assert_eq(dt.hour(), 14);
        assert_eq(dt.minute(), 35);
        assert_eq(dt.second(), 45);
        assert_eq(dt.millisecond(), 123);
    });
});

describe("DateTime Static Methods - Extended", fn() {
    test("DateTime.utc() creates datetime in UTC", fn() {
        let dt = DateTime.utc(2024, 6, 15, 10, 30, 0);
        assert_not_null(dt);
        assert_eq(dt.year(), 2024);
        assert_eq(dt.month(), 6);
        assert_eq(dt.day(), 15);
        assert_eq(dt.hour(), 10);
        assert_eq(dt.minute(), 30);
    });

    test("DateTime.utc() with seconds", fn() {
        let dt = DateTime.utc(2024, 1, 1, 0, 0, 30);
        assert_eq(dt.second(), 30);
    });

    test("DateTime.utc() with milliseconds", fn() {
        let dt = DateTime.utc(2024, 1, 1, 0, 0, 0, 500);
        assert_eq(dt.millisecond(), 500);
    });

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

    test("DateTime.parse() with different formats", fn() {
        let dt1 = DateTime.parse("2024-01-15");
        assert_not_null(dt1);
        assert_eq(dt1.year(), 2024);
        assert_eq(dt1.month(), 1);
        assert_eq(dt1.day(), 15);
    });

    test("DateTime.parse() with time only", fn() {
        let dt = DateTime.parse("14:30:00");
        assert_not_null(dt);
        assert_eq(dt.hour(), 14);
        assert_eq(dt.minute(), 30);
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
        assert_contains(str, "2");
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

    test("negated() creates negative duration", fn() {
        let dur1 = Duration.of_seconds(60);
        let dur2 = dur1.negated();
        assert(dur2.total_seconds() < 0);
    });

    test("abs() returns absolute duration", fn() {
        let dur1 = Duration.of_seconds(-60);
        let dur2 = dur1.abs();
        assert_eq(dur2.total_seconds(), 60);
    });

    test("is_zero() checks if duration is zero", fn() {
        let dur1 = Duration.of_seconds(0);
        let dur2 = Duration.of_seconds(1);
        assert(dur1.is_zero());
        assert_not(dur2.is_zero());
    });

    test("is_negative() checks if duration is negative", fn() {
        let dur1 = Duration.of_seconds(-1);
        let dur2 = Duration.of_seconds(1);
        assert(dur1.is_negative());
        assert_not(dur2.is_negative());
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

    test("subtract_hours() across day boundary", fn() {
        let dt = DateTime.parse("2024-01-15T02:00:00Z");
        let earlier = dt.subtract_hours(5);
        assert_eq(earlier.day(), 14);
        assert_eq(earlier.hour(), 21);
    });

    test("add_seconds() works", fn() {
        let dt = DateTime.from_unix(1704067200);
        let later = dt.add_seconds(60);
        assert_eq(later.to_unix(), 1704067260);
    });

    test("add_months() wraps years", fn() {
        let dt = DateTime.parse("2024-12-15T10:00:00Z");
        let later = dt.add_months(3);
        assert_eq(later.year(), 2025);
        assert_eq(later.month(), 3);
    });

    test("add_years() works", fn() {
        let dt = DateTime.parse("2024-01-15T10:00:00Z");
        let later = dt.add_years(5);
        assert_eq(later.year(), 2029);
    });
});

describe("DateTime Comparison", fn() {
    test("compare() returns -1, 0, or 1", fn() {
        let dt1 = DateTime.parse("2024-01-15T10:00:00Z");
        let dt2 = DateTime.parse("2024-01-15T11:00:00Z");
        let cmp = dt1.compare(dt2);
        assert(cmp <= 0);
    });

    test("is_before() checks if datetime is before another", fn() {
        let dt1 = DateTime.parse("2024-01-15T10:00:00Z");
        let dt2 = DateTime.parse("2024-01-15T11:00:00Z");
        assert(dt1.is_before(dt2));
        assert_not(dt2.is_before(dt1));
    });

    test("is_after() checks if datetime is after another", fn() {
        let dt1 = DateTime.parse("2024-01-15T10:00:00Z");
        let dt2 = DateTime.parse("2024-01-15T11:00:00Z");
        assert(dt2.is_after(dt1));
        assert_not(dt1.is_after(dt2));
    });

    test("is_same_day() checks if same calendar day", fn() {
        let dt1 = DateTime.parse("2024-01-15T10:00:00Z");
        let dt2 = DateTime.parse("2024-01-15T23:59:59Z");
        assert(dt1.is_same_day(dt2));
    });

    test("is_same_day() false for different days", fn() {
        let dt1 = DateTime.parse("2024-01-15T23:59:59Z");
        let dt2 = DateTime.parse("2024-01-16T00:00:00Z");
        assert_not(dt1.is_same_day(dt2));
    });
});

describe("Duration Arithmetic - Extended", fn() {
    test("plus() adds two durations", fn() {
        let dur1 = Duration.of_hours(1);
        let dur2 = Duration.of_minutes(30);
        let result = dur1.plus(dur2);
        assert_eq(result.total_minutes(), 90);
    });

    test("minus() subtracts durations", fn() {
        let dur1 = Duration.of_hours(2);
        let dur2 = Duration.of_minutes(30);
        let result = dur1.minus(dur2);
        assert_eq(result.total_minutes(), 90);
    });

    test("multiplied_by() scales duration", fn() {
        let dur = Duration.of_minutes(30);
        let result = dur.multiplied_by(2);
        assert_eq(result.total_minutes(), 60);
    });

    test("divided_by() divides duration", fn() {
        let dur = Duration.of_minutes(60);
        let result = dur.divided_by(2);
        assert_eq(result.total_minutes(), 30);
    });

    test("divided_by() by zero returns infinity", fn() {
        let dur = Duration.of_seconds(10);
        let result = dur.divided_by(0);
        assert(result.total_minutes() == Infinity);
    });

    test("modulo() returns remainder", fn() {
        let dur = Duration.of_seconds(100);
        let result = dur.modulo(Duration.of_seconds(30));
        assert_eq(result.total_seconds(), 10);
    });
});
