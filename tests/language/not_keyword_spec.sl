// ============================================================================
// Not Keyword and Bang Suffix Test Suite
// ============================================================================

describe("Not Keyword", fn() {
    test("not true equals false", fn() {
        assert_eq(not true, false);
    });

    test("not false equals true", fn() {
        assert_eq(not false, true);
    });

    test("not null equals true", fn() {
        assert_eq(not null, true);
    });

    test("not with variables", fn() {
        let flag = true;
        assert_eq(not flag, false);

        let flag2 = false;
        assert_eq(not flag2, true);
    });

    test("not with expressions", fn() {
        assert_eq(not (1 == 2), true);
        assert_eq(not (1 != 1), true);
        assert_eq(not (5 > 3), false);
    });

    test("double negation", fn() {
        assert_eq(not not true, true);
        assert_eq(not not false, false);
    });

    test("not with logical operators", fn() {
        assert(not (true && false));
        assert(not (false || false));
        assert(not (true && true) == false);
    });

    test("not with function calls", fn() {
        let empty = fn() { return []; };
        assert(not empty().empty?());
        assert(empty().empty?() == not not empty().empty?());
    });
});

describe("Bang Suffix for Methods", fn() {
    test("method names can end with bang", fn() {
        fn insert!() {
            return "insert called";
        }

        let result = insert!();
        assert_eq(result, "insert called");
    });

    test("bang method with parameters", fn() {
        fn delete!(id) {
            return "deleted " + id;
        }

        let result = delete!("123");
        assert_eq(result, "deleted 123");
    });

    test("bang methods in classes", fn() {
        class FileHelper {
            fn save!() {
                return "saved";
            }

            fn delete!() {
                return "deleted";
            }
        }

        let fh = new FileHelper();
        assert_eq(fh.save!(), "saved");
        assert_eq(fh.delete!(), "deleted");
    });

    test("multiple bang methods", fn() {
        fn fail!() { return "failed"; }
        fn insert!() { return "inserted"; }
        fn update!() { return "updated"; }
        fn delete!() { return "deleted"; }

        assert_eq(fail!(), "failed");
        assert_eq(insert!(), "inserted");
        assert_eq(update!(), "updated");
        assert_eq(delete!(), "deleted");
    });

    test("bang suffix with predicate", fn() {
        fn validate!() {
            return false;
        }

        assert_eq(validate!(), false);
    });
});

describe("Not Keyword vs Bang Operator Equivalence", fn() {
    test("not and ! produce same results", fn() {
        let values = [true, false, null, 1 == 1, 1 != 2];

        for v in values {
            assert_eq(not v, !v);
        }
    });

    test("not and ! have same precedence", fn() {
        let x = 5;
        let result1 = not x > 3;
        let result2 = !(x > 3);
        assert_eq(result1, result2);
    });
});

describe("Edge Cases", fn() {
    test("not with empty string", fn() {
        assert_eq(not "", true);
        assert_eq(not not "", false);
    });

    test("not with zero", fn() {
        assert_eq(not 0, true);
        assert_eq(not not 0, false);
    });

    test("not with empty array", fn() {
        assert_eq(not [], true);
        assert_eq(not not [], false);
    });

    test("not with empty hash", fn() {
        assert_eq(not {}, true);
        assert_eq(not not {}, false);
    });

    test("chained bang methods", fn() {
        fn step1!() { return 1; }
        fn step2!() { return 2; }

        let a = step1!();
        let b = step2!();
        assert_eq(a + b, 3);
    });
});
