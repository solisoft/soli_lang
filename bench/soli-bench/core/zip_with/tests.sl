describe("zip_with", fn() {
    test("equal-length arrays", fn() {
        assert_eq(
            zip_with([1, 2, 3], [10, 20, 30], fn(x, y) x + y),
            [11, 22, 33]
        );
    });

    test("first shorter", fn() {
        assert_eq(
            zip_with([1, 2], [10, 20, 30], fn(x, y) x + y),
            [11, 22]
        );
    });

    test("second shorter", fn() {
        assert_eq(
            zip_with([1, 2, 3], [10, 20], fn(x, y) x + y),
            [11, 22]
        );
    });

    test("first empty", fn() {
        assert_eq(
            zip_with([], [1, 2], fn(x, y) x + y),
            []
        );
    });
});
