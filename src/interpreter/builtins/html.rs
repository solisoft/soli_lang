//! HTML string manipulation functions.
//!
//! Provides shared implementations for HTML escaping, unescaping, sanitization, and stripping.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Register all HTML built-in functions.
pub fn register_html_builtins(env: &mut Environment) {
    // html_escape(string) - Escape HTML special characters
    env.define(
        "html_escape".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "html_escape",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(html_escape(s))),
                other => Err(format!(
                    "html_escape expects string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // html_unescape(string) - Unescape HTML entities
    env.define(
        "html_unescape".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "html_unescape",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(html_unescape(s))),
                other => Err(format!(
                    "html_unescape expects string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // sanitize_html(string) - Remove dangerous HTML tags and attributes
    env.define(
        "sanitize_html".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "sanitize_html",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(sanitize_html(s))),
                other => Err(format!(
                    "sanitize_html expects string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // strip_html(string) -> string - removes all HTML tags
    env.define(
        "strip_html".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "strip_html",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(strip_html(s))),
                other => Err(format!(
                    "strip_html expects string, got {}",
                    other.type_name()
                )),
            },
        )),
    );
}

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
        ("&#x27;", "'"),
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

                // Only allow these safe tags
                let allowed_tags = [
                    "p",
                    "br",
                    "b",
                    "i",
                    "u",
                    "em",
                    "strong",
                    "a",
                    "ul",
                    "ol",
                    "li",
                    "blockquote",
                    "code",
                    "pre",
                    "h1",
                    "h2",
                    "h3",
                    "h4",
                    "h5",
                    "h6",
                    "span",
                    "div",
                ];
                let is_allowed = allowed_tags.contains(&tag_name);

                // Check for dangerous event handlers (comprehensive list)
                let dangerous_patterns = [
                    "javascript:",
                    "vbscript:",
                    "data:",
                    "onload",
                    "onerror",
                    "onclick",
                    "onmouseover",
                    "onmouseout",
                    "onfocus",
                    "onblur",
                    "onchange",
                    "onsubmit",
                    "onreset",
                    "onkeydown",
                    "onkeyup",
                    "onkeypress",
                    "onmousedown",
                    "onmouseup",
                    "onabort",
                    "onresize",
                    "onscroll",
                    "onwheel",
                    "ontouchstart",
                    "ontouchend",
                    "ontouchmove",
                    "ontouchcancel",
                    "oncontextmenu",
                    "ondragstart",
                    "ondrag",
                    "ondragend",
                    "ondrop",
                    "onplay",
                    "onpause",
                    "onended",
                    "onvolumechange",
                    "onwaiting",
                    "expression(",
                    "url(",
                    "@import",
                    "<style",
                    "<link",
                    "<object",
                    "<embed",
                    "<iframe",
                    "<script",
                    "<form",
                    "<input",
                    "<button",
                    "<meta",
                    "<svg",
                    "<foreignobject",
                ];

                let mut is_dangerous = false;
                for pattern in &dangerous_patterns {
                    if tag.contains(pattern) {
                        is_dangerous = true;
                        break;
                    }
                }

                // Also check for attribute values containing dangerous content
                let tag_lower = tag.to_lowercase();
                if tag_lower.contains("href=\"javascript:")
                    || tag_lower.contains("href='javascript:")
                    || tag_lower.contains("src=\"javascript:")
                    || tag_lower.contains("src='javascript:")
                    || tag_lower.contains("href=\"data:")
                    || tag_lower.contains("href='data:")
                    || tag_lower.contains("src=\"data:")
                    || tag_lower.contains("src='data:")
                {
                    is_dangerous = true;
                }

                if is_allowed && !is_dangerous {
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
                        // Only these safe attributes
                        let safe_attrs = ["href", "title", "alt", "class", "id", "style"];
                        let safe_attrs_result: Vec<String> = attrs
                            .iter()
                            .filter_map(|&attr| {
                                let parts: Vec<&str> = attr.splitn(2, '=').collect();
                                if parts.len() == 2 {
                                    let attr_name = parts[0].to_lowercase();
                                    let attr_value = parts[1].trim_matches('"').trim_matches('\'');
                                    // Only allow specific safe attributes with safe values
                                    if safe_attrs.contains(&attr_name.as_str())
                                        && !attr_value.to_lowercase().contains("javascript:")
                                        && !attr_value.to_lowercase().contains("vbscript:")
                                        && !attr_value.to_lowercase().contains("data:")
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
