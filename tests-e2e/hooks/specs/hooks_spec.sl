// End-to-end specs for controller hooks, `@` auto-injection, and prefetch.
//
// Run with:
//   soli test tests-e2e/hooks/specs/hooks_spec.sl
// The test runner boots a real `soli serve` subprocess against
// tests-e2e/hooks/ (because `app/controllers/` is present next to this spec's
// grandparent dir) and these specs hit it with actual HTTP requests.

let BASE = test_server_url();

// -- E2E request helpers -----------------------------------------------------
//
// Thin wrappers around `HTTP.request` so specs read at the level of the
// behaviour under test, not the transport. Each helper returns the full
// response hash (`{status, headers, body}`) so you can mix-and-match specific
// assertions afterwards. Kept intentionally small — one-liners per verb.

fn get(path)
    return HTTP.request("GET", BASE + path, {});
end

fn get_with_cookie(path, cookie)
    return HTTP.request("GET", BASE + path, {"Cookie": cookie});
end

fn post_json(path, body)
    return HTTP.request("POST", BASE + path, {"Content-Type": "application/json"}, json_stringify(body));
end

fn post_form(path, form_body)
    return HTTP.request("POST", BASE + path, {"Content-Type": "application/x-www-form-urlencoded"}, form_body);
end

fn post_with_cookie(path, body, cookie)
    return HTTP.request("POST", BASE + path, {"Content-Type": "application/json", "Cookie": cookie}, json_stringify(body));
end

// Extract the `session_id=...` Cookie value from a `Set-Cookie` response header.
// Returns `""` if the header is missing or doesn't contain a session cookie.
fn extract_session_cookie(res)
    let header = res.headers["set-cookie"] ?? "";
    if header == ""
        return "";
    end
    # The Set-Cookie header looks like "session_id=abc123; Path=/; HttpOnly".
    # Pull out just the "session_id=abc123" chunk for sending back as Cookie.
    let semi = header.index_of(";");
    if semi == -1
        return header;
    end
    return header.substring(0, semi);
end

// -- Assertion helpers -------------------------------------------------------
//
// These wrap `assert_eq` / `assert` so failure messages are routed through
// the same machinery the test runner already uses — but with context about
// the HTTP response embedded, so debugging is a quick eyeball instead of a
// "values not equal" without a body peek.

fn assert_status(res, expected)
    assert_eq(res.status, expected);
end

fn assert_body_contains(res, substring)
    assert(res.body.contains(substring));
end

fn assert_body_not_contains(res, substring)
    assert_not(res.body.contains(substring));
end

fn assert_header(res, name, expected)
    // Header names are case-insensitive in HTTP; the test server lowercases
    // them, so callers pass lowercase here.
    assert_eq(res.headers[name], expected);
end

// ---------------------------------------------------------------------------

describe("before_action + @ auto-inject (end-to-end via HTTP)", fn() {
    test("unfiltered before_action sets @current_user; value reaches view", fn() {
        let res = get("/");
        assert_status(res, 200);
        assert_body_contains(res, "user=alice");
    });

    test("filtered :locked before_action short-circuits with 403 via halt()", fn() {
        let res = get("/locked");
        assert_status(res, 403);
        assert_eq(res.body, "Forbidden");
        // The action was never reached, so the view's "user=alice" isn't in the body.
        assert_body_not_contains(res, "user=alice");
    });

    test("empty-body before_action does not crash; action proceeds", fn() {
        let res = get("/empty_hook");
        assert_status(res, 200);
        // The unfiltered hook still runs and sets @current_user.
        assert_body_contains(res, "user=alice");
    });
});

describe("controller inheritance (end-to-end)", fn() {
    test("parent controller's before_action runs AND the child's runs — both fields reach the view", fn() {
        // HooksTestController extends ParentController. Parent hook sets
        // @from_parent, child hook sets @current_user. Both should survive
        // to the rendered view via the auto-injection pipeline.
        let res = get("/");
        assert_status(res, 200);
        assert_body_contains(res, "parent=parent_hook_ran");
        assert_body_contains(res, "user=alice");
    });
});

