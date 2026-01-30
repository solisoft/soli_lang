// ============================================================================
// Session Functions Test Suite
// ============================================================================
// Tests for session management functions
// ============================================================================

describe("Session Basic Functions", fn() {
    test("session_set() sets a value", fn() {
        session_set("test_key", "test_value");
        let value = session_get("test_key");
        assert_eq(value, "test_value");
    });

    test("session_get() retrieves value", fn() {
        session_set("user_id", "12345");
        let value = session_get("user_id");
        assert_eq(value, "12345");
    });

    test("session_get() returns null for unset key", fn() {
        let value = session_get("nonexistent_key_12345");
        assert_null(value);
    });

    test("session_has() returns true for existing key", fn() {
        session_set("existing_key", "value");
        assert(session_has("existing_key"));
    });

    test("session_has() returns false for nonexistent key", fn() {
        assert_not(session_has("nonexistent_key_12345"));
    });

    test("session_delete() removes key", fn() {
        session_set("to_delete", "value");
        assert(session_has("to_delete"));
        session_delete("to_delete");
        assert_not(session_has("to_delete"));
    });

    test("session_delete() returns deleted value", fn() {
        session_set("test_value", "test");
        let deleted = session_delete("test_value");
        assert_eq(deleted, "test");
    });

    test("session_destroy() clears all session data", fn() {
        session_set("key1", "value1");
        session_set("key2", "value2");
        session_destroy();
        assert_not(session_has("key1"));
        assert_not(session_has("key2"));
    });
});

describe("Session ID Functions", fn() {
    test("session_id() returns session ID", fn() {
        let id = session_id();
        assert_not_null(id);
        assert(len(id) > 0);
    });

    test("session_regenerate() generates new ID", fn() {
        let old_id = session_id();
        session_regenerate();
        let new_id = session_id();
        assert(old_id != new_id);
    });

    test("session_regenerate() preserves session data", fn() {
        session_set("important_data", "preserved");
        let old_id = session_id();
        session_regenerate();
        let new_id = session_id();
        let value = session_get("important_data");
        assert_eq(value, "preserved");
        assert(old_id != new_id);
    });
});

describe("Session Data Types", fn() {
    test("session can store string", fn() {
        session_set("string_value", "hello");
        assert_eq(session_get("string_value"), "hello");
    });

    test("session can store number", fn() {
        session_set("number_value", 42);
        assert_eq(session_get("number_value"), 42);
    });

    test("session can store boolean", fn() {
        session_set("bool_value", true);
        assert(session_get("bool_value"));
    });

    test("session can store array", fn() {
        session_set("array_value", [1, 2, 3]);
        let arr = session_get("array_value");
        assert_eq(len(arr), 3);
    });

    test("session can store hash", fn() {
        let h = hash();
        h["key"] = "value";
        session_set("hash_value", h);
        let retrieved = session_get("hash_value");
        assert_eq(retrieved["key"], "value");
    });

    test("session can store null", fn() {
        session_set("null_value", null);
        assert_null(session_get("null_value"));
    });
});

describe("Session Security", fn() {
    test("session_token() returns CSRF token", fn() {
        let token = session_token();
        assert_not_null(token);
        assert(len(token) > 0);
    });

    test("session_token() is consistent", fn() {
        let token1 = session_token();
        let token2 = session_token();
        assert_eq(token1, token2);
    });
});
