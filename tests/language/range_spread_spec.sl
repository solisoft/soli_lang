// ============================================================================
// Range and Spread Operator Test Suite
// ============================================================================

describe("Range Operator", fn() {
    test("range creates array from start to end (exclusive)", fn() {
        let r = range(1, 6);
        assert_eq(len(r), 5);
        assert_eq(r[0], 1);
        assert_eq(r[4], 5);
    });

    test("range with single element", fn() {
        let r = range(3, 4);
        assert_eq(len(r), 1);
        assert_eq(r[0], 3);
    });

    test("range with negative numbers", fn() {
        let r = range(-3, 4);
        assert_eq(len(r), 7);
        assert_eq(r[0], -3);
        assert_eq(r[6], 3);
    });

    test("range in for loop", fn() {
        let sum = 0;
        for (i in range(1, 4)) {
            sum = sum + i;
        }
        assert_eq(sum, 6);
    });

    test("range with same start and end gives empty array", fn() {
        let r = range(3, 3);
        assert_eq(len(r), 0);
    });
});

describe("Spread Operator", fn() {
    test("spread array into another array", fn() {
        let a = [1, 2, 3];
        let b = [...a, 4, 5];
        assert_eq(len(b), 5);
        assert_eq(b[0], 1);
        assert_eq(b[4], 5);
    });

    test("spread multiple arrays", fn() {
        let a = [1, 2];
        let b = [3, 4];
        let c = [...a, ...b];
        assert_eq(c, [1, 2, 3, 4]);
    });

    test("spread empty array", fn() {
        let a = [];
        let b = [...a, 1];
        assert_eq(b, [1]);
    });

    test("spread at beginning and end", fn() {
        let a = [2, 3];
        let b = [1, ...a, 4];
        assert_eq(b, [1, 2, 3, 4]);
    });

    test("spread range result", fn() {
        let r = range(1, 4);
        let combined = [...r, 4, 5];
        assert_eq(combined, [1, 2, 3, 4, 5]);
    });

    test("spread multiple ranges", fn() {
        let r1 = range(1, 3);
        let r2 = range(3, 5);
        let combined = [...r1, ...r2];
        assert_eq(combined, [1, 2, 3, 4]);
    });
});
