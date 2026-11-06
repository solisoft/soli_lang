//! SOAP built-in class for SoliLang.
//!
//! Provides the SOAP class with static methods for making SOAP calls:
//! - SOAP.call(url, action, envelope) -> Hash
//! - SOAP.call(url, action, envelope, headers) -> Hash
//! - SOAP.wrap(body, namespace?) -> String
//! - SOAP.parse(response) -> Hash

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::builtins::http_class::{
    get_user_http_client, header_map_pairs, read_capped_text_async, response_headers_hash,
    validate_url_for_ssrf,
};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFunction, Value};
use crate::serve::get_tokio_handle;

/// Default SOAP 1.1 namespace
const SOAP11_NS: &str = "http://schemas.xmlsoap.org/soap/envelope/";

pub fn register_soap_class(env: &mut Environment) {
    let mut soap_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // SOAP.call(url, action, envelope) or SOAP.call(url, action, envelope, headers)
    soap_static_methods.insert(
        "call".to_string(),
        Rc::new(NativeFunction::new("SOAP.call", Some(3), |args| {
            if args.len() < 3 {
                return Err(
                    "SOAP.call() requires at least 3 arguments: url, action, envelope".to_string(),
                );
            }

            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "SOAP.call() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_url_for_ssrf(&url)?;

            let action = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "SOAP.call() expects string action, got {}",
                        other.type_name()
                    ))
                }
            };

            let envelope = match &args[2] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "SOAP.call() expects string envelope, got {}",
                        other.type_name()
                    ))
                }
            };

            let mut headers: Vec<(String, String)> = vec![
                (
                    "Content-Type".to_string(),
                    "text/xml; charset=utf-8".to_string(),
                ),
                ("SOAPAction".to_string(), format!("\"{}\"", action)),
            ];

            if args.len() > 3 {
                if let Value::Hash(h) = &args[3] {
                    for (k, v) in h.borrow().iter() {
                        if let HashKey::String(key) = k {
                            let value_str = match v {
                                Value::String(s) => s.clone(),
                                _ => format!("{}", v).into(),
                            };
                            headers.push((key.clone().to_string(), value_str.to_string()));
                        }
                    }
                }
            }

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_user_http_client().clone();
                    match rt.block_on(async move {
                        let mut request = client.post(&*url);

                        for (key, value) in &headers {
                            request = request.header(key.as_str(), value.as_str());
                        }

                        let resp = request
                            .body(envelope.to_string())
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;

                        let status = resp.status().as_u16();
                        let status_text =
                            resp.status().canonical_reason().unwrap_or("").to_string();

                        let resp_headers = response_headers_hash(header_map_pairs(resp.headers()));

                        let body = read_capped_text_async(resp).await?;

                        let parsed_xml = parse_xml_to_value(&body).unwrap_or(Value::Null);

                        let mut result: HashPairs = HashPairs::default();
                        result.insert(HashKey::String("status".into()), Value::Int(status as i64));
                        result.insert(
                            HashKey::String("status_text".into()),
                            Value::String(status_text.into()),
                        );
                        result.insert(
                            HashKey::String("headers".into()),
                            Value::Hash(Rc::new(RefCell::new(resp_headers))),
                        );
                        result.insert(HashKey::String("body".into()), Value::String(body.into()));
                        result.insert(HashKey::String("parsed".into()), parsed_xml);

                        Ok(Value::Hash(Rc::new(RefCell::new(result))))
                    }) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_soap_future(
                    url.to_string(),
                    headers,
                    envelope.to_string(),
                )),
            }
        })),
    );

    // SOAP.wrap(body, namespace?) -> String
    soap_static_methods.insert(
        "wrap".to_string(),
        Rc::new(NativeFunction::new("SOAP.wrap", None, |args| {
            let body = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "SOAP.wrap() expects string body, got {}",
                        other.type_name()
                    ))
                }
            };

            let namespace = if args.len() > 1 {
                match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => SOAP11_NS.to_string().into(),
                }
            } else {
                SOAP11_NS.to_string().into()
            };

            let escape_body = if args.len() > 2 {
                if let Value::Hash(opts) = &args[2] {
                    let opts = opts.borrow();
                    opts.get(&HashKey::String("escape".into()))
                        .map(|v| matches!(v, Value::Bool(true)))
                        .unwrap_or(false)
                } else {
                    false
                }
            } else {
                false
            };

            let safe_body = if escape_body {
                let mut result = String::new();
                for c in body.chars() {
                    match c {
                        '<' => result.push_str("&lt;"),
                        '>' => result.push_str("&gt;"),
                        '&' => result.push_str("&amp;"),
                        '"' => result.push_str("&quot;"),
                        '\'' => result.push_str("&apos;"),
                        _ => result.push(c),
                    }
                }
                result
            } else {
                body.to_string()
            };

            let envelope = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<soap:Envelope xmlns:soap="{}">
  <soap:Body>
    {}
  </soap:Body>
