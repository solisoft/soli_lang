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

    let body = b"{\"ok\":true}";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}
