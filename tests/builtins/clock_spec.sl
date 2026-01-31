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

    test("DateTime.microtime() returns microseconds", fn() {
        let mt = DateTime.microtime();
        assert(mt > 0);
    });
});
