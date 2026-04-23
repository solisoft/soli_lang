# End-to-end fixture for before_action + `@` sigil + empty hook + prefetch.

class HooksTestController < Controller
    static {
        # Unfiltered: runs on every action. Setting `@current_user` here should
        # reach the view as a bare `current_user` local via auto-injection.
        this.before_action = fn(req) {
            @current_user = "alice";
            req
        }

        # Action-filtered: `:locked` short-circuits with 403.
        this.before_action(:locked) = fn(req) {
            return halt(403, "Forbidden");
        }

        # Action-filtered empty body: must not crash, request proceeds.
        this.before_action(:empty_hook) = fn(req) {

        }
    }

    fn index(req)
        render("hooks_test/index")
    end

    fn locked(req)
        render("hooks_test/index")    # never reached — 403 short-circuits above
    end

    fn empty_hook(req)
        render("hooks_test/index")
    end
end
