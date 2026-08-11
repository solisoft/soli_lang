# ============================================================================
# Job and Cron Class Test Suite
# ============================================================================
#
# Pure-API tests for the static helpers plus `perform_now`, which needs no
# database. The queue paths (`Job.enqueue`, `Cron.schedule`, claim/retry) are
# covered by the Rust integration suite in `src/jobs/claim.rs`, which runs
# against a real Postgres.

describe("Cron expression helpers", fn() {
    # All helpers emit 6-field expressions (sec min hour dom month dow) — the
    # scheduler parses with the `cron` crate, which rejects the 5-field form.
    test("Cron.every returns minute cron strings", fn() {
        assert_eq(Cron.every("5 minutes"), "0 */5 * * * *");
        assert_eq(Cron.every("15 minutes"), "0 */15 * * * *");
    });

    test("Cron.every recognizes hour granularity", fn() {
        assert_eq(Cron.every("1 hour"), "0 0 * * * *");
        assert_eq(Cron.every("2 hours"), "0 0 */2 * * *");
    });

    test("Cron.every recognizes day granularity", fn() {
        assert_eq(Cron.every("1 day"), "0 0 0 */1 * *");
    });

    test("Cron.daily_at parses HH:MM", fn() {
        assert_eq(Cron.daily_at("03:00"), "0 0 3 * * *");
        assert_eq(Cron.daily_at("23:45"), "0 45 23 * * *");
    });

    test("Cron.hourly returns top-of-hour cron string", fn() {
        assert_eq(Cron.hourly(), "0 0 * * * *");
    });

    test("Cron.weekly_at maps weekday names to cron DOW names", fn() {
        assert_eq(Cron.weekly_at("monday", "09:00"), "0 0 9 * * Mon");
        assert_eq(Cron.weekly_at("sunday", "00:00"), "0 0 0 * * Sun");
        assert_eq(Cron.weekly_at("fri", "17:30"), "0 30 17 * * Fri");
    });
});

describe("Job classes are registered", fn() {
    test("Job class exists", fn() {
        assert(defined("Job"));
    });

    test("Webhook class exists", fn() {
        assert(defined("Webhook"));
    });

    test("Cron class exists", fn() {
        assert(defined("Cron"));
    });
});