</soap:Envelope>"#,
                namespace, safe_body
            );

            Ok(Value::String(envelope.into()))
        })),
    );

    // SOAP.parse(xml_string) -> Hash
    soap_static_methods.insert(
        "parse".to_string(),
        Rc::new(NativeFunction::new("SOAP.parse", Some(1), |args| {
            let xml = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "SOAP.parse() expects string XML, got {}",
                        other.type_name()
                    ))
                }
            };

            parse_xml_to_value(&xml)
        })),
    );

    // SOAP.xml_escape(string) -> String
    soap_static_methods.insert(
        "xml_escape".to_string(),
        Rc::new(NativeFunction::new("SOAP.xml_escape", Some(1), |args| {
            let text = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "SOAP.xml_escape() expects string, got {}",
                        other.type_name()
                    ))
                }
            };

            let escaped = text
                .replace("&", "&amp;")
                .replace("<", "&lt;")
                .replace(">", "&gt;")
                .replace("\"", "&quot;")
                .replace("'", "&apos;");

            Ok(Value::String(escaped))
        })),
    );

    // SOAP.to_xml(hash, root_element?) -> String
    soap_static_methods.insert(
        "to_xml".to_string(),
        Rc::new(NativeFunction::new("SOAP.to_xml", Some(1), |args| {
            let hash = match &args[0] {
                Value::Hash(h) => h.clone(),
                other => {
                    return Err(format!(
                        "SOAP.to_xml() expects hash, got {}",
                        other.type_name()
                    ))
                }
            };

            let root_element = if args.len() > 1 {
                match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => "root".into(),
                }
            } else {
                "root".into()
            };

            let xml = value_to_xml(&Value::Hash(hash), &root_element);
            Ok(Value::String(xml.into()))
        })),
    );

    let soap_class = Class {
        name: "SOAP".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: soap_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };

    env.define("SOAP".to_string(), Value::Class(Rc::new(soap_class)));
}

#[allow(clippy::arc_with_non_send_sync)]
fn spawn_soap_future(url: String, headers: Vec<(String, String)>, envelope: String) -> Value {
    use crate::interpreter::value::{FutureState, HttpFutureKind};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;

    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        // SEC-007a + SEC-015a: route through the SSRF-aware reqwest client.
        // Builds a private current-thread tokio runtime per call so the
        // connect-time DNS lookup goes through `SsrfBlockingResolver`
        // (closing the DNS-rebinding TOCTOU `ureq` couldn't cover) while
        // preserving the existing redirect-disabled posture.
        let client = get_user_http_client().clone();
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                let _ = tx.send(Ok(format!(
                    "{{\"error\": \"Failed to build tokio runtime: {}\"}}",
                    e
                )));
                return;
            }
        };

        let result = rt.block_on(async {
            let mut request = client.post(&url);
            for (key, value) in &headers {
                request = request.header(key.as_str(), value.as_str());
            }

            match request.body(envelope).send().await {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let status_text = if resp.status().is_success() {
                        "OK"
                    } else {
                        "Error"
                    }
                    .to_string();
                    let body = read_capped_text_async(resp).await.unwrap_or_default();
                    let parsed = parse_xml_to_value(&body).unwrap_or(Value::Null);

                    let mut result: HashPairs = HashPairs::default();
                    result.insert(HashKey::String("status".into()), Value::Int(status as i64));
                    result.insert(
                        HashKey::String("status_text".into()),
                        Value::String(status_text.into()),
                    );
                    result.insert(
                        HashKey::String("headers".into()),
                        Value::Hash(Rc::new(RefCell::new(HashPairs::default()))),
                    );
                    result.insert(HashKey::String("body".into()), Value::String(body.into()));
                    result.insert(HashKey::String("parsed".into()), parsed);

                    build_json_from_value(&Value::Hash(Rc::new(RefCell::new(result))))
                }
                Err(e) => serde_json::json!({"error": e.to_string()}).to_string(),
            }
        });

        let _ = tx.send(Ok(result));
    });

    crate::interpreter::value::note_future_created();
    Value::Future(Arc::new(Mutex::new(FutureState::Pending {
        receiver: rx,
        kind: HttpFutureKind::FullResponse,
    })))
}

