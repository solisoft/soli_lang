//! Shared RFC822/MIME message parsing for the mail-reading builtins (`Pop3`,
//! `Imap`).
//!
//! Both clients turn a raw message into the same structured field set —
//! `size, subject, from, to, date, text_body, html_body, attachments, raw` — so
//! that logic lives here once. Each caller prepends its own identity fields
//! (POP3 `id`; IMAP `seq`/`uid`/`flags`) before building the final hash.

use std::cell::RefCell;
use std::rc::Rc;

use base64::Engine;
use mail_parser::{Addr, Address, MessageParser, MimeHeaders};

use crate::interpreter::value::{hash_from_pairs, Value};

fn opt_str(s: Option<&str>) -> Value {
    s.map(|s| Value::String(s.to_string().into()))
        .unwrap_or(Value::Null)
}

fn addr_to_hash(addr: &Addr) -> Value {
    hash_from_pairs([
        ("name".to_string(), opt_str(addr.name())),
        ("address".to_string(), opt_str(addr.address())),
    ])
}

/// First address of a header as `{name, address}`, or `null`.
fn addr_first(addr: Option<&Address>) -> Value {
    match addr.and_then(|a| a.first()) {
        Some(one) => addr_to_hash(one),
        None => Value::Null,
    }
}

/// All addresses of a header as an array of `{name, address}` hashes.
fn addr_all(addr: Option<&Address>) -> Value {
    let mut out = Vec::new();
    if let Some(a) = addr {
        for one in a.iter() {
            out.push(addr_to_hash(one));
        }
    }
    Value::Array(Rc::new(RefCell::new(out)))
}

