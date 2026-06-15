describe("dedup_preserve_order", fn() {
    test("removes dupes, keeps first occurrence order", fn() {
        assert_eq(dedup([1, 2, 2, 3, 1, 4, 3]), [1, 2, 3, 4]);
    });

    test("empty input", fn() {
        assert_eq(dedup([]), []);
    });

    test("all the same", fn() {
        assert_eq(dedup(["a", "a", "a"]), ["a"]);
    });

    test("already unique", fn() {
        assert_eq(dedup([1, 2, 3]), [1, 2, 3]);
    });
});