fn build_json_from_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => format!("\"{}\"", s.replace("\"", "\\\"")),
        Value::Array(arr) => {
            let items: Vec<String> = arr.borrow().iter().map(build_json_from_value).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Hash(h) => {
            let items: Vec<String> = h
                .borrow()
                .iter()
                .map(|(k, v)| {
                    let key = match k {
                        HashKey::String(s) => format!("\"{}\"", s),
                        HashKey::Symbol(s) => format!("\"{}\"", s),
                        HashKey::Int(i) => i.to_string(),
                        HashKey::Decimal(d) => d.to_string(),
                        HashKey::Bool(b) => b.to_string(),
                        HashKey::Null => "null".to_string(),
                        HashKey::ValueObject(key) => format!("\"{}\"", key.to_value()),
                    };
                    format!("{}: {}", key, build_json_from_value(v))
                })
                .collect();
            format!("{{{}}}", items.join(", "))
        }
        _ => "null".to_string(),
    }
}

/// `SOAP.parse` and the reply of `SOAP.call` (both paths): the shared XML
/// core under `ParsePolicy::soap()` (DOCTYPE refused, depth 64, 1 MiB of text
/// per element, unknown entities dropped, attributes ignored), projected into
/// today's shape by `soap_shape`. `Value`s are built here, on the calling
/// thread or the worker thread, exactly where the old parser built them.
fn parse_xml_to_value(xml: &str) -> Result<Value, String> {
    let tree = crate::xml::parse(xml, &crate::xml::ParsePolicy::soap())
        .map_err(|err| crate::xml::soap_error_message(&err))?;
    Ok(proj_to_value(crate::xml::soap_shape(&tree)))
}

/// A projection as a Soli value. Recursion is bounded by the SOAP depth
/// limit (64, plus one self-closing leaf).
fn proj_to_value(mut proj: crate::xml::Proj) -> Value {
    use crate::xml::Proj;
    match &mut proj {
        Proj::Null => Value::Null,
        Proj::Str(s) => Value::String(std::mem::take(s).into()),
        Proj::List(items) => Value::Array(Rc::new(RefCell::new(
            items.drain(..).map(proj_to_value).collect(),
        ))),
        Proj::Map(map) => {
            let mut pairs = HashPairs::default();
            for (key, value) in map.drain(..) {
                pairs.insert(HashKey::String(key.into()), proj_to_value(value));
            }
            Value::Hash(Rc::new(RefCell::new(pairs)))
        }
    }
}

fn value_to_xml(value: &Value, element_name: &str) -> String {
    match value {
        Value::Null => format!("<{} />", xml_escape_element(element_name)),
        Value::Bool(b) => format!("<{}>{}</{}>", element_name, b, element_name),
        Value::Int(i) => format!("<{}>{}</{}>", element_name, i, element_name),
        Value::Float(f) => format!("<{}>{}</{}>", element_name, f, element_name),
        Value::String(s) => format!(
            "<{}>{}</{}>",
            element_name,
            xml_escape_content(s),
            element_name
        ),
        Value::Array(arr) => {
            let items: Vec<String> = arr
                .borrow()
                .iter()
                .enumerate()
                .map(|(i, item)| value_to_xml(item, &format!("{}_{}", element_name, i)))
                .collect();
            if items.len() == 1 {
                items[0].clone()
            } else {
                items.join("\n")
            }
        }
        Value::Hash(h) => {
            let hash = h.borrow();
            let mut attributes = String::new();
            let mut children = String::new();
            let mut text_content = String::new();

            for (k, v) in hash.iter() {
                let key = match k {
                    HashKey::String(s) => s.clone(),
                    HashKey::Symbol(s) => s.clone(),
                    HashKey::Int(i) => i.to_string().into(),
                    HashKey::Decimal(d) => d.to_string().into(),
                    HashKey::Bool(b) => b.to_string().into(),
                    HashKey::Null => "null".into(),
                    HashKey::ValueObject(key) => key.to_value().to_string().into(),
                };

                if let Some(attr_name) = key.strip_prefix('@') {
                    let attr_value = get_value_string(v);
                    attributes.push_str(&format!(
                        " {}=\"{}\"",
                        xml_escape_attribute(attr_name),
                        xml_escape_attribute(&attr_value)
                    ));
                } else if key == "_text" {
                    text_content = xml_escape_content(&get_value_string(v));
                } else {
                    let child_xml = value_to_xml(v, &key);
                    children.push_str(&child_xml);
                    children.push('\n');
                }
            }

            if children.trim().is_empty() {
                if text_content.is_empty() {
                    format!("<{}{} />", element_name, attributes)
                } else {
                    format!(
                        "<{}{}>{}</{}>",
                        element_name, attributes, text_content, element_name
                    )
                }
            } else {
                format!(
                    "<{}{}>{}{}</{}>",
                    element_name,
                    attributes,
                    if !text_content.is_empty() {
                        text_content + "\n"
                    } else {
                        "".to_string()
                    },
                    children.trim_end(),
                    element_name
                )
            }
        }
        _ => format!("<{} />", element_name),
    }
}

