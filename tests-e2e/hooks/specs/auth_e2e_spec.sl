# End-to-end specs for authenticated controller testing and the
# view-introspection helpers: assigns(), assign(key), view_path(), and
# render_template().
#
# Auth here is the real session-login flow — POST /api/login sets the session
# server-side and returns Set-Cookie; the test client's cookie jar carries it
# to later requests automatically, so no DB and no manual cookie handling are
# needed. This is the recommended way to drive an authenticated request e2e.
#
# Run with:
#   soli test tests-e2e/hooks/specs/auth_e2e_spec.sl
# The runner boots a real `soli serve` subprocess against tests-e2e/hooks/
# (because app/controllers/ is present) and these specs hit it over HTTP.

describe("Auth + view introspection (e2e)", fn() {
    before_each(fn() {
        # Drop cookies/headers so each test starts as an unauthenticated guest.
        logout();
    });

    test("a guest is blocked from the protected dashboard", fn() {
        let res = get("/auth_demo/dashboard");
        assert_eq(res_status(res), 401);
        # A 401 renders no template, so render_template() is false.
        assert_not(render_template());
        assert_eq(view_path(), "");
    });

    test("after login, the dashboard renders and assigns() reflects its locals", fn() {
        # The Set-Cookie from this login lands in the client's cookie jar and
        # is sent on every later request in this test.
        let login = post("/api/login", {"user_id": 7});
        assert_eq(res_status(login), 200);

        let res = get("/auth_demo/dashboard");
        assert_eq(res_status(res), 200);

        # The view-introspection helpers reflect this rendered response.
        assert(render_template());
        assert_eq(view_path(), "auth_demo/dashboard.html");
        assert_eq(assign("title"), "Dashboard");
        assert_eq(assign("user_id"), 7);
        assert_hash_has_key(assigns(), "widgets");
    });

    test("auto-rendered actions populate assigns()/view_path() too", fn() {
        post("/api/login", {"user_id": 7});

        let res = get("/auth_demo/auto");
        assert_eq(res_status(res), 200);
        assert(render_template());
        assert_eq(view_path(), "auth_demo/auto.html");
        assert_eq(assign("title"), "Auto Dashboard");
        assert_eq(assign("user_id"), 7);
    });

    test("a JSON response reports no rendered template", fn() {
        post("/api/login", {"user_id": 7});

        let res = get("/api/me");
        assert_eq(res_status(res), 200);
        assert_eq(res_json(res)["user_id"], 7);
        # render_json() is not a template render.
        assert_not(render_template());
        assert_eq(view_path(), "");
    });

    test("the synthetic x-soli-test-* headers never leak into res_headers()", fn() {
        post("/api/login", {"user_id": 7});
        let res = get("/auth_demo/dashboard");

        let headers = res_headers(res);
        assert_not(headers.has_key("x-soli-test-view-path"));
        assert_not(headers.has_key("x-soli-test-assigns"));
    });
});
