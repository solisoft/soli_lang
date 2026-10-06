# FormBuilder / form_with. The builder lives in
# src/interpreter/builtins/form_builder.sl as pure Soli, evaluated into the
# template environment at render time. This spec imports an exported fixture
# copy (tests/builtins/_fixtures/form_builder_fixture.sl, which adds `export`
# on top-level declarations only) and checks below that it stays byte-identical
# to the canonical source.
#
# The template-only Rust builtins the builder calls are shimmed here: h/attr
# mirror the template engine's escapers; __soli_form_names mirrors template.rs'
# instance -> {collection, key} helper (hashes -> nil, instances -> naive plural).

def h(value)
  html_escape(value.to_s)
end

def attr(value)
  html_escape(value.to_s).replace("\"", "&quot;")
end

def __soli_form_names(record)
  return nil if record.nil?

  class_name = record.class.to_s
  return nil if class_name == "hash"

  {"collection": class_name.downcase + "s", "key": record._key}
end

import "./_fixtures/form_builder_fixture.sl"

# Model used to exercise form_with's instance-derived URLs
class Post < Model
end

# A builder over a hash record, posting to /posts
def form_for(record)
  form_with(record, {"url": "/posts"})
end

# A record whose title failed validation
def record_with_title_error(message = "can't be blank")
  {"title": "", "_errors": [{"field": "title", "message": message}]}
end

# Reassigned by the before_each hooks that share one builder
form = nil

describe("FormBuilder fixture hygiene") do
  test("the fixture matches the canonical form_builder.sl modulo export keywords") do
    # Paths are relative to the repo root, where `soli test` runs
    source = slurp("src/interpreter/builtins/form_builder.sl")
    fixture = slurp("tests/builtins/_fixtures/form_builder_fixture.sl")
    assert_gt(source.length, 0)
    assert_eq(fixture.replace("export ", ""), source)
  end
end

describe("form_with construction") do
  test("returns a FormBuilder instance") do
    assert_eq(form_for({"title": "Hello"}).class, "FormBuilder")
  end

  test("a hash record posts to the explicit url option") do
    assert_contains(form_for({"title": "Hello"}).open(), "<form action=\"/posts\" method=\"POST\"")
  end

  test("a new model instance derives POST to its collection") do
    assert_contains(form_with(Post.new()).open(), "<form action=\"/posts\" method=\"POST\"")
  end

  test("open embeds the session CSRF token for post forms") do
    assert_contains(form_for({"title": "Hello"}).open(), "<input type=\"hidden\" name=\"_csrf_token\" value=\"")
  end

  test("the get method renders GET and omits the CSRF field") do
    html = form_with({"title": "Hello"}, {"url": "/search", "method": "get"}).open()
    assert_contains(html, "<form action=\"/search\" method=\"GET\"")
    assert_not(html.includes?("name=\"_csrf_token\""))
  end

  test("other methods embed a _method override hidden input") do
    # Built directly so no persisted record (and no DB) is needed
    html = new FormBuilder({"title": "Hi"}, "/posts/42", "patch", {}, "").open()
    assert_contains(html, "<input type=\"hidden\" name=\"_method\" value=\"PATCH\">")
    assert_contains(html, "name=\"_csrf_token\"")
  end

  test("the multipart option adds an enctype attribute") do
    html = form_with({"title": "Hello"}, {"url": "/uploads", "multipart": true}).open()
    assert_contains(html, "enctype=\"multipart/form-data\"")
  end

  test("extra options become form tag attributes") do
    html = form_with({"title": "Hello"}, {"url": "/posts", "class": "big", "id": "new-post"}).open()
    assert_contains(html, "class=\"big\"")
    assert_contains(html, "id=\"new-post\"")
  end

  test("close closes the form") do
    assert_eq(form_for({}).close(), "</form>")
  end
end

