//! JSON class for parsing and stringifying JSON data.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{parse_json, stringify_to_string, Class, NativeFunction, Value};

/// Register the JSON class with static methods.
/// Escape a JSON document so it can be embedded in an HTML `<script>` body.
///
/// JSON is not safe there as-is. `</script>` inside any string value ends the
/// element — the HTML tokenizer does not care that it sits inside a JSON
/// string — and everything after it is parsed as markup, which is a stored XSS
/// in the most ordinary `window.__data = <%- json_stringify(post) %>` line. The
/// only alternative Soli documented, `j()`, is an HTML escape: it turns `&` into
/// `&amp;` *inside* the JSON, corrupting the data, which is precisely what
/// pushed people to the raw form.
///
/// `<`, `>` and `&` become `\uXXXX` escapes, which JSON parsers read back as
/// the original characters, so the value is unchanged for JavaScript while
/// carrying nothing the HTML parser reacts to. U+2028 and U+2029 are escaped
/// too: they are valid in JSON strings but are line terminators in JavaScript
/// source, and an unescaped one is a syntax error.
fn escape_json_for_script(json: &str) -> String {
    let mut out = String::with_capacity(json.len() + 16);
    for ch in json.chars() {
        match ch {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    out
}

pub fn register_json_class(env: &mut Environment) {
    let mut json_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // JSON.parse(string) - Parse JSON string to Value
    json_static_methods.insert(
        "parse".to_string(),
        Rc::new(NativeFunction::new("JSON.parse", Some(1), |args| {
            let json_str = match &args[0] {
                Value::String(s) => s.as_str(),
                other => {
                    return Err(format!(
                        "JSON.parse() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            parse_json(json_str)
        })),
    );

    // JSON.stringify(value) - Convert Value to JSON string
    json_static_methods.insert(
        "stringify".to_string(),
        Rc::new(NativeFunction::new("JSON.stringify", Some(1), |args| {
            let json_str = stringify_to_string(&args[0])
                .map_err(|e| format!("JSON serialization error: {}", e))?;
            Ok(Value::String(json_str.into()))
        })),
    );

    // JSON.parse_jsonp(string) - Unwrap a JSONP `callback({...});` string and
    // parse the inner JSON into a Value.
    json_static_methods.insert(
        "parse_jsonp".to_string(),
        Rc::new(NativeFunction::new("JSON.parse_jsonp", Some(1), |args| {
            let jsonp_str = match &args[0] {
                Value::String(s) => s.as_str(),
                other => {
                    return Err(format!(
                        "JSON.parse_jsonp() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let inner = crate::interpreter::jsonp::strip_jsonp_padding(jsonp_str)?;
            parse_json(inner)
        })),
    );

    // Create JSON class
    let json_class = Class {
        name: "JSON".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: json_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    env.define("JSON".to_string(), Value::Class(Rc::new(json_class)));

    // json_script(value) — JSON safe to embed in an HTML <script> body.
    env.define(
        "json_script".to_string(),
        Value::NativeFunction(NativeFunction::new("json_script", Some(1), |args| {
            let json_str = stringify_to_string(&args[0])
                .map_err(|e| format!("JSON serialization error: {}", e))?;
            Ok(Value::String(escape_json_for_script(&json_str).into()))
        })),
    );

    // Legacy standalone aliases: json_stringify() and json_parse()
    env.define(
        "json_stringify".to_string(),
        Value::NativeFunction(NativeFunction::new("json_stringify", Some(1), |args| {
            let json_str = stringify_to_string(&args[0])
                .map_err(|e| format!("JSON serialization error: {}", e))?;
            Ok(Value::String(json_str.into()))
        })),
    );

    env.define(
        "json_parse".to_string(),
        Value::NativeFunction(NativeFunction::new("json_parse", Some(1), |args| {
            let json_str = match &args[0] {
                Value::String(s) => s.as_str(),
                other => {
                    return Err(format!(
                        "json_parse() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            parse_json(json_str)
        })),
    );
}

#[cfg(test)]
mod json_script_tests {
    use super::*;

    /// The attack: a string value containing `</script>` closes the element, and
    /// everything after it is parsed as markup.
    #[test]
    fn a_closing_script_tag_cannot_end_the_element() {
        let json = r#"{"title":"</script><script>alert(1)</script>"}"#;
        let escaped = escape_json_for_script(json);
        assert!(!escaped.contains("</script>"), "{escaped}");
        assert!(!escaped.contains('<'), "{escaped}");
        assert!(!escaped.contains('>'), "{escaped}");
    }

    /// HTML entity contexts are neutralised too, so the payload cannot be
    /// reconstructed by the parser.
    #[test]
    fn ampersands_are_escaped() {
        let escaped = escape_json_for_script(r#"{"a":"x&y"}"#);
        assert!(!escaped.contains('&'), "{escaped}");
        assert!(escaped.contains(r"\u0026"), "{escaped}");
    }

    /// U+2028 and U+2029 are valid in a JSON string but are line terminators in
    /// JavaScript source: unescaped, they are a syntax error in the page.
    #[test]
    fn javascript_line_terminators_are_escaped() {
        let escaped = escape_json_for_script("{\"a\":\"x\u{2028}y\u{2029}z\"}");
        assert!(!escaped.contains('\u{2028}'), "{escaped}");
        assert!(!escaped.contains('\u{2029}'), "{escaped}");
        assert!(escaped.contains("\\u2028"), "{escaped}");
    }

    /// The escapes must be JSON `\uXXXX`, which parses back to the original
    /// characters — the data a script reads has to be unchanged.
    #[test]
    fn the_value_round_trips_through_a_json_parser() {
        let original = serde_json::json!({"title": "</script> & <b>bold</b>"});
        let escaped = escape_json_for_script(&original.to_string());
        let parsed: serde_json::Value = serde_json::from_str(&escaped).expect("still valid JSON");
        assert_eq!(parsed, original, "escaping must not change the data");
    }

    /// Ordinary payloads are untouched, so the helper costs nothing to adopt.
    #[test]
    fn plain_json_is_unchanged() {
        let plain = r#"{"id":7,"name":"Ada"}"#;
        assert_eq!(escape_json_for_script(plain), plain);
    }
}
