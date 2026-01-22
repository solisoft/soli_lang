//! HTML string manipulation functions.
//!
//! Provides shared implementations for HTML escaping, unescaping, sanitization, and stripping.

/// Escape HTML special characters.
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Unescape HTML entities back to characters.
pub fn html_unescape(s: &str) -> String {
    let mut result = s.to_string();
    let replacements = [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&apos;", "'"),
        ("&nbsp;", " "),
    ];
    for (from, to) in replacements {
        result = result.replace(from, to);
    }
    result
}

/// Strip all HTML tags from a string.
pub fn strip_html(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;

    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(c);
        }
    }
    result
}

/// Sanitize HTML by keeping only safe tags and removing dangerous attributes.
pub fn sanitize_html(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let mut tag_buffer = String::new();

    for c in s.chars() {
        if c == '<' {
            in_tag = true;
            tag_buffer.clear();
            tag_buffer.push(c);
        } else if c == '>' {
            if in_tag {
                tag_buffer.push(c);
                let tag = tag_buffer.trim().to_lowercase();
                let is_closing = tag.starts_with("</");
                let is_self_closing = tag.ends_with("/>");
                let tag_name = if is_closing {
                    tag.trim_start_matches('<')
                        .trim_start_matches('/')
                        .trim_end_matches('>')
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                } else {
                    tag.trim_start_matches('<')
                        .trim_end_matches('/')
                        .trim_end_matches('>')
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                };
                let allowed_tags = [
                    "p", "br", "b", "i", "u", "em", "strong", "a", "ul", "ol", "li",
                    "blockquote", "code", "pre", "h1", "h2", "h3", "h4", "h5", "h6",
                    "span", "div", "img",
                ];
                let is_allowed = allowed_tags.contains(&tag_name);
                let is_dangerous_attr = tag.contains("javascript:")
                    || tag.contains("onload=")
                    || tag.contains("onerror=")
                    || tag.contains("onclick=");
                if is_allowed && !is_dangerous_attr {
                    let cleaned_tag = if is_closing {
                        format!("</{}>", tag_name)
                    } else if is_self_closing {
                        format!("<{}/>", tag_name)
                    } else {
                        let attrs: Vec<&str> = tag
                            .strip_prefix('<')
                            .and_then(|t| t.strip_suffix('>').or(Some(t)))
                            .unwrap_or("")
                            .split_whitespace()
                            .skip(1)
                            .collect();
                        let safe_attrs = ["href", "src", "title", "alt", "class", "id", "style"];
                        let safe_attrs_result: Vec<String> = attrs
                            .iter()
                            .filter_map(|&attr| {
                                let parts: Vec<&str> = attr.splitn(2, '=').collect();
                                if parts.len() == 2 {
                                    let attr_name = parts[0].to_lowercase();
                                    let attr_value = parts[1].trim_matches('"').trim_matches('\'');
                                    if safe_attrs.contains(&attr_name.as_str())
                                        && !attr_value.to_lowercase().contains("javascript:")
                                    {
                                        Some(format!("{}={}", attr_name, parts[1]))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            })
                            .collect();
                        if safe_attrs_result.is_empty() {
                            format!("<{}>", tag_name)
                        } else {
                            format!("<{} {}>", tag_name, safe_attrs_result.join(" "))
                        }
                    };
                    result.push_str(&cleaned_tag);
                }
                in_tag = false;
            } else {
                result.push(c);
            }
        } else if in_tag {
            tag_buffer.push(c);
        } else {
            result.push(c);
        }
    }
    if in_tag {
        result.push_str(&tag_buffer);
    }
    result
}

/// Extract a substring from start to end index.
pub fn substring(s: &str, start: usize, end: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let end = end.min(chars.len());
    let start = start.min(end);
    chars[start..end].iter().collect()
}
