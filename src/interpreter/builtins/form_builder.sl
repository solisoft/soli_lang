# Rails-style form builder — engine-embedded Soli, evaluated into every
# template render environment (see template::register_form_builder).
#
# Usage in a view (`<%-` because the helpers return HTML):
#
#   <% f = form_with(post) %>
#   <%- f.open() %>
#     <%- f.label("title") %>
#     <%- f.text_field("title", {"placeholder": "Title"}) %>
#     <%- f.errors_for("title") %>
#     <%- f.submit("Save") %>
#   <%- f.close() %>
#
# form_with(record) derives the action URL and method from the record —
# a new record posts to /<collection>, a persisted one patches
# /<collection>/<key> via a hidden `_method` field the server honors.
# open() embeds the per-session CSRF token; field names are flat
# (name="title"), matching Soli's flat params model.

class FormBuilder
    record: Any
    url: String
    http_method: String
    form_attrs: Any
    name_prefix: String

    new(record, url, http_method, form_attrs, name_prefix = "")
        this.record = record
        this.url = url
        this.http_method = http_method
        this.form_attrs = form_attrs
        this.name_prefix = name_prefix
    end

    # Sub-builder for a nested document: fields render as name="author[name]"
    # and prefill from record[field]. Pass an index for collections:
    # fields_for("items", 0) renders name="items[0][sku]". The server nests
    # bracket names back into params["author"]["name"] etc.
    def fields_for(field, index = null)
        prefix = field
        prefix = this.name_prefix + "[" + field + "]" unless this.name_prefix.blank?
        prefix = prefix + "[" + index.to_s + "]" unless index.nil?
        new FormBuilder(this.value_for(field), "", "post", {}, prefix)
    end

    def open()
        browser_method = "POST"
        browser_method = "GET" if this.http_method == "get"
        action = attr(this.url)
        extra = this.attributes_without(this.form_attrs, [])
        html = "<form action=\"#{action}\" method=\"#{browser_method}\"#{extra}>"
        if !["get", "post"].includes?(this.http_method)
            override = this.http_method.upcase()
            html = html + "<input type=\"hidden\" name=\"_method\" value=\"#{override}\">"
        end
        html = html + csrf_field() unless this.http_method == "get"
        html
    end

    def close()
        "</form>"
    end

    def label(field, text = null, options = null)
        extra = this.attributes_without(options, [])
        caption = text ?? field.replace("_", " ").capitalize()
        input_id = this.id_for(field)
        escaped_caption = h(caption)
        "<label for=\"#{input_id}\"#{extra}>#{escaped_caption}</label>"
    end

    def text_field(field, options = null)
        this.input("text", field, options)
    end

    def email_field(field, options = null)
        this.input("email", field, options)
    end

    def password_field(field, options = null)
        this.input("password", field, options)
    end

    def number_field(field, options = null)
        this.input("number", field, options)
    end

    def date_field(field, options = null)
        this.input("date", field, options)
    end

    def datetime_field(field, options = null)
        this.input("datetime-local", field, options)
    end

    def hidden_field(field, options = null)
        this.input("hidden", field, options)
    end

    def file_field(field, options = null)
        this.input("file", field, options)
    end

    def text_area(field, options = null)
        opts = options ?? {}
        extra = this.attributes_without(opts, ["class", "value", "name"])
        class_attr = this.class_attribute(field, opts)
        invalid = this.invalid_attribute(field)
        value = opts["value"] ?? this.value_for(field)
        input_id = this.id_for(field)
        input_name = this.name_for(field, opts)
        escaped_value = h(value.to_s)
        "<textarea id=\"#{input_id}\" name=\"#{input_name}\"#{class_attr}#{invalid}#{extra}>#{escaped_value}</textarea>"
    end

    def check_box(field, options = null)
        opts = options ?? {}
        extra = this.attributes_without(opts, ["class", "value", "name"])
        class_attr = this.class_attribute(field, opts)
        current = this.value_for(field)
        checked = ""
        checked = " checked" if current == true || current.to_s == "true"
        input_id = this.id_for(field)
        input_name = this.name_for(field, opts)
        "<input type=\"checkbox\" id=\"#{input_id}\" name=\"#{input_name}\" value=\"true\"#{checked}#{class_attr}#{extra}>"
    end

    def radio_button(field, value, options = null)
        opts = options ?? {}
        extra = this.attributes_without(opts, ["class", "name"])
        class_attr = this.class_attribute(field, opts)
        checked = ""
        checked = " checked" if this.value_for(field).to_s == value.to_s
        input_id = this.id_for(field)
        input_name = this.name_for(field, opts)
        option_value = attr(value.to_s)
        "<input type=\"radio\" id=\"#{input_id}_#{option_value}\" name=\"#{input_name}\" value=\"#{option_value}\"#{checked}#{class_attr}#{extra}>"
    end

    # choices: array of strings, or array of [label, value] pairs. Pass
    # {"multiple": true} for a multi-select — the name gains [] so the
    # server collects the selections into an array.
    def select(field, choices, options = null)
        opts = options ?? {}
        extra = this.attributes_without(opts, ["class", "name", "multiple"])
        class_attr = this.class_attribute(field, opts)
        invalid = this.invalid_attribute(field)
        current = this.value_for(field).to_s
        options_html = ""
        for choice in choices
            choice_label = choice
            choice_value = choice
            if choice.class == "array"
                choice_label = choice[0]
                choice_value = choice[1]
            end
            selected = ""
            selected = " selected" if choice_value.to_s == current
            option_value = attr(choice_value.to_s)
            option_label = h(choice_label.to_s)
            options_html = options_html + "<option value=\"#{option_value}\"#{selected}>#{option_label}</option>"
        end
        input_id = this.id_for(field)
        input_name = this.name_for(field, opts)
        multiple_attr = ""
        if opts["multiple"] == true
            input_name = input_name + "[]"
            multiple_attr = " multiple"
        end
        "<select id=\"#{input_id}\" name=\"#{input_name}\"#{multiple_attr}#{class_attr}#{invalid}#{extra}>#{options_html}</select>"
    end

    def submit(text = null, options = null)
        extra = this.attributes_without(options, [])
        caption = h(text ?? "Save")
        "<button type=\"submit\"#{extra}>#{caption}</button>"
    end

    # Inline error messages for one field ("" when the field has none).
    def errors_for(field)
        messages = this.field_errors(field)
        return "" if messages.empty?

        html = ""
        for message in messages
            escaped_message = h(message.to_s)
            html = html + "<span class=\"field-error-message\">#{escaped_message}</span>"
        end
        html
    end

    # Top-of-form list of every validation error ("" when the record is
    # valid). Pass {"class": "..."} to restyle the wrapping div.
    def error_summary(options = null)
        return "" if this.record.nil?

        all_errors = this.record["_errors"] ?? []
        return "" if all_errors.empty?

        opts = options ?? {}
        css = attr((opts["class"] ?? "form-errors").to_s)
        html = "<div class=\"#{css}\"><ul>"
        for err in all_errors
            escaped_message = h(err["message"].to_s)
            html = html + "<li>#{escaped_message}</li>"
        end
        html + "</ul></div>"
    end

    # --- internals -------------------------------------------------------

    def input(input_type, field, options = null)
        opts = options ?? {}
        extra = this.attributes_without(opts, ["class", "value", "name"])
        class_attr = this.class_attribute(field, opts)
        invalid = this.invalid_attribute(field)
        value = opts["value"]
        if value.nil? && input_type != "password" && input_type != "file"
            value = this.value_for(field)
        end
        value_attr = ""
        if !value.nil?
            escaped_value = attr(value.to_s)
            value_attr = " value=\"#{escaped_value}\""
        end
        input_id = this.id_for(field)
        input_name = this.name_for(field, opts)
        "<input type=\"#{input_type}\" id=\"#{input_id}\" name=\"#{input_name}\"#{value_attr}#{class_attr}#{invalid}#{extra}>"
    end

    def value_for(field)
        return null if this.record.nil?

        this.record[field]
    end

    # name attribute: prefixed bracket form under fields_for, flat otherwise.
    # A "name" option on the helper overrides it verbatim.
    def name_for(field, opts)
        override = opts["name"]
        return attr(override.to_s) unless override.nil?

        return attr(field) if this.name_prefix.blank?

        attr(this.name_prefix + "[" + field + "]")
    end

    # id attribute: brackets flattened to underscores (author[name] →
    # author_name) so labels stay linkable.
    def id_for(field)
        return attr(field) if this.name_prefix.blank?

        flat_prefix = this.name_prefix.replace("][", "_").replace("[", "_").replace("]", "")
        attr(flat_prefix + "_" + field)
    end

    def field_errors(field)
        messages = []
        if !this.record.nil?
            all_errors = this.record["_errors"] ?? []
            for err in all_errors
                messages.push(err["message"]) if err["field"] == field
            end
        end
        messages
    end

    # class attribute merging the caller's classes with the error marker.
    def class_attribute(field, opts)
        css = ""
        css = opts["class"].to_s unless opts["class"].nil?
        if !this.field_errors(field).empty?
            if css.blank?
                css = "field-error"
            else
                css = css + " field-error"
            end
        end
        return "" if css.blank?

        escaped_css = attr(css)
        " class=\"#{escaped_css}\""
    end

    def invalid_attribute(field)
        return "" if this.field_errors(field).empty?

        " aria-invalid=\"true\""
    end

    # Render an options hash as HTML attributes, skipping excluded keys.
    # true renders a bare attribute, false/null skip it.
    def attributes_without(opts, excluded)
        return "" if opts.nil?

        html = ""
        for name in opts.keys()
            if !excluded.includes?(name)
                value = opts[name]
                if value == true
                    html = html + " #{name}"
                else
                    if !(value.nil? || value == false)
                        escaped_value = attr(value.to_s)
                        html = html + " #{name}=\"#{escaped_value}\""
                    end
                end
            end
        end
        html
    end