describe("field inputs") do
  test("text_field prefills the value from the record") do
    form = form_for({"title": "Hello", "published": true})
    assert_eq(form.text_field("title"), "<input type=\"text\" id=\"title\" name=\"title\" value=\"Hello\">")
  end

  test("text_field passes extra options through, including bare attributes") do
    html = form_for({}).text_field("title", {"placeholder": "Title", "required": true})
    assert_contains(html, "placeholder=\"Title\"")
    assert_contains(html, " required>")
  end

  test("email_field renders an email input") do
    form = form_with({"email": "a@b.c"}, {"url": "/users"})
    assert_eq(form.email_field("email"), "<input type=\"email\" id=\"email\" name=\"email\" value=\"a@b.c\">")
  end

  test("password_field never prefills") do
    html = form_with({"password": "hunter2"}, {"url": "/users"}).password_field("password")
    assert_contains(html, "<input type=\"password\" id=\"password\" name=\"password\"")
    assert_not(html.includes?("value="))
  end

  test("number_field renders a number input") do
    form = form_with({"age": 30}, {"url": "/users"})
    assert_eq(form.number_field("age"), "<input type=\"number\" id=\"age\" name=\"age\" value=\"30\">")
  end

  test("date_field renders a date input") do
    form = form_with({"due_on": "2026-08-23"}, {"url": "/tasks"})
    assert_eq(form.date_field("due_on"), "<input type=\"date\" id=\"due_on\" name=\"due_on\" value=\"2026-08-23\">")
  end

  test("datetime_field renders a datetime-local input") do
    form = form_with({"starts_at": "2026-08-23T10:00"}, {"url": "/events"})
    assert_eq(
      form.datetime_field("starts_at"),
      "<input type=\"datetime-local\" id=\"starts_at\" name=\"starts_at\" value=\"2026-08-23T10:00\">"
    )
  end

  test("hidden_field prefills the value") do
    assert_eq(
      form_for({"token": "abc"}).hidden_field("token"),
      "<input type=\"hidden\" id=\"token\" name=\"token\" value=\"abc\">"
    )
  end

  test("file_field never prefills") do
    html = form_with({"avatar": "x.png"}, {"url": "/users"}).file_field("avatar")
    assert_contains(html, "<input type=\"file\" id=\"avatar\" name=\"avatar\"")
    assert_not(html.includes?("value="))
  end

  test("an explicit value option overrides the record") do
    assert_contains(form_for({"title": "Hello"}).text_field("title", {"value": "Override"}), "value=\"Override\"")
  end

  test("a missing field yields no value attribute") do
    assert_eq(form_for({}).text_field("title"), "<input type=\"text\" id=\"title\" name=\"title\">")
  end

  test("input renders a generic input by type") do
    form = form_with({"qty": 3}, {"url": "/orders"})
    assert_eq(form.input("tel", "qty", nil), "<input type=\"tel\" id=\"qty\" name=\"qty\" value=\"3\">")
  end

  test("values are HTML-escaped") do
    html = form_with({"bio": "<script>"}, {"url": "/users"}).text_field("bio")
    assert_not(html.includes?("<script>"))
    assert_contains(html, "value=\"&lt;script&gt;\"")
  end

  test("a name option overrides the derived name verbatim") do
    form = form_with({"q": "x"}, {"url": "/search"})
    assert_contains(form.text_field("q", {"name": "query"}), "name=\"query\"")
  end
end

describe("text_area") do
  test("renders the record's content between the tags") do
    assert_eq(
      form_for({"body": "line1\nline2"}).text_area("body"),
      "<textarea id=\"body\" name=\"body\">line1\nline2</textarea>"
    )
  end

  test("a value option overrides the content") do
    assert_contains(form_for({"body": "original"}).text_area("body", {"value": "override"}), ">override</textarea>")
  end

  test("escapes the content") do
    assert_contains(form_for({"body": "<b>bold</b>"}).text_area("body"), "&lt;b&gt;bold&lt;/b&gt;")
  end

  test("supports rows and cols options") do
    html = form_for({}).text_area("body", {"rows": 5, "cols": 40})
    assert_contains(html, "rows=\"5\"")
    assert_contains(html, "cols=\"40\"")
  end
end

describe("check_box") do
  test("is checked when the record value is true") do
    assert_eq(
      form_for({"published": true}).check_box("published"),
      "<input type=\"checkbox\" id=\"published\" name=\"published\" value=\"true\" checked>"
    )
  end

  test("is unchecked when the record value is false") do
    html = form_for({"published": false}).check_box("published")
    assert_contains(html, "type=\"checkbox\"")
    assert_not(html.includes?("checked"))
  end

  test("is checked when the record value is the string true") do
    assert_contains(form_for({"published": "true"}).check_box("published"), " checked>")
  end

  test("is unchecked when the field is missing") do
    assert_not(form_for({}).check_box("published").includes?("checked"))
  end
end

