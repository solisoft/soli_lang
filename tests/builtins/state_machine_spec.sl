// ============================================================================
// State Machine Test Suite (Pure Soli Implementation)
// ============================================================================

import "../../stdlib/state_machine.sl";

describe("State Machine", fn() {
    // =========================================================================
    // Basic Creation Tests
    // =========================================================================

    test("create_state_machine() creates a state machine", fn() {
        let states = ["pending", "confirmed", "processing", "shipped", "delivered"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"},
            {"event": "process", "from": "confirmed", "to": "processing"},
            {"event": "ship", "from": "processing", "to": "shipped"},
            {"event": "deliver", "from": "shipped", "to": "delivered"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        assert_not_null(sm);
        assert_eq(sm.current_state(), "pending");
    });

    test("create_state_machine() with multiple from states", fn() {
        let states = ["pending", "authorized", "captured", "failed", "refunded"];
        let transitions = [
            {"event": "authorize", "from": "pending", "to": "authorized"},
            {"event": "fail", "from": ["pending", "authorized"], "to": "failed"},
            {"event": "refund", "from": ["captured", "failed"], "to": "refunded"}
        ];

        let payment = create_state_machine("pending", states, transitions);

        assert_eq(payment.current_state(), "pending");
        payment.transition("authorize");
        assert_eq(payment.current_state(), "authorized");
        payment.transition("fail");
        assert_eq(payment.current_state(), "failed");
    });

    // =========================================================================
    // State Query Tests
    // =========================================================================

    test("is() returns true when in correct state", fn() {
        let states = ["pending", "confirmed", "processing"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"},
            {"event": "process", "from": "confirmed", "to": "processing"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        assert_true(sm.is("pending"));
        assert_false(sm.is("confirmed"));
        assert_false(sm.is("processing"));
    });

    test("is_in() returns true when in any of the given states", fn() {
        let states = ["pending", "confirmed", "processing", "shipped", "delivered"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"},
            {"event": "process", "from": "confirmed", "to": "processing"},
            {"event": "ship", "from": "processing", "to": "shipped"},
            {"event": "deliver", "from": "shipped", "to": "delivered"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        assert_true(sm.is_in(["pending"]));
        assert_true(sm.is_in(["confirmed", "processing"]));
        assert_true(sm.is_in(["shipped", "delivered", "pending"]));
        assert_false(sm.is_in(["shipped", "delivered"]));
    });

    test("current_state() returns current state as string", fn() {
        let states = ["a", "b", "c"];
        let transitions = [
            {"event": "to_b", "from": "a", "to": "b"},
            {"event": "to_c", "from": "b", "to": "c"}
        ];

        let sm = create_state_machine("a", states, transitions);

        assert_eq(sm.current_state(), "a");
        sm.transition("to_b");
        assert_eq(sm.current_state(), "b");
        sm.transition("to_c");
        assert_eq(sm.current_state(), "c");
    });

    // =========================================================================
    // Transition Tests
    // =========================================================================

    test("transition() changes state", fn() {
        let states = ["pending", "confirmed", "processing", "shipped"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"},
            {"event": "process", "from": "confirmed", "to": "processing"},
            {"event": "ship", "from": "processing", "to": "shipped"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        assert_eq(sm.current_state(), "pending");
        let result = sm.transition("confirm");
        assert_true(result["success"]);
        assert_eq(sm.current_state(), "confirmed");
        sm.transition("process");
        assert_eq(sm.current_state(), "processing");
        sm.transition("ship");
        assert_eq(sm.current_state(), "shipped");
    });

    test("invalid transition returns error result", fn() {
        let states = ["pending", "confirmed", "processing"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"},
            {"event": "process", "from": "confirmed", "to": "processing"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        let result = sm.transition("ship");
        assert_false(result["success"]);
        assert_eq(result["error"], "invalid_transition");
        assert_contains(result["reason"], "Cannot transition 'ship'");
        assert_eq(sm.current_state(), "pending");
    });

    // =========================================================================
    // Context Storage Tests
    // =========================================================================

    test("set() and get() store and retrieve data", fn() {
        let states = ["pending", "confirmed"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        sm.set("customer_id", 12345);
        sm.set("total", 99.99);
        sm.set("items", ["Product A", "Product B"]);
        sm.set("is_vip", true);

        assert_eq(sm.get("customer_id"), 12345);
        assert_eq(sm.get("total"), 99.99);
        assert_eq(sm.get("is_vip"), true);
        assert_eq(sm.get("items")[0], "Product A");
    });

    test("get() returns null for missing keys", fn() {
        let states = ["pending"];
        let transitions = [];

        let sm = create_state_machine("pending", states, transitions);

        assert_null(sm.get("missing_key"));
        assert_null(sm.get("another_missing"));
    });

    test("context persists across transitions", fn() {
        let states = ["pending", "confirmed", "processing"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"},
            {"event": "process", "from": "confirmed", "to": "processing"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        sm.set("order_id", "ORD-123");
        sm.transition("confirm");
        sm.set("confirmed_at", "2024-01-15");
        sm.transition("process");

        assert_eq(sm.get("order_id"), "ORD-123");
        assert_eq(sm.get("confirmed_at"), "2024-01-15");
        assert_eq(sm.current_state(), "processing");
    });

    // =========================================================================
    // Guard Condition Tests
    // =========================================================================

    test("manual guard check before transition", fn() {
        let states = ["pending", "processing", "shipped"];
        let transitions = [
            {"event": "start", "from": "pending", "to": "processing"},
            {"event": "ship", "from": "processing", "to": "shipped"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        sm.transition("start");
        sm.set("can_ship", false);

        // Without guard check, ship would fail
        if sm.get("can_ship") != true {
            let result = sm.transition("ship");
            assert_false(result["success"]);
        }
        assert_eq(sm.current_state(), "processing");
    });

    // =========================================================================
    // Advanced Method Tests
    // =========================================================================

    test("can() returns true for available events", fn() {
        let states = ["pending", "confirmed", "processing"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"},
            {"event": "process", "from": "confirmed", "to": "processing"},
            {"event": "cancel", "from": "pending", "to": "cancelled"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        assert_true(sm.can("confirm"));
        assert_true(sm.can("cancel"));
        assert_false(sm.can("process"));
        assert_false(sm.can("ship"));
    });

    test("can() returns false after state change", fn() {
        let states = ["pending", "confirmed", "processing"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"},
            {"event": "process", "from": "confirmed", "to": "processing"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        assert_true(sm.can("confirm"));
        sm.transition("confirm");
        assert_false(sm.can("confirm"));
        assert_true(sm.can("process"));
    });

    test("available_events() returns array of available events", fn() {
        let states = ["pending", "confirmed", "processing", "shipped"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"},
            {"event": "process", "from": "confirmed", "to": "processing"},
            {"event": "ship", "from": "processing", "to": "shipped"},
            {"event": "cancel", "from": "pending", "to": "cancelled"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        let events = sm.available_events();
        assert_true(len(events) > 0);
        assert_true(events.contains("confirm"));
        assert_true(events.contains("cancel"));
        assert_false(events.contains("process"));
        assert_false(events.contains("ship"));
    });

    test("last_transition() returns null initially", fn() {
        let states = ["pending"];
        let transitions = [];

        let sm = create_state_machine("pending", states, transitions);

        assert_null(sm.last_transition());
    });

    test("last_transition() returns transition info after change", fn() {
        let states = ["pending", "confirmed"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        sm.transition("confirm");

        let last = sm.last_transition();
        assert_not_null(last);
        assert_eq(last["from"], "pending");
        assert_eq(last["to"], "confirmed");
        assert_eq(last["event"], "confirm");
    });

    test("last_transition() updates after each transition", fn() {
        let states = ["a", "b", "c"];
        let transitions = [
            {"event": "to_b", "from": "a", "to": "b"},
            {"event": "to_c", "from": "b", "to": "c"}
        ];

        let sm = create_state_machine("a", states, transitions);

        sm.transition("to_b");
        let last1 = sm.last_transition();
        assert_eq(last1["event"], "to_b");
        assert_eq(last1["to"], "b");

        sm.transition("to_c");
        let last2 = sm.last_transition();
        assert_eq(last2["event"], "to_c");
        assert_eq(last2["to"], "c");
    });

    // =========================================================================
    // History Tests
    // =========================================================================

    test("history() returns array data structure", fn() {
        let states = ["pending", "confirmed"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        let hist = sm.history();
        assert_not_null(hist);
    });

    test("history() records all transitions", fn() {
        let states = ["pending", "confirmed", "processing"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"},
            {"event": "process", "from": "confirmed", "to": "processing"}
        ];

        let sm = create_state_machine("pending", states, transitions);

        sm.transition("confirm");
        sm.transition("process");

        let hist = sm.history();
        assert_eq(len(hist), 2);
        assert_eq(hist[0]["from"], "pending");
        assert_eq(hist[0]["to"], "confirmed");
        assert_eq(hist[1]["from"], "confirmed");
        assert_eq(hist[1]["to"], "processing");
    });

    // =========================================================================
    // Complex Workflow Tests
    // =========================================================================

    test("complete order processing workflow", fn() {
        let states = ["pending", "confirmed", "processing", "shipped", "delivered", "cancelled"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"},
            {"event": "process", "from": "confirmed", "to": "processing"},
            {"event": "ship", "from": "processing", "to": "shipped"},
            {"event": "deliver", "from": "shipped", "to": "delivered"},
            {"event": "cancel", "from": "pending", "to": "cancelled"}
        ];

        let order = create_state_machine("pending", states, transitions);

        // Initial state
        assert_eq(order.current_state(), "pending");
        assert_true(order.is("pending"));
        assert_false(order.is("delivered"));

        // Process order
        order.transition("confirm");
        assert_eq(order.current_state(), "confirmed");
        assert_false(order.is("pending"));
        assert_true(order.is("confirmed"));

        order.transition("process");
        assert_eq(order.current_state(), "processing");

        order.transition("ship");
        assert_eq(order.current_state(), "shipped");

        order.transition("deliver");
        assert_eq(order.current_state(), "delivered");

        // Verify final state
        assert_true(order.is("delivered"));
        assert_false(order.is("pending"));
    });

    test("payment state machine with multiple source states", fn() {
        let states = ["pending", "authorized", "captured", "failed", "refunded"];
        let transitions = [
            {"event": "authorize", "from": "pending", "to": "authorized"},
            {"event": "capture", "from": "authorized", "to": "captured"},
            {"event": "fail", "from": ["pending", "authorized"], "to": "failed"},
            {"event": "refund", "from": ["captured", "failed"], "to": "refunded"},
            {"event": "retry", "from": "failed", "to": "pending"}
        ];

        let payment = create_state_machine("pending", states, transitions);

        // Normal flow
        payment.transition("authorize");
        assert_eq(payment.current_state(), "authorized");

        payment.transition("capture");
        assert_eq(payment.current_state(), "captured");

        // Can fail from multiple states
        let payment2 = create_state_machine("pending", states, transitions);
        payment2.transition("authorize");
        payment2.transition("fail");
        assert_eq(payment2.current_state(), "failed");

        let payment3 = create_state_machine("pending", states, transitions);
        payment3.transition("fail");  // Can fail directly from pending
        assert_eq(payment3.current_state(), "failed");
    });

    test("workflow with context data", fn() {
        let states = ["draft", "review", "approved", "published"];
        let transitions = [
            {"event": "submit", "from": "draft", "to": "review"},
            {"event": "approve", "from": "review", "to": "approved"},
            {"event": "publish", "from": "approved", "to": "published"}
        ];

        let article = create_state_machine("draft", states, transitions);

        article.set("title", "My Article");
        article.set("author_id", 123);
        article.set("word_count", 1500);
        article.set("tags", ["tech", "programming"]);

        assert_eq(article.get("title"), "My Article");
        assert_eq(article.get("author_id"), 123);
        assert_eq(article.get("word_count"), 1500);

        article.transition("submit");
        article.set("reviewer_id", 456);
        article.transition("approve");
        article.set("published_at", "2024-01-15");
        article.transition("publish");

        assert_eq(article.get("title"), "My Article");
        assert_eq(article.get("published_at"), "2024-01-15");
        assert_eq(article.current_state(), "published");
    });

    test("query methods reflect current state", fn() {
        let states = ["pending", "confirmed", "processing", "shipped", "delivered"];
        let transitions = [
            {"event": "confirm", "from": "pending", "to": "confirmed"},
            {"event": "process", "from": "confirmed", "to": "processing"},
            {"event": "ship", "from": "processing", "to": "shipped"},
            {"event": "deliver", "from": "shipped", "to": "delivered"},
            {"event": "cancel", "from": "pending", "to": "cancelled"}
        ];

        let order = create_state_machine("pending", states, transitions);

        // Initially
        assert_true(order.can("confirm"));
        assert_true(order.can("cancel"));
        assert_false(order.can("process"));
        assert_false(order.can("ship"));

        let events = order.available_events();
        assert_true(events.contains("confirm"));
        assert_true(events.contains("cancel"));

        // After confirm
        order.transition("confirm");
        assert_false(order.can("confirm"));
        assert_false(order.can("cancel"));
        assert_true(order.can("process"));
        assert_false(order.can("ship"));

        events = order.available_events();
        assert_false(events.contains("confirm"));
        assert_false(events.contains("cancel"));
        assert_true(events.contains("process"));
    });

    // =========================================================================
    // StateMachineBuilder Tests
    // =========================================================================

    test("StateMachineBuilder creates state machine", fn() {
        let sm = state_machine()
            .initial("pending")
            .states_list(["pending", "confirmed", "processing"])
            .transition("confirm", "pending", "confirmed")
            .transition("process", "confirmed", "processing")
            .build();

        assert_eq(sm.current_state(), "pending");
        sm.transition("confirm");
        assert_eq(sm.current_state(), "confirmed");
        sm.transition("process");
        assert_eq(sm.current_state(), "processing");
    });

    test("StateMachineBuilder with multiple source states", fn() {
        let sm = state_machine()
            .initial("pending")
            .states_list(["pending", "authorized", "failed"])
            .transition("authorize", "pending", "authorized")
            .transition("fail", ["pending", "authorized"], "failed")
            .build();

        assert_eq(sm.current_state(), "pending");
        sm.transition("authorize");
        assert_eq(sm.current_state(), "authorized");
        sm.transition("fail");
        assert_eq(sm.current_state(), "failed");
    });
});
