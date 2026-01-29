// ============================================================================
// Constants Test Suite
// ============================================================================

describe("Constants", fn() {
    test("basic const declaration", fn() {
        const PI = 3.14159;
        assert_eq(PI, 3.14159);
    });

    test("const with integer value", fn() {
        const MAX_SIZE = 1000;
        assert_eq(MAX_SIZE, 1000);
    });

    test("const with string value", fn() {
        const GREETING = "Hello, World!";
        assert_eq(GREETING, "Hello, World!");
    });

    test("const with boolean value", fn() {
        const ENABLED = true;
        assert_eq(ENABLED, true);
        const DISABLED = false;
        assert_eq(DISABLED, false);
    });

    test("const with array value", fn() {
        const COLORS = ["red", "green", "blue"];
        assert_eq(COLORS[0], "red");
        assert_eq(COLORS[1], "green");
        assert_eq(COLORS[2], "blue");
    });

    test("const with hash value", fn() {
        const CONFIG = {"host": "localhost", "port": 3000};
        assert_eq(CONFIG["host"], "localhost");
        assert_eq(CONFIG["port"], 3000);
    });

    test("const with type annotation", fn() {
        const PI: Float = 3.14159;
        assert_eq(PI, 3.14159);
    });

    test("multiple consts in same scope", fn() {
        const A = 1;
        const B = 2;
        const C = 3;
        assert_eq(A, 1);
        assert_eq(B, 2);
        assert_eq(C, 3);
    });

    test("const expression", fn() {
        const SUM = 10 + 20;
        assert_eq(SUM, 30);
    });

    test("const can be used in expressions", fn() {
        const VALUE = 100;
        const DOUBLED = VALUE * 2;
        assert_eq(DOUBLED, 200);
    });

    test("const in function", fn() {
        fn get_radius() {
            const PI = 3.14159;
            return PI * 10;
        }
        assert_eq(get_radius(), 31.4159);
    });

    test("const shadows let variable", fn() {
        let x = 10;
        const x = 20;
        assert_eq(x, 20);
    });
});

describe("Const Reassignment Error", fn() {
    test("reassigning const throws error", fn() {
        const VALUE = 42;
        assert_eq(VALUE, 42);

        fn try_reassign() {
            VALUE = 100;
        }

        assert_error(try_reassign, "cannot reassign constant");
    });

    test("cannot modify const array", fn() {
        const ARR = [1, 2, 3];
        fn try_modify() {
            ARR[0] = 100;
        }
        assert_error(try_modify, "cannot reassign constant");
    });

    test("cannot modify const hash", fn() {
        const H = {"key": "value"};
        fn try_modify() {
            H["key"] = "new";
        }
        assert_error(try_modify, "cannot reassign constant");
    });
});