describe("radio_button") do
  before_each() do
    form = form_with({"role": "admin"}, {"url": "/users"})
  end

  test("is checked when the value matches the record") do
    assert_eq(
      form.radio_button("role", "admin"),
      "<input type=\"radio\" id=\"role_admin\" name=\"role\" value=\"admin\" checked>"
    )
  end

  test("is unchecked when the value differs from the record") do
    html = form.radio_button("role", "guest")
    assert_contains(html, "<input type=\"radio\" id=\"role_guest\" name=\"role\" value=\"guest\"")
    assert_not(html.includes?("checked"))
  end

  test("matches numbers through string comparison") do
    assert_contains(form_with({"size": 2}, {"url": "/shirts"}).radio_button("size", 2), " checked>")
  end
end

describe("select") do
  test("renders options from string choices and selects the record's value") do
    html = form_with({"color": "blue"}, {"url": "/cars"}).select("color", ["red", "green", "blue"])
    assert_contains(html, "<select id=\"color\" name=\"color\">")
    assert_contains(html, "<option value=\"red\">red</option>")
    assert_contains(html, "<option value=\"blue\" selected>blue</option>")
    assert_not(html.includes?("<option value=\"red\" selected"))
  end

  test("accepts label/value pair choices") do
    html = form_with({"size": "m"}, {"url": "/shirts"}).select("size", [["Medium", "m"], ["Large", "l"]])
    assert_contains(html, "<option value=\"m\" selected>Medium</option>")
    assert_contains(html, "<option value=\"l\">Large</option>")
  end

  test("the multiple option appends [] to the name and adds the attribute") do
    html = form_for({"tags": "a"}).select("tags", ["a", "b"], {"multiple": true})
    assert_contains(html, "name=\"tags[]\"")
    assert_contains(html, " multiple")
  end

  test("escapes option labels and values") do
    html = form_with({"pick": ""}, {"url": "/things"}).select("pick", [["<b>A</b>", "<x>"]])
    assert_not(html.includes?("<option value=\"<x>\">"))
    assert_contains(html, "&lt;b&gt;A&lt;/b&gt;")
  end
end

describe("submit") do
  before_each() do
    form = form_for({})
  end

  test("defaults to a Save caption") do
    assert_eq(form.submit(), "<button type=\"submit\">Save</button>")
  end

  test("uses the provided caption") do
    assert_eq(form.submit("Create Post"), "<button type=\"submit\">Create Post</button>")
  end

  test("escapes the caption and passes extra attributes") do
    html = form.submit("<Save>", {"class": "btn"})
    assert_contains(html, "class=\"btn\"")
    assert_not(html.includes?("<Save>"))
    assert_contains(html, "&lt;Save&gt;")
  end
end

describe("error rendering") do
  test("errors_for renders a span per message for the field") do
    record = {"title": "", "_errors": [
      {"field": "title", "message": "cannot be blank"},
      {"field": "body", "message": "too short"}
    ]}
    assert_eq(form_for(record).errors_for("title"), "<span class=\"field-error-message\">cannot be blank</span>")
  end

  test("field_errors collects the messages for one field") do
    record = {"_errors": [
      {"field": "title", "message": "can't be blank"},
      {"field": "title", "message": "too long"},
      {"field": "body", "message": "too short"}
    ]}
    assert_eq(form_for(record).field_errors("title"), ["can't be blank", "too long"])
  end

  test("field_errors is empty for valid records and nil records") do
    assert_eq(form_for({"title": "ok"}).field_errors("title"), [])
    assert_eq(form_for(nil).field_errors("title"), [])
  end

  test("errors_for returns an empty string when the field has no errors") do
    record = {"title": "", "_errors": [{"field": "body", "message": "too short"}]}
    assert_eq(form_for(record).errors_for("title"), "")
  end

  test("error_summary lists every error regardless of field") do
    record = {"title": "", "_errors": [
      {"field": "title", "message": "cannot be blank"},
      {"field": "body", "message": "too short"}
    ]}
    assert_eq(
      form_for(record).error_summary(),
      "<div class=\"form-errors\"><ul><li>cannot be blank</li><li>too short</li></ul></div>"
    )
  end

  test("error_summary accepts a custom class") do
    html = form_for(record_with_title_error("bad")).error_summary({"class": "alert alert-danger"})
    assert_contains(html, "<div class=\"alert alert-danger\">")
  end

  test("error_summary returns an empty string for a valid record and for nil") do
    assert_eq(form_for({"title": "ok"}).error_summary(), "")
    assert_eq(form_for(nil).error_summary(), "")
  end

  test("errored fields get the field-error class and aria-invalid") do
    html = form_for(record_with_title_error()).text_field("title")
    assert_contains(html, "class=\"field-error\"")
    assert_contains(html, "aria-invalid=\"true\"")
  end

  test("caller classes merge with the field-error marker") do
    assert_contains(
      form_for(record_with_title_error()).text_field("title", {"class": "wide"}),
      "class=\"wide field-error\""
    )
  end

  test("clean fields carry no error markers") do
    html = form_for({"title": "ok"}).text_field("title")
    assert_not(html.includes?("aria-invalid"))
    assert_not(html.includes?("class="))
  end

  test("error messages are escaped") do
    summary = form_for(record_with_title_error("<img src=x>")).error_summary()
    assert_not(summary.includes?("<img"))
    assert_contains(summary, "&lt;img src=x&gt;")
  end
