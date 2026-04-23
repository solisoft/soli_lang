# Parent controller for the inheritance e2e test. Its before_action sets
# `@from_parent`; HooksTestController extends this so the child's hook runs
# AFTER the parent's, and the view should see both `current_user` (child) and
# `from_parent` (parent).

class ParentController extends Controller
    static {
        this.before_action = fn(req) {
            @from_parent = "parent_hook_ran";
            req
        }
    }
end
