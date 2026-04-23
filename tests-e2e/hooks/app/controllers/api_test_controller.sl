# API/JSON/session/error fixture for e2e coverage tests.

class ApiTestController extends Controller
    # JSON echo: POST body → parsed → same keys returned as JSON.
    fn echo_json(req)
        let body = req["json"];
        render_json(body)
    end

    # Static JSON response with a known shape.
    fn thing(req)
        render_json({
            "id": 42,
            "name": "answer",
            "tags": ["a", "b", "c"]
        })
    end

    # Form echo: URL-encoded body → params → echoed as JSON.
    fn form_echo(req)
        let name = req["form"]["name"] ?? "";
        let email = req["form"]["email"] ?? "";
        render_json({"name": name, "email": email})
    end

    # Session: store user_id, respond with the generated session cookie.
    fn login(req)
        let user_id = req["json"]["user_id"];
        session_set("user_id", user_id);
        render_json({"ok": true, "session": session_id()})
    end

    # Session: read back who we are.
    fn me(req)
        let uid = session_get("user_id");
        if uid.nil?
            return halt(401, "Not logged in");
        end
        render_json({"user_id": uid})
    end

    # Session: wipe the session.
    fn logout(req)
        session_destroy();
        render_json({"ok": true})
    end

    # Deliberately throws to exercise the 500 error path.
    fn boom(req)
        let x = null;
        # Accessing a property on null raises at runtime — perfect test driver.
        return x.definitely_not_a_method;
    end

    # Echoes back the field a global middleware stamped on the request, so the
    # e2e test can verify the middleware actually ran.
    fn echo_middleware_stamp(req)
        render_json({"stamp": req["middleware_stamp"] ?? "missing"})
    end
end
