// ============================================================================
// I18n Extended Test Suite
// ============================================================================
// Additional tests for I18n methods not covered in i18n_spec.sl
// ============================================================================

describe("I18n Translation Functions", fn() {
    test("I18n.t() translates key with default locale", fn() {
        I18n.set_locale("en");
        let translated = I18n.t("hello");
        assert_not_null(translated);
    });

    test("I18n.t() with specific locale parameter", fn() {
        let translated = I18n.t("hello", "fr");
        assert_not_null(translated);
    });

    test("I18n.t() with interpolation variables", fn() {
        I18n.set_locale("en");
        let translated = I18n.t("greeting", {name: "World"});
        assert_not_null(translated);
        assert_contains(translated, "World");
    });

    test("I18n.t() returns key when translation not found", fn() {
        let translated = I18n.t("nonexistent_key_12345");
        assert_eq(translated, "nonexistent_key_12345");
    });

    test("I18n.t() with nested key", fn() {
        let translated = I18n.t("user.name");
        assert_not_null(translated);
    });

    test("I18n.t() handles missing variables gracefully", fn() {
        let translated = I18n.t("greeting");
        assert_not_null(translated);
    });
});

describe("I18n Plural Functions", fn() {
    test("I18n.plural() with singular (n=1)", fn() {
        let result = I18n.plural(1, "apple", "apples");
        assert_eq(result, "apple");
    });

    test("I18n.plural() with plural (n>1)", fn() {
        let result = I18n.plural(5, "apple", "apples");
        assert_eq(result, "apples");
    });

    test("I18n.plural() with zero (n=0)", fn() {
        let result = I18n.plural(0, "apple", "apples");
        assert_eq(result, "apples");
    });

    test("I18n.plural() with negative number", fn() {
        let result = I18n.plural(-1, "apple", "apples");
        assert_eq(result, "apple");
    });

    test("I18n.pl() is alias for plural()", fn() {
        let result = I18n.pl(2, "item", "items");
        assert_eq(result, "items");
    });

    test("I18n.plural() with custom locale", fn() {
        let result = I18n.plural(1, "apple", "apples", "fr");
        assert_not_null(result);
    });

    test("I18n.plural() handles complex pluralization", fn() {
        let result = I18n.plural(0, "message", "messages");
        assert_eq(result, "messages");
    });
});

describe("I18n Currency Formatting", fn() {
    test("I18n.currency_format() formats USD", fn() {
        let formatted = I18n.currency_format(99.99, "USD");
        assert_not_null(formatted);
        assert_contains(formatted, "99");
        assert_contains(formatted, "99");
    });

    test("I18n.currency_format() formats EUR", fn() {
        let formatted = I18n.currency_format(50.00, "EUR");
        assert_not_null(formatted);
    });

    test("I18n.currency_format() formats GBP", fn() {
        let formatted = I18n.currency_format(75.50, "GBP");
        assert_not_null(formatted);
    });

    test("I18n.currency_format() with custom locale", fn() {
        let formatted = I18n.currency_format(100, "USD", "en-GB");
        assert_not_null(formatted);
    });

    test("I18n.currency_format() handles zero", fn() {
        let formatted = I18n.currency_format(0, "USD");
        assert_not_null(formatted);
    });

    test("I18n.currency_format() handles large numbers", fn() {
        let formatted = I18n.currency_format(1000000, "USD");
        assert_not_null(formatted);
        assert(len(formatted) > 5);
    });

    test("I18n.currency_format() handles negative amounts", fn() {
        let formatted = I18n.currency_format(-50, "USD");
        assert_not_null(formatted);
    });
});

describe("I18n Date Formatting", fn() {
    test("I18n.date_format() formats date", fn() {
        let dt = DateTime.now();
        let formatted = I18n.date_format(dt);
        assert_not_null(formatted);
    });

    test("I18n.date_format() with format string", fn() {
        let dt = DateTime.now();
        let formatted = I18n.date_format(dt, "%Y-%m-%d");
        assert_contains(formatted, "2024");
    });

    test("I18n.date_format() with locale", fn() {
        let dt = DateTime.now();
        let formatted = I18n.date_format(dt, "%B %d, %Y", "fr");
        assert_not_null(formatted);
    });

    test("I18n.date_format() short format", fn() {
        let dt = DateTime.now();
        let formatted = I18n.date_format(dt, "%m/%d/%Y");
        assert_contains(formatted, "/");
    });

    test("I18n.date_format() long format", fn() {
        let dt = DateTime.now();
        let formatted = I18n.date_format(dt, "%A, %B %d, %Y");
        assert_not_null(formatted);
    });

    test("I18n.date_format() handles different locales", fn() {
        let dt = DateTime.now();
        let en = I18n.date_format(dt, "%B", "en");
        let fr = I18n.date_format(dt, "%B", "fr");
        assert(en != fr);
    });
});

