# End-to-end fixture for before_action + `@` sigil + empty hook + prefetch +
# inheritance + halt-in-action + render-precedence + redirect + after_action
# + framework-field shadowing.

class HooksTestController extends ParentController
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

        # Action-filtered: tries to shadow the framework-injected `params` field.
        # The view should still see the real params hash, not the string.
        this.before_action(:param_shadow) = fn(req) {
            @params = "HIJACKED_BY_HOOK";
            req
        }

        # Action-filtered after_action: appends a marker to the response body
        # so the HTTP caller can see the after_action ran.
        this.after_action(:after_marked) = fn(req, response) {
            response["body"] = response["body"] + "<!--AFTER_ACTION_MARK-->";
            response
        }

        # Action-filtered empty body: must not crash, request proceeds.
        # Kept last in the static block — the parser doesn't accept an empty
        # `fn(x) { }` body followed by another statement in the same block.
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

    # Returns a halt() response from an action body (not a before_action hook).
    fn halt_in_action(req)
        return halt(404, "Not Here")
    end

    # Sets @title on the instance, then passes an explicit data hash to render().
    # Explicit render data must win over the @-auto-injected field.
    fn render_with_data(req)
        @title = "from_instance";
        render("hooks_test/render_with_data", {"title": "from_render"})
    end

    # Redirect to /after_redirect.
    fn redirect_elsewhere(req)
        redirect("/after_redirect")
    end

    # Destination for the redirect test.
    fn after_redirect(req)
        render("hooks_test/index")
    end

    # Action exercising after_action filter — the marker appears in the body.
    fn after_marked(req)
        render("hooks_test/index")
    end

    # Used by the param-shadowing test. The view asserts `params` is still a
    # hash (not the string the before_action wrote to `@params`).
    fn param_shadow(req)
        render("hooks_test/param_shadow")
    end

    # Regression: a nested `render("partial", {"class": "x"})` inside an ERB
    # tag used to fail at parse time because the template router fed any
    # `render(...)` call through a Rails-style DSL parser that choked on the
    # `"class"` hash key. Now paren-form render() goes through the normal
    # expression parser. The wrapper view itself does the render().
    fn render_with_hash_arg(req)
        render("hooks_test/render_with_hash_arg")
    end
end
