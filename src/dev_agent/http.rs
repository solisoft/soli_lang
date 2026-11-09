//! The one HTTP client `soli agent` needs: HTTP/1.1 to `127.0.0.1:<port>`,
//! one request per connection, blocking.
//!
//! The agent is a single-threaded stdio loop, so a runtime and a pooled client
//! would buy nothing. Every request names `Host: 127.0.0.1:<port>`, which is
//! what `dev_routes::dispatch` accepts against DNS rebinding, and every POST
//! names the same origin, which its same-origin gate requires.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub struct DevClient {
    pub port: u16,
    pub dev_token: Option<String>,
    pub timeout: Duration,
}

impl DevClient {
    pub fn get(&self, path: &str) -> Result<HttpResponse, String> {
        self.send("GET", path, &[], None)
    }

    /// A POST that passes the dev endpoints' same-origin gate.
    pub fn post(
        &self,
        path: &str,
        extra: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<HttpResponse, String> {
        let origin = format!("http://127.0.0.1:{}", self.port);
        let mut headers: Vec<(&str, &str)> = vec![("Origin", origin.as_str())];
        headers.extend_from_slice(extra);
        self.send("POST", path, &headers, body)
    }

    fn send(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<HttpResponse, String> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port));
        let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .map_err(|_| format!("dev server is not reachable on 127.0.0.1:{}", self.port))?;
        stream.set_read_timeout(Some(self.timeout)).ok();
        stream.set_write_timeout(Some(self.timeout)).ok();

        let body = body.unwrap_or("");
        let mut req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nAccept: application/json\r\nUser-Agent: soli-agent\r\n",
            self.port
        );
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        if method != "GET" {
            req.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        req.push_str("\r\n");
        req.push_str(body);
        stream
            .write_all(req.as_bytes())
            .map_err(|e| format!("writing to the dev server failed: {e}"))?;

        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).map_err(|e| {
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) {
                format!(
                    "the dev server did not answer within {}s",
                    self.timeout.as_secs()
                )
            } else {
                format!("reading from the dev server failed: {e}")
            }
        })?;
        parse_response(&raw)
    }
}

fn parse_response(raw: &[u8]) -> Result<HttpResponse, String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("malformed response from the dev server")?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or("malformed status line from the dev server")?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let rest = &raw[split + 4..];
    let chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked")
    });
    let body = if chunked {
        dechunk(rest)
    } else {
        rest.to_vec()
    };
    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

fn dechunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(eol) = data.windows(2).position(|w| w == b"\r\n") {
        let size_line = String::from_utf8_lossy(&data[..eol]);
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size_hex, 16) else {
            break;
        };
        data = &data[eol + 2..];
        if size == 0 || data.len() < size {
            out.extend_from_slice(&data[..size.min(data.len())]);
            break;
        }
        out.extend_from_slice(&data[..size]);
        data = data.get(size + 2..).unwrap_or(&[]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_response() {
        let raw = b"HTTP/1.1 404 Not Found\r\ncontent-type: text/plain\r\nX-Soli-Request-Id: abc\r\n\r\ngone";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.status, 404);
        assert_eq!(r.header("x-soli-request-id"), Some("abc"));
        assert_eq!(r.text(), "gone");
    }

    #[test]
    fn parses_a_chunked_response() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(parse_response(raw).unwrap().text(), "Wikipedia");
    }
}
