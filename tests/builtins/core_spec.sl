// ============================================================================
// Core Global Functions Test Suite
// ============================================================================

describe("Global Functions", fn() {
    test("clock returns positive number", fn() {
        let t = clock();
        assert(t > 0);
    });

    test("clock returns unix timestamp", fn() {
        let t = clock();
        assert(t > 1700000000);
    });

    test("clock is monotonically increasing", fn() {
        let t1 = clock();
        let t2 = clock();
        assert(t2 >= t1);
    });

    test("break returns breakpoint value", fn() {
        let result = break();
        assert(result != null);
    });

    test("len works with strings", fn() {
        assert_eq(len("hello"), 5);
    });

    test("len works with arrays", fn() {
        assert_eq(len([1, 2, 3]), 3);
    });

    test("len works with hashes", fn() {
        assert_eq(len({"a": 1, "b": 2}), 2);
    });
});

describe("Print Functions", fn() {
    test("print function exists", fn() {
        print("test");
        print("hello", "world");
    });

    test("println function exists", fn() {
        println("test");
        println("hello", "world");
    });
});
