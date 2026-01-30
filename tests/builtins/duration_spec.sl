// ============================================================================
// Duration Test Suite
// ============================================================================

describe("Duration Static Methods", fn() {
    test("Duration.of_seconds() creates duration", fn() {
        let dur = Duration.of_seconds(120);
        assert_not_null(dur);
        assert_eq(type(dur), "Duration");
    });

    test("Duration.of_minutes() creates duration", fn() {
        let dur = Duration.of_minutes(5);
        assert_not_null(dur);
    });

    test("Duration.of_hours() creates duration", fn() {
        let dur = Duration.of_hours(2);
        assert_not_null(dur);
    });

    test("Duration.of_days() creates duration", fn() {
        let dur = Duration.of_days(1);
        assert_not_null(dur);
    });

    test("Duration.of_weeks() creates duration", fn() {
        let dur = Duration.of_weeks(1);
        assert_not_null(dur);
    });

    test("Duration.seconds() creates duration", fn() {
        let dur = Duration.seconds(120);
        assert_not_null(dur);
    });

    test("Duration.minutes() creates duration", fn() {
        let dur = Duration.minutes(5);
        assert_not_null(dur);
    });

    test("Duration.hours() creates duration", fn() {
        let dur = Duration.hours(2);
        assert_not_null(dur);
    });

    test("Duration.days() creates duration", fn() {
        let dur = Duration.days(1);
        assert_not_null(dur);
    });

    test("Duration.weeks() creates duration", fn() {
        let dur = Duration.weeks(1);
        assert_not_null(dur);
    });

    test("Duration.between() calculates difference", fn() {
        let dt1 = DateTime.parse("2024-01-15T10:00:00Z");
        let dt2 = DateTime.parse("2024-01-15T11:30:00Z");
        let dur = Duration.between(dt1, dt2);
        assert_not_null(dur);
        assert_eq(type(dur), "Duration");
    });
});

describe("Duration Instance Methods", fn() {
    test("total_seconds() returns total seconds", fn() {
        let dur = Duration.of_seconds(120);
        assert_eq(dur.total_seconds(), 120);
    });

    test("total_minutes() returns total minutes", fn() {
        let dur = Duration.of_minutes(5);
        assert_eq(dur.total_minutes(), 5);
    });

    test("total_hours() returns total hours", fn() {
        let dur = Duration.of_hours(2);
        assert_eq(dur.total_hours(), 2);
    });

    test("total_days() returns total days", fn() {
        let dur = Duration.of_days(1);
        assert_eq(dur.total_days(), 1);
    });

    test("to_string() returns formatted duration", fn() {
        let dur = Duration.of_seconds(3661);
        let str = dur.to_string();
        assert_not_null(str);
        assert(len(str) > 0);
    });
});

describe("Duration Conversions", fn() {
    test("seconds to minutes conversion", fn() {
        let dur = Duration.of_seconds(120);
        assert_eq(dur.total_minutes(), 2);
    });

    test("minutes to seconds conversion", fn() {
        let dur = Duration.of_minutes(5);
        assert_eq(dur.total_seconds(), 300);
    });

    test("hours to minutes conversion", fn() {
        let dur = Duration.of_hours(2);
        assert_eq(dur.total_minutes(), 120);
    });

    test("days to hours conversion", fn() {
        let dur = Duration.of_days(1);
        assert_eq(dur.total_hours(), 24);
    });

    test("weeks to days conversion", fn() {
        let dur = Duration.of_weeks(2);
        assert_eq(dur.total_days(), 14);
    });
});

describe("Duration Between", fn() {
    test("between two datetimes gives positive duration", fn() {
        let dt1 = DateTime.parse("2024-01-15T10:00:00Z");
        let dt2 = DateTime.parse("2024-01-15T11:00:00Z");
        let dur = Duration.between(dt1, dt2);
        assert(dur.total_hours() > 0);
    });

    test("between same datetime gives zero", fn() {
        let dt = DateTime.parse("2024-01-15T10:00:00Z");
        let dur = Duration.between(dt, dt);
        assert_eq(dur.total_seconds(), 0);
    });
});