end

# Build a FormBuilder for a record (or null for a bare form). Options:
#   "url"       override the derived action URL (required with no record)
#   "method"    override the derived verb ("post"/"patch"/"put"/"delete"/"get")
#   "multipart" true adds enctype="multipart/form-data" (for file_field)
#   anything else becomes an attribute on the <form> tag
def form_with(record = null, options = null)
    opts = options ?? {}
    names = __soli_form_names(record)

    url = opts["url"]
    http_method = opts["method"].to_s.downcase()

    if !names.nil?
        key = names["key"]
        if url.nil?
            collection = names["collection"]
            if key.nil?
                url = "/" + collection
            else
                url = "/" + collection + "/" + key.to_s
            end
        end
        if http_method.blank?
            if key.nil?
                http_method = "post"
            else
                http_method = "patch"
            end
        end
    end
    http_method = "post" if http_method.blank?

    form_attrs = {}
    form_attrs["enctype"] = "multipart/form-data" if opts["multipart"] == true
    for name in opts.keys()
        if !["url", "method", "multipart"].includes?(name)
            form_attrs[name] = opts[name]
        end
    end

    new FormBuilder(record, url.to_s, http_method, form_attrs)
end

# Hidden input carrying the per-session CSRF token — embedded by
# FormBuilder.open() and button_to(); use directly in hand-written forms.
def csrf_field()
    token = attr(csrf_token())
    "<input type=\"hidden\" name=\"_csrf_token\" value=\"#{token}\">"
