// ============================================================================
// Math Functions Test Suite
// ============================================================================

describe("Math Functions", fn() {
    test(".abs returns absolute value", fn() {
        assert_eq((-5).abs, 5);
        assert_eq(5.abs, 5);
        assert_eq((-3.14).abs, 3.14);
    });

    test("[].min returns minimum value", fn() {
        assert_eq([3, 5].min, 3);
        assert_eq([10, 2].min, 2);
        assert_eq([-1, 1].min, -1);
    });

    test("[].max returns maximum value", fn() {
        assert_eq([3, 5].max, 5);
        assert_eq([10, 2].max, 10);
        assert_eq([-1, 1].max, 1);
    });

    test(".sqrt returns square root", fn() {
        assert_eq(4.sqrt, 2.0);
        assert_eq(9.sqrt, 3.0);
        assert_eq(2.sqrt, 1.4142135623730951);
    });

    test(".pow() returns power", fn() {
        assert_eq(2.pow(3), 8);
        assert_eq(10.pow(2), 100);
        assert_eq(2.pow(0), 1);
    });

    test("floor() rounds down", fn() {
        assert_eq(Math.floor(3.9), 3);
        assert_eq(Math.floor(3.1), 3);
        assert_eq(Math.floor(-3.1), -4);
    });

    test("ceil() rounds up", fn() {
        assert_eq(Math.ceil(3.1), 4);
        assert_eq(Math.ceil(3.9), 4);
        assert_eq(Math.ceil(-3.1), -3);
    });

    test("round() rounds to nearest", fn() {
        assert_eq(Math.round(3.4), 3);
        assert_eq(Math.round(3.5), 4);
        assert_eq(Math.round(3.6), 4);
    });

    test("random() returns value between 0 and 1", fn() {
        let r = Math.random();
        assert(r >= 0.0);
        assert(r < 1.0);
    });

    test("random() returns different values", fn() {
        let r1 = Math.random();
        let r2 = Math.random();
        assert(r1 != r2 || Math.random() > 0.999);
    });

    test("log() returns natural logarithm", fn() {
        let ln = Math.log(2.71828);
        assert(ln > 0.99 && ln < 1.01);
    });

    test("log10() returns base-10 logarithm", fn() {
        assert_eq(Math.log10(100), 2.0);
    });

    test("pi constant", fn() {
        assert(Math.pi > 3.14159 && Math.pi < 3.14160);
    });

    test("e constant", fn() {
        assert(Math.e > 2.71828 && Math.e < 2.71829);
    });

    test("Math.sin() returns sine", fn() {
        let result = Math.sin(0.0);
        assert(result > -0.001 && result < 0.001);
    });

    test("Math.cos() returns cosine", fn() {
        let result = Math.cos(0.0);
        assert(result > 0.999 && result < 1.001);
    });

    test("Math.tan() returns tangent", fn() {
        let result = Math.tan(0.0);
        assert(result > -0.001 && result < 0.001);
    });

    test("Math.exp() returns e^n", fn() {
        let result = Math.exp(1.0);
        assert(result > 2.718 && result < 2.719);
    });

    test("Math.exp(0) returns 1", fn() {
        let result = Math.exp(0.0);
        assert(result > 0.999 && result < 1.001);
    });
});
