// ============================================================================
// Array Functions Test Suite
// ============================================================================

describe("Array Functions", fn() {
    test("len() returns array length", fn() {
        assert_eq(len([1, 2, 3]), 3);
        assert_eq(len([]), 0);
    });

    test("push() adds element to array", fn() {
        let arr = [1, 2];
        push(arr, 3);
        assert_eq(len(arr), 3);
        assert_eq(arr[2], 3);
    });

    test("pop() removes last element", fn() {
        let arr = [1, 2, 3];
        let last = pop(arr);
        assert_eq(last, 3);
        assert_eq(len(arr), 2);
    });

    test("range() creates array of numbers", fn() {
        let r = range(0, 5);
        assert_eq(len(r), 5);
        assert_eq(r[0], 0);
        assert_eq(r[4], 4);
    });

    test("assert_contains works with arrays", fn() {
        let arr = [1, 2, 3];
        assert_contains(arr, 2);
    });

    test("first() returns first element", fn() {
        let arr = [1, 2, 3];
        assert_eq(first(arr), 1);
    });

    test("last() returns last element", fn() {
        let arr = [1, 2, 3];
        assert_eq(last(arr), 3);
    });

    test("reverse() reverses array", fn() {
        let arr = [1, 2, 3];
        let reversed = reverse(arr);
        assert_eq(reversed[0], 3);
        assert_eq(reversed[2], 1);
    });

    test("unique() removes duplicates", fn() {
        let arr = [1, 2, 2, 3, 3, 3];
        let uniq = unique(arr);
        assert_eq(len(uniq), 3);
    });

    test("flatten() flattens nested arrays", fn() {
        let arr = [[1, 2], [3, 4]];
        let flat = flatten(arr);
        assert_eq(len(flat), 4);
    });
});
