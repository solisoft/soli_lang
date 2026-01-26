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

    test("seconds() returns seconds component", fn() {
        let dur = Duration.of_seconds(125);
        assert_eq(dur.seconds(), 5);
    });

    test("minutes() returns minutes component", fn() {
        let dur = Duration.of_minutes(65);
        assert_eq(dur.minutes(), 5);
    });

    test("hours() returns hours component", fn() {
        let dur = Duration.of_hours(26);
        assert_eq(dur.hours(), 2);
    });

    test("in_seconds() converts to seconds", fn() {
        let dur = Duration.of_minutes(2);
        assert_eq(dur.in_seconds(), 120);
    });

    test("in_minutes() converts to minutes", fn() {
        let dur = Duration.of_hours(1);
        assert_eq(dur.in_minutes(), 60);
    });

    test("in_hours() converts to hours", fn() {
        let dur = Duration.of_days(2);
        assert_eq(dur.in_hours(), 48);
    });
});

describe("Duration Arithmetic", fn() {
    test("Duration addition", fn() {
        let dur1 = Duration.of_seconds(60);
        let dur2 = Duration.of_seconds(60);
        let combined = dur1.plus(Duration.of_seconds(60));
        assert_eq(combined.total_seconds(), 180);
    });

    test("Duration subtraction", fn() {
        let dur1 = Duration.of_seconds(120);
        let dur2 = Duration.of_seconds(60);
        let result = dur1.minus(Duration.of_seconds(60));
        assert_eq(result.total_seconds(), 60);
    });

    test("Duration multiplication", fn() {
        let dur = Duration.of_seconds(10);
        let result = dur.multiplied_by(3);
        assert_eq(result.total_seconds(), 30);
    });

    test("Duration division", fn() {
        let dur = Duration.of_seconds(60);
        let result = dur.divided_by(2);
        assert_eq(result.total_seconds(), 30);
    });
});
