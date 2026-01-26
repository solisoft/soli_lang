// ============================================================================
// Router Functions Test Suite
// ============================================================================
// Tests for router/URL routing functions
// ============================================================================

describe("Router Basic Functions", fn() {
    test("router_match() registers custom route", fn() {
        let result = router_match("GET", "/api/users", "users#show");
        assert(result);
    });

    test("router_match() with action", fn() {
        let result = router_match("POST", "/api/items", "items#create");
        assert(result);
    });

    test("router_match() returns route info", fn() {
        let route = router_match("PUT", "/api/update", "controller#action");
        assert_not_null(route);
    });
});

describe("Router Resource Functions", fn() {
    test("router_resource_enter() starts resource", fn() {
        let result = router_resource_enter("users");
        assert(result);
    });

    test("router_resource_enter() with options", fn() {
        let options = hash();
        options["only"] = ["index", "show"];
        options["except"] = ["destroy"];
        let result = router_resource_enter("posts", options);
        assert(result);
    });

    test("router_resource_exit() ends resource", fn() {
        router_resource_enter("comments");
        let result = router_resource_exit();
        assert(result);
    });
});

describe("Router Member Functions", fn() {
    test("router_member_enter() starts member block", fn() {
        let result = router_member_enter();
        assert(result);
    });

    test("router_member_exit() ends member block", fn() {
        router_member_enter();
        let result = router_member_exit();
        assert(result);
    });
});

describe("Router Collection Functions", fn() {
    test("router_collection_enter() starts collection block", fn() {
        let result = router_collection_enter();
        assert(result);
    });

    test("router_collection_exit() ends collection block", fn() {
        router_collection_enter();
        let result = router_collection_exit();
        assert(result);
    });
});

describe("Router Namespace Functions", fn() {
    test("router_namespace_enter() starts namespace", fn() {
        let result = router_namespace_enter("admin");
        assert(result);
    });

    test("router_namespace_exit() ends namespace", fn() {
        router_namespace_enter("api");
        let result = router_namespace_exit();
        assert(result);
    });

    test("namespaced routes have prefix", fn() {
        router_namespace_enter("v1");
        router_match("GET", "/users", "v1/users#index");
        router_namespace_exit();
    });
});

describe("Router Middleware Functions", fn() {
    test("router_middleware_scope() scopes middleware", fn() {
        let result = router_middleware_scope("auth");
        assert(result);
    });

    test("router_middleware_scope_exit() exits scope", fn() {
        router_middleware_scope("logging");
        let result = router_middleware_scope_exit();
        assert(result);
    });
});

describe("Router WebSocket Functions", fn() {
    test("router_websocket() registers WebSocket route", fn() {
        let result = router_websocket("/ws", "chat#connect");
        assert(result);
    });
});

describe("Router LiveView Functions", fn() {
    test("router_live() registers LiveView route", fn() {
        let result = router_live("/dashboard", "Dashboard#show");
        assert(result);
    });
});

describe("Router Utilities", fn() {
    test("router_routes() returns all routes", fn() {
        router_match("GET", "/test1", "test#one");
        router_match("GET", "/test2", "test#two");

        let routes = router_routes();
        assert(len(routes) >= 2);
    });

    test("router_route() finds route by path", fn() {
        router_match("GET", "/search", "search#index");
        let route = router_route("GET", "/search");
        assert_not_null(route);
    });

    test("router_clear() removes all routes", fn() {
        router_match("GET", "/temp", "temp#index");
        router_clear();
        let routes = router_routes();
        assert(len(routes) == 0);
    });
});

describe("Router URL Generation", fn() {
    test("router_path() generates path", fn() {
        let params = hash();
        params["id"] = 123;
        let path = router_path("users#show", params);
        assert_contains(path, "123");
    });

    test("router_path() with no parameters", fn() {
        let path = router_path("home#index");
        assert_not_null(path);
    });
});