fn get_value_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.clone().to_string(),
        Value::Array(arr) => format!(
            "[{}]",
            arr.borrow()
                .iter()
                .map(get_value_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Hash(h) => {
            let items: Vec<String> = h
                .borrow()
                .iter()
                .map(|(k, v)| {
                    let key = match k {
                        HashKey::String(s) => s.clone(),
                        HashKey::Symbol(s) => s.clone(),
                        HashKey::Int(i) => i.to_string().into(),
                        HashKey::Decimal(d) => d.to_string().into(),
                        HashKey::Bool(b) => b.to_string().into(),
                        HashKey::Null => "null".into(),
                        HashKey::ValueObject(key) => key.to_value().to_string().into(),
                    };
                    format!("{}: {}", key, get_value_string(v))
                })
                .collect();
            format!("{{{}}}", items.join(", "))
        }
        _ => "".to_string(),
    }
}

fn xml_escape_content(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn xml_escape_element(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .replace(' ', "_")
}

fn xml_escape_attribute(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    //! Regression coverage for the SEC-008 SOAP-XML hardening (DOCTYPE
    //! rejection, depth cap, text-size cap) and for the move onto the shared
    //! XML core. `tests/builtins/soap_compat_spec.sl` is the full table.
    use super::*;

    type TestResult = Result<(), String>;

    const SOAP_MAX_DEPTH: usize = 64;
    const SOAP_MAX_TEXT_BYTES: usize = 1024 * 1024;

    fn json(xml: &str) -> Result<String, String> {
        crate::interpreter::value::stringify_to_string(&parse_xml_to_value(xml)?)
    }

    fn refused(xml: &str) -> Result<String, String> {
        match parse_xml_to_value(xml) {
            Ok(v) => Err(format!("expected {xml:?} to be refused, got {v}")),
            Err(err) => Ok(err),
        }
    }

    #[test]
    fn rejects_doctype_declarations() -> TestResult {
        let xml = r#"<?xml version="1.0"?>
<!DOCTYPE foo [<!ENTITY a "hello">]>
<root><body>&a;</body></root>"#;
        assert_eq!(
            refused(xml)?,
            "XML parsing error: DOCTYPE declarations are not allowed in SOAP payloads"
        );
        Ok(())
    }

    #[test]
    fn rejects_excessive_nesting() -> TestResult {
        let nested = |depth: usize, leaf: &str| {
            let mut xml = String::from("<?xml version=\"1.0\"?>");
            for i in 0..depth {
                xml.push_str(&format!("<n{}>", i));
            }
            xml.push_str(leaf);
            for i in (0..depth).rev() {
                xml.push_str(&format!("</n{}>", i));
            }
            xml
        };
        parse_xml_to_value(&nested(SOAP_MAX_DEPTH, "x"))?;
        // As before, the limit counts start tags: one self-closing leaf
        // below the deepest element is still read.
        parse_xml_to_value(&nested(SOAP_MAX_DEPTH, "<leaf/>"))?;
        assert_eq!(
            refused(&nested(SOAP_MAX_DEPTH + 5, ""))?,
            "XML parsing error: nesting depth exceeded 64 levels (likely DoS payload)"
        );
        Ok(())
    }

    #[test]
    fn rejects_text_payload_over_cap() -> TestResult {
        let mut xml = String::from("<root>");
        // Push enough to clearly exceed SOAP_MAX_TEXT_BYTES across one or
        // more text events.
        let chunk = "A".repeat(64 * 1024);
        for _ in 0..((SOAP_MAX_TEXT_BYTES / chunk.len()) + 2) {
            xml.push_str(&chunk);
        }
        xml.push_str("</root>");
        assert_eq!(
            refused(&xml)?,
            "XML parsing error: element text exceeded 1048576 bytes (likely DoS payload)"
        );
        Ok(())
    }

    #[test]
    fn accepts_normal_soap_envelope() -> TestResult {
        let xml = r#"<?xml version="1.0"?>
<Envelope xmlns="http://schemas.xmlsoap.org/soap/envelope/">
    <Body>
        <GetUser>
            <id>42</id>
            <name>Alice</name>
        </GetUser>
    </Body>
</Envelope>"#;
        assert_eq!(
            json(xml)?,
            r#"{"Envelope":{"Body":{"GetUser":{"id":"42","name":"Alice"}}}}"#
        );
        Ok(())
    }

    #[test]
    fn the_six_fixes_and_crlf() -> TestResult {
        // 1. whitespace around entities, 2. CDATA, 3. self-closing repeats,
        // 4. self-closing root, 5. mixed text, CRLF.
        assert_eq!(json("<a>Tom &amp; Jerry</a>")?, r#"{"a":"Tom & Jerry"}"#);
        assert_eq!(json("<a><![CDATA[<b>]]></a>")?, r#"{"a":"<b>"}"#);
        assert_eq!(json("<r><x>1</x><x/></r>")?, r#"{"r":{"x":["1",null]}}"#);
        assert_eq!(json("<a/>")?, r#"{"a":null}"#);
        assert_eq!(
            json("<r>head <c>1</c> tail</r>")?,
            r#"{"r":{"_text":"head  tail","c":"1"}}"#
        );
        assert_eq!(json("<a>l1\r\nl2</a>")?, r#"{"a":"l1\nl2"}"#);
        // 6. truncated input raises, so `SOAP.call` gives `parsed: nil`.
        assert!(refused("<r><a>1</a>")?.starts_with("XML parsing error: "));
        Ok(())
    }

    #[test]
    fn keeps_what_the_old_parser_accepted() -> TestResult {
        // Text before the root starts the root's text; a comment still cuts
        // text into pieces trimmed one by one.
        assert_eq!(json("junk<a>x</a>")?, r#"{"a":"junkx"}"#);
        assert_eq!(json("<a>x <!-- c --> y</a>")?, r#"{"a":"xy"}"#);
        // Character references resolve the quick-xml way: an invalid one is
        // dropped, a control character is kept.
        assert_eq!(json("<a>&#0;v</a>")?, r#"{"a":"v"}"#);
        assert_eq!(json("<a>&#1;v</a>")?, r#"{"a":"\u0001v"}"#);
        // Nothing after the root is read.
        assert_eq!(json("<a>1</a></b>")?, r#"{"a":"1"}"#);
        assert_eq!(json("<a>1</a><!DOCTYPE x>")?, r#"{"a":"1"}"#);
        // Attributes never refuse a document.
        assert_eq!(json(r#"<a x="AT&T" b>v</a>"#)?, r#"{"a":"v"}"#);
        // More than quick-xml's 256 declarations on one element.
        let decls: Vec<String> = (0..300).map(|i| format!("xmlns:p{i}=\"u{i}\"")).collect();
        assert_eq!(
            json(&format!("<a {}>x</a>", decls.join(" ")))?,
            r#"{"a":"x"}"#
        );
        // A malformed declaration field is ignored.
        assert_eq!(
            json("<?xml version=\"1.0\" encoding?><a>1</a>")?,
            r#"{"a":"1"}"#
        );
        Ok(())
    }

    #[test]
    fn a_reply_that_does_not_parse_gives_nil() {
        // Both `SOAP.call` paths do `parse_xml_to_value(&body).unwrap_or(Null)`.
        assert!(matches!(
            parse_xml_to_value("<r><a>1</a>").unwrap_or(Value::Null),
            Value::Null
        ));
        assert!(matches!(
            parse_xml_to_value("").unwrap_or(Value::Null),
            Value::Null
        ));
    }
}
