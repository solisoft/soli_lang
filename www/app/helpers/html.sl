// HTML formatting helpers

fn truncate_html(html: String, length: Int) -> String {
    if (len(html) <= length) {
        return html;
    }
    return substring(html, 0, length) + "...";
}

fn strip_tags(html: String) -> String {
    return strip_html(html);
}

fn format_html_content(content: String) -> String {
    let escaped = html_escape(content);
    return escaped;
}
