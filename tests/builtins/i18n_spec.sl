// ============================================================================
// I18n.translate, I18n.plural, I18n.format_number, I18n.format_date Tests
// ============================================================================

describe("I18n.locale and I18n.set_locale", fn() {
    test("I18n.locale returns current locale", fn() {
        let locale = I18n.locale();
        assert(locale != null);
        assert(type(locale) == "string");
    });

    test("I18n.set_locale changes locale and returns it", fn() {
        let original = I18n.locale();
        let result = I18n.set_locale("fr");
        assert_eq(result, "fr");
        assert_eq(I18n.locale(), "fr");
        I18n.set_locale(original);
    });

    test("I18n.set_locale returns error for non-string", fn() {
        let failed = false;
        try {
            I18n.set_locale(123);
        } catch e {
            failed = true;
        }
        assert(failed);
    });
});

describe("I18n.translate", fn() {
    test("translate with translations hash", fn() {
        let t = {
            "en.greeting": "Hello",
            "fr.greeting": "Bonjour"
        };
        assert_eq(I18n.translate("greeting", "en", t), "Hello");
        assert_eq(I18n.translate("greeting", "fr", t), "Bonjour");
    });

    test("translate falls back to en if locale key not found", fn() {
        let t = {
            "en.greeting": "Hello"
        };
        assert_eq(I18n.translate("greeting", "de", t), "Hello");
    });

    test("translate returns key if no translation found", fn() {
        let t = {};
        assert_eq(I18n.translate("unknown_key", "en", t), "unknown_key");
    });

    test("translate expects string key", fn() {
        let failed = false;
        try {
            I18n.translate(123, "en", {});
        } catch e {
            failed = true;
        }
        assert(failed);
    });

    # ---- New (post-YAML-store) behavior ------------------------------------

    test("translate with one arg returns the key when nothing resolves", fn() {
        # In the test environment the auto-loaded store is empty, so a key
        # with no matching locale entry is returned verbatim.
        assert_eq(I18n.translate("missing.key"), "missing.key");
    });

    test("translate with null locale uses current locale", fn() {
        let original = I18n.locale();
        I18n.set_locale("en");
        let t = { "en.hi": "Hello" };
        assert_eq(I18n.translate("hi", null, t), "Hello");
        I18n.set_locale(original);
    });

    test("translate rejects non-string non-hash second arg", fn() {
        let failed = false;
        try {
            I18n.translate("greeting", 42);
        } catch e {
            failed = true;
        }
        assert(failed);
    });

    test("translate rejects non-hash third arg", fn() {
        let failed = false;
        try {
            I18n.translate("greeting", "en", "not a hash");
        } catch e {
            failed = true;
        }
        assert(failed);
    });

    test("translate with values-hash 2nd arg uses current locale", fn() {
        # No matching translation in the empty auto-store, so the key falls
        # through as-is. Asserts the disambiguation does not raise.
        let result = I18n.translate("missing", { name: "Alice" });
        assert_eq(result, "missing");
    });

    test("translate treats non-dotted-key 3rd arg as values not legacy", fn() {
        # 3rd-arg hash whose keys don't all contain a dot is interpolation
        # values. The auto-store miss returns the key (no `{}`), so values
        # are unused but the call must not raise nor be misclassified.
        let result = I18n.translate("missing", "en", { name: "Alice" });
        assert_eq(result, "missing");
    });
});

describe("I18n.plural", fn() {
    test("plural returns a string", fn() {
        let t = {};
        let result = I18n.plural("items", 0, "en", t);
        assert(type(result) == "string");
    });

    test("plural with no matching translation returns key", fn() {
        let t = {};
        assert_eq(I18n.plural("items", 5, "en", t), "items");
    });

    test("plural expects number as second argument", fn() {
        let failed = false;
        try {
            I18n.plural("items", "five", "en", {});
        } catch e {
            failed = true;
        }
        assert(failed);
    });

    test("plural with legacy translations selects suffix by count", fn() {
        let t = {
            "en.items_zero": "No items",
            "en.items_one": "1 item",
            "en.items_other": "Many items"
        };
        assert_eq(I18n.plural("items", 0, "en", t), "No items");
        assert_eq(I18n.plural("items", 1, "en", t), "1 item");
        assert_eq(I18n.plural("items", 5, "en", t), "Many items");
    });

    test("plural rejects non-string non-hash third arg", fn() {
        let failed = false;
        try {
            I18n.plural("items", 5, 99);
        } catch e {
            failed = true;
        }
        assert(failed);
    });
});

describe("I18n.format_number", fn() {
    test("format_number with English locale uses dot", fn() {
        assert_eq(I18n.format_number(1234.56, "en"), "1234.56");
    });

    test("format_number with French locale uses comma", fn() {
        assert_eq(I18n.format_number(1234.56, "fr"), "1234,56");
    });

    test("format_number with German locale uses comma", fn() {
        assert_eq(I18n.format_number(1234.56, "de"), "1234,56");
    });

    test("format_number with integer", fn() {
        assert_eq(I18n.format_number(100, "en"), "100");
    });

    test("format_number with float", fn() {
        assert_eq(I18n.format_number(9.99, "en"), "9.99");
    });

    test("format_number expects number", fn() {
        let failed = false;
        try {
            I18n.format_number("not a number");
        } catch e {
            failed = true;
        }
        assert(failed);
    });
});

describe("I18n.format_date", fn() {
    test("format_date formats timestamp", fn() {
        let ts = 1704067200;
        let result = I18n.format_date(ts, "en");
        assert(type(result) == "string");
        assert(result.contains("/"));
    });

    test("format_date with French locale uses dd/mm/yyyy", fn() {
        let ts = 1704067200;
        let result = I18n.format_date(ts, "fr");
        assert(result.contains("/"));
    });

    test("format_date with German locale uses dd.mm.yyyy", fn() {
        let ts = 1704067200;
        let result = I18n.format_date(ts, "de");
        assert(result.contains("."));
    });

    test("format_date with unknown locale uses yyyy-mm-dd", fn() {
        let ts = 1704067200;
        let result = I18n.format_date(ts, "xx");
        assert(result.contains("-"));
    });

    test("format_date expects timestamp", fn() {
        let failed = false;
        try {
            I18n.format_date("not a timestamp");
        } catch e {
            failed = true;
        }
        assert(failed);
    });

    test("format_date with null locale uses current locale", fn() {
        let ts = 1704067200;
        let original = I18n.locale();
        I18n.set_locale("en");
        let result = I18n.format_date(ts, null);
        assert(result != null);
        I18n.set_locale(original);
    });
});