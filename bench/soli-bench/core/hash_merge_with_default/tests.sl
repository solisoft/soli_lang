describe("hash_merge_with_default", fn() {
    test("override null falls back to default", fn() {
        let r = merge_with_default(
            {"a": 1, "b": 2},
            {"b": null, "c": 3},
            -1
        );
        assert_eq(r["a"], 1);
        assert_eq(r["b"], -1);
        assert_eq(r["c"], 3);
    });

    test("override non-null wins over base", fn() {
        let r = merge_with_default(
            {"a": 1},
            {"a": 99},
            -1
        );
        assert_eq(r["a"], 99);
    });

    test("does not mutate inputs", fn() {
        let base = {"a": 1};
        let over = {"b": 2};
        merge_with_default(base, over, 0);
        assert_eq(len(base), 1);
        assert_eq(len(over), 1);
    });
});
