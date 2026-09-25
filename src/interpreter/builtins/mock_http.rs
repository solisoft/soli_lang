//! Tiny in-process mock HTTP server for unit-testing the `HTTP.*` client.
//!
//! Replaces the previous `https://httpbin.org` calls in `tests/builtins/http_spec.sl`:
//! local TCP loopback is ~1000x faster, removes internet flakiness, and works
//! offline. The server is intentionally minimal — it accepts any method and
//! answers a fixed JSON payload, unless a spec has scripted the path with
//! `mock_http_route(path, status, body)`.
//!
//! Scripted routes are what let a spec stand in for a *third-party* server:
//! an OpenID provider's discovery document, key set and token endpoint, say.
//! The listener is a real loopback socket, so an application server running
//! in another process reaches it as it would reach the provider.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, OnceLock};
use std::thread;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

static MOCK_PORT: OnceLock<u16> = OnceLock::new();

/// Scripted answers, by path (query string excluded): status and body.
static ROUTES: OnceLock<Mutex<HashMap<String, (u16, String)>>> = OnceLock::new();

/// The body of the last request received on each path.
static LAST_BODIES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

/// Bodies are kept for inspection up to this size; the rest is drained unread.
const KEPT_BODY_MAX: usize = 64 * 1024;

fn routes() -> &'static Mutex<HashMap<String, (u16, String)>> {
    ROUTES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn last_bodies() -> &'static Mutex<HashMap<String, String>> {
    LAST_BODIES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register_mock_http_builtins(env: &mut Environment) {
    env.define(
        "mock_http_server_start".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "mock_http_server_start",
            Some(0),
            |_args| Ok(Value::Int(start_or_get_port() as i64)),
        )),
    );

    // mock_http_route(path, status, body) — answer `body` with `status` on
    // `path`, for any method, until scripted again.
    env.define(
        "mock_http_route".to_string(),
        Value::NativeFunction(NativeFunction::new("mock_http_route", Some(3), |args| {
            let path = match &args[0] {
                Value::String(s) => s.to_string(),
                other => {
                    return Err(format!(
                        "mock_http_route: path must be a string, got {}",
                        other.type_name()
                    ))
                }
            };
            let status = match &args[1] {
                Value::Int(n) if (100..=599).contains(n) => *n as u16,
                other => {
                    return Err(format!(
                        "mock_http_route: status must be an HTTP status code, got {}",
                        other
                    ))
                }
            };
            let body = match &args[2] {
                Value::String(s) => s.to_string(),
                other => other.to_string(),
            };
            if let Ok(mut map) = routes().lock() {
                map.insert(path, (status, body));
            }
            Ok(Value::Null)
        })),
    );

    // mock_http_last_body(path) — the body of the last request on `path`, or
    // nil if none arrived. What a client SENT is often the thing to prove.
    env.define(
        "mock_http_last_body".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "mock_http_last_body",
            Some(1),
            |args| {
                let path = args[0].to_string();
                let found = last_bodies()
                    .lock()
                    .ok()
                    .and_then(|map| map.get(&path).cloned());
                Ok(found
                    .map(|b| Value::String(b.into()))
                    .unwrap_or(Value::Null))
            },
        )),
    );
}

fn start_or_get_port() -> u16 {
    *MOCK_PORT.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock_http_server: failed to bind");
        let port = listener
            .local_addr()
            .expect("mock_http_server: local_addr")
            .port();
        thread::Builder::new()
            .name("mock-http".into())
            .spawn(move || accept_loop(listener))
            .expect("mock_http_server: spawn");
        port
    })
}

fn accept_loop(listener: TcpListener) {
    for stream in listener.incoming().flatten() {
        thread::spawn(move || {
            let _ = handle(stream);
        });
    }
}

