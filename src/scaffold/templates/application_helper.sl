# Application-wide view helpers

# Truncate text to a maximum length with ellipsis
def truncate_text(text: String, length: Int, suffix: String) -> String
    if len(text) <= length
        return text
    end
    return substring(text, 0, length - len(suffix)) + suffix
end

# Capitalize first letter of a string
def capitalize(text: String) -> String
    if len(text) == 0
        return text
    end
    return upcase(substring(text, 0, 1)) + substring(text, 1, len(text))
end

# Generate an HTML link
def link_to(text: String, url: String) -> String
    return "<a href=\"" + html_escape(url) + "\">" + html_escape(text) + "</a>"
end

# Generate an HTML link with CSS class
def link_to_class(text: String, url: String, css_class: String) -> String
    return "<a href=\"" + html_escape(url) + "\" class=\"" + html_escape(css_class) + "\">" + html_escape(text) + "</a>"
end

# Pluralize a word based on count
def pluralize(count: Int, singular: String, plural: String) -> String
    if count == 1
        return str(count) + " " + singular
    end
    return str(count) + " " + plural
end

# Simple pluralize (adds 's')
def pluralize_simple(count: Int, word: String) -> String
    if count == 1
        return str(count) + " " + word
    end
    return str(count) + " " + word + "s"
end
