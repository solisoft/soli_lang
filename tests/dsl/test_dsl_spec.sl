// ============================================================================
// Test DSL Test Suite
// ============================================================================

describe("Test DSL", fn() {
    test("test() creates a test case", fn() {
        assert(true);
    });

    context("with nested context", fn() {
        test("nested test works", fn() {
            assert(true);
        });
    });

    it("it() is an alias for test()", fn() {
        assert(true);
    });

    specify("specify() is an alias for test()", fn() {
        assert(true);
    });

    test("before_each() runs before each test", fn() {
        assert(true);
    });

    test("after_each() runs after each test", fn() {
        assert(true);
    });
});

describe("Test DSL - describe blocks", fn() {
    test("can nest describe blocks", fn() {
        assert(true);
    });

    context("nested context blocks", fn() {
        test("tests in deep context work", fn() {
            assert(true);
        });
    });
});

describe("Test DSL - before_each and after_each", fn() {
    let counter = 0;

    before_each(fn() {
        counter = counter + 1;
    });

    test("before_each increments counter", fn() {
        assert(counter > 0);
    });

    test("before_each runs for each test", fn() {
        assert(counter >= 2);
    });
});
