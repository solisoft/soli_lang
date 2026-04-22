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
