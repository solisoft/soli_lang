//! `Mailer` over HTTP: the standard envelope POSTed to a provider's API, the
//! key in a header, a `request` hook class that reshapes it, and errors that
//! name the status but never the key. Against a local server, opened through
//! `SOLI_HTTP_ALLOW_HOSTS` as an app would open a private endpoint.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

struct Received {
    path: String,
    headers: Vec<(String, String)>,
    body: serde_json::Value,
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
Mailer.configure({{"delivery_method": "http", "url": "{url}", "api_key": "sk_live_secret", "from": "Desk <desk@example.com>"}})
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
