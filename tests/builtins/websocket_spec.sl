describe("WebSocket Functions", fn() {
    test("ws_join exists and validates args", fn() {
        let caught = false;
        try {
            ws_join(123);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("ws_leave exists and validates args", fn() {
        let caught = false;
        try {
            ws_leave(123);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("ws_clients exists and returns hash", fn() {
        let clients = ws_clients();
        assert(clients.is_a?("hash"));
    });

    test("ws_clients_in exists and validates args", fn() {
        let caught = false;
        try {
            ws_clients_in(123);
        } catch (e) {
            caught = true;
        }
        assert(caught);
    });

    test("ws_count exists and returns int", fn() {
        let count = ws_count();
        assert(count.is_a?("int"));
        assert_eq(count, 0);
    });
});
