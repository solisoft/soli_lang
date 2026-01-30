// ============================================================================
// I18n Functions Test Suite
// ============================================================================

describe("I18n Functions", fn() {
    test("I18n.locale() returns current locale", fn() {
        let locale = I18n.locale();
        assert_not_null(locale);
    });

    test("I18n.set_locale() changes locale", fn() {
        I18n.set_locale("en");
        assert_eq(I18n.locale(), "en");
    });

    test("I18n.pluralize() returns correct form", fn() {
        assert_eq(I18n.pluralize(1, "item", "items"), "item");
        assert_eq(I18n.pluralize(2, "item", "items"), "items");
        assert_eq(I18n.pluralize(0, "item", "items"), "items");
    });

    test("I18n.number_format() formats numbers", fn() {
        let formatted = I18n.number_format(1234.56);
        assert_not_null(formatted);
    });

    test("I18n.currency_format() formats currency", fn() {
        let formatted = I18n.currency_format(99.99);
        assert_not_null(formatted);
    });

    test("I18n.date_format() formats dates", fn() {
        let dt = DateTime.now();
        let formatted = I18n.date_format(dt);
        assert_not_null(formatted);
    });

    test("I18n.translate() translates key", fn() {
        I18n.set_locale("en");
        let translated = I18n.t("hello");
        assert_not_null(translated);
    });

    test("I18n.translate() with interpolation", fn() {
        I18n.set_locale("en");
        let translated = I18n.t("greeting", {name: "World"});
        assert_not_null(translated);
    });
});
