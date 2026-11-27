# Translated model fields: `Model.translate("title")` keeps one value per
# locale in `translated_fields`, reads the current locale's, and falls back to
# the plain column.

class TranslatedArticle < Model
end
TranslatedArticle.translate("title")

class TranslatedPage < Model
  translates "title"
end

describe("Model.translate") do
  before_each() do
    requires_solidb()
    I18n.set_locale("en")
  end

  after_each() do
    I18n.set_locale("en")
    TranslatedArticle.delete_all()
  end

  test("an assignment reads back in its locale before save") do
    article = TranslatedArticle.new({"title": "Hello"})
    I18n.set_locale("fr")
    article.title = "Bonjour"
    assert_eq(article.title, "Bonjour")
    I18n.set_locale("en")
    assert_eq(article.title, "Hello")
  end

  test("save persists each locale's translation") do
    article = TranslatedArticle.create({"title": "Hello"})
    I18n.set_locale("fr")
    article.title = "Bonjour"
    assert(article.save)

    reloaded = TranslatedArticle.find(article._key)
    assert_eq(reloaded.title, "Bonjour")
    I18n.set_locale("de")
    assert_eq(reloaded.title, "Hello")
  end

  test("a write keeps the locale it was made in, whatever the locale at save") do
    article = TranslatedArticle.create({"title": "Hello"})
    I18n.set_locale("fr")
    article.title = "Bonjour"
    I18n.set_locale("en")
    article.save

    I18n.set_locale("fr")
    assert_eq(TranslatedArticle.find(article._key).title, "Bonjour")
  end

  test("a string filter reaches a translation by its path") do
    article = TranslatedArticle.create({"title": "Hello"})
    I18n.set_locale("fr")
    article.title = "Bonjour"
    article.save

    found = TranslatedArticle.where("doc.translated_fields.title.fr == @t", {"t": "Bonjour"})
    assert_eq(found.count, 1)
    assert_eq(TranslatedArticle.where({"title": "Bonjour"}).count, 0)
  end

  test("translate in the class body declares the fields") do
    assert_eq(TranslatedPage.new({"title": "Hello"}).title, "Hello")
    I18n.set_locale("fr")
    page = TranslatedPage.new({"title": "Hello"})
    page.title = "Bonjour"
    assert_eq(page.title, "Bonjour")
  end

  test("update keeps the translations already stored") do
    article = TranslatedArticle.create({"title": "Hello"})
    I18n.set_locale("fr")
    article.title = "Bonjour"
    article.save

    fresh = TranslatedArticle.find(article._key)
    I18n.set_locale("es")
    fresh.title = "Hola"
    assert(fresh.update)

    stored = TranslatedArticle.find(article._key)
    assert_eq(stored.translated_fields["title"]["fr"], "Bonjour")
    assert_eq(stored.translated_fields["title"]["es"], "Hola")
    assert_eq(stored["title"], "Hello")
  end
end

def seeded(title, translations)
  TranslatedArticle.create({"title": title, "translated_fields": {"title": translations}})
end

