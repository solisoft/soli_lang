describe("count_by", fn() {
    test("counts even vs odd", fn() {
        let result = count_by([1, 2, 3, 4, 5, 6], fn(x) x % 2);
        assert_eq(result["0"], 3);
        assert_eq(result["1"], 3);
    });

    test("counts by first letter", fn() {
        let result = count_by(
            ["apple", "apricot", "banana", "blueberry", "cherry"],
            fn(s) s[0]
        );
        assert_eq(result["a"], 2);
        assert_eq(result["b"], 2);
        assert_eq(result["c"], 1);
    });

    test("empty input", fn() {
        assert_eq(len(count_by([], fn(x) x)), 0);
    });
});
