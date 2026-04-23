// End-to-end specs for controller hooks, `@` auto-injection, and prefetch.
//
// Run with:
//   soli test tests-e2e/hooks/specs/hooks_spec.sl
// The test runner boots a real `soli serve` subprocess against
// tests-e2e/hooks/ (because `app/controllers/` is present next to this spec's
// grandparent dir) and these specs hit it with actual HTTP requests.

let BASE = test_server_url();

describe("before_action + @ auto-inject (end-to-end via HTTP)", fn() {
    test("unfiltered before_action sets @current_user; value reaches view", fn() {
        let res = HTTP.request("GET", BASE + "/", {});
        assert_eq(res.status, 200);
        assert(res.body.contains("user=alice"));
    });

    test("filtered :locked before_action short-circuits with 403 via halt()", fn() {
        let res = HTTP.request("GET", BASE + "/locked", {});
        assert_eq(res.status, 403);
        assert_eq(res.body, "Forbidden");
        // The action was never reached, so the view's "user=alice" isn't in the body.
        assert_not(res.body.contains("user=alice"));
    });

    test("empty-body before_action does not crash; action proceeds", fn() {
        let res = HTTP.request("GET", BASE + "/empty_hook", {});
        assert_eq(res.status, 200);
        // The unfiltered hook still runs and sets @current_user.
        assert(res.body.contains("user=alice"));
    });
});

describe("controller inheritance (end-to-end)", fn() {
    test("parent controller's before_action runs AND the child's runs — both fields reach the view", fn() {
        // HooksTestController extends ParentController. Parent hook sets
        // @from_parent, child hook sets @current_user. Both should survive
        // to the rendered view via the auto-injection pipeline.
        let res = HTTP.request("GET", BASE + "/", {});
        assert_eq(res.status, 200);
        assert(res.body.contains("parent=parent_hook_ran"));
        assert(res.body.contains("user=alice"));
    });
});

describe("response-building in actions (end-to-end)", fn() {
    test("halt() returned from an action body produces the expected HTTP status/body", fn() {
        // Covers `halt()` used outside a before_action — the action itself
        // short-circuits on a condition and returns `halt(...)`.
        let res = HTTP.request("GET", BASE + "/halt_in_action", {});
        assert_eq(res.status, 404);
        assert_eq(res.body, "Not Here");
    });

    test("redirect() ends on the destination page", fn() {
        // HTTP.request follows redirects, so we see the destination's 200
        // rather than the intermediate 302. That still proves redirect fired:
        // without it we'd land on /redirect_elsewhere's (absent) view and 500.
        // For a strict 302 + Location header assertion, see the unit test on
        // the `redirect` builtin in `src/interpreter/builtins/template.rs`.
        let res = HTTP.request("GET", BASE + "/redirect_elsewhere", {});
        assert_eq(res.status, 200);
        // The destination renders hooks_test/index, which includes "user=alice".
        assert(res.body.contains("user=alice"));
    });
});

describe("auto-inject precedence (end-to-end)", fn() {
    test("explicit render(..., data) wins over a conflicting @foo instance field", fn() {
        // Action sets `@title = "from_instance"`, then calls
        // `render("v", {"title": "from_render"})`. The view shows `title`.
        // Explicit render data must shadow the auto-injected instance field.
        let res = HTTP.request("GET", BASE + "/render_with_data", {});
        assert_eq(res.status, 200);
        assert(res.body.contains("title=from_render"));
        assert_not(res.body.contains("title=from_instance"));
    });

    test("framework-injected `params` is not shadowed by an instance field of the same name", fn() {
        // Before_action writes `@params = "HIJACKED_BY_HOOK"` (a String). The
        // auto-injection skip-list protects `req`/`params`/`session`/`headers`
        // so the string never leaks into the view. The key guarantee is that
        // the bare `params` local in the view doesn't resolve to the hijacked
        // string — `type(params)` should never report "string".
        let res = HTTP.request("GET", BASE + "/param_shadow", {});
        assert_eq(res.status, 200);
        assert_not(res.body.contains("HIJACKED_BY_HOOK"));
        assert_not(res.body.contains("params_type=string"));
    });
});

describe("after_action hooks (end-to-end)", fn() {
    test("filtered after_action mutates the response body before it's returned", fn() {
        let res = HTTP.request("GET", BASE + "/after_marked", {});
        assert_eq(res.status, 200);
        assert(res.body.contains("<!--AFTER_ACTION_MARK-->"));
    });

    test("after_action does NOT run on unrelated actions (filter applies)", fn() {
        // `/` is not in the `:after_marked` filter list, so the marker must NOT
        // appear. Guards against regression where the filter is ignored and the
        // hook fires on every action.
        let res = HTTP.request("GET", BASE + "/", {});
        assert_eq(res.status, 200);
        assert_not(res.body.contains("<!--AFTER_ACTION_MARK-->"));
    });
});

describe("defensive `defined(x) && !x.nil?` pattern in partials", fn() {
    test("partial invoking `!halt.nil?` does not crash even though `halt` is a framework Function", fn() {
        // Regression for the `.nil?`-on-Function bug this session: a form
        // partial used `defined("X") && !X.nil?` to guard an optional local,
        // but when `X` happened to collide with a global Function builtin the
        // `.nil?` call blew up. Now `.nil?` returns false on Function values,
        // so the ternary completes cleanly. The shared/_defensive partial
        // renders whichever branch the ternary picks as a plain string.
        let res = HTTP.request("GET", BASE + "/", {});
        assert_eq(res.status, 200);
        // We don't care which branch wins — only that the partial rendered.
        assert(res.body.contains("defensive="));
    });
});

describe("request-context view helpers (end-to-end)", fn() {
    test("current_path() returns the live pathname", fn() {
        let res = HTTP.request("GET", BASE + "/", {});
        assert(res.body.contains("path=/"));
    });

    test("current_path?(p) renders 'active' when p matches", fn() {
        let res = HTTP.request("GET", BASE + "/", {});
        // The view emits `class="active"` when current_path?("/") is true.
        assert(res.body.contains("class=\"active\""));
    });
});

describe("hover-prefetch (end-to-end)", fn() {
    test("auto-injects <script src=\"/__soli/prefetch.js\"> into HTML responses", fn() {
        let res = HTTP.request("GET", BASE + "/", {});
        assert_eq(res.status, 200);
        assert(res.body.contains("/__soli/prefetch.js"));
    });

    test("serves the prefetch JS at /__soli/prefetch.js with a JS content-type", fn() {
        let res = HTTP.request("GET", BASE + "/__soli/prefetch.js", {});
        assert_eq(res.status, 200);
        let ct = res.headers["content-type"];
        assert(ct.contains("javascript"));
        // Smoke-check it's the real script, not an HTML error page.
        assert(res.body.contains("soliPrefetchInstalled"));
    });
});
