describe("group_by_array", fn() {
    test("groups by simple numeric key", fn() {
        let result = group_by([1, 2, 3, 4], fn(x) x % 2);
        assert_eq(len(result["0"]), 2);
        assert_eq(len(result["1"]), 2);
        assert_eq(result["0"][0], 2);
        assert_eq(result["1"][0], 1);
    });

    test("groups by extracted hash field, preserves order", fn() {
        let users = [
            {"name": "Alice", "team": "red"},
            {"name": "Bob",   "team": "blue"},
            {"name": "Cara",  "team": "red"}
        ];
        let result = group_by(users, fn(p) p["team"]);
        assert_eq(len(result["red"]),  2);
        assert_eq(len(result["blue"]), 1);
        assert_eq(result["red"][0]["name"],  "Alice");
        assert_eq(result["red"][1]["name"],  "Cara");
        assert_eq(result["blue"][0]["name"], "Bob");
    });

    test("empty input returns empty hash", fn() {
        let result = group_by([], fn(x) x);
        assert_eq(len(result), 0);
    });

    test("does not mutate input", fn() {
        let input = [1, 2, 3];
        group_by(input, fn(x) x);
        assert_eq(len(input), 3);
    });
});
