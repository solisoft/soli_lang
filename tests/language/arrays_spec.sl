// ============================================================================
// Arrays Test Suite
// ============================================================================

describe("Array Operations", fn() {
    test("array indexing", fn() {
        let arr = ["a", "b", "c"];
        assert_eq(arr[0], "a");
        assert_eq(arr[1], "b");
        assert_eq(arr[2], "c");
    });

    test("array index assignment", fn() {
        let arr = [1, 2, 3];
        arr[1] = 20;
        assert_eq(arr[1], 20);
    });

    test("array spread operator", fn() {
        let a = [1, 2];
        let b = [3, 4];
        let c = [...a, ...b];
        assert_eq(len(c), 4);
        assert_eq(c[0], 1);
        assert_eq(c[3], 4);
    });

    test("array of mixed types", fn() {
        let arr = [1, "two", true, null];
        assert_eq(arr[0], 1);
        assert_eq(arr[1], "two");
        assert_eq(arr[2], true);
        assert_null(arr[3]);
    });

    test("nested array indexing", fn() {
        let arr = [[1, 2], [3, 4], [5, 6]];
        assert_eq(arr[0][1], 2);
        assert_eq(arr[2][0], 5);
    });

    test("array with negative index", fn() {
        let arr = [10, 20, 30];
        assert_eq(arr[-1], 30);
        assert_eq(arr[-2], 20);
    });
});

describe("Array Methods", fn() {
    test("map on array", fn() {
        let arr = [1, 2, 3];
        let doubled = arr.map(fn(x) { return x * 2; });
        assert_eq(doubled[0], 2);
        assert_eq(doubled[1], 4);
        assert_eq(doubled[2], 6);
    });

    test("filter on array", fn() {
        let arr = [1, 2, 3, 4, 5];
        let evens = arr.filter(fn(x) { return x % 2 == 0; });
        assert_eq(len(evens), 2);
        assert_eq(evens[0], 2);
        assert_eq(evens[1], 4);
    });

    test("each on array", fn() {
        let arr = ["a", "b", "c"];
        let result = "";
        arr.each(fn(x) { result = result + x; });
        assert_eq(result, "abc");
    });

    test("reduce on array", fn() {
        let arr = [1, 2, 3, 4];
        let sum = arr.reduce(fn(acc, x) { return acc + x; }, 0);
        assert_eq(sum, 10);
    });
});
