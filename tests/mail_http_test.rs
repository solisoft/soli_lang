//! `Mailer` over HTTP: the standard envelope POSTed to a provider's API, the
//! key in a header, a `request` hook class that reshapes it, and errors that
//! name the status but never the key. Against a local server, opened through
//! `SOLI_HTTP_ALLOW_HOSTS` as an app would open a private endpoint.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

/// The mailer configuration is one per application, shared by every thread:
/// the tests take turns.
static CONFIG: Mutex<()> = Mutex::new(());

struct Received {
    path: String,
    headers: Vec<(String, String)>,
    body: serde_json::Value,
    raw: String,
}

impl Received {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A server that answers each request with the next queued `(status, body)`
/// (200 when the queue is empty) and reports what it received.
fn serve(answers: Vec<(u16, &'static str)>) -> (u16, mpsc::Receiver<Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let answers = Arc::new(Mutex::new(VecDeque::from(answers)));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let mut headers = Vec::new();
            let mut length = 0usize;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                let header = header.trim_end();
                if header.is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    headers.push((name.trim().to_string(), value.trim().to_string()));
                }
            }
            let mut body = vec![0u8; length];
            reader.read_exact(&mut body).unwrap();
            let (status, answer) = answers.lock().unwrap().pop_front().unwrap_or((200, "{}"));
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                answer.len()
            );
            let _ = tx.send(Received {
                path,
                headers,
                body: serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
                raw: String::from_utf8_lossy(&body).into_owned(),
            });
        }
    });
    (port, rx)
}

fn run(source: &str) -> Result<(), String> {
    let source = source.to_string();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let (_, result) =
                solilang::run_with_path_and_coverage(&source, None, false, None, None, &[]);
            result.map_err(|e| e.to_string())
        })
        .unwrap()
        .join()
        .unwrap()
}

fn next(rx: &mpsc::Receiver<Received>) -> Received {
    rx.recv_timeout(std::time::Duration::from_secs(10))
        .expect("the server got no request")
}

