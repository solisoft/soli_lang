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

use indexmap::IndexMap;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::interpreter::builtins::http_class::{get_http_client, validate_url_for_ssrf};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, NativeFunction, Value};
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
                                _ => format!("{}", v),
                            };
                            headers.push((key.clone(), value_str));
                        }
                    }
                }
            }

            match get_tokio_handle() {
                Some(rt) => {
                    let client = get_http_client().clone();
                    match rt.block_on(async move {
                        let mut request = client.post(&url);

                        for (key, value) in &headers {
                            request = request.header(key.as_str(), value.as_str());
                        }

                        let resp = request
                            .body(envelope)
                            .send()
                            .await
                            .map_err(|e| e.to_string())?;

                        let status = resp.status().as_u16();
                        let status_text =
                            resp.status().canonical_reason().unwrap_or("").to_string();

                        let mut resp_headers = IndexMap::new();
                        for (name, value) in resp.headers().iter() {
                            if let Ok(v) = value.to_str() {
                                resp_headers.insert(
                                    HashKey::String(name.to_string()),
                                    Value::String(v.to_string()),
                                );
                            }
                        }

                        let body = resp.text().await.map_err(|e| e.to_string())?;

                        let parsed_xml = parse_xml_to_value(&body).unwrap_or(Value::Null);

                        let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                        result.insert(
                            HashKey::String("status".to_string()),
                            Value::Int(status as i64),
                        );
                        result.insert(
                            HashKey::String("status_text".to_string()),
                            Value::String(status_text),
                        );
                        result.insert(
                            HashKey::String("headers".to_string()),
                            Value::Hash(Rc::new(RefCell::new(resp_headers))),
                        );
                        result.insert(HashKey::String("body".to_string()), Value::String(body));
                        result.insert(HashKey::String("parsed".to_string()), parsed_xml);

                        Ok(Value::Hash(Rc::new(RefCell::new(result))))
                    }) {
                        Ok(v) => Ok(v),
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(spawn_soap_future(url, headers, envelope)),
            }
        })),
    );

    // SOAP.wrap(body, namespace?) -> String
    soap_static_methods.insert(
        "wrap".to_string(),
        Rc::new(NativeFunction::new("SOAP.wrap", Some(1), |args| {
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
                    _ => SOAP11_NS.to_string(),
                }
            } else {
                SOAP11_NS.to_string()
            };

            let envelope = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<soap:Envelope xmlns:soap="{}">
  <soap:Body>
    {}
  </soap:Body>
</soap:Envelope>"#,
                namespace, body
            );

            Ok(Value::String(envelope))
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
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&apos;");

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
                    _ => "root".to_string(),
                }
            } else {
                "root".to_string()
            };

            let xml = value_to_xml(&Value::Hash(hash), &root_element);
            Ok(Value::String(xml))
        })),
    );

    let soap_class = Class {
        name: "SOAP".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: soap_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
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
        let mut request = ureq::post(&url);

        for (key, value) in &headers {
            request = request.set(key, value);
        }

        let result = match request.send_string(&envelope) {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.into_string().unwrap_or_default();
                let parsed = parse_xml_to_value(&body).unwrap_or(Value::Null);

                let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                result.insert(
                    HashKey::String("status".to_string()),
                    Value::Int(status as i64),
                );
                result.insert(
                    HashKey::String("status_text".to_string()),
                    Value::String("OK".to_string()),
                );
                result.insert(
                    HashKey::String("headers".to_string()),
                    Value::Hash(Rc::new(RefCell::new(IndexMap::new()))),
                );
                result.insert(HashKey::String("body".to_string()), Value::String(body));
                result.insert(HashKey::String("parsed".to_string()), parsed);

                build_json_from_value(&Value::Hash(Rc::new(RefCell::new(result))))
            }
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                let parsed = parse_xml_to_value(&body).unwrap_or(Value::Null);

                let mut result: IndexMap<HashKey, Value> = IndexMap::new();
                result.insert(
                    HashKey::String("status".to_string()),
                    Value::Int(code as i64),
                );
                result.insert(
                    HashKey::String("status_text".to_string()),
                    Value::String("Error".to_string()),
                );
                result.insert(
                    HashKey::String("headers".to_string()),
                    Value::Hash(Rc::new(RefCell::new(IndexMap::new()))),
                );
                result.insert(HashKey::String("body".to_string()), Value::String(body));
                result.insert(HashKey::String("parsed".to_string()), parsed);

                build_json_from_value(&Value::Hash(Rc::new(RefCell::new(result))))
            }
            Err(e) => format!("{{\"error\": \"{}\"}}", e),
        };

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
        Value::String(s) => format!("\"{}\"", s.replace('"', "\\\"")),
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
                        HashKey::Int(i) => i.to_string(),
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

fn parse_xml_to_value(xml: &str) -> Result<Value, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text_end = true;
    reader.config_mut().trim_text_start = true;

    let mut buf = Vec::new();
    let mut stack: Vec<(String, IndexMap<HashKey, Value>)> = Vec::new();
    let mut current_text = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                if !current_text.trim().is_empty() && !stack.is_empty() {
                    if let Some((_, parent)) = stack.last_mut() {
                        parent.insert(
                            HashKey::String("_text".to_string()),
                            Value::String(current_text.trim().to_string()),
                        );
                    }
                    current_text.clear();
                }

                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                stack.push((name, IndexMap::new()));
            }
            Ok(Event::Text(e)) => {
                if let Ok(text) = e.unescape() {
                    current_text.push_str(&text);
                }
            }
            Ok(Event::End(_)) => {
                if let Some((name, mut attrs)) = stack.pop() {
                    if !current_text.trim().is_empty() {
                        attrs.insert(
                            HashKey::String("_text".to_string()),
                            Value::String(current_text.trim().to_string()),
                        );
                        current_text.clear();
                    }

                    let value = if attrs.is_empty() {
                        Value::Null
                    } else if attrs.len() == 1
                        && attrs.contains_key(&HashKey::String("_text".to_string()))
                    {
                        attrs
                            .swap_remove(&HashKey::String("_text".to_string()))
                            .unwrap_or(Value::Null)
                    } else {
                        Value::Hash(Rc::new(RefCell::new(attrs)))
                    };

                    if let Some((_parent_name, parent)) = stack.last_mut() {
                        let key = HashKey::String(name.clone());
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
                        let mut root = IndexMap::new();
                        root.insert(HashKey::String(name), value);
                        return Ok(Value::Hash(Rc::new(RefCell::new(root))));
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if let Some((_, parent)) = stack.last_mut() {
                    parent.insert(HashKey::String(name), Value::Null);
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
            let mut root = IndexMap::new();
            root.insert(
                HashKey::String(name),
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
                    HashKey::Int(i) => i.to_string(),
                    HashKey::Bool(b) => b.to_string(),
                    HashKey::Null => "null".to_string(),
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
        Value::String(s) => s.clone(),
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
                        HashKey::Int(i) => i.to_string(),
                        HashKey::Bool(b) => b.to_string(),
                        HashKey::Null => "null".to_string(),
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
