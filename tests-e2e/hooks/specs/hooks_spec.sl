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

    test("filtered :locked before_action short-circuits with 403 via error()", fn() {
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
