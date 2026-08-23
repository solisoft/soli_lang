// ============================================================================
// FormBuilder / form_with Test Suite
//
// The FormBuilder lives in src/interpreter/builtins/form_builder.sl as pure
// Soli, evaluated into the shared *template* environment at render time
// (template::register_form_builder). Plain `soli test` interpreters never run
// that registration, and module imports are sandboxed to the spec directory,
// so we import an exported fixture copy (same pattern as feature_flags_spec)
// and assert below that it stays in sync with the canonical source.
//
// The fixture copy adds `export` on top-level declarations only. A drift test
// strips those back off and byte-compares against src/, so any change to the
// real form builder fails this suite until the fixture is refreshed.
//
// Template-only Rust builtins the form builder calls are shimmed here (they
// resolve dynamically from module bodies): h/attr mirror the template engine's
// escapers; __soli_form_names mirrors template.rs' instance→{collection,key}
// helper (hashes → null, instances → naive pluralization).
// ============================================================================

def h(value)
    html_escape(value.to_s())
end

def attr(value)
    html_escape(value.to_s()).replace("\"", "&quot;")
end

def __soli_form_names(record)
    if record.nil?
        return null
    end
    let klass = record.class.to_s()
    if klass == "hash"
        return null
    end
    {"collection": klass.downcase() + "s", "key": record._key}
end

import "./form_builder_fixture.sl";

describe("FormBuilder fixture hygiene", fn() {
    test("fixture matches canonical form_builder.sl modulo export keywords", fn() {
        # Run from the repo root (how `soli test` runs in CI).
        let source = slurp("src/interpreter/builtins/form_builder.sl") rescue null
        let fixture = slurp("tests/builtins/form_builder_fixture.sl") rescue null
        if !source.nil? && !fixture.nil?
            assert_eq(fixture.replace("export ", ""), source)
        end
    });
});

describe("form_with construction", fn() {
    test("returns a FormBuilder instance", fn() {
        let f = form_with({"title": "Hello"}, {"url": "/posts"});
        assert_eq(f.class, "FormBuilder");
    });

    test("hash record requires explicit url option", fn() {
        let f = form_with({"title": "Hello"}, {"url": "/posts"});
        assert_contains(f.open(), "<form action=\"/posts\" method=\"POST\"");
    });

    test("new model instance derives POST to collection", fn() {
        let post = Post.new();
        let f = form_with(post);
        assert_contains(f.open(), "<form action=\"/posts\" method=\"POST\"");
    });

    test("open embeds the session CSRF token for post forms", fn() {
        let f = form_with({"title": "Hello"}, {"url": "/posts"});
        assert_contains(f.open(), "<input type=\"hidden\" name=\"_csrf_token\" value=\"");
    });

    test("get method renders GET and omits the csrf field", fn() {
        let f = form_with({"title": "Hello"}, {"url": "/search", "method": "get"});
        let html = f.open();
        assert_contains(html, "<form action=\"/search\" method=\"GET\"");
        assert(!html.includes?("name=\"_csrf_token\""));
    });

    test("non-get/post methods embed a _method override hidden input", fn() {
        # Built directly so no persisted record (and no DB) is needed.
        let f = new FormBuilder({"title": "Hi"}, "/posts/42", "patch", {}, "");
        let html = f.open();
        assert_contains(html, "<input type=\"hidden\" name=\"_method\" value=\"PATCH\">");
        assert_contains(html, "name=\"_csrf_token\"");
    });

    test("multipart option adds enctype attribute", fn() {
        let f = form_with({"title": "Hello"}, {"url": "/uploads", "multipart": true});
        assert_contains(f.open(), "enctype=\"multipart/form-data\"");
    });

    test("extra options become form tag attributes", fn() {
        let f = form_with({"title": "Hello"}, {"url": "/posts", "class": "big", "id": "new-post"});
        let html = f.open();
        assert_contains(html, "class=\"big\"");
        assert_contains(html, "id=\"new-post\"");
    });

    test("close closes the form", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_eq(f.close(), "</form>");
    });
});

