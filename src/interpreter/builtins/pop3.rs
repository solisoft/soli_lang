//! POP3 email-reading builtin: the `Pop3` client class.
//!
//! Provides a small synchronous POP3(S) client so Soli code can read mail:
//!
//! ```soli
//! mail = Pop3.new("pop.gmail.com", "me@gmail.com", "app-password")
//! for msg in mail.fetch_all()
//!   print(msg["subject"])
//! end
//! mail.quit()
//! ```
//!
//! TLS is done synchronously with `rustls` over a `std::net::TcpStream` (ring
//! provider, matching the rest of the build). Fetched messages are parsed into
//! structured hashes with the `mail-parser` crate.
//!
//! The live connection is kept in a process-global registry keyed by an
//! integer id stored in the instance's `_id` field — the same pattern used by
//! the `Solidb` builtin class (see `solidb.rs`).

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use lazy_static::lazy_static;
use mail_parser::{Addr, Address, MessageParser, MimeHeaders};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{hash_from_pairs, Class, HashKey, Instance, NativeFunction, Value};

/// Default network timeout for connect / read / write.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Default cap on messages downloaded by `.fetch_all()`; override with
/// `SOLI_POP3_MAX_MESSAGES`.
const DEFAULT_MAX_MESSAGES: i64 = 200;

/// A boxed, owning POP3 stream — either a rustls TLS stream or a plain TCP
/// stream (the latter only for local/testing servers).
trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

/// A live, authenticated POP3 connection.
struct Pop3Conn {
    reader: BufReader<Box<dyn Stream>>,
}

lazy_static! {
    /// Process-global registry of open connections, keyed by instance id.
    static ref POP3_CONNS: Mutex<HashMap<usize, Pop3Conn>> = Mutex::new(HashMap::new());
}
static POP3_NEXT_ID: AtomicUsize = AtomicUsize::new(1);

// ---------------------------------------------------------------------------
// TLS
// ---------------------------------------------------------------------------

/// Build (once) a rustls client config trusting the Mozilla root set.
fn tls_config() -> Result<Arc<ClientConfig>, String> {
    static CONFIG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    if let Some(cfg) = CONFIG.get() {
        return Ok(cfg.clone());
    }
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("POP3 TLS init failed: {e}"))?
            .with_root_certificates(roots)
            .with_no_client_auth();
    let arc = Arc::new(config);
    let _ = CONFIG.set(arc.clone());
    Ok(arc)
}

/// Open a TCP (optionally TLS-wrapped) connection to the POP3 server.
fn connect(host: &str, port: u16, use_tls: bool) -> Result<Box<dyn Stream>, String> {
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed for {host}:{port}: {e}"))?
        .next()
        .ok_or_else(|| format!("No address found for {host}:{port}"))?;
    let tcp = TcpStream::connect_timeout(&addr, DEFAULT_TIMEOUT)
        .map_err(|e| format!("Connect to {host}:{port} failed: {e}"))?;
    let _ = tcp.set_read_timeout(Some(DEFAULT_TIMEOUT));
    let _ = tcp.set_write_timeout(Some(DEFAULT_TIMEOUT));

    if use_tls {
        let config = tls_config()?;
        let server_name = ServerName::try_from(host.to_string())
            .map_err(|_| format!("Invalid TLS server name: {host}"))?;
        let conn = ClientConnection::new(config, server_name)
            .map_err(|e| format!("TLS handshake setup failed: {e}"))?;
        Ok(Box::new(StreamOwned::new(conn, tcp)))
    } else {
        Ok(Box::new(tcp))
    }
}

// ---------------------------------------------------------------------------
// POP3 protocol primitives (independent of the registry, for testability)
// ---------------------------------------------------------------------------