end

describe("fields_for nesting") do
  test("a nested builder prefixes bracket names and flattens ids") do
    author = form_for({"author": {"name": "Alice"}}).fields_for("author")
    assert_eq(
      author.text_field("name"),
      "<input type=\"text\" id=\"author_name\" name=\"author[name]\" value=\"Alice\">"
    )
  end

  test("a nested builder prefills from record[field]") do
    author = form_for({"author": {"name": "Alice", "email": "a@b.c"}}).fields_for("author")
    assert_contains(author.email_field("email"), "value=\"a@b.c\"")
  end

  test("an index produces indexed names and ids") do
    # The index only affects naming — prefill still reads record[field] as a
    # whole, so use an empty nested document here
    item = form_with({"items": {}}, {"url": "/orders"}).fields_for("items", 0)
    assert_eq(item.text_field("sku"), "<input type=\"text\" id=\"items_0_sku\" name=\"items[0][sku]\">")
  end

  test("deep nesting accumulates the prefix") do
    address = form_for({"author": {"address": {"city": "Paris"}}}).fields_for("author").fields_for("address")
    assert_eq(
      address.text_field("city"),
      "<input type=\"text\" id=\"author_address_city\" name=\"author[address][city]\" value=\"Paris\">"
    )
  end

  test("a nested builder still renders labels and checkboxes") do
    author = form_for({"author": {"active": true}}).fields_for("author")
    assert_eq(author.label("active"), "<label for=\"author_active\">Active</label>")
    assert_contains(author.check_box("active"), "name=\"author[active]\" value=\"true\" checked")
  end
end

describe("builder internals") do
  before_each() do
    form = form_for({})
  end

  context("value_for") do
    test("reads the field from the record") do
      form = form_for({"title": "Hello", "count": 7})
      assert_eq(form.value_for("title"), "Hello")
      assert_eq(form.value_for("count"), 7)
    end

    test("returns nil for a missing field and for a nil record") do
      assert_null(form.value_for("nope"))
      assert_null(form_for(nil).value_for("title"))
    end
  end

  context("name_for and id_for") do
    test("name_for is flat at the top level") do
      assert_eq(form.name_for("title", {}), "title")
    end

    test("name_for honors an explicit override") do
      assert_eq(form.name_for("title", {"name": "custom"}), "custom")
    end

    test("name_for nests under a prefix") do
      assert_eq(form_for({"author": {}}).fields_for("author").name_for("name", {}), "author[name]")
    end

    test("id_for is flat at the top level") do
      assert_eq(form.id_for("title"), "title")
    end

    test("id_for flattens brackets to underscores") do
      form = form_for({"author": {}})
      assert_eq(form.fields_for("author").id_for("name"), "author_name")
      assert_eq(form.fields_for("items", 0).id_for("sku"), "items_0_sku")
    end
  end

  context("attributes_without") do
    test("skips the excluded keys") do
      options = {"name": "x", "class": "y", "placeholder": "P"}
      assert_eq(form.attributes_without(options, ["name", "class"]), " placeholder=\"P\"")
    end

    test("handles nil options") do
      assert_eq(form.attributes_without(nil, []), "")
    end

    test("true renders a bare attribute; false and nil are skipped") do
      assert_eq(form.attributes_without({"required": true, "disabled": false, "autofocus": nil}, []), " required")
    end

    test("escapes values") do
      assert_eq(form.attributes_without({"data-tip": "a\"b"}, []), " data-tip=\"a&quot;b\"")
    end
  end

  context("class_attribute and invalid_attribute") do
    test("class_attribute uses the caller's class when the field is clean") do
      assert_eq(form_for({"title": "ok"}).class_attribute("title", {"class": "wide"}), " class=\"wide\"")
    end

    test("class_attribute is empty without a class or errors") do
      assert_eq(form_for({"title": "ok"}).class_attribute("title", {}), "")
    end

    test("class_attribute falls back to field-error alone") do
      assert_eq(form_for(record_with_title_error("bad")).class_attribute("title", {}), " class=\"field-error\"")
    end

    test("invalid_attribute is empty for clean fields") do
      assert_eq(form_for({"title": "ok"}).invalid_attribute("title"), "")
    end

    test("invalid_attribute marks errored fields") do
      assert_eq(form_for(record_with_title_error("bad")).invalid_attribute("title"), " aria-invalid=\"true\"")
    end
  end

  context("label") do
    test("derives the caption from the field name") do
      assert_eq(form.label("first_name"), "<label for=\"first_name\">First name</label>")
    end

    test("accepts explicit text and options") do
      assert_eq(
        form.label("title", "Post title", {"class": "lbl"}),
        "<label for=\"title\" class=\"lbl\">Post title</label>"
      )
    end
  end
