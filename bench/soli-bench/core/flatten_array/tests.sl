describe("flatten_array", fn() {
    test("flattens one level", fn() {
        assert_eq(flatten([1, [2, 3], 4]), [1, 2, 3, 4]);
    });

    test("flattens deeply nested", fn() {
        assert_eq(flatten([1, [2, [3, [4, 5]]], 6]), [1, 2, 3, 4, 5, 6]);
    });

    test("already flat", fn() {
        assert_eq(flatten([1, 2, 3]), [1, 2, 3]);
    });

    test("empty", fn() {
        assert_eq(flatten([]), []);
    });
});