describe("field inputs", fn() {
    test("text_field prefills value from record", fn() {
        let f = form_with({"title": "Hello", "published": true}, {"url": "/posts"});
        assert_contains(
            f.text_field("title"),
            "<input type=\"text\" id=\"title\" name=\"title\" value=\"Hello\">"
        );
    });

    test("text_field passes through extra options incl. bare attributes", fn() {
        let f = form_with({}, {"url": "/posts"});
        let html = f.text_field("title", {"placeholder": "Title", "required": true});
        assert_contains(html, "placeholder=\"Title\"");
        assert_contains(html, " required>");
    });

    test("email_field renders email input", fn() {
        let f = form_with({"email": "a@b.c"}, {"url": "/users"});
        assert_contains(
            f.email_field("email"),
            "<input type=\"email\" id=\"email\" name=\"email\" value=\"a@b.c\">"
        );
    });

    test("password_field never prefills", fn() {
        let f = form_with({"password": "hunter2"}, {"url": "/users"});
        let html = f.password_field("password");
        assert_contains(html, "<input type=\"password\" id=\"password\" name=\"password\"");
        assert(!html.includes?("value="));
    });

    test("number_field renders number input", fn() {
        let f = form_with({"age": 30}, {"url": "/users"});
        assert_contains(f.number_field("age"), "<input type=\"number\" id=\"age\" name=\"age\" value=\"30\">");
    });

    test("date_field renders date input", fn() {
        let f = form_with({"due_on": "2026-08-23"}, {"url": "/tasks"});
        assert_contains(
            f.date_field("due_on"),
            "<input type=\"date\" id=\"due_on\" name=\"due_on\" value=\"2026-08-23\">"
        );
    });

    test("datetime_field renders datetime-local input", fn() {
        let f = form_with({"starts_at": "2026-08-23T10:00"}, {"url": "/events"});
        assert_contains(
            f.datetime_field("starts_at"),
            "<input type=\"datetime-local\" id=\"starts_at\" name=\"starts_at\" value=\"2026-08-23T10:00\">"
        );
    });

    test("hidden_field prefills value", fn() {
        let f = form_with({"token": "abc"}, {"url": "/posts"});
        assert_contains(
            f.hidden_field("token"),
            "<input type=\"hidden\" id=\"token\" name=\"token\" value=\"abc\">"
        );
    });

    test("file_field never prefills", fn() {
        let f = form_with({"avatar": "x.png"}, {"url": "/users"});
        let html = f.file_field("avatar");
        assert_contains(html, "<input type=\"file\" id=\"avatar\" name=\"avatar\"");
        assert(!html.includes?("value="));
    });

    test("explicit value option overrides record", fn() {
        let f = form_with({"title": "Hello"}, {"url": "/posts"});
        assert_contains(f.text_field("title", {"value": "Override"}), "value=\"Override\"");
    });

    test("missing field yields no value attribute", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_eq(f.text_field("title"), "<input type=\"text\" id=\"title\" name=\"title\">");
    });

    test("input renders a generic input by type", fn() {
        let f = form_with({"qty": 3}, {"url": "/orders"});
        assert_contains(
            f.input("tel", "qty", null),
            "<input type=\"tel\" id=\"qty\" name=\"qty\" value=\"3\">"
        );
    });

    test("values are HTML-escaped", fn() {
        let f = form_with({"bio": "<script>"}, {"url": "/users"});
        let html = f.text_field("bio");
        assert(!html.includes?("<script>"));
        assert_contains(html, "value=\"&lt;script&gt;\"");
    });

    test("name option overrides derived name verbatim", fn() {
        let f = form_with({"q": "x"}, {"url": "/search"});
        assert_contains(f.text_field("q", {"name": "query"}), "name=\"query\"");
    });
});

describe("text_area", fn() {
    test("renders content between tags from record", fn() {
        let f = form_with({"body": "line1\nline2"}, {"url": "/posts"});
        assert_contains(
            f.text_area("body"),
            "<textarea id=\"body\" name=\"body\">line1\nline2</textarea>"
        );
    });

    test("value option overrides content", fn() {
        let f = form_with({"body": "original"}, {"url": "/posts"});
        assert_contains(f.text_area("body", {"value": "override"}), ">override</textarea>");
    });

    test("escapes content", fn() {
        let f = form_with({"body": "<b>bold</b>"}, {"url": "/posts"});
        assert_contains(f.text_area("body"), "&lt;b&gt;bold&lt;/b&gt;");
    });

    test("supports rows and cols options", fn() {
        let f = form_with({}, {"url": "/posts"});
        let html = f.text_area("body", {"rows": 5, "cols": 40});
        assert_contains(html, "rows=\"5\"");
        assert_contains(html, "cols=\"40\"");
    });
});

