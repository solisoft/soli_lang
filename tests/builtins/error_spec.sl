// ============================================================================
// Error Class Test Suite
// ============================================================================

describe("Error Subclasses Typed Catch", fn() {
    test("ValueError can be caught with typed catch", fn() {
        let result = "";
        try {
            throw new ValueError();
        } catch ValueError e {
            result = "caught ValueError";
        } catch e {
            result = "other";
        }
        assert_eq(result, "caught ValueError");
    });

    test("TypeError can be caught with typed catch", fn() {
        let result = "";
        try {
            throw new TypeError();
        } catch TypeError e {
            result = "caught TypeError";
        } catch e {
            result = "other";
        }
        assert_eq(result, "caught TypeError");
    });

    test("KeyError can be caught with typed catch", fn() {
        let result = "";
        try {
            throw new KeyError();
        } catch KeyError e {
            result = "caught KeyError";
        } catch e {
            result = "other";
        }
        assert_eq(result, "caught KeyError");
    });

    test("IndexError can be caught with typed catch", fn() {
        let result = "";
        try {
            throw new IndexError();
        } catch IndexError e {
            result = "caught IndexError";
        } catch e {
            result = "other";
        }
        assert_eq(result, "caught IndexError");
    });

    test("RuntimeError can be caught with typed catch", fn() {
        let result = "";
        try {
            throw new RuntimeError();
        } catch RuntimeError e {
            result = "caught RuntimeError";
        } catch e {
            result = "other";
        }
        assert_eq(result, "caught RuntimeError");
    });

    test("Error subclasses can be caught as Error", fn() {
        let result = "";
        try {
            throw new ValueError();
        } catch Error e {
            result = "caught Error";
        }
        assert_eq(result, "caught Error");
    });

    test("first matching typed catch is used", fn() {
        let result = "";
        try {
            throw new ValueError();
        } catch TypeError e {
            result = "TypeError";
        } catch ValueError e {
            result = "ValueError";
        } catch Error e {
            result = "Error";
        }
        assert_eq(result, "ValueError");
    });

    test("typed catch catches subclass via inheritance", fn() {
        class ChildValueError extends ValueError {}
        let result = "";
        try {
            throw new ChildValueError();
        } catch ValueError e {
            result = "caught ValueError";
        } catch Error e {
            result = "caught Error";
        }
        assert_eq(result, "caught ValueError");
    });

    test("non-matching typed catch falls through", fn() {
        let result = "";
        try {
            throw new ValueError();
        } catch TypeError e {
            result = "TypeError";
        } catch Error e {
            result = "caught Error";
        }
        assert_eq(result, "caught Error");
    });

    test("multiple different error types can be caught", fn() {
        let results = [];

        try {
            throw new ValueError();
        } catch ValueError e {
            results.push("ValueError");
        }

        try {
            throw new TypeError();
        } catch TypeError e {
            results.push("TypeError");
        }

        try {
            throw new KeyError();
        } catch KeyError e {
            results.push("KeyError");
        }

        try {
            throw new IndexError();
        } catch IndexError e {
            results.push("IndexError");
        }

        try {
            throw new RuntimeError();
        } catch RuntimeError e {
            results.push("RuntimeError");
        }

        assert_eq(len(results), 5);
        assert_eq(results[0], "ValueError");
        assert_eq(results[1], "TypeError");
        assert_eq(results[2], "KeyError");
        assert_eq(results[3], "IndexError");
        assert_eq(results[4], "RuntimeError");
    });
});

describe("Custom Error Classes", fn() {
    test("can create custom error with message field", fn() {
        class MyError {
            message: String;
            new(msg: String) {
                this.message = msg;
            }
        }

        let result = "";
        try {
            throw new MyError("custom error");
        } catch MyError e {
            result = e.message;
        }
        assert_eq(result, "custom error");
    });

    test("custom error inherits from Error for catch", fn() {
        class MyError extends Error {
            message: String;
            new(msg: String) {
                this.message = msg;
            }
        }

        let result = "";
        try {
            throw new MyError("test error");
        } catch Error e {
            result = "caught Error";
        } catch e {
            result = "other";
        }
        assert_eq(result, "caught Error");
    });

    test("custom error with multiple fields", fn() {
        class ValidationError {
            message: String;
            field: String;
            new(msg: String, fld: String) {
                this.message = msg;
                this.field = fld;
            }
        }

        let result = "";
        try {
            throw new ValidationError("invalid", "email");
        } catch ValidationError e {
            result = e.field + ": " + e.message;
        }
        assert_eq(result, "email: invalid");
    });
});