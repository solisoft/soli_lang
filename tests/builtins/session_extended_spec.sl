// ============================================================================
// Session Functions Extended Test Suite
//
// Covers the core session builtins against the default in_memory driver,
// which is fully functional without a server. Complements session_spec.sl.
// ============================================================================

describe("session_set / session_get round-trip", fn() {
    test("stores and retrieves an int", fn() {
        session_set("ext_int", 42);
        assert_eq(session_get("ext_int"), 42);
    });

    test("stores and retrieves a string", fn() {
        session_set("ext_string", "hello session");
        assert_eq(session_get("ext_string"), "hello session");
    });

    test("stores and retrieves a hash", fn() {
        session_set("ext_hash", {"name": "Alice", "admin": true});
        let stored = session_get("ext_hash");
        assert_eq(stored["name"], "Alice");
        assert_eq(stored["admin"], true);
    });

    test("stores and retrieves an array", fn() {
        session_set("ext_array", [1, 2, 3]);
        assert_eq(session_get("ext_array"), [1, 2, 3]);
    });

    test("overwrites an existing key", fn() {
        session_set("ext_overwrite", "first");
        session_set("ext_overwrite", "second");
        assert_eq(session_get("ext_overwrite"), "second");
    });

    test("returns null for a missing key", fn() {
        assert_null(session_get("ext_never_set"));
    });
});

describe("session_has", fn() {
    test("returns true for a stored key", fn() {
        session_set("ext_has_key", "present");
        assert(session_has("ext_has_key"));
    });

    test("returns false for an unknown key", fn() {
        assert(!session_has("ext_has_missing"));
    });

    test("returns false after the key is deleted", fn() {
        session_set("ext_has_deleted", 1);
        assert(session_has("ext_has_deleted"));
        session_delete("ext_has_deleted");
        assert(!session_has("ext_has_deleted"));
    });
});

describe("session_delete", fn() {
    test("returns the deleted value", fn() {
        session_set("ext_del_value", "payload");
        assert_eq(session_delete("ext_del_value"), "payload");
    });

    test("removes the key from the session", fn() {
        session_set("ext_del_removed", "gone soon");
        session_delete("ext_del_removed");
        assert_null(session_get("ext_del_removed"));
    });

    test("returns null when deleting an unknown key", fn() {
        assert_null(session_delete("ext_del_unknown"));
    });

    test("returns null when deleting twice", fn() {
        session_set("ext_del_twice", 7);
        assert_eq(session_delete("ext_del_twice"), 7);
        assert_null(session_delete("ext_del_twice"));
    });
});

describe("session_id", fn() {
    test("is available after a write (lazy creation)", fn() {
        session_set("ext_lazy", true);
        let sid = session_id();
        assert_not_null(sid);
        assert_eq(sid.length(), 36);
    });

    test("stays stable across writes", fn() {
        session_set("ext_stable_a", 1);
        let first = session_id();
        session_set("ext_stable_b", 2);
        assert_eq(session_id(), first);
    });
});

describe("session_regenerate", fn() {
    test("mints a new id different from the old one", fn() {
        session_set("ext_regen_marker", "before");
        let old_id = session_id();
        let new_id = session_regenerate();
        assert_not_null(new_id);
        assert_ne(new_id, old_id);
        assert_eq(session_id(), new_id);
    });

    test("migrates session data to the new id", fn() {
        session_set("ext_regen_data", "carried over");
        session_regenerate();
        assert_eq(session_get("ext_regen_data"), "carried over");
    });
});

describe("session_destroy", fn() {
    test("clears all session data", fn() {
        session_set("ext_destroy_key", "doomed");
        assert_not_null(session_get("ext_destroy_key"));
        session_destroy();
        assert_null(session_get("ext_destroy_key"));
        assert(!session_has("ext_destroy_key"));
    });

    test("can be called safely and a fresh session starts after regenerate", fn() {
        session_destroy();
        # The old id still points at a destroyed store entry, so a
        # regenerate is needed before writes land again.
        session_regenerate();
        session_set("ext_after_destroy", "reborn");
        assert_eq(session_get("ext_after_destroy"), "reborn");
    });
});

describe("session_driver", fn() {
    test("defaults to in_memory", fn() {
        assert_eq(session_driver(), "in_memory");
    });
});

describe("session_config", fn() {
    test("returns a hash describing the active configuration", fn() {
        let config = session_config();
        assert_eq(config["driver"], "in_memory");
        assert(config["ttl"] > 0);
    });

    test("session_configure accepts options and reports success", fn() {
        assert(session_configure({"driver": "in_memory"}));
        assert_eq(session_config()["driver"], "in_memory");
    });
});

describe("destroy_session (test helper)", fn() {
    test("signs out the test user", fn() {
        destroy_session();
        assert(signed_out());
    });

    test("returns null", fn() {
        assert_null(destroy_session());
    });
});

describe("create_session (test helper)", fn() {
    test("creates a named test session id for a user id", fn() {
        let sid = create_session(42);
        assert_eq(sid, "session_test_42");
    });

    test("marks the caller signed in until destroyed", fn() {
        create_session(42);
        assert(signed_in());
        destroy_session();
        assert(signed_out());
    });
});

describe("with_session (test helper)", fn() {
    test("seeds server-side session fields and signs the test client in", fn() {
        destroy_session();
        with_session({"user_id": 42, "role": "editor"});
        assert(signed_in());
        destroy_session();
    });

    test("rejects a non-hash argument", fn() {
        let failed = false;
        try {
            with_session(7);
        } catch e {
            failed = true;
        }
        assert(failed);
    });
});
