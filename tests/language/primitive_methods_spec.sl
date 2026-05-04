// ============================================================================
// Primitive method dispatch coverage
// ----------------------------------------------------------------------------
// Exercises Bool, Null, Int, Float, Decimal method dispatch — the with-args
// branch (`is_a?`, `round(n)`, `between?`, `clamp`, `gcd`, `lcm`). Zero-arg
// member access on primitives is already covered by other specs; this file
// targets the `call_<type>_method` paths in `interpreter/executor/calls/`.
// ============================================================================

describe("Bool method dispatch", fn() {
    test("is_a? returns true for bool and object", fn() {
        assert(true.is_a?("bool"));
        assert(false.is_a?("bool"));
        assert(true.is_a?("object"));
        assert(false.is_a?("object"));
    });

    test("is_a? returns false for unrelated types", fn() {
        assert_not(true.is_a?("int"));
        assert_not(false.is_a?("string"));
        assert_not(true.is_a?("float"));
    });

    test("is_a? with non-string arg throws", fn() {
        let caught = false;
        try {
            true.is_a?(42);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("is_a? with wrong arity throws", fn() {
        let caught = false;
        try {
            true.is_a?("bool", "extra");
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("unknown bool method throws", fn() {
        let caught = false;
        try {
            true.frobnicate(1);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });
});

describe("Null method dispatch", fn() {
    test("is_a? returns true for null and object", fn() {
        assert(null.is_a?("null"));
        assert(null.is_a?("object"));
    });

    test("is_a? returns false for unrelated types", fn() {
        assert_not(null.is_a?("int"));
        assert_not(null.is_a?("string"));
        assert_not(null.is_a?("bool"));
    });

    test("is_a? with non-string arg throws", fn() {
        let caught = false;
        try {
            null.is_a?(0);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("is_a? with wrong arity throws", fn() {
        let caught = false;
        try {
            null.is_a?();
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });
});

describe("Int method dispatch (with args)", fn() {
    test("gcd returns greatest common divisor", fn() {
        assert_eq((12).gcd(8), 4);
        assert_eq((17).gcd(5), 1);
        assert_eq((0).gcd(7), 7);
    });

    test("gcd handles negative inputs", fn() {
        assert_eq((-12).gcd(8), 4);
        assert_eq((12).gcd(-8), 4);
    });

    test("gcd with non-integer arg throws", fn() {
        let caught = false;
        try {
            (12).gcd("eight");
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("lcm returns least common multiple", fn() {
        assert_eq((4).lcm(6), 12);
        assert_eq((3).lcm(5), 15);
    });

    test("lcm of 0 and 0 is 0", fn() {
        assert_eq((0).lcm(0), 0);
    });

    test("lcm with non-integer arg throws", fn() {
        let caught = false;
        try {
            (4).lcm(6.0);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("between? returns true when in range", fn() {
        assert((5).between?(1, 10));
        assert((1).between?(1, 10));
        assert((10).between?(1, 10));
    });

    test("between? returns false when out of range", fn() {
        assert_not((0).between?(1, 10));
        assert_not((11).between?(1, 10));
    });

    test("between? accepts float bounds", fn() {
        assert((5).between?(1.5, 10.5));
        assert_not((1).between?(1.5, 10.5));
    });

    test("between? with non-numeric arg throws", fn() {
        let caught_min = false;
        try {
            (5).between?("low", 10);
        } catch (e) {
            caught_min = true;
        }
        assert(caught_min);

        let caught_max = false;
        try {
            (5).between?(1, "high");
        } catch (e) {
            caught_max = true;
        }
        assert(caught_max);
    });

    test("clamp restricts value to range", fn() {
        assert_eq((5).clamp(1, 10), 5);
        assert_eq((0).clamp(1, 10), 1);
        assert_eq((20).clamp(1, 10), 10);
    });

    test("clamp with non-integer arg throws", fn() {
        let caught_min = false;
        try {
            (5).clamp(1.0, 10);
        } catch (e) {
            caught_min = true;
        }
        assert(caught_min);

        let caught_max = false;
        try {
            (5).clamp(1, 10.0);
        } catch (e) {
            caught_max = true;
        }
        assert(caught_max);
    });

    test("is_a? returns true for int, numeric, object", fn() {
        assert((42).is_a?("int"));
        assert((42).is_a?("numeric"));
        assert((42).is_a?("object"));
    });

    test("is_a? returns false for unrelated types", fn() {
        assert_not((42).is_a?("string"));
        assert_not((42).is_a?("bool"));
    });

    test("pow with negative exponent returns float", fn() {
        let r = (2).pow(-2);
        assert_eq(r, 0.25);
    });

    test("pow with float exponent returns float", fn() {
        let r = (4).pow(0.5);
        assert_eq(r, 2.0);
    });

    test("pow with non-numeric arg throws", fn() {
        let caught = false;
        try {
            (2).pow("three");
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("times with non-function arg throws", fn() {
        let caught = false;
        try {
            (3).times(42);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("upto with non-integer limit throws", fn() {
        let caught = false;
        try {
            (1).upto("ten", fn(i) { i });
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("downto with non-function body throws", fn() {
        let caught = false;
        try {
            (3).downto(1, "noop");
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });
});

describe("Float method dispatch", fn() {
    test("round with no args returns int", fn() {
        assert_eq((3.4).round, 3);
        assert_eq((3.6).round, 4);
        assert_eq((-2.5).round, -3);
    });

    test("round with digits returns float", fn() {
        assert_eq((3.14159).round(2), 3.14);
        assert_eq((3.14159).round(4), 3.1416);
        assert_eq((1.5).round(0), 2.0);
    });

    test("round honors decimal value, not binary representation", fn() {
        # 38.995 stored as f64 is ~38.99499999999999744, so naive
        # `(n * 100).round() / 100` would yield 38.99. Soli rounds via
        # the shortest round-trip decimal so the answer is 39.0 — matching
        # Ruby 2.4+ and user intent.
        assert_eq((38.995).round(2), 39.0);
        assert_eq((2.675).round(2), 2.68);
        assert_eq((1.235).round(2), 1.24);
        assert_eq((38.985).round(2), 38.99);
        assert_eq((-38.995).round(2), -39.0);
    });

    test("round with negative digits rounds to nearest power of 10", fn() {
        assert_eq((12345.0).round(-2), 12300.0);
        assert_eq((12345.0).round(-3), 12000.0);
        assert_eq((1234.0).round(-4), 0.0);
    });

    test("round with non-int arg throws", fn() {
        let caught = false;
        try {
            (3.14).round("two");
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("round with too many args throws", fn() {
        let caught = false;
        try {
            (3.14).round(2, 3);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("between? returns true when in range", fn() {
        assert((3.14).between?(1.0, 5.0));
        assert((1.0).between?(1.0, 5.0));
    });

    test("between? returns false when out of range", fn() {
        assert_not((0.5).between?(1.0, 5.0));
        assert_not((6.0).between?(1.0, 5.0));
    });

    test("between? accepts integer bounds", fn() {
        assert((3.14).between?(1, 5));
    });

    test("between? with non-numeric arg throws", fn() {
        let caught = false;
        try {
            (3.14).between?("low", 5.0);
        } catch (e) {
            caught = true;
        }
        assert(caught);

        let caught2 = false;
        try {
            (3.14).between?(1.0, "high");
        } catch (e) {
            caught2 = true;
        }
        assert(caught2);
    });

    test("clamp restricts value", fn() {
        assert_eq((3.14).clamp(0.0, 2.0), 2.0);
        assert_eq((3.14).clamp(5.0, 10.0), 5.0);
        assert_eq((3.14).clamp(0.0, 5.0), 3.14);
    });

    test("clamp accepts integer bounds", fn() {
        assert_eq((3.14).clamp(0, 2), 2.0);
    });

    test("clamp with non-numeric arg throws", fn() {
        let caught = false;
        try {
            (3.14).clamp("zero", 10.0);
        } catch (e) {
            caught = true;
        }
        assert(caught);

        let caught2 = false;
        try {
            (3.14).clamp(0.0, "ten");
        } catch (e) {
            caught2 = true;
        }
        assert(caught2);
    });

    test("is_a? returns true for float, numeric, object", fn() {
        assert((3.14).is_a?("float"));
        assert((3.14).is_a?("numeric"));
        assert((3.14).is_a?("object"));
    });

    test("is_a? returns false for unrelated types", fn() {
        assert_not((3.14).is_a?("int"));
        assert_not((3.14).is_a?("string"));
    });

    test("is_a? with non-string arg throws", fn() {
        let caught = false;
        try {
            (3.14).is_a?(1);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("unknown float method throws", fn() {
        let caught = false;
        try {
            (3.14).frobnicate(1);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });
});

describe("Decimal method dispatch", fn() {
    test("round with no args returns int", fn() {
        let d = 3.7D;
        assert_eq(d.round, 4);
        let d2 = 3.4D;
        assert_eq(d2.round, 3);
    });

    test("round with digits returns decimal", fn() {
        let d = 3.14159D;
        let r = d.round(2);
        assert_eq(str(r), "3.14");
    });

    test("round with non-int arg throws", fn() {
        let caught = false;
        try {
            3.14D.round("two");
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("round with too many args throws", fn() {
        let caught = false;
        try {
            3.14D.round(2, 3);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("between? returns true when in range", fn() {
        let d = 3.14D;
        assert(d.between?(1, 5));
        assert(d.between?(1.0, 5.0));
        assert(d.between?(1.00D, 5.00D));
    });

    test("between? returns false when out of range", fn() {
        let d = 10.00D;
        assert_not(d.between?(0, 5));
    });

    test("between? with non-numeric arg throws", fn() {
        let caught = false;
        try {
            3.14D.between?("low", 5);
        } catch (e) {
            caught = true;
        }
        assert(caught);

        let caught2 = false;
        try {
            3.14D.between?(1, "high");
        } catch (e) {
            caught2 = true;
        }
        assert(caught2);
    });

    test("clamp restricts decimal value", fn() {
        let d = 3.14D;
        let clamped = d.clamp(0.00D, 2.00D);
        assert_eq(str(clamped), "2.00");
    });

    test("clamp accepts integer bounds", fn() {
        let d = 3.14D;
        let clamped = d.clamp(0, 2);
        assert_eq(str(clamped), "2");
    });

    test("clamp with non-numeric arg throws", fn() {
        let caught = false;
        try {
            3.14D.clamp("zero", 10.00D);
        } catch (e) {
            caught = true;
        }
        assert(caught);

        let caught2 = false;
        try {
            3.14D.clamp(0.00D, "ten");
        } catch (e) {
            caught2 = true;
        }
        assert(caught2);
    });

    test("is_a? returns true for decimal, numeric, object", fn() {
        let d = 3.14D;
        assert(d.is_a?("decimal"));
        assert(d.is_a?("numeric"));
        assert(d.is_a?("object"));
    });

    test("is_a? returns false for unrelated types", fn() {
        let d = 3.14D;
        assert_not(d.is_a?("float"));
        assert_not(d.is_a?("int"));
    });

    test("is_a? with non-string arg throws", fn() {
        let caught = false;
        try {
            3.14D.is_a?(1);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("unknown decimal method throws", fn() {
        let caught = false;
        try {
            3.14D.frobnicate(1);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });
});
