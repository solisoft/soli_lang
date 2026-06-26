# E2E fixture: authenticated controller testing + the view-introspection
# helpers (assigns / view_path / render_template).
#
# The dashboard is session-protected. Signing in is the real `POST /api/login`
# flow (see api_test#login) whose `Set-Cookie` the test client's cookie jar
# carries forward automatically — no DB required.
class AuthDemoController extends Controller
    # Guests get 401; signed-in users get an explicitly-rendered dashboard
    # whose locals the test introspects via assigns() / view_path() /
    # render_template().
    fn dashboard(req)
        let user_id = session_get("user_id");
        if user_id.nil?
            return halt(401, "Sign in first");
        end

        render("auth_demo/dashboard", {
            "title": "Dashboard",
            "user_id": user_id,
            "widgets": ["inbox", "tasks", "calendar"]
        })
    end

    # Same idea, but *auto-rendered*: the action sets `@vars` and lets the
    # matching `auth_demo/auto` view render by convention (no explicit
    # render()). Proves assigns()/view_path() work on the auto-render path too.
    fn auto(req)
        let user_id = session_get("user_id");
        if user_id.nil?
            return halt(401, "Sign in first");
        end

        @title = "Auto Dashboard";
        @user_id = user_id;
        @widgets = ["inbox", "tasks"];
    end
end
