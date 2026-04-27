// ============================================================================
// Clock/Time Functions Test Suite
// ============================================================================

describe("Clock Functions", fn() {
    test("clock() returns current time", fn() {
        let t1 = clock();
        let t2 = clock();
        assert(t2 >= t1);
        assert(t1 > 0);
    });

    test("clock() returns Unix timestamp", fn() {
        let now = clock();
        assert(now > 1600000000);
    });

    test("sleep() pauses execution", fn() {
        let start = clock();
        sleep(0.01);
        let elapsed = clock() - start;
        assert(elapsed >= 0.01);
    });

    test("Int.sleep pauses execution", fn() {
        let start = clock();
        0.sleep;
        let elapsed = clock() - start;
        assert(elapsed >= 0);
        assert(elapsed < 0.5);
    });

    test("Float.sleep pauses execution", fn() {
        let start = clock();
        (0.01).sleep;
        let elapsed = clock() - start;
        assert(elapsed >= 0.01);
    });

    test("Int.sleep returns null", fn() {
        let r = 0.sleep;
        assert_null(r);
    });

    test("Float.sleep returns null", fn() {
        let r = (0.0).sleep;
        assert_null(r);
    });

    test("Negative sleep is a no-op", fn() {
        let start = clock();
        (-1).sleep;
        (-0.5).sleep;
        let elapsed = clock() - start;
        assert(elapsed < 0.5);
    });

    test("DateTime.microtime() returns microseconds", fn() {
        let mt = DateTime.microtime();
        assert(mt > 0);
    });
});