end

# <meta> tag for layouts so JS (fetch/htmx) can send X-CSRF-Token.
def csrf_meta_tag()
    token = attr(csrf_token())
    "<meta name=\"csrf-token\" content=\"#{token}\">"
end

# A single-button form for state-changing links (delete buttons etc.).
# Options: "method" (default "post"), "confirm" (JS confirm dialog),
# "form_class" (class on the <form>); anything else becomes a button
# attribute.
def button_to(text, target_url, options = null)
    opts = options ?? {}
    http_method = opts["method"].to_s.downcase()
    http_method = "post" if http_method.blank?

    browser_method = "POST"
    browser_method = "GET" if http_method == "get"

    form_class_attr = ""
    if !opts["form_class"].nil?
        form_css = attr(opts["form_class"].to_s)
        form_class_attr = " class=\"#{form_css}\""
    end

    action = attr(target_url)
    html = "<form action=\"#{action}\" method=\"#{browser_method}\"#{form_class_attr}>"
    if !["get", "post"].includes?(http_method)
        override = http_method.upcase()
        html = html + "<input type=\"hidden\" name=\"_method\" value=\"#{override}\">"
    end
    html = html + csrf_field() unless http_method == "get"

    button_attrs = ""
    for name in opts.keys()
        if !["method", "confirm", "form_class"].includes?(name)
            value = opts[name]
            if value == true
                button_attrs = button_attrs + " #{name}"
            else
                if !(value.nil? || value == false)
                    escaped_value = attr(value.to_s)
                    button_attrs = button_attrs + " #{name}=\"#{escaped_value}\""
                end
            end
        end
    end
    if !opts["confirm"].nil?
        confirm_js = j(opts["confirm"].to_s)
        button_attrs = button_attrs + " onclick=\"return confirm('#{confirm_js}')\""
    end

    caption = h(text.to_s)
    html + "<button type=\"submit\"#{button_attrs}>#{caption}</button></form>"
end