describe("Translated fields: chains, helpers and queries") do
  before_each() do
    requires_solidb()
    I18n.set_locale("en")
  end

  after_each() do
    I18n.set_locale("en")
    I18n.set_fallbacks({})
    TranslatedArticle.delete_all()
  end

  test("a region reads its language's translation before the column") do
    article = seeded("Hello", {"fr": "Bonjour"})
    I18n.set_locale("fr-CA")
    assert_eq(TranslatedArticle.find(article._key).title, "Bonjour")
  end

  test("declared fallbacks are read before the column") do
    article = seeded("Hello", {"es": "Hola"})
    I18n.set_fallbacks({"ca": ["es"]})
    assert_eq(I18n.fallbacks(), {"ca": ["es"]})
    assert_eq(I18n.locale_chain("ca"), ["ca", "es", "en"])
    I18n.set_locale("ca")
    assert_eq(TranslatedArticle.find(article._key).title, "Hola")
  end

  test("translation, translations and set_translation") do
    article = seeded("Hello", {"fr": "Bonjour"})
    article.set_translation("title", "de", "Hallo")
    assert_eq(article.translation("title", "fr"), "Bonjour")
    assert_eq(article.translation("title", "de"), "Hallo")
    assert_eq(article.translation("title", "it"), nil)
    assert_eq(article.translations("title"), {"en": "Hello", "fr": "Bonjour", "de": "Hallo"})
    article.save
    assert_eq(TranslatedArticle.find(article._key).translations("title"), {"en": "Hello", "fr": "Bonjour", "de": "Hallo"})
    assert_raises("not a translated field") do
      article.translation("body", "fr")
    end
  end

  test("update(hash) writes a translated field in the request's locale") do
    article = seeded("Hello", {"fr": "Bonjour"})
    I18n.set_locale("es")
    assert(article.update({"title": "Hola"}))
    stored = TranslatedArticle.find(article._key)
    assert_eq(stored["title"], "Hello")
    assert_eq(stored.translations("title"), {"en": "Hello", "fr": "Bonjour", "es": "Hola"})
  end

  test("a translated_fields hash merges locale by locale") do
    article = seeded("Hello", {"fr": "Bonjour", "es": "Hola"})
    assert(article.update({"translated_fields": {"title": {"fr": "Salut"}}}))
    stored = TranslatedArticle.find(article._key)
    assert_eq(stored.translations("title"), {"en": "Hello", "fr": "Salut", "es": "Hola"})
  end

  test("where_translated compares what a reader in the locale sees") do
    seeded("Hello", {"fr": "Bonjour"})
    seeded("Bonjour", {})
    seeded("Other", {"fr": "Autre"})
    I18n.set_locale("fr")
    assert_eq(TranslatedArticle.where_translated({"title": "Bonjour"}).count, 2)
    I18n.set_locale("en")
    assert_eq(TranslatedArticle.where_translated({"title": "Bonjour"}).count, 1)
    I18n.set_locale("fr")
    chained = TranslatedArticle.where({"title": "Hello"}).where_translated({"title": "Bonjour"})
    assert_eq(chained.count, 1)
  end

  test("order_translated sorts by what a reader in the locale sees") do
    seeded("Zebra", {"fr": "Abeille"})
    seeded("Mouse", {})
    I18n.set_locale("fr")
    titles = TranslatedArticle.order_translated("title").all.map { |a| a.title }
    assert_eq(titles, ["Abeille", "Mouse"])
    I18n.set_locale("en")
    descending = TranslatedArticle.where({"title": {"ne": "x"}}).order_translated("title", "desc").all
    assert_eq(descending.map { |a| a.title }, ["Zebra", "Mouse"])
  end

  test("order_translated ignores case and accents, as a reader would") do
    seeded("a", {"fr": "Zèbre"})
    seeded("b", {"fr": "école"})
    seeded("c", {"fr": "Abricot"})
    seeded("d", {"fr": "banane"})
    seeded("e", {"fr": "Éclair"})
    seeded(nil, {})
    I18n.set_locale("fr")
    titles = TranslatedArticle.order_translated("title").all.map { |a| a.title }
    assert_eq(titles, [nil, "Abricot", "banane", "Éclair", "école", "Zèbre"])
    descending = TranslatedArticle.order_translated("title", "desc").all.map { |a| a.title }
    assert_eq(descending, ["Zèbre", "école", "Éclair", "banane", "Abricot", nil])
  end

  test("order on a translated field sorts the translation, not the column") do
    seeded("apple", {"fr": "Zèbre"})
    seeded("bear", {"fr": "abricot"})
    I18n.set_locale("fr")
    assert_eq(TranslatedArticle.order("title").all.map { |a| a.title }, ["abricot", "Zèbre"])
    chained = TranslatedArticle.where({"title": {"ne": "x"}}).order({"title": "desc"}).all
    assert_eq(chained.map { |a| a.title }, ["Zèbre", "abricot"])
    assert(TranslatedArticle.order("title").to_query.includes?("LOWER(UNACCENT("))
    assert(!TranslatedArticle.order("_key").to_query.includes?("UNACCENT"))
  end

  test("missing_translations lists the records a locale lacks") do
    seeded("Hello", {"fr": "Bonjour"})
    seeded("World", {})
    assert_eq(TranslatedArticle.missing_translations("fr").all.map { |a| a["title"] }, ["World"])
    assert_eq(TranslatedArticle.missing_translations("de").count, 2)
  end
