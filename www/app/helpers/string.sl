// String manipulation helpers

fn truncate(text: String, length: Int, suffix: String) -> String {
    if (len(text) <= length) {
        return text;
    }
    return substring(text, 0, length - len(suffix)) + suffix;
}

fn truncate_default(text: String, length: Int) -> String {
    return truncate(text, length, "...");
}

fn slugify(text: String) -> String {
    let lower = downcase(text);
    let with_spaces = replace(lower, "_", "-");
    let with_dashes = replace(with_spaces, " ", "-");
    let cleaned = sanitize_html(with_dashes);
    return cleaned;
}

fn titleize(text: String) -> String {
    let words = split(text, " ");
    let titleized = map(words, fn(w) {
        if (len(w) > 0) {
            return upcase(substring(w, 0, 1)) + substring(w, 1, len(w));
        }
        return w;
    });
    return join(titleized, " ");
}
