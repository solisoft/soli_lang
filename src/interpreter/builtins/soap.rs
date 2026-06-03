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

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::interpreter::builtins::http_class::{
    get_user_http_client, read_capped_text_async, validate_url_for_ssrf,
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

                        let mut resp_headers = HashPairs::default();
                        for (name, value) in resp.headers().iter() {
                            if let Ok(v) = value.to_str() {
                                resp_headers.insert(
                                    HashKey::String(name.to_string().into()),
                                    Value::String(v.to_string().into()),
                                );
                            }
                        }

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
                    };
                    format!("{}: {}", key, build_json_from_value(v))
                })
                .collect();
            format!("{{{}}}", items.join(", "))
        }
        _ => "null".to_string(),
    }
}

/// Maximum element-nesting depth the SOAP parser will accept. A
/// billion-laughs / deeply-nested XML body is the classic XML DoS
/// vector — well past 64 levels we're not parsing real-world SOAP
/// anyway. Picked to match common defaults in defensive XML libraries.
const SOAP_MAX_DEPTH: usize = 64;

/// Maximum total bytes of unencoded text content this parser will
/// accumulate per element. Stops a single `<x>AAAA…</x>` payload from
/// growing the in-memory buffer without bound. 1 MiB is well above any
/// legitimate SOAP body field.
const SOAP_MAX_TEXT_BYTES: usize = 1024 * 1024;

fn parse_xml_to_value(xml: &str) -> Result<Value, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text_end = true;
    reader.config_mut().trim_text_start = true;

    let mut buf = Vec::new();
    let mut stack: Vec<(String, HashPairs)> = Vec::new();
    let mut current_text = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            // SEC-008: an XML document with a DOCTYPE may declare
            // entities (XXE) or expand recursively (billion laughs).
            // quick_xml does not honour entity definitions, but the
            // mere presence of a DOCTYPE in untrusted input is a
            // strong "this isn't normal SOAP" signal — refuse rather
            // than let it through silently.
            Ok(Event::DocType(_)) => {
                return Err(
                    "XML parsing error: DOCTYPE declarations are not allowed in SOAP payloads"
                        .to_string(),
                );
            }
            Ok(Event::Start(e)) => {
                if !current_text.trim().is_empty() && !stack.is_empty() {
                    if let Some((_, parent)) = stack.last_mut() {
                        parent.insert(
                            HashKey::String("_text".into()),
                            Value::String(current_text.trim().to_string().into()),
                        );
                    }
                    current_text.clear();
                }

                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if stack.len() >= SOAP_MAX_DEPTH {
                    return Err(format!(
                        "XML parsing error: nesting depth exceeded {} levels (likely DoS payload)",
                        SOAP_MAX_DEPTH
                    ));
                }
                stack.push((name, HashPairs::default()));
            }
            Ok(Event::Text(e)) => {
                if let Ok(text) = e.unescape() {
                    if current_text.len().saturating_add(text.len()) > SOAP_MAX_TEXT_BYTES {
                        return Err(format!(
                            "XML parsing error: element text exceeded {} bytes (likely DoS payload)",
                            SOAP_MAX_TEXT_BYTES
                        ));
                    }
                    current_text.push_str(&text);
                }
            }
            Ok(Event::End(_)) => {
                if let Some((name, mut attrs)) = stack.pop() {
                    if !current_text.trim().is_empty() {
                        attrs.insert(
                            HashKey::String("_text".into()),
                            Value::String(current_text.trim().to_string().into()),
                        );
                        current_text.clear();
                    }

                    let value = if attrs.is_empty() {
                        Value::Null
                    } else if attrs.len() == 1
                        && attrs.contains_key(&HashKey::String("_text".into()))
                    {
                        attrs
                            .swap_remove(&HashKey::String("_text".into()))
                            .unwrap_or(Value::Null)
                    } else {
                        Value::Hash(Rc::new(RefCell::new(attrs)))
                    };

                    if let Some((_parent_name, parent)) = stack.last_mut() {
                        let key = HashKey::String(name.clone().into());
                        if let Some(existing) = parent.get(&key) {
                            // Convert to array if multiple elements with same name
                            let new_arr = match existing {
                                Value::Array(arr) => {
                                    let mut new_vec = arr.borrow().clone();
                                    new_vec.push(value);
                                    Value::Array(Rc::new(RefCell::new(new_vec)))
                                }
                                _ => Value::Array(Rc::new(RefCell::new(vec![
                                    existing.clone(),
                                    value,
                                ]))),
                            };
                            parent.insert(key, new_arr);
                        } else {
                            parent.insert(key, value);
                        }
                    } else {
                        // Root element
                        let mut root = HashPairs::default();
                        root.insert(HashKey::String(name.into()), value);
                        return Ok(Value::Hash(Rc::new(RefCell::new(root))));
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if let Some((_, parent)) = stack.last_mut() {
                    parent.insert(HashKey::String(name.into()), Value::Null);
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parsing error: {}", e)),
            _ => {}
        }
        buf.clear();
    }

    match stack.pop() {
        Some((name, attrs)) => {
            let mut root = HashPairs::default();
            root.insert(
                HashKey::String(name.into()),
                Value::Hash(Rc::new(RefCell::new(attrs))),
            );
            Ok(Value::Hash(Rc::new(RefCell::new(root))))
        }
        _ => Ok(Value::Null),
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
    //! Regression coverage for the SEC-008 SOAP-XML hardening:
    //! DOCTYPE rejection, depth cap, text-size cap.
    use super::*;

    #[test]
    fn rejects_doctype_declarations() {
        let xml = r#"<?xml version="1.0"?>
<!DOCTYPE foo [<!ENTITY a "hello">]>
<root><body>&a;</body></root>"#;
        let err = parse_xml_to_value(xml).unwrap_err();
        assert!(
            err.contains("DOCTYPE"),
            "expected DOCTYPE rejection, got: {}",
            err
        );
    }

    #[test]
    fn rejects_excessive_nesting() {
        // Build an XML payload with nesting just past the cap.
        let mut xml = String::from("<?xml version=\"1.0\"?>");
        let depth = SOAP_MAX_DEPTH + 5;
        for i in 0..depth {
            xml.push_str(&format!("<n{}>", i));
        }
        for i in (0..depth).rev() {
            xml.push_str(&format!("</n{}>", i));
        }
        let err = parse_xml_to_value(&xml).unwrap_err();
        assert!(
            err.contains("nesting depth exceeded"),
            "expected depth-cap rejection, got: {}",
            err
        );
    }

    #[test]
    fn rejects_text_payload_over_cap() {
        let mut xml = String::from("<root>");
        // Push enough to clearly exceed SOAP_MAX_TEXT_BYTES across one or
        // more text events.
        let chunk = "A".repeat(64 * 1024);
        for _ in 0..((SOAP_MAX_TEXT_BYTES / chunk.len()) + 2) {
            xml.push_str(&chunk);
        }
        xml.push_str("</root>");
        let err = parse_xml_to_value(&xml).unwrap_err();
        assert!(
            err.contains("element text exceeded"),
            "expected text-cap rejection, got: {}",
            err
        );
    }

    #[test]
    fn accepts_normal_soap_envelope() {
        let xml = r#"<?xml version="1.0"?>
<Envelope xmlns="http://schemas.xmlsoap.org/soap/envelope/">
    <Body>
        <GetUser>
            <id>42</id>
            <name>Alice</name>
        </GetUser>
    </Body>
</Envelope>"#;
        let v = parse_xml_to_value(xml).expect("normal SOAP body must parse");
        assert!(matches!(v, Value::Hash(_)));
    }
}