#[test]
fn mail_goes_out_over_http() {
    let _turn = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("SOLI_HTTP_ALLOW_HOSTS", "127.0.0.1");
    let (port, rx) = serve(vec![
        (200, "{}"),
        (200, "{}"),
        (202, "{}"),
        (422, r#"{"message":"sender not verified"}"#),
        (200, "{}"),
    ]);
    let url = format!("http://127.0.0.1:{port}/send");

    // 1. The standard envelope, the key as a bearer token.
    run(&format!(
        r#"
Mailer.configure({{"delivery_method": "http", "url": "{url}", "api_key": "sk_live_secret", "api_key_header": nil, "request": nil, "from": "Desk <desk@example.com>"}})
Mailer.deliver({{
  "to": ["Ana <ana@example.com>"], "subject": "Hello", "text": "Hi Ana", "html": "<p>Hi Ana</p>",
  "headers": {{"X-Campaign": "welcome"}},
  "attachments": [{{"filename": "note.txt", "content": "hello", "content_type": "text/plain"}}]
}})
"#
    ))
    .expect("delivered");
    let got = next(&rx);
    assert_eq!(got.path, "/send");
    assert_eq!(got.header("authorization"), Some("Bearer sk_live_secret"));
    assert_eq!(got.header("content-type"), Some("application/json"));
    let body = &got.body;
    assert_eq!(
        body["from"], "Desk <desk@example.com>",
        "the configured from: {body}"
    );
    assert_eq!(body["to"][0], "Ana <ana@example.com>");
    assert_eq!(body["subject"], "Hello");
    assert_eq!(body["text"], "Hi Ana");
    assert_eq!(body["html"], "<p>Hi Ana</p>");
    assert_eq!(body["headers"]["X-Campaign"], "welcome");
    assert_eq!(body["attachments"][0]["filename"], "note.txt");
    assert_eq!(body["attachments"][0]["content_base64"], "aGVsbG8=");
    assert!(
        body["mime"].as_str().unwrap().contains("Subject: Hello"),
        "{body}"
    );
    assert!(
        body["message_id"]
            .as_str()
            .is_some_and(|id| id.contains('@')),
        "{body}"
    );

    // 2. A provider that wants the bare key in a header of its own.
    run(&format!(
        r#"
Mailer.configure({{"delivery_method": "http", "url": "{url}", "api_key": "xkeysib-1", "api_key_header": "api-key", "from": "desk@example.com"}})
Mailer.deliver({{"to": "bob@example.com", "subject": "Hi", "text": "yo"}})
"#
    ))
    .expect("delivered");
    let got = next(&rx);
    assert_eq!(got.header("api-key"), Some("xkeysib-1"));
    assert!(got.header("authorization").is_none());

    // 3. A request hook: an app class reshapes the request for its provider.
    let hook = format!(
        r#"
class MailRequest
  static def request(mail, req)
    envelope = req["body"]
    req["url"] = req["url"] + "/v3/smtp/email"
    req["body"] = {{
      "sender": {{"email": envelope["from"]}},
      "to": envelope["to"].map {{ |a| {{"email": a}} }},
      "subject": envelope["subject"],
      "textContent": envelope["text"]
    }}
    req
  end
end

# What a mailer action builds (`mail(...)` renders through the templates,
# which a bare script has not loaded).
def welcome(to)
  Message.new({{"data": {{"to": to, "subject": "Welcome", "text": "Glad you're here", "from": "desk@example.com"}}}})
end

Mailer.configure({{"delivery_method": "http", "url": "{url}", "api_key": "k", "request": "MailRequest", "from": "desk@example.com"}})
"#
    );
    run(&format!(
        "{hook}\nwelcome(\"cy@example.com\").deliver_now\n"
    ))
    .expect("delivered");
    let got = next(&rx);
    assert_eq!(got.path, "/send/v3/smtp/email");
    assert_eq!(
        got.body["sender"]["email"], "desk@example.com",
        "{}",
        got.body
    );
    assert_eq!(got.body["to"][0]["email"], "cy@example.com");
    assert_eq!(got.body["textContent"], "Glad you're here");
    assert!(
        got.body.get("mime").is_none(),
        "the hook's body is sent as is"
    );

    // 4. An error names the status and the provider's message, never the key.
    let err = run(&format!(
        r#"
Mailer.configure({{"delivery_method": "http", "url": "{url}?key=sk_in_query", "api_key": "sk_live_secret", "request": nil, "from": "desk@example.com"}})
Mailer.deliver({{"to": "dee@example.com", "subject": "x", "text": "y"}})
"#
    ))
    .unwrap_err();
    let _ = next(&rx);
    assert!(
        err.contains("422") && err.contains("sender not verified"),
        "{err}"
    );
    assert!(
        !err.contains("sk_live_secret") && !err.contains("sk_in_query"),
        "{err}"
    );

    // A hook naming a class the app does not define says so.
    let err = run(&format!(
        r#"
Mailer.configure({{"delivery_method": "http", "url": "{url}", "request": "NoSuchHook", "from": "desk@example.com"}})
Mailer.deliver({{"to": "fay@example.com", "subject": "x", "text": "y"}})
"#
    ))
    .unwrap_err();
    assert!(err.contains("NoSuchHook is not defined"), "{err}");

    // 5. deliver_later with no job queue sends now — through the hook too.
    run(&format!(
        "{hook}\nwelcome(\"eve@example.com\").deliver_later\n"
    ))
    .expect("delivered");
    let got = next(&rx);
    assert_eq!(got.path, "/send/v3/smtp/email");
    assert_eq!(got.body["to"][0]["email"], "eve@example.com");
}

/// One rich mail through a provider class from examples/mail_providers.
fn through_provider(class: &str, file: &str, rx: &mpsc::Receiver<Received>, port: u16) -> Received {
    let source = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/mail_providers")
            .join(file),
    )
    .unwrap();
    run(&format!(
        r#"{source}
Mailer.configure({{"delivery_method": "http", "url": "http://127.0.0.1:{port}", "api_key": "key-{class}", "api_key_header": nil, "request": "{class}", "from": "Desk <desk@example.com>"}})
Mailer.deliver({{
  "to": ["Ana Vieira <ana@example.com>", "bo@example.com"], "cc": ["cy@example.com"], "bcc": ["dee@example.com"],
  "reply_to": "Help <help@example.com>", "subject": "Your order", "text": "Plain", "html": "<p>Rich</p>",
  "headers": {{"X-Order": "42"}},
  "attachments": [{{"filename": "invoice.txt", "content": "hello", "content_type": "text/plain"}}]
}})
"#
    ))
    .unwrap_or_else(|e| panic!("{class}: {e}"));
    next(rx)
}

