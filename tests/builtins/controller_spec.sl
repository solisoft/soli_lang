// ============================================================================
// Controller Test Suite
// ============================================================================

describe("Controller Base Class", fn() {
    test("controller can be extended", fn() {
        class TestController extends Controller {
            static {
                this.layout = "application";
            }
        }
        assert(true);
    });

    test("before_action can be set with function", fn() {
        class TestController extends Controller {
            static {
                this.before_action = fn(req) {
                    return req;
                };
            }
        }
        assert(true);
    });

    test("after_action can be set with function", fn() {
        class TestController extends Controller {
            static {
                this.after_action = fn(req) {
                    return req;
                };
            }
        }
        assert(true);
    });

    test("layout can be set via static block", fn() {
        class TestController extends Controller {
            static {
                this.layout = "custom_layout";
            }
        }
        assert(true);
    });

    test("child controller inherits from parent", fn() {
        class ParentController extends Controller {
            static {
                this.layout = "parent";
            }
        }
        class ChildController extends ParentController {
        }
        assert(true);
    });

    test("halt(status, message) builds a response hash the short-circuit logic accepts", fn() {
        // Hooks use `return halt(403, "Forbidden")` to abort a request.
        // The return value must be a hash with a `status` field so
        // `check_for_response` in serve/mod.rs treats it as a response, not as
        // a modified request hash. Regression guard against the builtin going
        // missing (it was undefined for a while, causing hooks to silently
        // no-op and let requests through). Named `halt` rather than `error`
        // because `error` collides with the common local name in form partials.
        let r = halt(403, "Forbidden");
        assert_eq(r["status"], 403);
        assert_eq(r["body"], "Forbidden");
        assert_eq(r["headers"]["Content-Type"], "text/plain; charset=utf-8");

        let r2 = halt(422, "Bad payload");
        assert_eq(r2["status"], 422);
        assert_eq(r2["body"], "Bad payload");
    });

    test("filtered before_action DSL parses at the language level", fn() {
        // `this.before_action(:show) = fn(req) {...}` must actually parse — the
        // parser desugars call-assignment to `this.before_action(:show, fn(req) {...})`
        // and Controller provides a no-op static method so the call resolves.
        // Real hook registration happens via the registry's textual scanner at
        // `soli serve` startup; this test just proves the DSL doesn't raise.
        class FilteredController extends Controller {
            static {
                this.before_action(:show, :edit) = fn(req) {
                    return req;
                };
                this.after_action(:create) = fn(req, response) {
                    return response;
                };
            }
        }
        assert(true);
    });
});
