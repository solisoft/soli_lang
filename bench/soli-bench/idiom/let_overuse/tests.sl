describe("let_overuse", fn() {
    test("joins first and last", fn() {
        assert_eq(full_name("Ada", "Lovelace"), "Ada Lovelace");
    });

    test("handles single-letter names", fn() {
        assert_eq(full_name("J", "K"), "J K");
    });
});