end

describe("The default locale's value is the column") do
  before_each() do
    requires_solidb()
    I18n.set_locale("en")
  end

  after_each() do
    I18n.set_locale("en")
    TranslatedArticle.delete_all()
  end

  test("a write in the default locale updates the column") do
    article = TranslatedArticle.create({"title": "Car"})
    article.title = "Automobile"
    assert(article.save)
    stored = TranslatedArticle.find(article._key)
    assert_eq(stored["title"], "Automobile")
    assert_null(stored.translation("title", "fr"))
    assert_eq(TranslatedArticle.where({"title": "Automobile"}).count, 1)

    article.update({"title": "Vehicle"})
    assert_eq(TranslatedArticle.find(article._key)["title"], "Vehicle")
    assert_eq(TranslatedArticle.where({"title": "Vehicle"}).count, 1)
  end

  test("a default-locale write drops the entry that hid the column") do
    legacy = seeded("Car", {"en": "Vehicle", "fr": "Voiture"})
    assert_eq(TranslatedArticle.find(legacy._key).title, "Vehicle")
    record = TranslatedArticle.find(legacy._key)
    record.update({"title": "Truck"})
    stored = TranslatedArticle.find(legacy._key)
    assert_eq(stored["title"], "Truck")
    assert_eq(stored.title, "Truck")
    assert_eq(stored["translated_fields"]["title"].keys, ["fr"])
  end

  test("a record created in another locale keeps that translation") do
    I18n.set_locale("fr")
    article = TranslatedArticle.create({"title": "Voiture"})
    I18n.set_locale("en")
    stored = TranslatedArticle.find(article._key)
    assert_eq(stored.title, "Voiture")
    stored.update({"title": "Car"})
    assert_eq(TranslatedArticle.find(article._key).title, "Car")
    I18n.set_locale("fr")
    assert_eq(TranslatedArticle.find(article._key).title, "Voiture")
  end

  test("translation and translations read the column for the default locale") do
    article = seeded("Hello", {"fr": "Bonjour"})
    stored = TranslatedArticle.find(article._key)
    assert_eq(stored.translation("title", "en"), "Hello")
    assert_eq(stored.translations("title"), {"en": "Hello", "fr": "Bonjour"})
    assert_eq(TranslatedArticle.missing_translations("en").count, 0)
    assert_eq(TranslatedArticle.missing_translations("fr").count, 0)
    stored.set_translation("title", "en", "Hi")
    stored.save
    assert_eq(TranslatedArticle.find(article._key)["title"], "Hi")
  end

  test("sync_translated_columns moves old default-locale entries into the column") do
    legacy = seeded("Car", {"en": "Vehicle", "fr": "Voiture"})
    current = seeded("Bike", {"fr": "Vélo"})
    assert_eq(TranslatedArticle.sync_translated_columns, 1)
    stored = TranslatedArticle.find(legacy._key)
    assert_eq(stored["title"], "Vehicle")
    assert_eq(stored["translated_fields"]["title"], {"fr": "Voiture"})
    assert_eq(TranslatedArticle.find(current._key)["title"], "Bike")
    assert_eq(TranslatedArticle.where({"title": "Vehicle"}).count, 1)
    assert_eq(TranslatedArticle.sync_translated_columns, 0)
  end
end
