// ============================================================================
// Environment Extended Test Suite
// ============================================================================
// Additional tests for environment functions
// ============================================================================

describe("Environment Get Functions", fn() {
    test("env_get() retrieves environment variable", fn() {
        setenv("TEST_VAR_123", "test_value");
        let value = env_get("TEST_VAR_123");
        assert_eq(value, "test_value");
    });

    test("env_get() returns null for unset variable", fn() {
        let value = env_get("NONEXISTENT_ENV_VAR_12345");
        assert_null(value);
    });

    test("env_get() is case sensitive", fn() {
        setenv("TestCase", "value1");
        setenv("testcase", "value2");
        assert_eq(env_get("TestCase"), "value1");
        assert_eq(env_get("testcase"), "value2");
    });
});

describe("Environment Set Functions", fn() {
    test("env_set() sets environment variable", fn() {
        env_set("NEW_ENV_VAR", "new_value");
        assert_eq(env_get("NEW_ENV_VAR"), "new_value");
    });

    test("env_set() overwrites existing value", fn() {
        setenv("OVERWRITE_VAR", "original");
        env_set("OVERWRITE_VAR", "updated");
        assert_eq(env_get("OVERWRITE_VAR"), "updated");
    });

    test("env_set() handles empty string", fn() {
        env_set("EMPTY_VAR", "");
        assert_eq(env_get("EMPTY_VAR"), "");
    });

    test("env_set() handles special characters", fn() {
        env_set("SPECIAL_VAR", "!@#$%^&*()_+-=[]{}|;':\",./<>?");
        let value = env_get("SPECIAL_VAR");
        assert_eq(value, "!@#$%^&*()_+-=[]{}|;':\",./<>?");
    });

    test("env_set() handles unicode", fn() {
        env_set("UNICODE_VAR", "Hello 世界 🌍");
        assert_eq(env_get("UNICODE_VAR"), "Hello 世界 🌍");
    });
});

describe("Environment Has Functions", fn() {
    test("env_has() returns true for existing variable", fn() {
        setenv("EXISTING_ENV", "value");
        assert(env_has("EXISTING_ENV"));
    });

    test("env_has() returns false for unset variable", fn() {
        assert_not(env_has("NONEXISTENT_ENV_12345"));
    });

    test("env_has() is case sensitive", fn() {
        setenv("HasTest", "value");
        assert(env_has("HasTest"));
        assert_not(env_has("hastest"));
    });
});

describe("Environment Unset Functions", fn() {
    test("env_unset() removes environment variable", fn() {
        setenv("TO_REMOVE", "value");
        assert(env_has("TO_REMOVE"));
        env_unset("TO_REMOVE");
        assert_not(env_has("TO_REMOVE"));
    });

    test("env_unset() returns true for removed", fn() {
        setenv("TO_REMOVE2", "value");
        let result = env_unset("TO_REMOVE2");
        assert(result);
    });

    test("env_unset() returns false for nonexistent", fn() {
        let result = env_unset("NONEXISTENT_12345");
        assert_not(result);
    });
});

describe("Environment List Functions", fn() {
    test("env_all() returns all variables", fn() {
        setenv("TEST_ENV_1", "value1");
        setenv("TEST_ENV_2", "value2");

        let all = env_all();
        assert(has_key(all, "TEST_ENV_1"));
        assert(has_key(all, "TEST_ENV_2"));
    });

    test("env_keys() returns variable names", fn() {
        setenv("KEY_TEST_1", "v1");
        setenv("KEY_TEST_2", "v2");

        let keys = env_keys();
        assert_contains(keys, "KEY_TEST_1");
        assert_contains(keys, "KEY_TEST_2");
    });

    test("env_values() returns variable values", fn() {
        setenv("VAL_TEST_1", "value1");
        setenv("VAL_TEST_2", "value2");

        let values = env_values();
        assert_contains(values, "value1");
        assert_contains(values, "value2");
    });
});

describe("Environment Import/Export", fn() {
    test("env_import() loads from hash", fn() {
        let vars = hash();
        vars["IMPORT_VAR1"] = "imported1";
        vars["IMPORT_VAR2"] = "imported2";

        env_import(vars);

        assert_eq(env_get("IMPORT_VAR1"), "imported1");
        assert_eq(env_get("IMPORT_VAR2"), "imported2");
    });

    test("env_export() exports to hash", fn() {
        setenv("EXPORT_VAR", "exported");

        let exported = env_export("EXPORT_VAR");
        assert_eq(exported["EXPORT_VAR"], "exported");
    });

    test("env_clear() removes all test variables", fn() {
        setenv("CLEAR_TEST_1", "value");
        setenv("CLEAR_TEST_2", "value");

        env_clear();

        assert_not(env_has("CLEAR_TEST_1"));
        assert_not(env_has("CLEAR_TEST_2"));
    });
});

describe("Environment Default Functions", fn() {
    test("env_default() sets if not exists", fn() {
        env_unset("DEFAULT_TEST");
        env_default("DEFAULT_TEST", "default_value");
        assert_eq(env_get("DEFAULT_TEST"), "default_value");
    });

    test("env_default() does not overwrite existing", fn() {
        setenv("DEFAULT_EXISTS", "original");
        env_default("DEFAULT_EXISTS", "new_value");
        assert_eq(env_get("DEFAULT_EXISTS"), "original");
    });
});

describe("Environment Snapshot", fn() {
    test("env_snapshot() creates snapshot", fn() {
        let snapshot = env_snapshot();
        assert_not_null(snapshot);
        assert(has_key(snapshot, "PATH"));
    });

    test("env_restore() restores snapshot", fn() {
        setenv("RESTORE_TEST", "modified");
        let snapshot = env_snapshot();

        env_unset("RESTORE_TEST");
        assert_not(env_has("RESTORE_TEST"));

        env_restore(snapshot);
        assert_eq(env_get("RESTORE_TEST"), "modified");
    });
});

describe("Environment Path Functions", fn() {
    test("env_path() returns PATH variable", fn() {
        let path = env_path();
        assert_not_null(path);
        assert(len(path) > 0);
    });

    test("env_add_path() adds to PATH", fn() {
        let original = env_path();
        env_add_path("/custom/path");
        let updated = env_path();
        assert_contains(updated, "/custom/path");
    });

    test("env_prepend_path() prepends to PATH", fn() {
        env_prepend_path("/new/path");
        let path = env_path();
        assert(path.starts_with("/new/path"));
    });
});

describe("Environment Security", fn() {
    test("env_sanitize() removes sensitive values", fn() {
        let vars = hash();
        vars["API_KEY"] = "secret123";
        vars["PUBLIC_VAR"] = "safe";

        let sanitized = env_sanitize(vars);
        assert_eq(sanitized["PUBLIC_VAR"], "safe");
        assert_ne(sanitized["API_KEY"], "secret123");
    });

    test("env_mask() masks value", fn() {
        let masked = env_mask("secret");
        assert_ne(masked, "secret");
        assert(len(masked) < len("secret"));
    });
});