describe("check_box", fn() {
    test("checked when record value is true", fn() {
        let f = form_with({"published": true}, {"url": "/posts"});
        assert_contains(
            f.check_box("published"),
            "<input type=\"checkbox\" id=\"published\" name=\"published\" value=\"true\" checked>"
        );
    });

    test("unchecked when record value is false", fn() {
        let f = form_with({"published": false}, {"url": "/posts"});
        let html = f.check_box("published");
        assert_contains(html, "type=\"checkbox\"");
        assert(!html.includes?("checked"));
    });

    test("checked when record value is the string true", fn() {
        let f = form_with({"published": "true"}, {"url": "/posts"});
        assert_contains(f.check_box("published"), " checked>");
    });

    test("unchecked when field missing", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert(!f.check_box("published").includes?("checked"));
    });
});

describe("radio_button", fn() {
    test("checked when value matches record", fn() {
        let f = form_with({"role": "admin"}, {"url": "/users"});
        assert_contains(
            f.radio_button("role", "admin"),
            "<input type=\"radio\" id=\"role_admin\" name=\"role\" value=\"admin\" checked>"
        );
    });

    test("unchecked when value differs from record", fn() {
        let f = form_with({"role": "admin"}, {"url": "/users"});
        let html = f.radio_button("role", "guest");
        assert_contains(html, "<input type=\"radio\" id=\"role_guest\" name=\"role\" value=\"guest\"");
        assert(!html.includes?("checked"));
    });

    test("matches numbers via string comparison", fn() {
        let f = form_with({"size": 2}, {"url": "/shirts"});
        assert_contains(f.radio_button("size", 2), " checked>");
    });
});

describe("select", fn() {
    test("renders options from string choices", fn() {
        let f = form_with({"color": "blue"}, {"url": "/cars"});
        let html = f.select("color", ["red", "green", "blue"]);
        assert_contains(html, "<select id=\"color\" name=\"color\">");
        assert_contains(html, "<option value=\"red\">red</option>");
        assert_contains(html, "<option value=\"blue\" selected>blue</option>");
        assert(!html.includes?("<option value=\"red\" selected"));
    });

    test("accepts label/value pair choices", fn() {
        let medium = ["Medium", "m"];
        let large = ["Large", "l"];
        let f = form_with({"size": "m"}, {"url": "/shirts"});
        let html = f.select("size", [medium, large]);
        assert_contains(html, "<option value=\"m\" selected>Medium</option>");
        assert_contains(html, "<option value=\"l\">Large</option>");
    });

    test("multiple option appends [] to name and adds multiple attribute", fn() {
        let f = form_with({"tags": "a"}, {"url": "/posts"});
        let html = f.select("tags", ["a", "b"], {"multiple": true});
        assert_contains(html, "name=\"tags[]\"");
        assert_contains(html, " multiple");
    });

    test("escapes option labels and values", fn() {
        let bold_label = ["<b>A</b>", "<x>"];
        let f = form_with({"pick": ""}, {"url": "/things"});
        let html = f.select("pick", [bold_label]);
        assert(!html.includes?("<option value=\"<x>\">"));
        assert_contains(html, "&lt;b&gt;A&lt;/b&gt;");
    });
});

describe("submit", fn() {
    test("defaults to Save caption", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_eq(f.submit(), "<button type=\"submit\">Save</button>");
    });

    test("uses provided caption", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_eq(f.submit("Create Post"), "<button type=\"submit\">Create Post</button>");
    });

    test("escapes caption and passes extra attributes", fn() {
        let f = form_with({}, {"url": "/posts"});
        let html = f.submit("<Save>", {"class": "btn"});
        assert_contains(html, "class=\"btn\"");
        assert(!html.includes?("<Save>"));
        assert_contains(html, "&lt;Save&gt;");
    });
});

