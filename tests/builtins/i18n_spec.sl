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