// ============================================================================
// Control Flow Test Suite
// ============================================================================

describe("If/Else Statements", fn() {
    test("if true executes block", fn() {
        let result = 0;
        if (true) {
            result = 1;
        }
        assert_eq(result, 1);
    });

    test("if false skips block", fn() {
        let result = 0;
        if (false) {
            result = 1;
        }
        assert_eq(result, 0);
    });

    test("if-else executes else branch", fn() {
        let result = 0;
        if (false) {
            result = 1;
        } else {
            result = 2;
        }
        assert_eq(result, 2);
    });

    test("if-elsif-else chain", fn() {
        let x = 2;
        let result = "";
        if (x == 1) {
            result = "one";
        } elsif (x == 2) {
            result = "two";
        } else {
            result = "other";
        }
        assert_eq(result, "two");
    });

    test("nested if statements", fn() {
        let a = true;
        let b = true;
        let result = 0;
        if (a) {
            if (b) {
                result = 1;
            }
        }
        assert_eq(result, 1);
    });

    test("if with complex condition", fn() {
        let x = 5;
        let y = 10;
        let result = "";
        if (x > 0 && y > 0) {
            result = "both positive";
        }
        assert_eq(result, "both positive");
    });
});

describe("While Loops", fn() {
    test("while loop iterates", fn() {
        let count = 0;
        while (count < 5) {
            count = count + 1;
        }
        assert_eq(count, 5);
    });

    test("while loop with false condition never executes", fn() {
        let executed = false;
        while (false) {
            executed = true;
        }
        assert_not(executed);
    });

    test("while loop with complex condition", fn() {
        let i = 0;
        let sum = 0;
        while (i < 10 && sum < 20) {
            sum = sum + i;
            i = i + 1;
        }
        assert(sum >= 20 || i >= 10);
    });
});

describe("For-In Loops", fn() {
    test("for-in iterates over array", fn() {
        let arr = [1, 2, 3];
        let sum = 0;
        for (x in arr) {
            sum = sum + x;
        }
        assert_eq(sum, 6);
    });

    test("for-in iterates over range", fn() {
        let sum = 0;
        for (i in range(1, 5)) {
            sum = sum + i;
        }
        assert_eq(sum, 10);
    });

    test("for-in with empty array", fn() {
        let count = 0;
        for (x in []) {
            count = count + 1;
        }
        assert_eq(count, 0);
    });

    test("for-in can access loop variable", fn() {
        let result = [];
        for (i in range(0, 3)) {
            result.push(i * 2);
        }
        assert_eq(result[0], 0);
        assert_eq(result[1], 2);
        assert_eq(result[2], 4);
    });

    test("nested for-in loops", fn() {
        let sum = 0;
        for (i in range(0, 3)) {
            for (j in range(0, 3)) {
                sum = sum + 1;
            }
        }
        assert_eq(sum, 9);
    });
});

describe("Postfix Conditionals", fn() {
    test("postfix if executes when true", fn() {
        let result = 0;
        result = 42 if (true);
        assert_eq(result, 42);
    });

    test("postfix if skips when false", fn() {
        let result = 0;
        result = 42 if (false);
        assert_eq(result, 0);
    });

    test("postfix unless executes when false", fn() {
        let result = 0;
        result = 42 unless (false);
        assert_eq(result, 42);
    });

    test("postfix unless skips when true", fn() {
        let result = 0;
        result = 42 unless (true);
        assert_eq(result, 0);
    });
});
