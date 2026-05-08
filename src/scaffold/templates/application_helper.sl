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

# SEC-012: Reject href values that would let an attacker run JS through
# `javascript:` (or similar) URL schemes. HTML-escaping the URL is *not*
# enough — the browser still parses `javascript:alert(1)` inside an
# `href` attribute. Mirror the allowlist used by the markdown sanitiser.
def _is_safe_link_url(url)
    lower = url.downcase()
    if lower.starts_with("http://") or lower.starts_with("https://") or lower.starts_with("mailto:")
        return true
    end
    if lower.starts_with("/") or lower.starts_with("#") or lower.starts_with("?")
        return true
    end
    # No allowed scheme prefix; treat as relative *only* if there is no
    # scheme separator (`:`) before the first /?#. Anything else is a
    # custom scheme like javascript:/data: and must be refused.
    cut = len(lower)
    s = lower.index_of("/")
    if s != -1 and s < cut
        cut = s
    end
    q = lower.index_of("?")
    if q != -1 and q < cut
        cut = q
    end
    h = lower.index_of("#")
    if h != -1 and h < cut
        cut = h
    end
    return !lower.substring(0, cut).contains(":")
end

def _safe_link_url(url)
    if _is_safe_link_url(url)
        return url
    end
    return "#"
end

# Generate an HTML link
def link_to(text: String, url: String) -> String
    return "<a href=\"" + html_escape(_safe_link_url(url)) + "\">" + html_escape(text) + "</a>"
end

# Generate an HTML link with CSS class
def link_to_class(text: String, url: String, css_class: String) -> String
    return "<a href=\"" + html_escape(_safe_link_url(url)) + "\" class=\"" + html_escape(css_class) + "\">" + html_escape(text) + "</a>"
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
