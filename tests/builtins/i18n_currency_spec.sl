// ============================================================================
// I18n.format_currency Test Suite
// ============================================================================

describe("I18n.format_currency basics", fn() {
    test("formats EUR with French locale (symbol after, comma decimal)", fn() {
        assert_eq(I18n.format_currency(9.10, "EUR", "fr"), "9,10 €");
    });

    test("formats USD with English locale (symbol before, dot decimal)", fn() {
        assert_eq(I18n.format_currency(9.10, "USD", "en"), "$9.10");
    });

    test("uses thousands separator", fn() {
        assert_eq(I18n.format_currency(1234.56, "USD", "en"), "$1,234.56");
        assert_eq(I18n.format_currency(1234.56, "EUR", "fr"), "1.234,56 €");
    });

    test("integer amount has no decimals", fn() {
        assert_eq(I18n.format_currency(10, "EUR", "fr"), "10 €");
    });
});

describe("I18n.format_currency carry-over (regression)", fn() {
    // Regression: 8.33 * 1.20 = 9.996 used to format as "9,100 €"
    // because frac_part rounded up to 100 while int_part stayed at 9.
    test("8.33 * 1.20 carries to 10 €", fn() {
        assert_eq(I18n.format_currency(8.33 * 1.20, "EUR", "fr"), "10 €");
    });

    test("9.996 carries to 10 €", fn() {
        assert_eq(I18n.format_currency(9.996, "EUR", "fr"), "10 €");
    });

    test("0.999 rounds to 1 €", fn() {
        assert_eq(I18n.format_currency(0.999, "EUR", "fr"), "1 €");
    });

    test("999.999 carries past thousands boundary", fn() {
        assert_eq(I18n.format_currency(999.999, "EUR", "fr"), "1.000 €");
    });
});
