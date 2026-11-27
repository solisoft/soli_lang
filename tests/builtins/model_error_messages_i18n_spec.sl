# Validation messages follow the request's locale: Soli ships them for en,
# fr, de, es, it and pt, and an application's config/locales keys override
# them one by one (not exercised here: no locale files in this tree).

class I18nWidget < Model
  validates("name", {"presence": true, "min_length": 3})
  validates("kind", {"type": "int"})
  validates("slug", {"format": "^[a-z-]+$", "message": :slug_format})
  validates("code", {"format": "^[A-Z]+$", "message": "must be upper case"})
  validates("size", {"custom": "check_size"})

  def check_size(size)
    return :too_big if size && size > 10

    true
  end
end

def messages_for(record, field)
  record._errors.filter { |e| e["field"] == field }.map { |e| e["message"] }
end

describe("Validation messages in the request's locale") do
  after_each() do
    I18n.set_locale("en")
  end

  test("English stays as it was") do
    widget = I18nWidget.new({"name": "", "kind": "x"})
    assert(!widget.save)
    assert_eq(messages_for(widget, "name")[0], "can't be blank")
    assert_eq(messages_for(widget, "kind"), ["must be an int"])
  end

  test("French reads the built-in catalog") do
    I18n.set_locale("fr")
    widget = I18nWidget.new({"name": "ab", "kind": "x"})
    widget.save
    assert_eq(messages_for(widget, "name"), ["est trop court (au moins 3 caractères)"])
    assert_eq(messages_for(widget, "kind"), ["doit être de type entier"])
  end

  test("every shipped language has its sentence") do
    expected = {
      "de": "muss ausgefüllt werden",
      "es": "no puede estar en blanco",
      "it": "non può essere lasciato in bianco",
      "pt": "não pode ficar em branco"
    }
    expected.each do |locale, sentence|
      I18n.set_locale(locale)
      widget = I18nWidget.new({"name": ""})
      widget.save
      assert_eq(messages_for(widget, "name")[0], sentence)
    end
  end

  test("a region reads its language, an unknown language reads English") do
    I18n.set_locale("pt-BR")
    widget = I18nWidget.new({"name": ""})
    widget.save
    assert_eq(messages_for(widget, "name")[0], "não pode ficar em branco")

    I18n.set_locale("nl")
    other = I18nWidget.new({"name": ""})
    other.save
    assert_eq(messages_for(other, "name")[0], "can't be blank")
  end

  test("message: a String replaces the text, a Symbol names a key") do
    I18n.set_locale("fr")
    widget = I18nWidget.new({"name": "abc", "slug": "Not A Slug", "code": "abc"})
    widget.save
    assert_eq(messages_for(widget, "code"), ["must be upper case"])
    # No translation for :slug_format anywhere: the key shows.
    assert_eq(messages_for(widget, "slug"), ["slug_format"])
  end

  test("a custom validator may return a key") do
    widget = I18nWidget.new({"name": "abc", "size": 12})
    widget.save
    assert_eq(messages_for(widget, "size"), ["too_big"])
  end

  test("full_messages name the field, in the request's locale") do
    I18n.set_locale("fr")
    widget = I18nWidget.new({"name": "abc", "kind": "x"})
    widget.save
    assert_eq(widget.full_messages, ["Kind doit être de type entier"])
  end

  test("names fall back to the field and class read as words") do
    assert_eq(I18nWidget.human_attribute_name("first_name"), "First name")
    assert_eq(I18nWidget.human_name, "I18n widget")
    assert_eq(I18nWidget.human_name(2), "I18n widget")
  end

  test("I18n.translate reads the built-in catalog") do
    assert_eq(I18n.translate("errors.messages.taken", "de"), "ist bereits vergeben")
    assert_eq(I18n.translate("errors.messages.blank", "fr"), "doit être rempli(e)")
  end
end