#[test]
fn the_provider_classes_speak_each_api() {
    let _turn = CONFIG.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("SOLI_HTTP_ALLOW_HOSTS", "127.0.0.1");
    let (port, rx) = serve(vec![]);

    // Brevo: objects for addresses, its own key header, `attachment` (singular).
    let got = through_provider("BrevoMail", "brevo_mail.sl", &rx, port);
    assert_eq!(got.path, "/v3/smtp/email");
    assert_eq!(got.header("api-key"), Some("key-BrevoMail"));
    let b = &got.body;
    assert_eq!(
        b["sender"],
        serde_json::json!({"name": "Desk", "email": "desk@example.com"})
    );
    assert_eq!(
        b["to"][0],
        serde_json::json!({"name": "Ana Vieira", "email": "ana@example.com"})
    );
    assert_eq!(b["to"][1], serde_json::json!({"email": "bo@example.com"}));
    assert_eq!(b["cc"][0]["email"], "cy@example.com");
    assert_eq!(b["bcc"][0]["email"], "dee@example.com");
    assert_eq!(
        b["replyTo"],
        serde_json::json!({"name": "Help", "email": "help@example.com"})
    );
    assert_eq!(
        (&b["textContent"], &b["htmlContent"]),
        (
            &serde_json::json!("Plain"),
            &serde_json::json!("<p>Rich</p>")
        )
    );
    assert_eq!(b["headers"]["X-Order"], "42");
    assert_eq!(
        b["attachment"][0],
        serde_json::json!({"name": "invoice.txt", "content": "aGVsbG8="})
    );

    // Resend: the envelope's own names, strings for addresses, Bearer.
    let got = through_provider("ResendMail", "resend_mail.sl", &rx, port);
    assert_eq!(got.path, "/emails");
    assert_eq!(got.header("authorization"), Some("Bearer key-ResendMail"));
    let b = &got.body;
    assert_eq!(b["from"], "Desk <desk@example.com>");
    assert_eq!(
        b["to"],
        serde_json::json!(["Ana Vieira <ana@example.com>", "bo@example.com"])
    );
    assert_eq!(b["reply_to"], "Help <help@example.com>");
    assert_eq!(
        b["attachments"][0],
        serde_json::json!({"filename": "invoice.txt", "content": "aGVsbG8="})
    );

    // Postmark: PascalCase, comma-joined recipients, its token header.
    let got = through_provider("PostmarkMail", "postmark_mail.sl", &rx, port);
    assert_eq!(got.path, "/email");
    assert_eq!(
        got.header("x-postmark-server-token"),
        Some("key-PostmarkMail")
    );
    let b = &got.body;
    assert_eq!(b["To"], "Ana Vieira <ana@example.com>, bo@example.com");
    assert_eq!(
        (&b["Cc"], &b["Bcc"]),
        (
            &serde_json::json!("cy@example.com"),
            &serde_json::json!("dee@example.com")
        )
    );
    assert_eq!(b["TextBody"], "Plain");
    assert_eq!(b["HtmlBody"], "<p>Rich</p>");
    assert_eq!(
        b["Headers"],
        serde_json::json!([{"Name": "X-Order", "Value": "42"}])
    );
    assert_eq!(
        b["Attachments"][0],
        serde_json::json!({"Name": "invoice.txt", "Content": "aGVsbG8=", "ContentType": "text/plain"})
    );

    // SendGrid: personalizations, content parts plain then html.
    let got = through_provider("SendgridMail", "sendgrid_mail.sl", &rx, port);
    assert_eq!(got.path, "/v3/mail/send");
    assert_eq!(got.header("authorization"), Some("Bearer key-SendgridMail"));
    let b = &got.body;
    assert_eq!(
        b["personalizations"][0]["to"][0],
        serde_json::json!({"name": "Ana Vieira", "email": "ana@example.com"})
    );
    assert_eq!(
        b["personalizations"][0]["bcc"][0]["email"],
        "dee@example.com"
    );
    assert_eq!(
        b["from"],
        serde_json::json!({"name": "Desk", "email": "desk@example.com"})
    );
    assert_eq!(
        b["content"],
        serde_json::json!([{"type": "text/plain", "value": "Plain"}, {"type": "text/html", "value": "<p>Rich</p>"}])
    );
    assert_eq!(b["attachments"][0]["type"], "text/plain");

    // Mailgun: Basic auth, multipart with every recipient and the MIME.
    std::env::set_var("MAILGUN_DOMAIN", "mg.example.com");
    let got = through_provider("MailgunMail", "mailgun_mail.sl", &rx, port);
    assert_eq!(got.path, "/v3/mg.example.com/messages.mime");
    use base64::Engine;
    let basic = base64::engine::general_purpose::STANDARD.encode("api:key-MailgunMail");
    assert_eq!(
        got.header("authorization"),
        Some(format!("Basic {basic}").as_str())
    );
    assert!(got
        .header("content-type")
        .unwrap()
        .starts_with("multipart/form-data; boundary="));
    let raw = got.raw.as_str();
    for rcpt in [
        "Ana Vieira <ana@example.com>",
        "bo@example.com",
        "cy@example.com",
        "dee@example.com",
    ] {
        assert!(
            raw.contains(&format!("name=\"to\"\r\n\r\n{rcpt}\r\n")),
            "{rcpt} in {raw}"
        );
    }
    assert!(raw.contains("filename=\"message.mime\""), "{raw}");
    assert!(raw.contains("Subject: Your order"), "{raw}");
    assert!(raw.contains("invoice.txt"), "{raw}");
}
