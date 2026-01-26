// ============================================================================
// Math Functions Test Suite
// ============================================================================

describe("Math Functions", fn() {
    test("abs() returns absolute value", fn() {
        assert_eq(abs(-5), 5);
        assert_eq(abs(5), 5);
        assert_eq(abs(-3.14), 3.14);
    });

    test("min() returns minimum value", fn() {
        assert_eq(min(3, 5), 3);
        assert_eq(min(10, 2), 2);
        assert_eq(min(-1, 1), -1);
    });

    test("max() returns maximum value", fn() {
        assert_eq(max(3, 5), 5);
        assert_eq(max(10, 2), 10);
        assert_eq(max(-1, 1), 1);
    });

    test("sqrt() returns square root", fn() {
        assert_eq(sqrt(4), 2.0);
        assert_eq(sqrt(9), 3.0);
        assert_eq(sqrt(2), 1.4142135623730951);
    });

    test("pow() returns power", fn() {
        assert_eq(pow(2, 3), 8.0);
        assert_eq(pow(10, 2), 100.0);
        assert_eq(pow(2, 0), 1.0);
    });

    test("floor() rounds down", fn() {
        assert_eq(floor(3.9), 3);
        assert_eq(floor(3.1), 3);
        assert_eq(floor(-3.1), -4);
    });

    test("ceil() rounds up", fn() {
        assert_eq(ceil(3.1), 4);
        assert_eq(ceil(3.9), 4);
        assert_eq(ceil(-3.1), -3);
    });

    test("round() rounds to nearest", fn() {
        assert_eq(round(3.4), 3);
        assert_eq(round(3.5), 4);
        assert_eq(round(3.6), 4);
    });

    test("random() returns value between 0 and 1", fn() {
        let r = random();
        assert(r >= 0.0);
        assert(r < 1.0);
    });

    test("random() returns different values", fn() {
        let r1 = random();
        let r2 = random();
        assert(r1 != r2 || random() > 0.999);
    });

    test("log() returns natural logarithm", fn() {
        let ln = log(2.71828);
        assert(ln > 0.99 && ln < 1.01);
    });

    test("log10() returns base-10 logarithm", fn() {
        assert_eq(log10(100), 2.0);
    });

    test("pi constant", fn() {
        assert(pi > 3.14159 && pi < 3.14160);
    });

    test("e constant", fn() {
        assert(e > 2.71828 && e < 2.71829);
    });
});