describe("error rendering", fn() {
    test("errors_for renders a span per message for the field", fn() {
        let record = {
            "title": "",
            "_errors": [
                {"field": "title", "message": "cannot be blank"},
                {"field": "body", "message": "too short"}
            ]
        };
        let f = form_with(record, {"url": "/posts"});
        assert_eq(
            f.errors_for("title"),
            "<span class=\"field-error-message\">cannot be blank</span>"
        );
    });

    test("field_errors collects messages for one field", fn() {
        let record = {
            "_errors": [
                {"field": "title", "message": "can't be blank"},
                {"field": "title", "message": "too long"},
                {"field": "body", "message": "too short"}
            ]
        };
        let f = form_with(record, {"url": "/posts"});
        let messages = f.field_errors("title");
        assert_eq(messages.length(), 2);
        assert_contains(messages[0], "blank");
        assert_contains(messages[1], "long");
    });

    test("field_errors is empty for valid records and null builders", fn() {
        assert_eq(form_with({"title": "ok"}, {"url": "/posts"}).field_errors("title").length(), 0);
        assert_eq(form_with(null, {"url": "/posts"}).field_errors("title").length(), 0);
    });

    test("errors_for returns empty string when field has no errors", fn() {
        let record = {"title": "", "_errors": [{"field": "body", "message": "too short"}]};
        let f = form_with(record, {"url": "/posts"});
        assert_eq(f.errors_for("title"), "");
    });

    test("error_summary lists every error regardless of field", fn() {
        let record = {
            "title": "",
            "_errors": [
                {"field": "title", "message": "cannot be blank"},
                {"field": "body", "message": "too short"}
            ]
        };
        let f = form_with(record, {"url": "/posts"});
        assert_eq(
            f.error_summary(),
            "<div class=\"form-errors\"><ul><li>cannot be blank</li><li>too short</li></ul></div>"
        );
    });

    test("error_summary accepts a custom class", fn() {
        let record = {"_errors": [{"field": "title", "message": "bad"}]};
        let f = form_with(record, {"url": "/posts"});
        assert_contains(f.error_summary({"class": "alert alert-danger"}), "<div class=\"alert alert-danger\">");
    });

    test("error_summary returns empty string for valid records", fn() {
        let f = form_with({"title": "ok"}, {"url": "/posts"});
        assert_eq(f.error_summary(), "");
    });

    test("error_summary returns empty string for null record", fn() {
        let f = form_with(null, {"url": "/posts"});
        assert_eq(f.error_summary(), "");
    });

    test("errored fields get field-error class and aria-invalid", fn() {
        let record = {"title": "", "_errors": [{"field": "title", "message": "can't be blank"}]};
        let f = form_with(record, {"url": "/posts"});
        let html = f.text_field("title");
        assert_contains(html, "class=\"field-error\"");
        assert_contains(html, "aria-invalid=\"true\"");
    });

    test("caller classes merge with the field-error marker", fn() {
        let record = {"title": "", "_errors": [{"field": "title", "message": "can't be blank"}]};
        let f = form_with(record, {"url": "/posts"});
        assert_contains(f.text_field("title", {"class": "wide"}), "class=\"wide field-error\"");
    });

    test("clean fields carry no error markers", fn() {
        let f = form_with({"title": "ok"}, {"url": "/posts"});
        let html = f.text_field("title");
        assert(!html.includes?("aria-invalid"));
        assert(!html.includes?("class="));
    });

    test("error messages are escaped", fn() {
        let record = {"_errors": [{"field": "title", "message": "<img src=x>"}]};
        let f = form_with(record, {"url": "/posts"});
        assert(!f.error_summary().includes?("<img"));
    });
});

describe("fields_for nesting", fn() {
    test("nested builder prefixes bracket names and flattens ids", fn() {
        let f = form_with({"author": {"name": "Alice"}}, {"url": "/posts"});
        let author = f.fields_for("author");
        assert_contains(
            author.text_field("name"),
            "<input type=\"text\" id=\"author_name\" name=\"author[name]\" value=\"Alice\">"
        );
    });

    test("nested builder prefills from record[field]", fn() {
        let f = form_with({"author": {"name": "Alice", "email": "a@b.c"}}, {"url": "/posts"});
        let author = f.fields_for("author");
        assert_contains(author.email_field("email"), "value=\"a@b.c\"");
    });

    test("index produces indexed names and ids", fn() {
        # Note: the index only affects naming — prefill still reads
        # record[field] as a whole, so use an empty nested document here.
        let f = form_with({"items": {}}, {"url": "/orders"});
        let item = f.fields_for("items", 0);
        assert_contains(item.text_field("sku"), "id=\"items_0_sku\"");
        assert_contains(item.text_field("sku"), "name=\"items[0][sku]\"");
    });

    test("deep nesting accumulates the prefix", fn() {
        let f = form_with({"author": {"address": {"city": "Paris"}}}, {"url": "/posts"});
        let address = f.fields_for("author").fields_for("address");
        assert_contains(address.text_field("city"), "name=\"author[address][city]\"");
        assert_contains(address.text_field("city"), "id=\"author_address_city\"");
        assert_contains(address.text_field("city"), "value=\"Paris\"");
    });

    test("nested builder still renders labels and checkboxes", fn() {
        let f = form_with({"author": {"active": true}}, {"url": "/posts"});
        let author = f.fields_for("author");
        assert_contains(author.label("active"), "<label for=\"author_active\">Active</label>");
        assert_contains(author.check_box("active"), "name=\"author[active]\" value=\"true\" checked");
    });
});