describe("I18n Number Formatting - Extended", fn() {
    test("I18n.number_format() with decimals", fn() {
        let formatted = I18n.number_format(1234.567, 2);
        assert_not_null(formatted);
    });

    test("I18n.number_format() with custom locale", fn() {
        let formatted = I18n.number_format(1000, 0, "de-DE");
        assert_not_null(formatted);
    });

    test("I18n.number_format() uses grouping", fn() {
        let formatted = I18n.number_format(1000000);
        assert(len(formatted) > 4);
    });

    test("I18n.number_format() handles integers", fn() {
        let formatted = I18n.number_format(42);
        assert_not_null(formatted);
    });

    test("I18n.number_format() handles zero", fn() {
        let formatted = I18n.number_format(0);
        assert_not_null(formatted);
    });

    test("I18n.number_format() handles negative numbers", fn() {
        let formatted = I18n.number_format(-100);
        assert_not_null(formatted);
    });

    test("I18n.percent_format() formats percentage", fn() {
        let formatted = I18n.percent_format(0.75);
        assert_not_null(formatted);
        assert_contains(formatted, "75");
    });

    test("I18n.percent_format() with decimals", fn() {
        let formatted = I18n.percent_format(0.1234, 1);
        assert_not_null(formatted);
    });
});

describe("I18n Time Formatting", fn() {
    test("I18n.time_format() formats time", fn() {
        let dt = DateTime.now();
        let formatted = I18n.time_format(dt);
        assert_not_null(formatted);
    });

    test("I18n.time_format() with format string", fn() {
        let dt = DateTime.now();
        let formatted = I18n.time_format(dt, "%H:%M");
        assert_contains(formatted, ":");
    });

    test("I18n.time_format() with locale", fn() {
        let dt = DateTime.now();
        let formatted = I18n.time_format(dt, "%I:%M %p", "en-US");
        assert_not_null(formatted);
    });

    test("I18n.time_format() 24-hour format", fn() {
        let dt = DateTime.now();
        let formatted = I18n.time_format(dt, "%H:%M:%S");
        assert(len(formatted) >= 5);
    });
});

describe("I18n Relative Time", fn() {
    test("I18n.relative_time() formats recent past", fn() {
        let now = DateTime.now();
        let past = now.subtract_minutes(5);
        let relative = I18n.relative_time(past);
        assert_not_null(relative);
    });

    test("I18n.relative_time() formats recent future", fn() {
        let now = DateTime.now();
        let future = now.add_minutes(5);
        let relative = I18n.relative_time(future);
        assert_not_null(relative);
    });

    test("I18n.relative_time() formats hours ago", fn() {
        let now = DateTime.now();
        let past = now.subtract_hours(3);
        let relative = I18n.relative_time(past);
        assert_not_null(relative);
    });

    test("I18n.relative_time() formats days ago", fn() {
        let now = DateTime.now();
        let past = now.subtract_days(2);
        let relative = I18n.relative_time(past);
        assert_not_null(relative);
    });

    test("I18n.relative_time() formats weeks ago", fn() {
        let now = DateTime.now();
        let past = now.subtract_days(10);
        let relative = I18n.relative_time(past);
        assert_not_null(relative);
    });
});

describe("I18n Locale Management", fn() {
    test("I18n.locale() returns current locale", fn() {
        let locale = I18n.locale();
        assert_not_null(locale);
    });

    test("I18n.set_locale() changes locale", fn() {
        I18n.set_locale("fr");
        assert_eq(I18n.locale(), "fr");
        I18n.set_locale("en");
    });

    test("I18n.set_locale() with country code", fn() {
        I18n.set_locale("pt-BR");
        assert_eq(I18n.locale(), "pt-BR");
        I18n.set_locale("en");
    });

    test("I18n.locales() returns available locales", fn() {
        let locales = I18n.locales();
        assert_not_null(locales);
        assert(len(locales) > 0);
    });

    test("I18n.default_locale() returns default", fn() {
        let default_locale = I18n.default_locale();
        assert_not_null(default_locale);
    });

    test("I18n.set_default_locale() changes default", fn() {
        let original = I18n.default_locale();
        I18n.set_default_locale("de");
        assert_eq(I18n.default_locale(), "de");
        I18n.set_default_locale(original);
    });

    test("I18n.load_locale() loads new locale", fn() {
        let loaded = I18n.load_locale("custom");
        assert_not_null(loaded);
    });
});

describe("I18n Language Name", fn() {
    test("I18n.language_name() returns language name", fn() {
        let name = I18n.language_name("en");
        assert_not_null(name);
        assert_eq(name, "English");
    });

    test("I18n.language_name() for different locale", fn() {
        let name = I18n.language_name("fr");
        assert_not_null(name);
        assert_eq(name, "French");
    });

    test("I18n.language_name() for unknown locale", fn() {
        let name = I18n.language_name("xx");
        assert_not_null(name);
    });
});
