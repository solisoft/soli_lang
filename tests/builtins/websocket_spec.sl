// ============================================================================
// WebSocket Functions Test Suite
// ============================================================================
// Tests for WebSocket functions
// ============================================================================

describe("WebSocket Connection Management", fn() {
    test("ws_clients() returns array of client IDs", fn() {
        let clients = ws_clients();
        assert_not_null(clients);
    });

    test("ws_count() returns connection count", fn() {
        let count = ws_count();
        assert(count >= 0);
    });

    test("ws_count() is consistent with clients", fn() {
        let clients = ws_clients();
        let count = ws_count();
        assert_eq(len(clients), count);
    });
});

describe("WebSocket Send Functions", fn() {
    test("ws_send() sends message to client", fn() {
        let clients = ws_clients();
        if (len(clients) > 0) {
            let result = ws_send(clients[0], "test message");
            assert(result);
        }
    });

    test("ws_send() returns false for invalid client", fn() {
        let result = ws_send("invalid_client_id", "message");
        assert_not(result);
    });

    test("ws_broadcast() sends to all clients", fn() {
        let result = ws_broadcast("broadcast message");
        assert(result);
    });

    test("ws_broadcast_room() sends to channel", fn() {
        let result = ws_broadcast_room("general", "room message");
        assert(result);
    });
});

describe("WebSocket Room Functions", fn() {
    test("ws_join() adds client to room", fn() {
        let clients = ws_clients();
        if (len(clients) > 0) {
            let result = ws_join(clients[0], "test_room");
            assert(result);
        }
    });

    test("ws_leave() removes client from room", fn() {
        let clients = ws_clients();
        if (len(clients) > 0) {
            ws_join(clients[0], "test_room");
            let result = ws_leave(clients[0], "test_room");
            assert(result);
        }
    });

    test("ws_clients_in() returns clients in room", fn() {
        let clients_in = ws_clients_in("general");
        assert_not_null(clients_in);
    });

    test("ws_clients_in() returns empty array for empty room", fn() {
        let clients_in = ws_clients_in("nonexistent_room_12345");
        assert_eq(len(clients_in), 0);
    });
});

describe("WebSocket Close Functions", fn() {
    test("ws_close() closes connection", fn() {
        let clients = ws_clients();
        if (len(clients) > 0) {
            let result = ws_close(clients[0], "Connection closed");
            assert(result);
        }
    });

    test("ws_close() with reason", fn() {
        let clients = ws_clients();
        if (len(clients) > 0) {
            let result = ws_close(clients[0], "Test closure");
            assert(result);
        }
    });

    test("ws_close() fails for invalid client", fn() {
        let result = ws_close("invalid_client", "reason");
        assert_not(result);
    });
});

describe("WebSocket Broadcast Functions", fn() {
    test("ws_broadcast() handles empty message", fn() {
        let result = ws_broadcast("");
        assert(result);
    });

    test("ws_broadcast() handles JSON message", fn() {
        let data = hash();
        data["type"] = "test";
        data["value"] = 123;
        let result = ws_broadcast(data);
        assert(result);
    });

    test("ws_broadcast_room() to specific room", fn() {
        let result = ws_broadcast_room("chat", "Hello chat!");
        assert(result);
    });

    test("ws_broadcast_room() to nonexistent room", fn() {
        let result = ws_broadcast_room("nonexistent_room", "message");
        assert(result);
    });

    test("ws_broadcast_to() sends to specific client", fn() {
        let clients = ws_clients();
        if (len(clients) > 0) {
            let result = ws_broadcast_to(clients[0], "Direct message");
            assert(result);
        }
    });
});

describe("WebSocket Error Handling", fn() {
    test("ws_send() handles empty message", fn() {
        let clients = ws_clients();
        if (len(clients) > 0) {
            let result = ws_send(clients[0], "");
            assert(result);
        }
    });

    test("ws_send() handles large message", fn() {
        let clients = ws_clients();
        if (len(clients) > 0) {
            let large_message = "x" * 10000;
            let result = ws_send(clients[0], large_message);
            assert(result);
        }
    });

    test("ws_join() handles non-existent client", fn() {
        let result = ws_join("fake_client", "room");
        assert_not(result);
    });

    test("ws_leave() handles non-existent client", fn() {
        let result = ws_leave("fake_client", "room");
        assert_not(result);
    });
});

describe("WebSocket Status Functions", fn() {
    test("ws_is_connected() checks connection", fn() {
        let clients = ws_clients();
        if (len(clients) > 0) {
            assert(ws_is_connected(clients[0]));
        }
    });

    test("ws_is_connected() returns false for invalid client", fn() {
        assert_not(ws_is_connected("invalid_client"));
    });

    test("ws_last_activity() returns timestamp", fn() {
        let clients = ws_clients();
        if (len(clients) > 0) {
            let last = ws_last_activity(clients[0]);
            assert(last > 0);
        }
    });
});

describe("WebSocket Ping/Pong", fn() {
    test("ws_ping() sends ping to client", fn() {
        let clients = ws_clients();
        if (len(clients) > 0) {
            let result = ws_ping(clients[0]);
            assert(result);
        }
    });

    test("ws_pong() sends pong response", fn() {
        let clients = ws_clients();
        if (len(clients) > 0) {
            let result = ws_pong(clients[0]);
            assert(result);
        }
    });
});
