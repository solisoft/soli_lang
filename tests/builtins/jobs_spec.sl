// ============================================================================
// Job and Cron Class Test Suite
// ============================================================================
//
// Pure-API tests for the static helpers. Network-touching paths
// (`Job.enqueue`, `Cron.schedule`, etc.) require a running SolidB and are
// intentionally not exercised here — they're covered by manual e2e in the
// jobs documentation.

describe("Cron expression helpers", fn() {
    test("Cron.every returns minute cron strings", fn() {
        assert_eq(Cron.every("5 minutes"), "*/5 * * * *");
        assert_eq(Cron.every("15 minutes"), "*/15 * * * *");
    });

    test("Cron.every recognizes hour granularity", fn() {
        assert_eq(Cron.every("1 hour"), "0 * * * *");
        assert_eq(Cron.every("2 hours"), "0 */2 * * *");
    });

    test("Cron.every recognizes day granularity", fn() {
        assert_eq(Cron.every("1 day"), "0 0 */1 * *");
    });

    test("Cron.daily_at parses HH:MM", fn() {
        assert_eq(Cron.daily_at("03:00"), "0 3 * * *");
        assert_eq(Cron.daily_at("23:45"), "45 23 * * *");
    });

    test("Cron.hourly returns top-of-hour cron string", fn() {
        assert_eq(Cron.hourly(), "0 * * * *");
    });

    test("Cron.weekly_at maps weekday names to numeric DOW", fn() {
        assert_eq(Cron.weekly_at("monday", "09:00"), "0 9 * * 1");
        assert_eq(Cron.weekly_at("sunday", "00:00"), "0 0 * * 0");
        assert_eq(Cron.weekly_at("fri", "17:30"), "30 17 * * 5");
    });
});

describe("Job class is registered", fn() {
    test("Job class exists", fn() {
        assert(defined("Job"));
    });

    test("Cron class exists", fn() {
        assert(defined("Cron"));
    });
});