end

describe("full form assembly") do
  test("open, fields and close compose into a complete form, in order") do
    form = form_for({"title": "Hello", "published": true})
    html = form.open() + form.label("title") + form.text_field("title") + form.check_box("published") +
      form.errors_for("title") + form.error_summary() + form.submit("Save") + form.close()
    assert_contains(html, "<form action=\"/posts\" method=\"POST\"")
    assert_contains(html, "<label for=\"title\">Title</label>")
    assert_contains(html, "type=\"checkbox\" id=\"published\" name=\"published\" value=\"true\" checked")
    assert_contains(html, "<button type=\"submit\">Save</button>")
    assert(html.ends_with("</form>"))
    assert_lt(html.index_of("<form"), html.index_of("<button"))
    assert_lt(html.index_of("<button"), html.index_of("</form>"))
  end
end

describe("form builder output safety") do
  test("button_to confirm text cannot break out of its attribute") do
    # The old form emitted onclick="return confirm('#{j(text)}')". `j()` is a
    # JavaScript escape, so a quote became \" — which the HTML tokenizer reads
    # as a backslash then the end of the attribute, opening a live handler.
    html = button_to("Delete", "/posts/7", {"confirm": "Delete x\" onmouseover=alert(1) y=\"?"})
    # No inline JavaScript at all, so there is no seam to break out of
    assert_not(html.contains("onclick"))
    assert_contains(html, "data-confirm=")
    # The quotes that used to end the attribute are escaped: the payload stays inert text
    assert_not(html.contains("y=\"?\""))
    assert_contains(html, "&quot; onmouseover=alert(1) y=&quot;")
  end

  test("button_to refuses a javascript: target") do
    html = button_to("Visit", "javascript:alert(1)")
    assert_not(html.contains("javascript:"))
    assert_contains(html, "action=\"#\"")
  end

  test("button_to keeps ordinary targets") do
    assert_contains(button_to("Go", "/posts/7"), "action=\"/posts/7\"")
    assert_contains(button_to("Go", "https://example.com/x"), "action=\"https://example.com/x\"")
    assert_contains(button_to("Go", "posts/7"), "action=\"posts/7\"")
  end

  test("form_with refuses a javascript: url") do
    html = form_with({"title": "x"}, {"url": "javascript:alert(1)"}).open()
    assert_not(html.contains("javascript:"))
    assert_contains(html, "action=\"#\"")
  end

  test("an attribute name built from data cannot open a handler") do
    # Value escaping never sees the name, so a name carrying a quote used to
    # produce a live attribute
    assert_raises("button_to(): invalid HTML attribute name") do
      button_to("X", "/x", {"placeholder\" onfocus=\"alert(1)": "v"})
    end
  end

  test("ordinary attribute names still work") do
    html = button_to("X", "/x", {"class": "btn", "data-role": "delete"})
    assert_contains(html, "class=\"btn\"")
    assert_contains(html, "data-role=\"delete\"")
  end
end