/// Parse a raw RFC822 message into the fields common to every mail client, in a
/// stable order: `size, subject, from, to, date, text_body, html_body,
/// parts, attachments, raw`. `from` is a single `{name, address}` hash (or `null`);
/// `to` is an array of them. Missing headers/bodies are `null`. The `raw` field
/// is the message source as a (lossily decoded) string.
pub fn common_fields(raw: &[u8]) -> Vec<(String, Value)> {
    let size = raw.len() as i64;
    let parsed = MessageParser::default().parse(raw);

    let (subject, from, to, date, text_body, html_body, parts, attachments) = match &parsed {
        Some(msg) => {
            let date = msg
                .date()
                .map(|d| Value::String(d.to_rfc3339().into()))
                .unwrap_or(Value::Null);
            let text_body = msg
                .body_text(0)
                .map(|c| Value::String(c.into_owned().into()))
                .unwrap_or(Value::Null);
            let html_body = msg
                .body_html(0)
                .map(|c| Value::String(c.into_owned().into()))
                .unwrap_or(Value::Null);
            // Every text part, with the type it declares.
            //
            // `text_body`/`html_body` answer "the plain one" and "the HTML
            // one", which is every message there is until one carries a
            // third face: a `text/markdown` part is `PartType::Text` like
            // any other text/*, and neither of those two accessors will
            // ever return it. A client that wants to *prefer* the source a
            // message was written in has to be able to see it.
            let mut parts_out = Vec::new();
            for part in &msg.parts {
                let ctype = match part.content_type() {
                    Some(ct) => match ct.subtype() {
                        Some(sub) => format!("{}/{}", ct.ctype(), sub).to_lowercase(),
                        None => ct.ctype().to_lowercase(),
                    },
                    None => String::new(),
                };
                let body = match &part.body {
                    mail_parser::PartType::Text(t) => Some(t.to_string()),
                    mail_parser::PartType::Html(t) => Some(t.to_string()),
                    _ => None,
                };
                if let Some(body) = body {
                    let ctype = if ctype.is_empty() {
                        match &part.body {
                            mail_parser::PartType::Html(_) => "text/html".to_string(),
                            _ => "text/plain".to_string(),
                        }
                    } else {
                        ctype
                    };
                    parts_out.push(hash_from_pairs([
                        ("content_type".to_string(), Value::String(ctype.into())),
                        ("body".to_string(), Value::String(body.into())),
                    ]));
                }
            }
            let parts_out = Value::Array(Rc::new(RefCell::new(parts_out)));

            let mut atts = Vec::new();
            for part in msg.attachments() {
                let content_type = part
                    .content_type()
                    .map(|ct| match ct.subtype() {
                        Some(sub) => Value::String(format!("{}/{}", ct.ctype(), sub).into()),
                        None => Value::String(ct.ctype().to_string().into()),
                    })
                    .unwrap_or(Value::Null);
                // The bytes, base64, beside the name.
                //
                // A client that only lists attachments needs three fields;
                // one that can *hand them over* needs a fourth, and the
                // bytes are already here -- the message was fetched whole
                // to be read at all. Base64 because a Soli string is UTF-8
                // and an attachment is not text: `File.write_base64` is
                // the other end of this.
                let bytes = part.contents();
                atts.push(hash_from_pairs([
                    ("name".to_string(), opt_str(part.attachment_name())),
                    ("content_type".to_string(), content_type),
                    ("size".to_string(), Value::Int(part.len() as i64)),
                    (
                        "base64".to_string(),
                        Value::String(
                            base64::engine::general_purpose::STANDARD
                                .encode(bytes)
                                .into(),
                        ),
                    ),
                ]));
            }
            (
                opt_str(msg.subject()),
                addr_first(msg.from()),
                addr_all(msg.to()),
                date,
                text_body,
                html_body,
                parts_out,
                Value::Array(Rc::new(RefCell::new(atts))),
            )
        }
        None => (
            Value::Null,
            Value::Null,
            Value::Array(Rc::new(RefCell::new(Vec::new()))),
            Value::Null,
            Value::Null,
            Value::Null,
            Value::Array(Rc::new(RefCell::new(Vec::new()))),
            Value::Array(Rc::new(RefCell::new(Vec::new()))),
        ),
    };

    vec![
        ("size".to_string(), Value::Int(size)),
        ("subject".to_string(), subject),
        ("from".to_string(), from),
        ("to".to_string(), to),
        ("date".to_string(), date),
        ("text_body".to_string(), text_body),
        ("html_body".to_string(), html_body),
        ("parts".to_string(), parts),
        ("attachments".to_string(), attachments),
        (
            "raw".to_string(),
            Value::String(String::from_utf8_lossy(raw).into_owned().into()),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::HashKey;

    fn field<'a>(pairs: &'a [(String, Value)], key: &str) -> &'a Value {
        &pairs
            .iter()
            .find(|(k, _)| k == key)
            .expect("field present")
            .1
    }

    #[test]
    fn an_attachment_carries_its_bytes() {
        let raw = b"From: a@b.com\r\nTo: c@d.com\r\nSubject: devis\r\n\
MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"x\"\r\n\r\n\
--x\r\nContent-Type: text/plain\r\n\r\nvoici\r\n\
--x\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"devis.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\r\naGVsbG8gcGRm\r\n--x--\r\n";
        let got = common_fields(raw);
        let Value::Array(atts) = field(&got, "attachments") else {
            panic!("attachments is an array");
        };
        let atts = atts.borrow();
        assert_eq!(atts.len(), 1);
        let Value::Hash(one) = &atts[0] else {
            panic!("an attachment is a hash");
        };
        let one = one.borrow();
        let b64 = match one.get(&HashKey::String("base64".into())) {
            Some(Value::String(s)) => s.to_string(),
            other => panic!("base64 is a string, got {other:?}"),
        };
        // "hello pdf", which is what the part decodes to.
        assert_eq!(b64, "aGVsbG8gcGRm");
    }

    #[test]
    fn a_third_face_is_visible_among_the_parts() {
        // Three alternatives, the middle one a source no accessor returns.
        let raw = b"From: a@b.com\r\nTo: c@d.com\r\nSubject: three\r\n\
MIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"x\"\r\n\r\n\
--x\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nLe point\r\n\
--x\r\nContent-Type: text/markdown; charset=utf-8\r\n\r\n# Le point\r\n\
--x\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<h1>Le point</h1>\r\n--x--\r\n";
        let got = common_fields(raw);
        let Value::Array(parts) = field(&got, "parts") else {
            panic!("parts is an array");
        };
        let types: Vec<String> = parts
            .borrow()
            .iter()
            .filter_map(|p| match p {
                Value::Hash(h) => h
                    .borrow()
                    .get(&HashKey::String("content_type".into()))
                    .and_then(|v| match v {
                        Value::String(s) => Some(s.to_string()),
                        _ => None,
                    }),
                _ => None,
            })
            .collect();
        assert!(types.iter().any(|t| t == "text/markdown"), "{types:?}");
        assert!(types.iter().any(|t| t == "text/plain"), "{types:?}");
        assert!(types.iter().any(|t| t == "text/html"), "{types:?}");
    }

    #[test]
    fn extracts_headers_and_body() {
        let raw = "From: Alice <alice@example.com>\r\n\
                   To: Bob <bob@example.com>\r\n\
                   Subject: Hello\r\n\
                   Date: Mon, 1 Jun 2026 10:00:00 +0000\r\n\
                   Content-Type: text/plain\r\n\
                   \r\n\
                   Hi there!\r\n";
        let pairs = common_fields(raw.as_bytes());
        assert!(matches!(field(&pairs, "subject"), Value::String(s) if **s == *"Hello"));
        assert!(matches!(field(&pairs, "text_body"), Value::String(s) if s.contains("Hi there!")));
        match field(&pairs, "from") {
            Value::Hash(from) => {
                let from = from.borrow();
                assert!(matches!(
                    from.get(&HashKey::String("address".into())),
                    Some(Value::String(a)) if **a == *"alice@example.com"
                ));
            }
            other => panic!("expected from hash, got {other:?}"),
        }
    }

    #[test]
    fn extracts_attachment_metadata() {
        let raw = "Subject: With attachment\r\n\
                   Content-Type: multipart/mixed; boundary=\"BOUND\"\r\n\
                   \r\n\
                   --BOUND\r\n\
                   Content-Type: text/plain\r\n\
                   \r\n\
                   Body text\r\n\
                   --BOUND\r\n\
                   Content-Type: text/csv; name=\"data.csv\"\r\n\
                   Content-Disposition: attachment; filename=\"data.csv\"\r\n\
                   \r\n\
                   a,b,c\r\n\
                   --BOUND--\r\n";
        let pairs = common_fields(raw.as_bytes());
        match field(&pairs, "attachments") {
            Value::Array(arr) => {
                let arr = arr.borrow();
                assert_eq!(arr.len(), 1);
                let Value::Hash(att) = &arr[0] else {
                    panic!("attachment not a hash")
                };
                let att = att.borrow();
                assert!(matches!(
                    att.get(&HashKey::String("name".into())),
                    Some(Value::String(n)) if **n == *"data.csv"
                ));
            }
            other => panic!("expected attachments array, got {other:?}"),
        }
    }
}