describe("builder internals", fn() {
    test("value_for reads the field from the record", fn() {
        let f = form_with({"title": "Hello", "count": 7}, {"url": "/posts"});
        assert_eq(f.value_for("title"), "Hello");
        assert_eq(f.value_for("count"), 7);
    });

    test("value_for returns null for missing fields", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_null(f.value_for("nope"));
    });

    test("value_for returns null when record is null", fn() {
        let f = form_with(null, {"url": "/posts"});
        assert_null(f.value_for("title"));
    });

    test("name_for is flat at top level", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_eq(f.name_for("title", {}), "title");
    });

    test("name_for honors explicit override", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_eq(f.name_for("title", {"name": "custom"}), "custom");
    });

    test("name_for nests under a prefix", fn() {
        let f = form_with({"author": {}}, {"url": "/posts"});
        assert_eq(f.fields_for("author").name_for("name", {}), "author[name]");
    });

    test("id_for is flat at top level", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_eq(f.id_for("title"), "title");
    });

    test("id_for flattens brackets to underscores", fn() {
        let f = form_with({"author": {}}, {"url": "/posts"});
        assert_eq(f.fields_for("author").id_for("name"), "author_name");
        assert_eq(f.fields_for("items", 0).id_for("sku"), "items_0_sku");
    });

    test("attributes_without skips excluded keys", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_eq(
            f.attributes_without({"name": "x", "class": "y", "placeholder": "P"}, ["name", "class"]),
            " placeholder=\"P\""
        );
    });

    test("attributes_without handles nil options", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_eq(f.attributes_without(null, []), "");
    });

    test("true renders a bare attribute, false/null are skipped", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_eq(f.attributes_without({"required": true, "disabled": false, "autofocus": null}, []), " required");
    });

    test("attributes_without escapes values", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_eq(f.attributes_without({"data-tip": "a\"b"}, []), " data-tip=\"a&quot;b\"");
    });

    test("class_attribute uses caller class when field is clean", fn() {
        let f = form_with({"title": "ok"}, {"url": "/posts"});
        assert_eq(f.class_attribute("title", {"class": "wide"}), " class=\"wide\"");
    });

    test("class_attribute is empty without class or errors", fn() {
        let f = form_with({"title": "ok"}, {"url": "/posts"});
        assert_eq(f.class_attribute("title", {}), "");
    });

    test("class_attribute falls back to field-error alone", fn() {
        let record = {"title": "", "_errors": [{"field": "title", "message": "bad"}]};
        let f = form_with(record, {"url": "/posts"});
        assert_eq(f.class_attribute("title", {}), " class=\"field-error\"");
    });

    test("invalid_attribute is empty for clean fields", fn() {
        let f = form_with({"title": "ok"}, {"url": "/posts"});
        assert_eq(f.invalid_attribute("title"), "");
    });

    test("invalid_attribute marks errored fields", fn() {
        let record = {"title": "", "_errors": [{"field": "title", "message": "bad"}]};
        let f = form_with(record, {"url": "/posts"});
        assert_eq(f.invalid_attribute("title"), " aria-invalid=\"true\"");
    });

    test("label derives caption from the field name", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_eq(f.label("first_name"), "<label for=\"first_name\">First name</label>");
    });

    test("label accepts explicit text and options", fn() {
        let f = form_with({}, {"url": "/posts"});
        assert_eq(
            f.label("title", "Post title", {"class": "lbl"}),
            "<label for=\"title\" class=\"lbl\">Post title</label>"
        );
    });
});

describe("full form assembly", fn() {
    test("open, fields and close compose into a complete form", fn() {
        let record = {"title": "Hello", "published": true};
        let f = form_with(record, {"url": "/posts"});
        let html = f.open()
            + f.label("title")
            + f.text_field("title")
            + f.check_box("published")
            + f.errors_for("title")
            + f.error_summary()
            + f.submit("Save")
            + f.close();
        assert_contains(html, "<form action=\"/posts\" method=\"POST\"");
        assert_contains(html, "<label for=\"title\">Title</label>");
        assert_contains(html, "type=\"checkbox\" id=\"published\" name=\"published\" value=\"true\" checked");
        assert_contains(html, "<button type=\"submit\">Save</button>");
        assert_contains(html, "</form>");
        # order sanity: form opens before the button, button before close
        assert(html.index_of("<form") < html.index_of("<button"));
        assert(html.index_of("<button") < html.index_of("</form>"));
    });
});

// Model used to exercise form_with's instance-derived URLs (via the
// __soli_form_names shim above).
class Post < Model
end
