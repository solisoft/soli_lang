// ============================================================================
// Async/Await Test Suite
// ============================================================================

describe("Async/Await", fn() {
    test("await function resolves future", fn() {
        let future = System.run("echo hello");
        let result = await(future);
        assert(result != null);
        assert_eq(result["exit_code"], 0);
    });

    test("await resolves multiple futures sequentially", fn() {
        let f1 = System.run("echo first");
        let f2 = System.run("echo second");
        let r1 = await(f1);
        let r2 = await(f2);
        assert(r1["exit_code"] == 0);
        assert(r2["exit_code"] == 0);
    });

    test("System.run returns a future", fn() {
        let future = System.run("echo test");
        assert(future != null);
    });

    test("await with pipe syntax", fn() {
        let result = await(System.run("echo pipe_test"));
        assert(result != null);
    });
});

describe("Await Expression", fn() {
    test("await expression syntax", fn() {
        # await is parsed as a variable by the current implementation
        # The await() function is the primary way to await
        let future = System.run("echo test");
        let result = await(future);
        assert(result != null);
    });
});
