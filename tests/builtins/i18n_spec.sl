# I18n: the current locale, translate, CLDR plural, format_number, format_date
# and the per-locale table cache. Currency formatting is in i18n_currency_spec.sl.

# Legacy translation tables: "<locale>.<key>" => text.
GREETINGS = {"en.greeting": "Hello", "fr.greeting": "Bonjour", "de.greeting": "Hallo"}

ITEMS = {
  "en.item_zero": "No items",
  "en.item_one": "1 item",
  "en.item_other": "{count} items",
  "fr.item_one": "{count} article",
  "fr.item_other": "{count} articles",
  "ru.item_one": "one",
  "ru.item_few": "few",
  "ru.item_many": "many",
  "ru.item_other": "other",
  "ja.item_one": "never used",
  "ja.item_other": "{count} ko"
}

# 2026-01-15 11:00 UTC. format_date reads the local zone; this is the 15th from
# UTC-11 to UTC+12, and a day past 12 tells day and month order apart.
JANUARY_15 = 1768474800

describe("I18n") do
  before_each() do
    I18n.set_locale("en")
  end

  # The locale is process-wide and outlives this file.
  after_each() do
    I18n.set_locale("en")
  end

  describe("locale and set_locale") do
    test("locale is a string, en by default here") do
      assert_eq(I18n.locale, "en")
    end

    test("set_locale changes the locale and returns it") do
      assert_eq(I18n.set_locale("fr"), "fr")
      assert_eq(I18n.locale, "fr")
    end

    test("the global set_locale is the same setting") do
      assert_eq(set_locale("de"), "de")
      assert_eq(I18n.locale, "de")
    end

    test("set_locale raises on a non-string") do
      assert_raises("I18n.set_locale expects a string, got int") do
        I18n.set_locale([123][0])
      end
    end
  end

  describe("translate") do
    test("looks the key up in the given locale") do
      assert_eq(I18n.translate("greeting", "en", GREETINGS), "Hello")
      assert_eq(I18n.translate("greeting", "fr", GREETINGS), "Bonjour")
      assert_eq(I18n.translate("greeting", "de", GREETINGS), "Hallo")
    end

    test("a nil locale means the current one") do
      I18n.set_locale("fr")
      assert_eq(I18n.translate("greeting", nil, GREETINGS), "Bonjour")
    end

    test("falls back to en when the locale has no entry") do
      assert_eq(I18n.translate("greeting", "es", {"en.greeting": "Hello"}), "Hello")
    end

    test("dotted keys are looked up whole") do
      assert_eq(I18n.translate("nav.home", "en", {"en.nav.home": "Home"}), "Home")
    end

    test("returns the key when nothing resolves") do
      assert_eq(I18n.translate("unknown_key", "en", {}), "unknown_key")
    end

    # The test environment loads no translation files, so the store is empty.
    test("with only a key, reads the empty store and returns the key") do
      assert_eq(I18n.translate("missing.key"), "missing.key")
    end

    test("a values hash as second argument means the current locale") do
      assert_eq(I18n.translate("missing", {"name": "Alice"}), "missing")
    end

    # A third-argument hash whose keys are not all dotted is interpolation
    # values, not a legacy table.
    test("a non-dotted third-argument hash is values, not a table") do
      assert_eq(I18n.translate("missing", "en", {"name": "Alice"}), "missing")
    end

    test("raises on a non-string key") do
      assert_raises("I18n.translate expects a key string") do
        I18n.translate([123][0], "en", {})
      end
    end

    test("raises on a second argument that is neither locale nor values") do
      assert_raises("second arg must be a locale string or values hash, got int") do
        I18n.translate("greeting", [42][0])
      end
    end

    test("raises on a third argument that is not a hash") do
      assert_raises("third arg must be a hash, got string") do
        I18n.translate("greeting", "en", ["not a hash"][0])
      end
    end
  end

  describe("plural") do
    test("English: zero, one, other, with the count interpolated") do
      assert_eq(I18n.plural("item", 0, "en", ITEMS), "No items")
      assert_eq(I18n.plural("item", 1, "en", ITEMS), "1 item")
      assert_eq(I18n.plural("item", 5, "en", ITEMS), "5 items")
    end

    test("French: 0 is singular when no _zero key exists") do
      assert_eq(I18n.plural("item", 0, "fr", ITEMS), "0 article")
      assert_eq(I18n.plural("item", 3, "fr", ITEMS), "3 articles")
    end

    test("Russian: one, few, many") do
      assert_eq(I18n.plural("item", 1, "ru", ITEMS), "one")
      assert_eq(I18n.plural("item", 3, "ru", ITEMS), "few")
      assert_eq(I18n.plural("item", 5, "ru", ITEMS), "many")
      assert_eq(I18n.plural("item", 21, "ru", ITEMS), "one")
    end

    test("Japanese: always other") do
      assert_eq(I18n.plural("item", 1, "ja", ITEMS), "1 ko")
    end

    test("a locale without the key falls back to en") do
      assert_eq(I18n.plural("item", 2, "de", ITEMS), "2 items")
    end

    test("a nil locale means the current one") do
      I18n.set_locale("fr")
      assert_eq(I18n.plural("item", 3, nil, ITEMS), "3 articles")
    end

    test("returns the key when nothing resolves") do
      assert_eq(I18n.plural("items", 5, "en", {}), "items")
    end

    test("raises on a non-number count") do
      assert_raises("I18n.plural expects a number") do
        I18n.plural("items", ["five"][0], "en", {})
      end
    end

    test("raises on a third argument that is neither locale nor values") do
      assert_raises("third arg must be a locale string or values hash, got int") do
        I18n.plural("items", 5, [99][0])
      end
    end
  end

  describe("format_number") do
    test("en uses a decimal point") do
      assert_eq(I18n.format_number(1234.56, "en"), "1234.56")
    end

    test("fr, de and es use a decimal comma") do
      assert_eq(I18n.format_number(1234.56, "fr"), "1234,56")
      assert_eq(I18n.format_number(1234.56, "de"), "1234,56")
      assert_eq(I18n.format_number(1234.56, "es"), "1234,56")
    end

    test("an Int has no decimals") do
      assert_eq(I18n.format_number(100, "en"), "100")
      assert_eq(I18n.format_number(0, "fr"), "0")
    end

    test("keeps the Float's digits and sign") do
      assert_eq(I18n.format_number(9.99, "en"), "9.99")
      assert_eq(I18n.format_number(-1234.5, "fr"), "-1234,5")
    end

    test("an unknown locale formats like en") do
      assert_eq(I18n.format_number(1234.56, "xx"), "1234.56")
    end

    test("defaults to the current locale") do
      I18n.set_locale("fr")
      assert_eq(I18n.format_number(1.5), "1,5")
    end

    test("raises on a non-number") do
      assert_raises("I18n.format_number expects a number") do
        I18n.format_number(["not a number"][0])
      end
    end
  end

  describe("format_date") do
    test("en is MM/DD/YYYY") do
      assert_eq(I18n.format_date(JANUARY_15, "en"), "01/15/2026")
    end

    test("fr is DD/MM/YYYY") do
      assert_eq(I18n.format_date(JANUARY_15, "fr"), "15/01/2026")
    end

    test("de is DD.MM.YYYY") do
      assert_eq(I18n.format_date(JANUARY_15, "de"), "15.01.2026")
    end

    test("es, ja and unknown locales are ISO") do
      assert_eq(I18n.format_date(JANUARY_15, "es"), "2026-01-15")
      assert_eq(I18n.format_date(JANUARY_15, "ja"), "2026-01-15")
      assert_eq(I18n.format_date(JANUARY_15, "xx"), "2026-01-15")
    end

    test("a nil or missing locale means the current one") do
      I18n.set_locale("fr")
      assert_eq(I18n.format_date(JANUARY_15, nil), "15/01/2026")
      assert_eq(I18n.format_date(JANUARY_15), "15/01/2026")
    end

    test("raises on a non-timestamp") do
      assert_raises("I18n.format_date requires a timestamp") do
        I18n.format_date(["not a timestamp"][0])
      end
    end
  end

  # Synthetic locale codes ("zz*") keep these independent of any real locale
  # another test might cache on this thread.
  describe("cache_table and cached_table") do
    test("cached_table is nil before anything is cached") do
      assert_null(I18n.cached_table("zz-fresh"))
    end

    test("cache_table stores the table and returns it") do
      table = {"greeting": "hi", "bye": "ciao"}
      assert_eq(I18n.cache_table("zz1", table), table)
      assert_eq(I18n.cached_table("zz1"), {"greeting": "hi", "bye": "ciao"})
    end

    test("locales are cached independently") do
      I18n.cache_table("zz1", {"greeting": "hi"})
      I18n.cache_table("zz2", {"k": "two"})
      assert_eq(I18n.cached_table("zz1")["greeting"], "hi")
      assert_eq(I18n.cached_table("zz2")["k"], "two")
    end

    test("caching again replaces the table") do
      I18n.cache_table("zz4", {"k": "old"})
      I18n.cache_table("zz4", {"k": "new"})
      assert_eq(I18n.cached_table("zz4"), {"k": "new"})
    end

    # The `tr(key)` helper shape: the cached table goes to every translate call.
    # It is classified once, when cached, and read in place.
    test("translate and plural read a cached legacy table") do
      table = I18n.cache_table("zz5", {
        "en.greeting": "Hello",
        "fr.greeting": "Bonjour",
        "en.apple_one": "1 apple",
        "en.apple_other": "{count} apples"
      })
      assert_eq(I18n.translate("greeting", "fr", I18n.cached_table("zz5")), "Bonjour")
      assert_eq(I18n.translate("greeting", "es", table), "Hello")
      assert_eq(I18n.translate("missing", "en", table), "missing")
      assert_eq(I18n.plural("apple", 1, "en", table), "1 apple")
      assert_eq(I18n.plural("apple", 3, "en", table), "3 apples")
    end

    test("a cached hash whose keys are not all dotted stays values") do
      values = I18n.cache_table("zz6", {"en.greeting": "Hello", "name": "Alice"})
      assert_eq(I18n.translate("greeting", "en", values), "greeting")
    end

    test("cached_table raises on a non-string locale") do
      assert_raises("I18n.cached_table expects a locale string, got int") do
        I18n.cached_table([123][0])
      end
    end

    test("cache_table raises on a non-hash table") do
      assert_raises("I18n.cache_table expects a hash table, got string") do
        I18n.cache_table("zz3", ["not-a-hash"][0])
      end
    end
  end
end
