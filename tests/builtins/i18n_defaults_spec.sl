# I18n.default_locale / I18n.set_default_locale: the process-wide fallback a
# request starts from and a lookup falls back to. Each test restores the
# original default and clears the current locale.

original_default_locale = nil

TRANSLATIONS = {
  "en.greeting": "Hello",
  "fr.greeting": "Bonjour",
  "fr.items_one": "un article",
  "fr.items_other": "{count} articles",
  "en.items_one": "one item",
  "en.items_other": "{count} items"
}

describe("I18n default locale") do
  before_each() do
    original_default_locale = I18n.default_locale
    I18n.set_locale("")
  end

  after_each() do
    I18n.set_default_locale(original_default_locale)
    I18n.set_locale("")
  end

  context("reading and setting") do
    test("is en until something sets it") do
      assert_eq(I18n.default_locale, "en")
    end

    test("set_default_locale returns the new locale") do
      assert_eq(I18n.set_default_locale("fr"), "fr")
      assert_eq(I18n.default_locale, "fr")
    end

    test("refuses a non-string locale") do
      assert_raises("I18n.set_default_locale expects a string, got int") do
        I18n.set_default_locale(5)
      end
      assert_raises("I18n.set_default_locale expects a string, got null") do
        I18n.set_default_locale(nil)
      end
      assert_eq(I18n.default_locale, "en")
    end
  end

  context("the current locale") do
    test("follows the default while no locale is set") do
      I18n.set_default_locale("fr")
      assert_eq(I18n.locale, "fr")
    end

    test("an explicit set_locale wins over the default") do
      I18n.set_default_locale("fr")
      I18n.set_locale("de")
      assert_eq(I18n.locale, "de")
      assert_eq(I18n.default_locale, "fr")
    end

    test("set_locale with an empty string falls back to the default again") do
      I18n.set_locale("de")
      I18n.set_default_locale("fr")
      I18n.set_locale("")
      assert_eq(I18n.locale, "fr")
    end
  end

  context("lookup fallback") do
    test("a missing key in the asked locale falls back to the default") do
      I18n.set_default_locale("fr")
      assert_eq(I18n.translate("greeting", "de", TRANSLATIONS), "Bonjour")
    end

    test("the fallback follows a change of default") do
      I18n.set_default_locale("fr")
      I18n.set_default_locale("en")
      assert_eq(I18n.translate("greeting", "de", TRANSLATIONS), "Hello")
    end

    test("a key missing everywhere comes back as the key") do
      I18n.set_default_locale("fr")
      assert_eq(I18n.translate("missing", "de", TRANSLATIONS), "missing")
    end

    test("a translation without a locale uses the default") do
      I18n.set_default_locale("fr")
      assert_eq(I18n.translate("greeting", nil, TRANSLATIONS), "Bonjour")
    end

    test("plural falls back with the default locale's plural rules") do
      I18n.set_default_locale("fr")
      assert_eq(I18n.plural("items", 0, "de", TRANSLATIONS), "un article")
      assert_eq(I18n.plural("items", 3, "de", TRANSLATIONS), "3 articles")
    end
  end
end