/// Read a single status line, returning the text after `+OK`, or an error for
/// `-ERR` / unexpected responses.
fn read_status_line<R: BufRead>(reader: &mut R) -> Result<String, String> {
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .map_err(|e| format!("POP3 read error: {e}"))?;
    if n == 0 {
        return Err("POP3 connection closed by server".to_string());
    }
    let line = line.trim_end_matches(['\r', '\n']);
    if let Some(rest) = line.strip_prefix("+OK") {
        Ok(rest.trim_start().to_string())
    } else if let Some(rest) = line.strip_prefix("-ERR") {
        Err(format!("POP3 server error:{}", rest))
    } else {
        Err(format!("Unexpected POP3 response: {line}"))
    }
}

/// Read a dot-terminated multiline response (the body following a `+OK`),
/// performing dot-unstuffing and preserving CRLF line endings for the parser.
fn read_multiline<R: BufRead>(reader: &mut R) -> Result<String, String> {
    let mut out = String::new();
    loop {
        let mut line = String::new();
        let n = reader
            .read_line(&mut line)
            .map_err(|e| format!("POP3 read error: {e}"))?;
        if n == 0 {
            return Err("POP3 connection closed during multiline read".to_string());
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "." {
            break;
        }
        // Dot-unstuffing: a line that began with '.' was doubled by the server.
        let content = trimmed.strip_prefix('.').unwrap_or(trimmed);
        out.push_str(content);
        out.push_str("\r\n");
    }
    Ok(out)
}

impl Pop3Conn {
    /// Write a command line (CRLF-terminated) and flush.
    fn send(&mut self, cmd: &str) -> Result<(), String> {
        let writer = self.reader.get_mut();
        writer
            .write_all(cmd.as_bytes())
            .and_then(|_| writer.write_all(b"\r\n"))
            .and_then(|_| writer.flush())
            .map_err(|e| format!("POP3 write error: {e}"))
    }

    /// Send a command and read its single-line `+OK` status (text after `+OK`).
    fn command(&mut self, cmd: &str) -> Result<String, String> {
        self.send(cmd)?;
        read_status_line(&mut self.reader)
    }

    /// Send a command, consume the `+OK` line, then read the dot-terminated body.
    fn command_multiline(&mut self, cmd: &str) -> Result<String, String> {
        self.send(cmd)?;
        read_status_line(&mut self.reader)?;
        read_multiline(&mut self.reader)
    }
}

// ---------------------------------------------------------------------------
// Registry helpers
// ---------------------------------------------------------------------------

/// Extract the connection id from the instance passed as `args[0]`.
fn instance_id(args: &[Value], method: &str) -> Result<usize, String> {
    let inst = match args.first() {
        Some(Value::Instance(inst)) => inst,
        _ => return Err(format!("Pop3.{method}() must be called on a Pop3 instance")),
    };
    match inst.borrow().get("_id") {
        Some(Value::Int(id)) => Ok(id as usize),
        _ => Err("Pop3 instance has no open connection (already closed?)".to_string()),
    }
}

/// Run `f` against the live connection for `id`.
fn with_conn<R>(
    id: usize,
    f: impl FnOnce(&mut Pop3Conn) -> Result<R, String>,
) -> Result<R, String> {
    let mut conns = POP3_CONNS.lock().map_err(|e| e.to_string())?;
    let conn = conns
        .get_mut(&id)
        .ok_or_else(|| "Pop3 connection is closed (call .quit() only once)".to_string())?;
    f(conn)
}

// ---------------------------------------------------------------------------
// Message parsing
// ---------------------------------------------------------------------------

fn opt_str(s: Option<&str>) -> Value {
    s.map(|s| Value::String(s.to_string()))
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

/// Parse a raw RFC822 message into a structured Soli hash.
fn parse_message(raw: &str, msg_id: i64) -> Value {
    let size = raw.len() as i64;
    let parsed = MessageParser::default().parse(raw.as_bytes());

    let (subject, from, to, date, text_body, html_body, attachments) = match &parsed {
        Some(msg) => {
            let date = msg
                .date()
                .map(|d| Value::String(d.to_rfc3339()))
                .unwrap_or(Value::Null);
            let text_body = msg
                .body_text(0)
                .map(|c| Value::String(c.into_owned()))
                .unwrap_or(Value::Null);
            let html_body = msg
                .body_html(0)
                .map(|c| Value::String(c.into_owned()))
                .unwrap_or(Value::Null);
            let mut atts = Vec::new();
            for part in msg.attachments() {
                let content_type = part
                    .content_type()
                    .map(|ct| match ct.subtype() {
                        Some(sub) => Value::String(format!("{}/{}", ct.ctype(), sub)),
                        None => Value::String(ct.ctype().to_string()),
                    })
                    .unwrap_or(Value::Null);
                atts.push(hash_from_pairs([
                    ("name".to_string(), opt_str(part.attachment_name())),
                    ("content_type".to_string(), content_type),
                    ("size".to_string(), Value::Int(part.len() as i64)),
                ]));
            }
            (
                opt_str(msg.subject()),
                addr_first(msg.from()),
                addr_all(msg.to()),
                date,
                text_body,
                html_body,
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
        ),
    };

    hash_from_pairs([
        ("id".to_string(), Value::Int(msg_id)),
        ("size".to_string(), Value::Int(size)),
        ("subject".to_string(), subject),
        ("from".to_string(), from),
        ("to".to_string(), to),
        ("date".to_string(), date),
        ("text_body".to_string(), text_body),
        ("html_body".to_string(), html_body),
        ("attachments".to_string(), attachments),
        ("raw".to_string(), Value::String(raw.to_string())),
    ])
}

// ---------------------------------------------------------------------------
// Constructor + instance methods
// ---------------------------------------------------------------------------

/// Validate a credential/host string and reject CR/LF (POP3 command injection).
fn as_string(v: &Value, field: &str) -> Result<String, String> {
    match v {
        Value::String(s) => {
            if s.contains('\r') || s.contains('\n') {
                Err(format!(
                    "Pop3.new() {field} must not contain CR/LF characters"
                ))
            } else {
                Ok(s.clone())
            }
        }
        other => Err(format!(
            "Pop3.new() expects a string {field}, got {}",
            other.type_name()
        )),
    }
}

fn pop3_new(class: Rc<Class>, args: Vec<Value>) -> Result<Value, String> {
    if args.len() < 3 || args.len() > 4 {
        return Err(format!(
            "Pop3.new(host, user, password, opts?) expects 3 or 4 arguments, got {}",
            args.len()
        ));
    }
    let host = as_string(&args[0], "host")?;
    let user = as_string(&args[1], "user")?;
    let pass = as_string(&args[2], "password")?;

    let mut port: u16 = 995;
    let mut use_tls = true;
    match args.get(3) {
        None | Some(Value::Null) => {}
        Some(Value::Hash(opts)) => {
            let opts = opts.borrow();
            if let Some(Value::Int(p)) = opts.get(&HashKey::String("port".to_string())) {
                if *p < 1 || *p > 65535 {
                    return Err(format!("Pop3.new() opts.port {p} out of range 1..65535"));
                }
                port = *p as u16;
            }
            if let Some(Value::Bool(b)) = opts.get(&HashKey::String("tls".to_string())) {
                use_tls = *b;
            }
        }
        Some(other) => {
            return Err(format!(
                "Pop3.new() opts must be a Hash, got {}",
                other.type_name()
            ))
        }
    }

    let stream = connect(&host, port, use_tls)?;
    let mut conn = Pop3Conn {
        reader: BufReader::new(stream),
    };

    // Server greeting.
    read_status_line(&mut conn.reader).map_err(|e| format!("POP3 greeting failed: {e}"))?;
    // Authenticate.
    conn.command(&format!("USER {user}"))
        .map_err(|e| format!("POP3 USER failed: {e}"))?;
    conn.command(&format!("PASS {pass}"))
        .map_err(|e| format!("POP3 authentication failed: {e}"))?;

    let id = POP3_NEXT_ID.fetch_add(1, Ordering::SeqCst);
    POP3_CONNS
        .lock()
        .map_err(|e| e.to_string())?
        .insert(id, conn);

    let mut inst = Instance::new(class);
    inst.set("_id".to_string(), Value::Int(id as i64));
    Ok(Value::Instance(Rc::new(RefCell::new(inst))))
}

fn pop3_stat(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "stat")?;
    let resp = with_conn(id, |c| c.command("STAT"))?;
    let mut it = resp.split_whitespace();
    let count = it.next().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    let size = it.next().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    Ok(hash_from_pairs([
        ("count".to_string(), Value::Int(count)),
        ("size".to_string(), Value::Int(size)),
    ]))
}

fn pop3_list(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "list")?;
    let body = with_conn(id, |c| c.command_multiline("LIST"))?;
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let msg_id = it.next().and_then(|s| s.parse::<i64>().ok());
        let size = it.next().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
        if let Some(mid) = msg_id {
            out.push(hash_from_pairs([
                ("id".to_string(), Value::Int(mid)),
                ("size".to_string(), Value::Int(size)),
            ]));
        }
    }
    Ok(Value::Array(Rc::new(RefCell::new(out))))
}

