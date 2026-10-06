# I18n.format_currency(amount, currency, locale?): symbol placement, decimal
# and thousands separators per locale, and rounding that carries.

describe("I18n.format_currency") do
  before_each() do
    I18n.set_locale("en")
  end

  # The locale is process-wide and outlives this file.
  after_each() do
    I18n.set_locale("en")
  end

  describe("symbols and separators") do
    test("EUR in French: symbol after, decimal comma") do
      assert_eq(I18n.format_currency(9.1, "EUR", "fr"), "9,10 €")
    end

    test("USD in English: symbol before, decimal point") do
      assert_eq(I18n.format_currency(9.1, "USD", "en"), "$9.10")
    end

    test("groups thousands") do
      assert_eq(I18n.format_currency(1234.56, "USD", "en"), "$1,234.56")
      assert_eq(I18n.format_currency(1234.56, "EUR", "fr"), "1.234,56 €")
      assert_eq(I18n.format_currency(1234567.5, "USD", "en"), "$1,234,567.50")
    end

    test("GBP and JPY symbols") do
      assert_eq(I18n.format_currency(10, "GBP", "en"), "£10")
      assert_eq(I18n.format_currency(2500, "JPY", "en"), "¥2,500")
    end

    test("the locale, not the currency, places the symbol") do
      assert_eq(I18n.format_currency(10.5, "USD", "fr"), "10,50 $")
      assert_eq(I18n.format_currency(10.5, "EUR", "de"), "10,50 €")
      assert_eq(I18n.format_currency(1234.5, "GBP", "de"), "1.234,50 £")
    end

    test("an unknown currency prints its code") do
      assert_eq(I18n.format_currency(10, "XYZ", "en"), "XYZ10")
    end

    test("defaults to the current locale") do
      assert_eq(I18n.format_currency(10.5, "EUR"), "€10.50")
      I18n.set_locale("fr")
      assert_eq(I18n.format_currency(1.5, "EUR"), "1,50 €")
    end
  end

  describe("amounts") do
    test("a whole amount has no decimals") do
      assert_eq(I18n.format_currency(10, "EUR", "fr"), "10 €")
      assert_eq(I18n.format_currency(0, "EUR", "fr"), "0 €")
    end

    test("rounds half a cent up") do
      assert_eq(I18n.format_currency(0.005, "USD", "en"), "$0.01")
    end

    test("a negative amount puts the sign before the symbol") do
      pending("bug: format_currency(-9.5, \"USD\", \"en\") gives \"$-9.50\", not \"-$9.50\"")
      assert_eq(I18n.format_currency(-9.5, "USD", "en"), "-$9.50")
    end
  end

  # Regression: 8.33 * 1.20 = 9.996 formatted as "9,100 €" because the
  # fraction rounded up to 100 while the integer part stayed at 9.
  describe("rounding carries into the integer part") do
    test("8.33 * 1.20 carries to 10 €") do
      assert_eq(I18n.format_currency(8.33 * 1.2, "EUR", "fr"), "10 €")
    end

    test("9.996 carries to 10 €") do
      assert_eq(I18n.format_currency(9.996, "EUR", "fr"), "10 €")
    end

    test("0.999 rounds to 1 €") do
      assert_eq(I18n.format_currency(0.999, "EUR", "fr"), "1 €")
    end

    test("999.999 carries past the thousands boundary") do
      assert_eq(I18n.format_currency(999.999, "EUR", "fr"), "1.000 €")
    end
  end

  describe("errors") do
    test("raises on a non-number amount") do
      assert_raises("I18n.format_currency expects a number") do
        I18n.format_currency(["x"][0], "EUR", "fr")
      end
    end

    test("raises on a non-string currency") do
      assert_raises("I18n.format_currency expects a currency code") do
        I18n.format_currency(1, [1][0], "fr")
      end
    end
  end
end