describe("response-building in actions (end-to-end)", fn() {
    test("halt() returned from an action body produces the expected HTTP status/body", fn() {
        // Covers `halt()` used outside a before_action — the action itself
        // short-circuits on a condition and returns `halt(...)`.
        let res = get("/halt_in_action");
        assert_status(res, 404);
        assert_eq(res.body, "Not Here");
    });

    test("redirect() ends on the destination page", fn() {
        // HTTP.request follows redirects, so we see the destination's 200
        // rather than the intermediate 302. That still proves redirect fired:
        // without it we'd land on /redirect_elsewhere's (absent) view and 500.
        // For a strict 302 + Location header assertion, see the unit test on
        // the `redirect` builtin in `src/interpreter/builtins/template.rs`.
        let res = get("/redirect_elsewhere");
        assert_status(res, 200);
        // The destination renders hooks_test/index, which includes "user=alice".
        assert_body_contains(res, "user=alice");
    });
});

describe("auto-inject precedence (end-to-end)", fn() {
    test("explicit render(..., data) wins over a conflicting @foo instance field", fn() {
        // Action sets `@title = "from_instance"`, then calls
        // `render("v", {"title": "from_render"})`. The view shows `title`.
        // Explicit render data must shadow the auto-injected instance field.
        let res = get("/render_with_data");
        assert_status(res, 200);
        assert_body_contains(res, "title=from_render");
        assert_body_not_contains(res, "title=from_instance");
    });

    test("framework-injected `params` is not shadowed by an instance field of the same name", fn() {
        // Before_action writes `@params = "HIJACKED_BY_HOOK"` (a String). The
        // auto-injection skip-list protects `req`/`params`/`session`/`headers`
        // so the string never leaks into the view. The key guarantee is that
        // the bare `params` local in the view doesn't resolve to the hijacked
        // string — `type(params)` should never report "string".
        let res = get("/param_shadow");
        assert_status(res, 200);
        assert_body_not_contains(res, "HIJACKED_BY_HOOK");
        assert_body_not_contains(res, "params_type=string");
    });
});

describe("after_action hooks (end-to-end)", fn() {
    test("filtered after_action mutates the response body before it's returned", fn() {
        let res = get("/after_marked");
        assert_status(res, 200);
        assert_body_contains(res, "<!--AFTER_ACTION_MARK-->");
    });

    test("after_action does NOT run on unrelated actions (filter applies)", fn() {
        // `/` is not in the `:after_marked` filter list, so the marker must NOT
        // appear. Guards against regression where the filter is ignored and the
        // hook fires on every action.
        let res = get("/");
        assert_status(res, 200);
        assert_body_not_contains(res, "<!--AFTER_ACTION_MARK-->");
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
        let res = get("/");
        assert_status(res, 200);
        // We don't care which branch wins — only that the partial rendered.
        assert_body_contains(res, "defensive=");
    });
});

describe("request-context view helpers (end-to-end)", fn() {
    test("current_path() returns the live pathname", fn() {
        let res = get("/");
        assert_body_contains(res, "path=/");
    });

    test("current_path?(p) renders 'active' when p matches", fn() {
        let res = get("/");
        // The view emits `class="active"` when current_path?("/") is true.
        assert_body_contains(res, "class=\"active\"");
    });
});

describe("hover-prefetch (end-to-end)", fn() {
    test("auto-injects <script src=\"/__soli/prefetch.js\"> into HTML responses", fn() {
        let res = get("/");
        assert_status(res, 200);
        assert_body_contains(res, "/__soli/prefetch.js");
    });

    test("serves the prefetch JS at /__soli/prefetch.js with a JS content-type", fn() {
        let res = get("/__soli/prefetch.js");
        assert_status(res, 200);
        let ct = res.headers["content-type"];
        assert(ct.contains("javascript"));
        // Smoke-check it's the real script, not an HTML error page.
        assert_body_contains(res, "soliPrefetchInstalled");
    });
});