fn message_id_arg(args: &[Value], method: &str) -> Result<i64, String> {
    match args.get(1) {
        Some(Value::Int(n)) => Ok(*n),
        _ => Err(format!("Pop3.{method}(id) expects an integer message id")),
    }
}

fn pop3_fetch(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "fetch")?;
    let n = message_id_arg(&args, "fetch")?;
    let raw = with_conn(id, |c| c.command_multiline(&format!("RETR {n}")))?;
    Ok(parse_message(&raw, n))
}

fn pop3_fetch_all(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "fetch_all")?;
    let stat = with_conn(id, |c| c.command("STAT"))?;
    let count = stat
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);

    let cap = std::env::var("SOLI_POP3_MAX_MESSAGES")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_MAX_MESSAGES);
    let fetch_count = count.min(cap);
    if count > cap {
        eprintln!(
            "[pop3] fetch_all: mailbox has {count} messages; fetching first {cap} \
             (raise SOLI_POP3_MAX_MESSAGES to fetch more)"
        );
    }

    let mut out = Vec::new();
    for n in 1..=fetch_count {
        let raw = with_conn(id, |c| c.command_multiline(&format!("RETR {n}")))?;
        out.push(parse_message(&raw, n));
    }
    Ok(Value::Array(Rc::new(RefCell::new(out))))
}