fn handle(mut stream: TcpStream) -> std::io::Result<()> {
    // Read until end of headers. We don't need to parse the request — every
    // response is the same — but we must drain the request line + headers
    // (and any Content-Length body) so the client sees a clean exchange.
    let mut buf = [0u8; 4096];
    let mut total = Vec::new();
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total.extend_from_slice(&buf[..n]);
        if total.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if total.len() > 64 * 1024 {
            break;
        }
    }

    // Drain the request body, which the loop above stops short of.
    //
    // Not cosmetic: closing a socket that still has unread data makes the
    // kernel send RST instead of FIN, and the client loses the response it was
    // about to read. A POST to this mock would intermittently look like a
    // network failure rather than the `{"ok":true}` it answers with. The
    // comment above has always said "and any Content-Length body" — this is the
    // part that was missing.
    let head_end = total
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(total.len());
    let content_length = String::from_utf8_lossy(&total[..head_end])
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut kept: Vec<u8> = total[head_end..]
        .iter()
        .copied()
        .take(KEPT_BODY_MAX)
        .collect();
    let mut body_read = total.len() - head_end;
    while body_read < content_length {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = KEPT_BODY_MAX.saturating_sub(kept.len());
                kept.extend_from_slice(&buf[..n.min(room)]);
                body_read += n;
            }
        }
    }

    let path = request_path(&total[..head_end]);
    if let Ok(mut map) = last_bodies().lock() {
        map.insert(path.clone(), String::from_utf8_lossy(&kept).into_owned());
    }

    let (status, body) = routes()
        .lock()
        .ok()
        .and_then(|map| map.get(&path).cloned())
        .unwrap_or((200, "{\"ok\":true}".to_string()));
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status,
        reason_phrase(status),
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    Ok(())
}

/// The path of a request, from its request line, without the query string.
fn request_path(head: &[u8]) -> String {
    let head = String::from_utf8_lossy(head);
    let target = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    target.split('?').next().unwrap_or("/").to_string()
}

fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn exchange(port: u16, request: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect to mock");
        s.write_all(request.as_bytes()).expect("write request");
        let mut got = String::new();
        s.read_to_string(&mut got).expect("read response");
        got
    }

    /// A scripted path answers its own status and body, with or without a
    /// query string; any other path keeps the fixed payload. The body a client
    /// posted is kept for the spec to read back.
    #[test]
    fn scripted_routes_answer_and_keep_the_posted_body() {
        let port = super::start_or_get_port();
        super::routes().lock().unwrap().insert(
            "/scripted/.well-known/openid-configuration".to_string(),
            (200, "{\"issuer\":\"x\"}".to_string()),
        );
        super::routes()
            .lock()
            .unwrap()
            .insert("/scripted/token".to_string(), (401, "{}".to_string()));

        let discovery = exchange(
            port,
            "GET /scripted/.well-known/openid-configuration?a=1 HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert!(discovery.starts_with("HTTP/1.1 200"), "{discovery}");
        assert!(discovery.ends_with("{\"issuer\":\"x\"}"), "{discovery}");

        let body = "code=abc&code_verifier=v";
        let token = exchange(
            port,
            &format!(
                "POST /scripted/token HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            ),
        );
        assert!(token.starts_with("HTTP/1.1 401 Unauthorized"), "{token}");
        assert_eq!(
            super::last_bodies()
                .lock()
                .unwrap()
                .get("/scripted/token")
                .cloned(),
            Some(body.to_string())
        );

        let other = exchange(
            port,
            "GET /scripted/else HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert!(other.ends_with("{\"ok\":true}"), "{other}");
    }

    /// A POST must be answered, repeatedly.
    ///
    /// The handler used to stop reading at the end of the headers and leave the
    /// body in the socket. Closing a socket with unread data makes the kernel
    /// send RST rather than FIN, so the client lost the response it was about
    /// to read and the call looked like a network failure. It reproduced a few
    /// times in twenty, which is why this loops rather than posting once.
    #[test]
    fn post_with_a_body_is_answered_every_time() {
        let port = super::start_or_get_port();
        // Large enough to exceed the socket buffer, which is what makes the
        // difference: a small body is consumed by the kernel regardless, so it
        // never exercises the close-with-unread-data path.
        let body = format!("{{\"a\":\"{}\"}}", "x".repeat(512 * 1024));

        for attempt in 0..40 {
            let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect to mock");
            let req = format!(
                "POST /x HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            s.write_all(req.as_bytes()).expect("write request");

            let mut got = String::new();
            let read = s.read_to_string(&mut got);
            assert!(
                read.is_ok(),
                "attempt {attempt}: reading the response failed ({read:?}) — \
                 the handler closed the socket with the body unread"
            );
            assert!(
                got.contains("200 OK") && got.contains("{\"ok\":true}"),
                "attempt {attempt}: expected the mock's JSON answer, got {got:?}"
            );
        }
    }
}
