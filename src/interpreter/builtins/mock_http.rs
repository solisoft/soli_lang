//! Tiny in-process mock HTTP server for unit-testing the `HTTP.*` client.
//!
//! Replaces the previous `https://httpbin.org` calls in `tests/builtins/http_spec.sl`:
//! local TCP loopback is ~1000x faster, removes internet flakiness, and works
//! offline. The server is intentionally minimal — it accepts any method/path,
//! ignores headers/body, and returns a fixed JSON payload.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::OnceLock;
use std::thread;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

static MOCK_PORT: OnceLock<u16> = OnceLock::new();

pub fn register_mock_http_builtins(env: &mut Environment) {
    env.define(
        "mock_http_server_start".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "mock_http_server_start",
            Some(0),
            |_args| Ok(Value::Int(start_or_get_port() as i64)),
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
    let mut body_read = total.len() - head_end;
    while body_read < content_length {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => body_read += n,
        }
    }

    let body = b"{\"ok\":true}";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;

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
