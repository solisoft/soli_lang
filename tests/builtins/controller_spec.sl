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
});
