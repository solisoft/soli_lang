// ============================================================================
// Error Handling (Try/Catch/Finally) Test Suite
// ============================================================================

describe("Try/Catch/Finally", fn() {
    test("try without error executes normally", fn() {
        let result = 0;
        try {
            result = 42;
        } catch (e) {
            result = -1;
        }
        assert_eq(result, 42);
    });

    test("catch handles thrown error", fn() {
        let result = "";
        try {
            throw "error message";
            result = "not reached";
        } catch (e) {
            result = "caught";
        }
        assert_eq(result, "caught");
    });

    test("finally always executes after try", fn() {
        let finally_ran = false;
        try {
            let x = 1;
        } catch (e) {
            let x = 2;
        } finally {
            finally_ran = true;
        }
        assert(finally_ran);
    });

    test("finally runs after catch", fn() {
        let sequence = [];
        try {
            throw "error";
        } catch (e) {
            sequence.push("catch");
        } finally {
            sequence.push("finally");
        }
        assert_eq(len(sequence), 2);
        assert_eq(sequence[0], "catch");
        assert_eq(sequence[1], "finally");
    });

    test("nested try/catch", fn() {
        let result = "";
        try {
            try {
                throw "inner";
            } catch (e) {
                result = "inner caught";
                throw "outer";
            }
        } catch (e) {
            result = result + " outer caught";
        }
        assert_eq(result, "inner caught outer caught");
    });

    test("try with return in try block", fn() {
        let finally_ran = false;
        fn test_fn() {
            try {
                return 42;
            } finally {
                finally_ran = true;
            }
        }
        let result = test_fn();
        assert_eq(result, 42);
        assert(finally_ran);
    });

    test("try with return in catch block", fn() {
        let finally_ran = false;
        fn test_fn() {
            try {
                throw "error";
            } catch (e) {
                return 100;
            } finally {
                finally_ran = true;
            }
        }
        let result = test_fn();
        assert_eq(result, 100);
        assert(finally_ran);
    });

    test("catch with different error types", fn() {
        let caught_type = "";
        try {
            throw 42;
        } catch (e) {
            caught_type = type(e);
        }
        assert_eq(caught_type, "int");

        let caught_string = "";
        try {
            throw "error";
        } catch (e) {
            caught_string = e;
        }
        assert_eq(caught_string, "error");
    });

    test("empty try block", fn() {
        let ran = false;
        try {
        } finally {
            ran = true;
        }
        assert(ran);
    });

    // ---- end syntax ----

    test("try/catch with end syntax", fn() {
        let result = ""
        try
            throw "boom"
        catch e
            result = "caught: " + e
        end
        assert_eq(result, "caught: boom")
    });

    test("try/catch/finally with end syntax", fn() {
        let order = []
        try
            throw "error"
        catch e
            order.push("catch")
        finally
            order.push("finally")
        end
        assert_eq(len(order), 2)
        assert_eq(order[0], "catch")
        assert_eq(order[1], "finally")
    });

    test("try/finally without catch using end syntax", fn() {
        let ran = false
        try
            let x = 1
        finally
            ran = true
        end
        assert(ran)
    });

    test("try without error using end syntax", fn() {
        let result = 0
        try
            result = 42
        catch e
            result = -1
        end
        assert_eq(result, 42)
    });

    test("nested try/catch with end syntax", fn() {
        let result = ""
        try
            try
                throw "inner"
            catch e
                result = "inner caught"
                throw "outer"
            end
        catch e
            result = result + " outer caught"
        end
        assert_eq(result, "inner caught outer caught")
    });

    // ---- typed catch ----

    test("typed catch matches specific class", fn() {
        class MyError {
            message: String;
            new(msg: String) {
                this.message = msg;
            }
        }

        let result = "";
        try {
            throw new MyError("oops");
        } catch (MyError e) {
            result = "caught: " + e.message;
        } catch (e) {
            result = "generic";
        }
        assert_eq(result, "caught: oops");
    });

    test("typed catch skips non-matching types", fn() {
        class ErrorA {
            message: String;
            new(msg: String) {
                this.message = msg;
            }
        }
        class ErrorB {
            message: String;
            new(msg: String) {
                this.message = msg;
            }
        }

        let result = "";
        try {
            throw new ErrorB("b");
        } catch (ErrorA e) {
            result = "A";
        } catch (ErrorB e) {
            result = "B: " + e.message;
        }
        assert_eq(result, "B: b");
    });

    test("typed catch matches subclass via inheritance", fn() {
        class BaseError {
            message: String;
            new(msg: String) {
                this.message = msg;
            }
        }
        class ChildError extends BaseError {
            new(msg: String) {
                super(msg);
            }
        }

        let result = "";
        try {
            throw new ChildError("child");
        } catch (BaseError e) {
            result = "base caught: " + e.message;
        }
        assert_eq(result, "base caught: child");
    });

    test("bare catch catches non-instance values", fn() {
        let result = "";
        try {
            throw "a string";
        } catch (e) {
            result = "bare: " + e;
        }
        assert_eq(result, "bare: a string");
    });

    test("typed catch does not match string throw", fn() {
        class MyError2 {}

        let result = "";
        try {
            throw "a string";
        } catch (MyError2 e) {
            result = "typed";
        } catch (e) {
            result = "bare: " + e;
        }
        assert_eq(result, "bare: a string");
    });

    test("no matching typed catch re-throws to outer", fn() {
        class ErrorC {}
        class ErrorD {}

        let result = "";
        try {
            try {
                throw new ErrorD();
            } catch (ErrorC e) {
                result = "C";
            }
        } catch (e) {
            result = "outer";
        }
        assert_eq(result, "outer");
    });

    test("typed catch with end syntax", fn() {
        class AppError
            message: String
            new(msg: String)
                this.message = msg
            end
        end

        let result = ""
        try
            throw new AppError("fail")
        catch AppError e
            result = "caught: " + e.message
        catch e
            result = "generic"
        end
        assert_eq(result, "caught: fail")
    });

    test("multiple typed catches with end syntax", fn() {
        class NotFound
            message: String
            new(msg: String)
                this.message = msg
            end
        end
        class Forbidden
            message: String
            new(msg: String)
                this.message = msg
            end
        end

        let result = ""
        try
            throw new Forbidden("no access")
        catch NotFound e
            result = "404"
        catch Forbidden e
            result = "403: " + e.message
        catch e
            result = "other"
        end
        assert_eq(result, "403: no access")
    });

    test("typed catch with finally", fn() {
        class CustomError {
            message: String;
            new(msg: String) {
                this.message = msg;
            }
        }

        let result = "";
        let finally_ran = false;
        try {
            throw new CustomError("test");
        } catch (CustomError e) {
            result = e.message;
        } finally {
            finally_ran = true;
        }
        assert_eq(result, "test");
        assert(finally_ran);
    });

    test("finally with nested try", fn() {
        let order = [];
        try {
            try {
                throw "inner";
            } finally {
                order.push("inner finally");
            }
        } catch (e) {
            order.push("outer catch");
        } finally {
            order.push("outer finally");
        }
        assert_eq(len(order), 3);
        assert_eq(order[0], "inner finally");
        assert_eq(order[1], "outer catch");
        assert_eq(order[2], "outer finally");
    });
});
