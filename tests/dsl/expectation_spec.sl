// ============================================================================
// Expectation Class Test Suite
// ============================================================================

describe("expect() Function", fn() {
    test("expect creates an Expectation object", fn() {
        let exp = expect(42);
        assert(exp != null);
    });

    test("expect stores the actual value", fn() {
        let exp = expect(42);
        assert(exp.actual == 42);
    });

    test("expect works with different types", fn() {
        assert(expect("hello").actual == "hello");
        assert(expect(true).actual == true);
        assert(expect(null).actual == null);
    });
});

describe("Expectation.to_be", fn() {
    test("to_be passes when values match", fn() {
        expect(42).to_be(42);
    });

    test("to_be fails when values do not match", fn() {
        let passed = false;
        try {
            expect(42).to_be(100);
        } catch e {
            passed = true;
        }
        assert(passed);
    });

    test("to_be works with strings", fn() {
        expect("hello").to_be("hello");
    });

    test("to_be works with booleans", fn() {
        expect(true).to_be(true);
        expect(false).to_be(false);
    });
});

describe("Expectation.to_equal", fn() {
    test("to_equal passes when values match", fn() {
        expect(42).to_equal(42);
    });

    test("to_equal fails when values do not match", fn() {
        let passed = false;
        try {
            expect(42).to_equal(100);
        } catch e {
            passed = true;
        }
        assert(passed);
    });

    test("to_equal works with arrays", fn() {
        expect([1, 2, 3]).to_equal([1, 2, 3]);
    });

    test("to_equal works with hashes", fn() {
        expect({"a": 1, "b": 2}).to_equal({"a": 1, "b": 2});
    });
});

describe("Expectation.to_not_be", fn() {
    test("to_not_be passes when values differ", fn() {
        expect(42).to_not_be(100);
    });

    test("to_not_be fails when values match", fn() {
        let passed = false;
        try {
            expect(42).to_not_be(42);
        } catch e {
            passed = true;
        }
        assert(passed);
    });
});

describe("Expectation.to_not_equal", fn() {
    test("to_not_equal passes when values differ", fn() {
        expect(42).to_not_equal(100);
    });

    test("to_not_equal fails when values match", fn() {
        let passed = false;
        try {
            expect(42).to_not_equal(42);
        } catch e {
            passed = true;
        }
        assert(passed);
    });
});

describe("Expectation.to_be_null", fn() {
    test("to_be_null passes for null", fn() {
        expect(null).to_be_null();
    });

    test("to_be_null fails for non-null", fn() {
        let passed = false;
        try {
            expect(42).to_be_null();
        } catch e {
            passed = true;
        }
        assert(passed);
    });
});

describe("Expectation.to_not_be_null", fn() {
    test("to_not_be_null passes for non-null", fn() {
        expect(42).to_not_be_null();
        expect("hello").to_not_be_null();
    });

    test("to_not_be_null fails for null", fn() {
        let passed = false;
        try {
            expect(null).to_not_be_null();
        } catch e {
            passed = true;
        }
        assert(passed);
    });
});

describe("Expectation.to_be_greater_than", fn() {
    test("to_be_greater_than passes when actual > expected", fn() {
        expect(100).to_be_greater_than(50);
    });

    test("to_be_greater_than fails when actual <= expected", fn() {
        let passed = false;
        try {
            expect(50).to_be_greater_than(100);
        } catch e {
            passed = true;
        }
        assert(passed);
    });

    test("to_be_greater_than fails when equal", fn() {
        let passed = false;
        try {
            expect(50).to_be_greater_than(50);
        } catch e {
            passed = true;
        }
        assert(passed);
    });

    test("to_be_greater_than works with floats", fn() {
        expect(10.5).to_be_greater_than(10.0);
    });

    test("to_be_greater_than works with int and float", fn() {
        expect(10).to_be_greater_than(9.5);
    });
});

describe("Expectation.to_be_less_than", fn() {
    test("to_be_less_than passes when actual < expected", fn() {
        expect(50).to_be_less_than(100);
    });

    test("to_be_less_than fails when actual >= expected", fn() {
        let passed = false;
        try {
            expect(100).to_be_less_than(50);
        } catch e {
            passed = true;
        }
        assert(passed);
    });

    test("to_be_less_than works with floats", fn() {
        expect(10.0).to_be_less_than(10.5);
    });
});

describe("Expectation.to_be_greater_than_or_equal", fn() {
    test("passes when actual > expected", fn() {
        expect(100).to_be_greater_than_or_equal(50);
    });

    test("passes when actual == expected", fn() {
        expect(50).to_be_greater_than_or_equal(50);
    });

    test("fails when actual < expected", fn() {
        let passed = false;
        try {
            expect(50).to_be_greater_than_or_equal(100);
        } catch e {
            passed = true;
        }
        assert(passed);
    });
});

describe("Expectation.to_be_less_than_or_equal", fn() {
    test("passes when actual < expected", fn() {
        expect(50).to_be_less_than_or_equal(100);
    });

    test("passes when actual == expected", fn() {
        expect(50).to_be_less_than_or_equal(50);
    });

    test("fails when actual > expected", fn() {
        let passed = false;
        try {
            expect(100).to_be_less_than_or_equal(50);
        } catch e {
            passed = true;
        }
        assert(passed);
    });
});

describe("Expectation.to_contain", fn() {
    test("to_contain passes when array contains item", fn() {
        expect([1, 2, 3]).to_contain(2);
    });

    test("to_contain fails when array does not contain item", fn() {
        let passed = false;
        try {
            expect([1, 2, 3]).to_contain(5);
        } catch e {
            passed = true;
        }
        assert(passed);
    });

    test("to_contain passes when string contains substring", fn() {
        expect("hello world").to_contain("world");
    });

    test("to_contain fails when string does not contain substring", fn() {
        let passed = false;
        try {
            expect("hello world").to_contain("goodbye");
        } catch e {
            passed = true;
        }
        assert(passed);
    });
});

describe("Expectation.to_be_valid_json", fn() {
    test("to_be_valid_json passes for valid JSON string", fn() {
        expect('{"name": "test"}').to_be_valid_json();
        expect('[1, 2, 3]').to_be_valid_json();
        expect('"string"').to_be_valid_json();
    });

    test("to_be_valid_json fails for invalid JSON", fn() {
        let passed = false;
        try {
            expect("not json").to_be_valid_json();
        } catch e {
            passed = true;
        }
        assert(passed);
    });

    test("to_be_valid_json fails for non-string", fn() {
        let passed = false;
        try {
            expect(123).to_be_valid_json();
        } catch e {
            passed = true;
        }
        assert(passed);
    });
});

describe("Expectation chaining", fn() {
    test("multiple expectations can be chained", fn() {
        expect(10).to_be_greater_than(5);
        expect(10).to_be_less_than(20);
        expect(10).to_not_be(null);
    });
});