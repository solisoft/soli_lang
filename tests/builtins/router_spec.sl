// ============================================================================
// Router Test Suite
// ============================================================================

describe("Router DSL", fn() {
    test("router functions can be called", fn() {
        # These are used in config/routes.sl
        # Verify they don't error when called
        router_resource_enter("test", {});
        router_resource_exit();
        assert(true);
    });
});
