// ============================================================================
// DateTime Test Suite
// ============================================================================

describe("DateTime Functions", fn() {
    test("DateTime.now() returns current time", fn() {
        let now = DateTime.now();
        assert_not_null(now);
        assert(now.year() >= 2024);
    });

    test("DateTime.from_unix() creates from timestamp", fn() {
        let dt = DateTime.from_unix(0);
        assert_eq(dt.year(), 1970);
        assert_eq(dt.month(), 1);
        assert_eq(dt.day(), 1);
    });

    test("datetime instance methods work", fn() {
        let dt = DateTime.from_unix(1704067200);
        assert(dt.year() >= 2024);
        assert(dt.month() >= 1);
        assert(dt.month() <= 12);
        assert(dt.day() >= 1);
        assert(dt.day() <= 31);
        assert(dt.hour() >= 0);
        assert(dt.hour() <= 23);
        assert(dt.minute() >= 0);
        assert(dt.minute() <= 59);
        assert(dt.second() >= 0);
        assert(dt.second() <= 59);
    });

    test("datetime arithmetic works", fn() {
        let dt = DateTime.from_unix(1704067200);
        let later = dt.add_days(1);
        assert(later.to_unix() > dt.to_unix());

        let earlier = dt.subtract_days(1);
        assert(earlier.to_unix() < dt.to_unix());
    });

    test("dt.to_unix() converts to timestamp", fn() {
        let dt = DateTime.from_unix(1704067200);
        let ts = dt.to_unix();
        assert_eq(ts, 1704067200);
    });

    test("datetime to_iso() formatting", fn() {
        let dt = DateTime.from_unix(0);
        let iso = dt.to_iso();
        assert_contains(iso, "1970");
    });
});

describe("DateTime Static Methods", fn() {
    test("DateTime.now() returns current datetime", fn() {
        let now = DateTime.now();
        assert_not_null(now);
        assert_eq(type(now), "DateTime");
    });

    test("DateTime.parse() parses ISO string", fn() {
        let dt = DateTime.parse("2024-01-15T10:30:00Z");
        assert_not_null(dt);
        assert_eq(type(dt), "DateTime");
    });

    test("DateTime.epoch() returns epoch datetime", fn() {
        let epoch = DateTime.epoch();
        assert_not_null(epoch);
        assert_eq(epoch.year(), 1970);
    });

    test("DateTime.from_unix() creates DateTime from timestamp", fn() {
        let dt = DateTime.from_unix(1704067200);
        assert_not_null(dt);
        assert_eq(dt.year(), 2024);
    });
});

describe("DateTime Instance Methods", fn() {
    test("add_days() adds days", fn() {
        let dt = DateTime.parse("2024-01-15T00:00:00Z");
        let later = dt.add_days(10);
        assert_eq(later.day(), 25);
    });

    test("add_hours() adds hours", fn() {
        let dt = DateTime.parse("2024-01-15T10:00:00Z");
        let later = dt.add_hours(5);
        assert_eq(later.hour(), 15);
    });

    test("add_minutes() adds minutes", fn() {
        let dt = DateTime.parse("2024-01-15T10:30:00Z");
        let later = dt.add_minutes(30);
        assert_eq(later.minute(), 0);
        assert_eq(later.hour(), 11);
    });

    test("subtract_days() subtracts days", fn() {
        let dt = DateTime.parse("2024-01-15T00:00:00Z");
        let earlier = dt.subtract_days(5);
        assert_eq(earlier.day(), 10);
    });

    test("to_unix() returns timestamp", fn() {
        let dt = DateTime.from_unix(1704067200);
        assert_eq(dt.to_unix(), 1704067200);
    });

    test("format() formats date", fn() {
        let dt = DateTime.from_unix(1704067200);
        let formatted = dt.format("%Y-%m-%d");
        assert_contains(formatted, "2024");
    });

    test("DateTime.utc() returns current UTC time", fn() {
        let utc = DateTime.utc();
        assert_not_null(utc);
        assert(utc.year() >= 2024);
    });
});