fn pop3_delete(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "delete")?;
    let n = message_id_arg(&args, "delete")?;
    with_conn(id, |c| c.command(&format!("DELE {n}")))?;
    Ok(Value::Bool(true))
}

fn pop3_quit(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "quit")?;
    let mut conns = POP3_CONNS.lock().map_err(|e| e.to_string())?;
    if let Some(mut conn) = conns.remove(&id) {
        // Best-effort: tell the server to commit deletions and close.
        let _ = conn.command("QUIT");
    }
    Ok(Value::Bool(true))
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register the `Pop3` builtin class into `env`.
pub fn register_pop3_class(env: &mut Environment) {
    let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    native_methods.insert(
        "stat".to_string(),
        Rc::new(NativeFunction::new("Pop3.stat", Some(0), pop3_stat)),
    );
    native_methods.insert(
        "list".to_string(),
        Rc::new(NativeFunction::new("Pop3.list", Some(0), pop3_list)),
    );
    native_methods.insert(
        "fetch".to_string(),
        Rc::new(NativeFunction::new("Pop3.fetch", Some(1), pop3_fetch)),
    );
    native_methods.insert(
        "fetch_all".to_string(),
        Rc::new(NativeFunction::new(
            "Pop3.fetch_all",
            Some(0),
            pop3_fetch_all,
        )),
    );
    native_methods.insert(
        "delete".to_string(),
        Rc::new(NativeFunction::new("Pop3.delete", Some(1), pop3_delete)),
    );
    native_methods.insert(
        "quit".to_string(),
        Rc::new(NativeFunction::new("Pop3.quit", Some(0), pop3_quit)),
    );

    // The `new` static method needs the class Rc to build instances, but the
    // class embeds the method — break the cycle with a Weak upgraded at call time.
    let pop3_class = Rc::new_cyclic(|weak: &Weak<Class>| {
        let weak = weak.clone();
        let mut native_static: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        native_static.insert(
            "new".to_string(),
            Rc::new(NativeFunction::new("Pop3.new", None, move |args| {
                let class = weak
                    .upgrade()
                    .ok_or_else(|| "Pop3 class was dropped".to_string())?;
                pop3_new(class, args)
            })),
        );
        Class {
            name: "Pop3".to_string(),
            native_static_methods: native_static,
            native_methods,
            ..Default::default()
        }
    });

    env.define("Pop3".to_string(), Value::Class(pop3_class));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn status_line_ok() {
        let mut r = Cursor::new(b"+OK 3 1024\r\n".to_vec());
        assert_eq!(read_status_line(&mut r).unwrap(), "3 1024");
    }

    #[test]
    fn status_line_err() {
        let mut r = Cursor::new(b"-ERR bad password\r\n".to_vec());
        assert!(read_status_line(&mut r).is_err());
    }

    #[test]
    fn status_line_closed() {
        let mut r = Cursor::new(Vec::new());
        assert!(read_status_line(&mut r).is_err());
    }

    #[test]
    fn multiline_terminator_and_dot_unstuffing() {
        // Body lines, a dot-stuffed line, then the lone-dot terminator.
        let mut r = Cursor::new(b"line one\r\n..dotted\r\n.\r\nignored\r\n".to_vec());
        let body = read_multiline(&mut r).unwrap();
        assert_eq!(body, "line one\r\n.dotted\r\n");
    }

    #[test]
    fn parse_message_extracts_fields() {
        let raw = "From: Alice <alice@example.com>\r\n\
                   To: Bob <bob@example.com>\r\n\
                   Subject: Hello\r\n\
                   Date: Mon, 1 Jun 2026 10:00:00 +0000\r\n\
                   Content-Type: text/plain\r\n\
                   \r\n\
                   Hi there!\r\n";
        let v = parse_message(raw, 1);
        let Value::Hash(h) = v else {
            panic!("expected hash")
        };
        let h = h.borrow();
        assert!(matches!(
            h.get(&HashKey::String("subject".to_string())),
            Some(Value::String(s)) if s == "Hello"
        ));
        assert!(matches!(
            h.get(&HashKey::String("id".to_string())),
            Some(Value::Int(1))
        ));
        // from is a {name, address} hash
        match h.get(&HashKey::String("from".to_string())) {
            Some(Value::Hash(from)) => {
                let from = from.borrow();
                assert!(matches!(
                    from.get(&HashKey::String("address".to_string())),
                    Some(Value::String(a)) if a == "alice@example.com"
                ));
            }
            other => panic!("expected from hash, got {other:?}"),
        }
        // text body present
        assert!(matches!(
            h.get(&HashKey::String("text_body".to_string())),
            Some(Value::String(s)) if s.contains("Hi there!")
        ));
    }

    #[test]
    fn parse_message_with_attachment() {
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
        let v = parse_message(raw, 7);
        let Value::Hash(h) = v else {
            panic!("expected hash")
        };
        let h = h.borrow();
        match h.get(&HashKey::String("attachments".to_string())) {
            Some(Value::Array(arr)) => {
                let arr = arr.borrow();
                assert_eq!(arr.len(), 1);
                let Value::Hash(att) = &arr[0] else {
                    panic!("attachment not a hash")
                };
                let att = att.borrow();
                assert!(matches!(
                    att.get(&HashKey::String("name".to_string())),
                    Some(Value::String(n)) if n == "data.csv"
                ));
            }
            other => panic!("expected attachments array, got {other:?}"),
        }
    }
}
