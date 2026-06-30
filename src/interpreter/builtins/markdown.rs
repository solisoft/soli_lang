//! Markdown rendering builtin.
//!
//! Exposes `Markdown.to_html(string)` to convert Markdown source text into HTML.
//! Uses pulldown-cmark with tables, strikethrough, and task list extensions enabled.
//!
//! ## Example
//!
//! ```soli
//! let html = Markdown.to_html("# Hello\n\nThis is **bold**.")
//! // => "<h1>Hello</h1>\n<p>This is <strong>bold</strong>.</p>\n"
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{hash_from_pairs, Class, NativeFunction, Value};
use crate::template::{markdown_to_html, markdown_to_safe_html, markdown_to_spans};

/// Register the `Markdown` class with its static methods.
pub fn register_markdown_builtins(env: &mut Environment) {
    let mut static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Markdown.to_html(string) -> string
    // Converts Markdown source text to HTML.
    static_methods.insert(
        "to_html".to_string(),
        Rc::new(NativeFunction::new("Markdown.to_html", Some(1), |args| {
            let md = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Markdown.to_html() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(markdown_to_html(&md).into()))
        })),
    );

    // Markdown.to_safe_html(string) -> string
    // Converts Markdown source text to HTML, escaping raw HTML and neutralizing unsafe URLs.
    static_methods.insert(
        "to_safe_html".to_string(),
        Rc::new(NativeFunction::new(
            "Markdown.to_safe_html",
            Some(1),
            |args| {
                let md = match &args[0] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "Markdown.to_safe_html() expects string, got {}",
                            other.type_name()
                        ))
                    }
                };
                Ok(Value::String(markdown_to_safe_html(&md).into()))
            },
        )),
    );

    // Markdown.to_spans(string) -> Array<Hash>
    // Parses inline markdown into PDF paragraph spans: `**bold**` -> fontWeight,
    // `*italic*` -> italic, `` `code` `` -> mono, `[t](url)` -> link. The hashes
    // use the camelCase keys a PDF template's `spans` expects.
    static_methods.insert(
        "to_spans".to_string(),
        Rc::new(NativeFunction::new("Markdown.to_spans", Some(1), |args| {
            let md = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Markdown.to_spans() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let spans: Vec<Value> = markdown_to_spans(&md)
                .into_iter()
                .map(|s| {
                    let mut pairs: Vec<(&str, Value)> =
                        vec![("text", Value::String(s.text.into()))];
                    if s.bold {
                        pairs.push(("fontWeight", Value::String("bold".into())));
                    }
                    if s.italic {
                        pairs.push(("italic", Value::Bool(true)));
                    }
                    if s.mono {
                        pairs.push(("mono", Value::Bool(true)));
                    }
                    if let Some(url) = s.link {
                        pairs.push(("link", Value::String(url.into())));
                    }
                    hash_from_pairs(pairs)
                })
                .collect();
            Ok(Value::Array(Rc::new(RefCell::new(spans))))
        })),
    );

    let markdown_class = Class {
        name: "Markdown".to_string(),
        native_static_methods: static_methods,
        ..Default::default()
    };

    env.define(
        "Markdown".to_string(),
        Value::Class(Rc::new(markdown_class)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::environment::Environment;
    use crate::interpreter::value::Value;

    fn call_markdown_method(method_name: &str, input: &str) -> Result<Value, String> {
        let mut env = Environment::new();
        register_markdown_builtins(&mut env);

        let class = env.get("Markdown").unwrap();
        if let Value::Class(cls) = class {
            let method = cls.native_static_methods.get(method_name).unwrap();
            (method.func)(vec![Value::String(input.to_string().into())])
        } else {
            panic!("Markdown is not a class");
        }
    }

    fn call_to_html(input: &str) -> Result<Value, String> {
        call_markdown_method("to_html", input)
    }

    fn call_to_safe_html(input: &str) -> Result<Value, String> {
        call_markdown_method("to_safe_html", input)
    }

    #[test]
    fn test_heading() {
        let result = call_to_html("# Hello").unwrap();
        if let Value::String(s) = result {
            assert!(s.contains("<h1>Hello</h1>"));
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn test_bold_and_italic() {
        let result = call_to_html("**bold** and *italic*").unwrap();
        if let Value::String(s) = result {
            assert!(s.contains("<strong>bold</strong>"));
            assert!(s.contains("<em>italic</em>"));
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn test_table() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        let result = call_to_html(md).unwrap();
        if let Value::String(s) = result {
            assert!(s.contains("<table>"));
            assert!(s.contains("<th>A</th>"));
            assert!(s.contains("<td>1</td>"));
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn test_strikethrough() {
        let result = call_to_html("~~deleted~~").unwrap();
        if let Value::String(s) = result {
            assert!(s.contains("<del>deleted</del>"));
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn test_code_block() {
        let md = "```\nlet x = 1\n```";
        let result = call_to_html(md).unwrap();
        if let Value::String(s) = result {
            assert!(s.contains("<code>"));
            assert!(s.contains("let x = 1"));
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn test_list() {
        let md = "- one\n- two\n- three";
        let result = call_to_html(md).unwrap();
        if let Value::String(s) = result {
            assert!(s.contains("<ul>"));
            assert!(s.contains("<li>one</li>"));
            assert!(s.contains("<li>two</li>"));
            assert!(s.contains("<li>three</li>"));
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn test_link() {
        let result = call_to_html("[click](https://example.com)").unwrap();
        if let Value::String(s) = result {
            assert!(s.contains("<a href=\"https://example.com\">click</a>"));
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn test_safe_html_escapes_raw_html() {
        let result = call_to_safe_html("Hello <script>alert(1)</script>").unwrap();
        if let Value::String(s) = result {
            assert!(s.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
            assert!(!s.contains("<script>"));
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn test_safe_html_neutralizes_javascript_links() {
        let result = call_to_safe_html("[click](javascript:alert(1))").unwrap();
        if let Value::String(s) = result {
            assert!(s.contains("<a href=\"#\">click</a>"));
            assert!(!s.contains("javascript:"));
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn test_empty_input() {
        let result = call_to_html("").unwrap();
        if let Value::String(s) = result {
            assert_eq!(s.as_str(), "");
        } else {
            panic!("expected string");
        }
    }

    #[test]
    fn test_wrong_type_returns_error() {
        let mut env = Environment::new();
        register_markdown_builtins(&mut env);

        let class = env.get("Markdown").unwrap();
        if let Value::Class(cls) = class {
            let method = cls.native_static_methods.get("to_html").unwrap();
            let result = (method.func)(vec![Value::Int(42)]);
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("expects string"));
        }
    }
}
