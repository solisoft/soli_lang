//! Outbound email: the `Mailer` base class and `Message` object.
//!
//! Apps define a mailer the way Rails does — a subclass with one method per
//! email, setting instance variables and calling `this.mail(...)`:
//!
//! ```soli
//! class UserMailer < Mailer
//!   def welcome(user)
//!     @user = user
//!     this.mail(to: user.email, subject: "Welcome!")  # renders
//!   end                                                # views/user_mailer/welcome
//! end
//!
//! UserMailer.welcome(user).deliver_now    # send synchronously
//! UserMailer.welcome(user).deliver_later  # enqueue via the Job queue
//! ```
//!
//! The ergonomic surface (the `Mailer`/`Message` classes) ships as a Soli
//! prelude (see [`MAILER_PRELUDE`]); the heavy lifting lives in the native
//! builtins below (`__mail_render`, `__mailer_deliver`, …). `UserMailer.welcome`
//! dispatches through `Mailer.method_missing` — the class-level method_missing
//! fallback in `executor/access/member.rs` instantiates the subclass, stamps
//! the action name, and runs the matching instance method.
//!
//! SMTP is hand-rolled over the same synchronous `rustls`/`TcpStream` stack as
//! the POP3 client (see `pop3.rs`), adding STARTTLS (port 587) on top of
//! implicit TLS (port 465). MIME construction uses the `mail-builder` crate.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::rc::Rc;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use base64::Engine;
use lazy_static::lazy_static;
use mail_builder::headers::address::Address;
use mail_builder::headers::date::Date;
use mail_builder::MessageBuilder;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, HashPairs, NativeFunction, Value};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeliveryMethod {
    Smtp,
    Test,
    Logger,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TlsMode {
    /// Implicit TLS from the first byte (SMTPS, typically port 465).
    Tls,
    /// Upgrade a plaintext connection with STARTTLS (submission, port 587).
    Starttls,
    /// No transport security (local catchers / tests).
    None,
    /// Implicit TLS on 465, STARTTLS otherwise.
    Auto,
}

#[derive(Clone)]
struct MailerConfig {
    delivery_method: DeliveryMethod,
    host: String,
    port: u16,
    user: Option<String>,
    pass: Option<String>,
    tls: TlsMode,
    /// Default `From` when a mailer doesn't set one.
    from: Option<String>,
    /// EHLO name and the host part of generated `Message-ID`s.
    domain: String,
}

impl MailerConfig {
    fn from_env() -> Self {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let delivery_method = match env("SOLI_MAIL_DELIVERY_METHOD").as_deref() {
            Some("test") => DeliveryMethod::Test,
            Some("logger") => DeliveryMethod::Logger,
            _ => DeliveryMethod::Smtp,
        };
        let tls = match env("SOLI_SMTP_TLS").as_deref() {
            Some("tls") => TlsMode::Tls,
            Some("starttls") => TlsMode::Starttls,
            Some("none") => TlsMode::None,
            _ => TlsMode::Auto,
        };
        MailerConfig {
            delivery_method,
            host: env("SOLI_SMTP_HOST").unwrap_or_default(),
            port: env("SOLI_SMTP_PORT")
                .and_then(|p| p.parse().ok())
                .unwrap_or(587),
            user: env("SOLI_SMTP_USER"),
            pass: env("SOLI_SMTP_PASS"),
            tls,
            from: env("SOLI_SMTP_FROM"),
            domain: env("SOLI_SMTP_DOMAIN").unwrap_or_else(|| "localhost".to_string()),
        }
    }

    fn implicit_tls(&self) -> bool {
        matches!(self.tls, TlsMode::Tls) || (matches!(self.tls, TlsMode::Auto) && self.port == 465)
    }

    fn use_starttls(&self) -> bool {
        matches!(self.tls, TlsMode::Starttls)
            || (matches!(self.tls, TlsMode::Auto) && self.port != 465)
    }
}

lazy_static! {
    static ref MAILER_CONFIG: RwLock<MailerConfig> = RwLock::new(MailerConfig::from_env());
}

thread_local! {
    // Captured mail in `test` delivery mode, for `Mailer.deliveries()`. Thread-
    // local because `Value` is `!Send`; tests send and assert on one thread.
    static DELIVERIES: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
}

// ---------------------------------------------------------------------------
// Hash / Value helpers
// ---------------------------------------------------------------------------

fn hash_get(h: &HashPairs, key: &str) -> Option<Value> {
    h.get(&HashKey::String(key.into()))
        .cloned()
        .filter(|v| !matches!(v, Value::Null))
}

fn as_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.to_string()),
        _ => None,
    }
}

/// Normalize a recipient field — a single string, an array of strings, or
/// absent — into a list of address strings.
fn as_address_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.to_string()],
        Some(Value::Array(arr)) => arr
            .borrow()
            .iter()
            .filter_map(as_string)
            .filter(|s| !s.trim().is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Split `"Display Name <addr@host>"` into `(Some(name), addr)`, or `(None, s)`
/// when there's no angle-bracket form. Used for the `From`/`Reply-To` display
/// name; recipients are passed through as bare addresses.
fn parse_address(s: &str) -> (Option<String>, String) {
    let s = s.trim();
    if let (Some(lt), Some(gt)) = (s.rfind('<'), s.rfind('>')) {
        if lt < gt {
            let email = s[lt + 1..gt].trim().to_string();
            let name = s[..lt].trim().trim_matches('"').trim().to_string();
            return (if name.is_empty() { None } else { Some(name) }, email);
        }
    }
    (None, s.to_string())
}

fn make_address(s: &str) -> Address<'static> {
    match parse_address(s) {
        (Some(name), email) => Address::from((name, email)),
        (None, email) => Address::from(email),
    }
}

/// The bare `addr@host` part, for the SMTP envelope (`MAIL FROM` / `RCPT TO`).
fn envelope_address(s: &str) -> String {
    parse_address(s).1
}

// ---------------------------------------------------------------------------
// MIME construction (mail-builder)
// ---------------------------------------------------------------------------

/// Build an RFC 5322 message from a rendered mail hash.
fn build_mime(mail: &HashPairs) -> Result<String, String> {
    let from = hash_get(mail, "from")
        .and_then(|v| as_string(&v))
        .ok_or_else(|| "mail is missing a `from` address".to_string())?;
    let to = as_address_list(mail.get(&HashKey::String("to".into())));
    if to.is_empty() {
        return Err("mail is missing a `to` recipient".to_string());
    }
    let cc = as_address_list(mail.get(&HashKey::String("cc".into())));
    let subject = hash_get(mail, "subject")
        .and_then(|v| as_string(&v))
        .unwrap_or_default();

    let mut builder = MessageBuilder::new()
        .from(make_address(&from))
        .to(to)
        .subject(subject)
        .date(Date::now());

    if !cc.is_empty() {
        builder = builder.cc(cc);
    }
    // Bcc is intentionally NOT added as a header; bcc recipients are only in
    // the SMTP envelope (see deliver_smtp).
    if let Some(reply_to) = hash_get(mail, "reply_to").and_then(|v| as_string(&v)) {
        builder = builder.reply_to(make_address(&reply_to));
    }
    if let Some(text) = hash_get(mail, "text").and_then(|v| as_string(&v)) {
        builder = builder.text_body(text);
    }
    if let Some(html) = hash_get(mail, "html").and_then(|v| as_string(&v)) {
        builder = builder.html_body(html);
    }

    if let Some(Value::Array(atts)) = mail.get(&HashKey::String("attachments".into())) {
        for att in atts.borrow().iter() {
            if let Value::Hash(att) = att {
                let att = att.borrow();
                let filename = hash_get(&att, "filename")
                    .and_then(|v| as_string(&v))
                    .unwrap_or_else(|| "attachment".to_string());
                let content_type = hash_get(&att, "content_type")
                    .and_then(|v| as_string(&v))
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                // A `base64` field carries binary content (decoded to bytes);
                // otherwise `content` is treated as a UTF-8 text body.
                let bytes: Vec<u8> = if let Some(b64) =
                    hash_get(&att, "base64").and_then(|v| as_string(&v))
                {
                    base64::engine::general_purpose::STANDARD
                        .decode(b64.trim().as_bytes())
                        .map_err(|e| format!("attachment '{filename}' has invalid base64: {e}"))?
                } else {
                    hash_get(&att, "content")
                        .and_then(|v| as_string(&v))
                        .unwrap_or_default()
                        .into_bytes()
                };
                builder = builder.attachment(content_type, filename, bytes);
            }
        }
    }

    builder
        .write_to_string()
        .map_err(|e| format!("failed to build MIME message: {e}"))
}

// ---------------------------------------------------------------------------
// SMTP transport (sync rustls; STARTTLS + implicit TLS + AUTH LOGIN)
// ---------------------------------------------------------------------------

/// Shared rustls client config (Mozilla roots), built once. Mirrors `pop3.rs`.
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
            .map_err(|e| format!("SMTP TLS init failed: {e}"))?
            .with_root_certificates(roots)
            .with_no_client_auth();
    let arc = Arc::new(config);
    let _ = CONFIG.set(arc.clone());
    Ok(arc)
}

/// A plaintext or TLS SMTP connection. Plain can be upgraded in place for
/// STARTTLS; both variants buffer reads for line-oriented reply parsing.
enum SmtpStream {
    Plain(BufReader<TcpStream>),
    Tls(Box<BufReader<StreamOwned<ClientConnection, TcpStream>>>),
}

impl SmtpStream {
    fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        match self {
            SmtpStream::Plain(r) => r.read_line(buf),
            SmtpStream::Tls(r) => r.read_line(buf),
        }
    }

    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        match self {
            SmtpStream::Plain(r) => r.get_mut().write_all(bytes),
            SmtpStream::Tls(r) => r.get_mut().write_all(bytes),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            SmtpStream::Plain(r) => r.get_mut().flush(),
            SmtpStream::Tls(r) => r.get_mut().flush(),
        }
    }

    /// Upgrade a plaintext connection to TLS (after the server's 220 to
    /// STARTTLS). A TLS stream is returned unchanged.
    fn upgrade(self, host: &str) -> Result<SmtpStream, String> {
        match self {
            SmtpStream::Plain(reader) => {
                let tcp = reader.into_inner();
                let config = tls_config()?;
                let server_name = ServerName::try_from(host.to_string())
                    .map_err(|_| format!("invalid TLS server name: {host}"))?;
                let conn = ClientConnection::new(config, server_name)
                    .map_err(|e| format!("TLS handshake setup failed: {e}"))?;
                Ok(SmtpStream::Tls(Box::new(BufReader::new(StreamOwned::new(
                    conn, tcp,
                )))))
            }
            tls => Ok(tls),
        }
    }
}

/// Read one SMTP reply (handling `250-` multiline continuations) and return its
/// status code.
fn read_reply(stream: &mut SmtpStream) -> Result<u16, String> {
    loop {
        let mut line = String::new();
        let n = stream
            .read_line(&mut line)
            .map_err(|e| format!("SMTP read error: {e}"))?;
        if n == 0 {
            return Err("SMTP connection closed by server".to_string());
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.len() < 3 {
            return Err(format!("malformed SMTP reply: {trimmed}"));
        }
        let code: u16 = trimmed[..3]
            .parse()
            .map_err(|_| format!("malformed SMTP reply: {trimmed}"))?;
        // A space after the code marks the final line; '-' means more follow.
        if trimmed.as_bytes().get(3) != Some(&b'-') {
            return Ok(code);
        }
    }
}

fn send_command(stream: &mut SmtpStream, command: &str) -> Result<u16, String> {
    stream
        .write_all(command.as_bytes())
        .and_then(|_| stream.write_all(b"\r\n"))
        .and_then(|_| stream.flush())
        .map_err(|e| format!("SMTP write error: {e}"))?;
    read_reply(stream)
}

/// Send a command and require the reply code to be in `expected`.
fn expect_command(stream: &mut SmtpStream, command: &str, expected: &[u16]) -> Result<(), String> {
    let code = send_command(stream, command)?;
    if expected.contains(&code) {
        Ok(())
    } else {
        Err(format!(
            "SMTP command `{}` rejected with code {}",
            command.split(' ').next().unwrap_or(command),
            code
        ))
    }
}

fn b64(s: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(s.as_bytes())
}

/// Connect, authenticate, and transmit one message. `rcpts` is the full
/// envelope recipient set (to + cc + bcc).
fn deliver_smtp(cfg: &MailerConfig, rcpts: &[String], data: &str) -> Result<(), String> {
    if cfg.host.is_empty() {
        return Err(
            "Mailer is not configured: set a host via Mailer.configure({...}) or SOLI_SMTP_HOST"
                .to_string(),
        );
    }
    let addr = (cfg.host.as_str(), cfg.port)
        .to_socket_addrs()
        .map_err(|e| format!("DNS resolution failed for {}:{}: {e}", cfg.host, cfg.port))?
        .next()
        .ok_or_else(|| format!("no address found for {}:{}", cfg.host, cfg.port))?;
    let tcp = TcpStream::connect_timeout(&addr, DEFAULT_TIMEOUT)
        .map_err(|e| format!("connect to {}:{} failed: {e}", cfg.host, cfg.port))?;
    let _ = tcp.set_read_timeout(Some(DEFAULT_TIMEOUT));
    let _ = tcp.set_write_timeout(Some(DEFAULT_TIMEOUT));

    let mut stream = if cfg.implicit_tls() {
        SmtpStream::Plain(BufReader::new(tcp)).upgrade(&cfg.host)?
    } else {
        SmtpStream::Plain(BufReader::new(tcp))
    };

    // Greeting.
    let code = read_reply(&mut stream)?;
    if code != 220 {
        return Err(format!("unexpected SMTP greeting code {code}"));
    }

    let ehlo = format!("EHLO {}", cfg.domain);
    expect_command(&mut stream, &ehlo, &[250])?;

    if !cfg.implicit_tls() && cfg.use_starttls() {
        expect_command(&mut stream, "STARTTLS", &[220])?;
        stream = stream.upgrade(&cfg.host)?;
        expect_command(&mut stream, &ehlo, &[250])?;
    }

    if let (Some(user), Some(pass)) = (cfg.user.as_ref(), cfg.pass.as_ref()) {
        expect_command(&mut stream, "AUTH LOGIN", &[334])?;
        expect_command(&mut stream, &b64(user), &[334])?;
        expect_command(&mut stream, &b64(pass), &[235])?;
    }

    let envelope_from = envelope_from_address(cfg, data);
    expect_command(
        &mut stream,
        &format!("MAIL FROM:<{}>", envelope_from),
        &[250],
    )?;
    for rcpt in rcpts {
        expect_command(&mut stream, &format!("RCPT TO:<{}>", rcpt), &[250, 251])?;
    }
    expect_command(&mut stream, "DATA", &[354])?;

    // Dot-stuff (RFC 5321 §4.5.2) and terminate with <CRLF>.<CRLF>.
    let stuffed = data.replace("\r\n.", "\r\n..");
    stream
        .write_all(stuffed.as_bytes())
        .and_then(|_| stream.write_all(b"\r\n.\r\n"))
        .and_then(|_| stream.flush())
        .map_err(|e| format!("SMTP DATA write error: {e}"))?;
    let code = read_reply(&mut stream)?;
    if code != 250 {
        return Err(format!("SMTP server rejected message with code {code}"));
    }

    let _ = send_command(&mut stream, "QUIT");
    Ok(())
}

/// The envelope sender: the configured default `from` if set, else parse it
/// from the message's `From:` header.
fn envelope_from_address(cfg: &MailerConfig, data: &str) -> String {
    if let Some(from) = &cfg.from {
        return envelope_address(from);
    }
    for line in data.lines() {
        if let Some(rest) = line.strip_prefix("From:") {
            return envelope_address(rest);
        }
        if line.is_empty() {
            break; // end of headers
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Native builtins (wired to the Soli prelude below)
// ---------------------------------------------------------------------------

/// `__mailer_configure(opts_hash)` — merge options into the global config.
fn mailer_configure(args: Vec<Value>) -> Result<Value, String> {
    let Some(Value::Hash(opts)) = args.first() else {
        return Err("Mailer.configure expects a Hash of options".to_string());
    };
    let opts = opts.borrow();
    let mut cfg = MAILER_CONFIG
        .write()
        .map_err(|e| format!("mailer config lock poisoned: {e}"))?;

    if let Some(dm) = hash_get(&opts, "delivery_method").and_then(|v| as_string(&v)) {
        cfg.delivery_method = match dm.as_str() {
            "test" => DeliveryMethod::Test,
            "logger" => DeliveryMethod::Logger,
            "smtp" => DeliveryMethod::Smtp,
            other => return Err(format!("unknown delivery_method: {other}")),
        };
    }
    if let Some(host) = hash_get(&opts, "host").and_then(|v| as_string(&v)) {
        cfg.host = host;
    }
    if let Some(Value::Int(port)) = opts.get(&HashKey::String("port".into())) {
        if *port < 1 || *port > 65535 {
            return Err(format!("port {port} out of range 1..65535"));
        }
        cfg.port = *port as u16;
    }
    if let Some(user) = hash_get(&opts, "user").and_then(|v| as_string(&v)) {
        cfg.user = Some(user);
    }
    if let Some(pass) = hash_get(&opts, "pass").and_then(|v| as_string(&v)) {
        cfg.pass = Some(pass);
    }
    if let Some(tls) = hash_get(&opts, "tls").and_then(|v| as_string(&v)) {
        cfg.tls = match tls.as_str() {
            "tls" => TlsMode::Tls,
            "starttls" => TlsMode::Starttls,
            "none" => TlsMode::None,
            "auto" => TlsMode::Auto,
            other => return Err(format!("unknown tls mode: {other}")),
        };
    }
    if let Some(from) = hash_get(&opts, "from").and_then(|v| as_string(&v)) {
        cfg.from = Some(from);
    }
    if let Some(domain) = hash_get(&opts, "domain").and_then(|v| as_string(&v)) {
        cfg.domain = domain;
    }
    Ok(Value::Null)
}

/// `__mailer_deliver(mail_hash)` — send (or capture) a fully-rendered mail.
fn mailer_deliver(args: Vec<Value>) -> Result<Value, String> {
    let Some(Value::Hash(mail)) = args.first() else {
        return Err("Mailer.deliver expects a Hash".to_string());
    };
    let cfg = MAILER_CONFIG
        .read()
        .map_err(|e| format!("mailer config lock poisoned: {e}"))?
        .clone();

    match cfg.delivery_method {
        DeliveryMethod::Test => {
            // Capture the rendered mail hash for assertions.
            let captured = args[0].clone();
            DELIVERIES.with(|d| d.borrow_mut().push(captured));
            Ok(Value::Bool(true))
        }
        DeliveryMethod::Logger => {
            let mail = mail.borrow();
            let to = as_address_list(mail.get(&HashKey::String("to".into()))).join(", ");
            let subject = hash_get(&mail, "subject")
                .and_then(|v| as_string(&v))
                .unwrap_or_default();
            eprintln!("[mailer] (logger) to={to} subject={subject:?}");
            Ok(Value::Bool(true))
        }
        DeliveryMethod::Smtp => {
            let mail = mail.borrow();
            let mut rcpts = as_address_list(mail.get(&HashKey::String("to".into())));
            rcpts.extend(as_address_list(mail.get(&HashKey::String("cc".into()))));
            rcpts.extend(as_address_list(mail.get(&HashKey::String("bcc".into()))));
            let rcpts: Vec<String> = rcpts.iter().map(|a| envelope_address(a)).collect();
            let data = build_mime(&mail)?;
            deliver_smtp(&cfg, &rcpts, &data)?;
            Ok(Value::Bool(true))
        }
    }
}

/// `__mailer_deliveries()` — captured mail in test mode (newest last).
fn mailer_deliveries(_args: Vec<Value>) -> Result<Value, String> {
    let list = DELIVERIES.with(|d| d.borrow().clone());
    Ok(Value::Array(Rc::new(RefCell::new(list))))
}

/// `__mailer_clear_deliveries()` — reset the test capture buffer.
fn mailer_clear_deliveries(_args: Vec<Value>) -> Result<Value, String> {
    DELIVERIES.with(|d| d.borrow_mut().clear());
    Ok(Value::Null)
}

/// `__mail_render(mailer_instance, opts_hash)` — resolve the convention view,
/// render it with the mailer's instance variables, and return the rendered
/// mail hash ready for delivery.
fn mail_render(args: Vec<Value>) -> Result<Value, String> {
    let inst = match args.first() {
        Some(Value::Instance(inst)) => inst.clone(),
        _ => return Err("__mail_render must be called with a mailer instance".to_string()),
    };
    let opts = match args.get(1) {
        Some(Value::Hash(h)) => h.borrow().clone(),
        _ => HashPairs::default(),
    };

    // Build the locals hash from the mailer's instance variables (skip the
    // framework-internal `_`-prefixed fields, e.g. `__action`).
    let (class_name, action, data) = {
        let inst_ref = inst.borrow();
        let class_name = inst_ref.class.name.clone();
        let action = inst_ref
            .get("__action")
            .and_then(|v| as_string(&v))
            .unwrap_or_default();
        let mut data: HashPairs = HashPairs::default();
        for (name, value) in inst_ref.fields.iter() {
            if !name.starts_with('_') {
                data.insert(HashKey::String(name.clone().into()), value.clone());
            }
        }
        (class_name, action, data)
    };

    let mailer_snake = class_name_to_snake(&class_name);
    let template = hash_get(&opts, "template")
        .and_then(|v| as_string(&v))
        .unwrap_or_else(|| format!("{mailer_snake}/{action}"));

    let data_value = Value::Hash(Rc::new(RefCell::new(data.clone())));

    // HTML body: explicit `html:` wins, else render the convention view.
    let html = match hash_get(&opts, "html").and_then(|v| as_string(&v)) {
        Some(html) => Some(html),
        None => {
            let cache = crate::interpreter::builtins::template::get_template_cache()?;
            Some(
                cache
                    .render(&template, &data_value, Some(None))
                    .map_err(|e| format!("failed to render mailer view `{template}`: {e}"))?,
            )
        }
    };

    // Text body: explicit `text:` wins, else auto-render a `<template>.text.slv`
    // companion view when present. Best-effort: if the template cache isn't
    // initialized (e.g. a plain script), skip the text part instead of erroring.
    let text = match hash_get(&opts, "text").and_then(|v| as_string(&v)) {
        Some(text) => Some(text),
        None => match crate::interpreter::builtins::template::get_template_cache() {
            Ok(cache) => {
                let text_view = cache.views_dir().join(format!("{template}.text.slv"));
                if text_view.exists() {
                    let text_template = format!("{template}.text");
                    Some(
                        cache
                            .render(&text_template, &data_value, Some(None))
                            .map_err(|e| {
                                format!("failed to render mailer text view `{text_template}`: {e}")
                            })?,
                    )
                } else {
                    None
                }
            }
            Err(_) => None,
        },
    };

    // Assemble the rendered mail hash.
    let cfg_from = MAILER_CONFIG.read().ok().and_then(|c| c.from.clone());
    let from = hash_get(&opts, "from")
        .and_then(|v| as_string(&v))
        .or(cfg_from);

    let mut out: HashPairs = HashPairs::default();
    // Carry recipient/header fields straight through; `to`/`cc`/`bcc` keep
    // their string-or-array shape for as_address_list to normalize at send.
    for key in ["to", "cc", "bcc", "reply_to", "subject", "attachments"] {
        if let Some(v) = opts.get(&HashKey::String(key.into())) {
            if !matches!(v, Value::Null) {
                out.insert(HashKey::String(key.into()), v.clone());
            }
        }
    }
    if let Some(html) = html {
        out.insert(HashKey::String("html".into()), Value::String(html.into()));
    }
    if let Some(text) = text {
        out.insert(HashKey::String("text".into()), Value::String(text.into()));
    }
    out.insert(
        HashKey::String("from".into()),
        from.map(|f| Value::String(f.into())).unwrap_or(Value::Null),
    );

    Ok(Value::Hash(Rc::new(RefCell::new(out))))
}

/// `UserMailer` → `user_mailer`. Shared with the view-path convention.
fn class_name_to_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(ch.to_lowercase());
    }
    out
}

// ---------------------------------------------------------------------------
// Registration + prelude
// ---------------------------------------------------------------------------

/// Signature of a native mailer builtin.
type MailerBuiltin = fn(Vec<Value>) -> Result<Value, String>;

/// Register the native mailer builtins. The user-facing `Mailer`/`Message`
/// classes are defined by [`MAILER_PRELUDE`], loaded into top-level
/// interpreters via [`ensure_prelude`].
pub fn register_mailer_builtins(env: &mut Environment) {
    let defs: [(&str, Option<usize>, MailerBuiltin); 7] = [
        ("__mailer_configure", Some(1), mailer_configure),
        ("__mailer_deliver", Some(1), mailer_deliver),
        ("__mailer_deliveries", Some(0), mailer_deliveries),
        (
            "__mailer_clear_deliveries",
            Some(0),
            mailer_clear_deliveries,
        ),
        ("__mail_render", Some(2), mail_render),
        ("__mail_enqueue", Some(2), mail_enqueue),
        ("__mailer_invoke", Some(3), mail_invoke),
    ];
    for (name, arity, func) in defs {
        env.define(
            name.to_string(),
            Value::NativeFunction(NativeFunction::new(name, arity, func)),
        );
    }
}

/// `__mail_enqueue(mail_hash, queue)` — schedule background delivery.
///
/// In `test`/`logger` mode this delivers synchronously (so tests see it in
/// `Mailer.deliveries()` without standing up a queue). In `smtp` mode it
/// enqueues a `__MailDelivery` job onto the SolidB-backed Job queue; if the
/// queue is unavailable it logs and falls back to sending synchronously so
/// mail is never silently dropped.
pub(crate) fn mail_enqueue(args: Vec<Value>) -> Result<Value, String> {
    let mail = args.first().cloned().unwrap_or(Value::Null);
    let method = MAILER_CONFIG
        .read()
        .map(|c| c.delivery_method)
        .unwrap_or(DeliveryMethod::Smtp);

    if matches!(method, DeliveryMethod::Test | DeliveryMethod::Logger) {
        return mailer_deliver(vec![mail]);
    }

    let mut enqueue_args = vec![Value::String("__MailDelivery".into()), mail.clone()];
    if let Some(queue) = args.get(1) {
        if !matches!(queue, Value::Null) {
            enqueue_args.push(queue.clone());
        }
    }
    match crate::interpreter::builtins::jobs::enqueue(enqueue_args) {
        Ok(id) => Ok(id),
        Err(e) => {
            eprintln!("[mailer] deliver_later: queue unavailable ({e}); sending synchronously");
            mailer_deliver(vec![mail])
        }
    }
}

/// `__mailer_invoke(mailer, action_name, args_array)` — call the mailer's
/// instance method `action_name` with the array's elements as positional
/// arguments, returning the `Message` it builds. This is how
/// `Mailer.method_missing` forwards an arbitrary arity to the action (Soli has
/// no call-site spread). Missing parameters bind to null, so mailer actions
/// should not rely on default parameter values.
pub(crate) fn mail_invoke(args: Vec<Value>) -> Result<Value, String> {
    let inst = match args.first() {
        Some(Value::Instance(i)) => i.clone(),
        _ => return Err("__mailer_invoke expects a mailer instance".to_string()),
    };
    let action = args
        .get(1)
        .and_then(as_string)
        .ok_or_else(|| "__mailer_invoke expects an action name".to_string())?;
    let mut call_args: Vec<Value> = match args.get(2) {
        Some(Value::Array(a)) => a.borrow().clone(),
        _ => Vec::new(),
    };

    let method = inst.borrow().class.find_method(&action);
    let method = method.ok_or_else(|| format!("mailer action '{action}' is not defined"))?;

    while call_args.len() < method.params.len() {
        call_args.push(Value::Null);
    }

    let mut interpreter = crate::interpreter::Interpreter::default();
    interpreter
        .call_function_with_this(&method, Some(Value::Instance(inst.clone())), call_args)
        .map_err(|e| e.to_string())
}

/// The Soli-level `Mailer` and `Message` classes. Loaded once into each
/// top-level interpreter (before any `app/mailers/*.sl`). `mail` is a Soli
/// method so it can accept named arguments (native functions can't).
pub const MAILER_PRELUDE: &str = r#"
class Mailer {
  static def configure(opts) { return __mailer_configure(opts); }
  static def deliver(mail) { return __mailer_deliver(mail); }
  static def deliveries() { return __mailer_deliveries(); }
  static def clear_deliveries() { return __mailer_clear_deliveries(); }

  static def method_missing(action, args) {
    # `args` is the Array of call arguments (Ruby-style class method_missing).
    let mailer = this.new();
    mailer.__action = action;
    return __mailer_invoke(mailer, action, args);
  }

  def mail(to, subject, html = null, text = null, cc = null, bcc = null, reply_to = null, sender = null, attachments = null, template = null) {
    let rendered = __mail_render(this, {
      "to": to, "subject": subject, "html": html, "text": text,
      "cc": cc, "bcc": bcc, "reply_to": reply_to, "from": sender,
      "attachments": attachments, "template": template
    });
    # `Message.new({...})` mass-assigns the hash's keys as fields, so wrap the
    # rendered mail under `data` to land it in a single `@data` field.
    return Message.new({ "data": rendered });
  }
}

class Message {
  def deliver_now() { return __mailer_deliver(@data); }
  def deliver_later(queue = null) { return __mail_enqueue(@data, queue); }
  def to_h() { return @data; }

  # Attach a text payload. Chainable: returns this.
  def attach(filename, content, content_type = null) {
    return this.__add_attachment({
      "filename": filename, "content": content,
      "content_type": content_type ?? "text/plain"
    });
  }

  # Attach binary content provided as a base64 string. Chainable.
  def attach_base64(filename, base64, content_type = null) {
    return this.__add_attachment({
      "filename": filename, "base64": base64,
      "content_type": content_type ?? "application/octet-stream"
    });
  }

  def __add_attachment(att) {
    if @data["attachments"].nil? { @data["attachments"] = []; }
    @data["attachments"].push(att);
    return this;
  }
}

# Background-delivery job target. `deliver_later` enqueues this class; the Job
# queue worker POSTs back to /_jobs/run/__MailDelivery, which calls perform.
class __MailDelivery {
  static def perform(args) { return __mailer_deliver(args); }
}
"#;

/// Define the `Mailer`/`Message` prelude classes in `interpreter` if not
/// already present. Idempotent and cheap to re-call (one env lookup).
pub fn ensure_prelude(interpreter: &mut crate::interpreter::Interpreter) {
    if interpreter.global_env().borrow().get("Mailer").is_some() {
        return;
    }
    let tokens = match crate::lexer::Scanner::new(MAILER_PRELUDE).scan_tokens() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("mailer prelude lex error: {e}");
            return;
        }
    };
    let program = match crate::parser::Parser::new(tokens).parse() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("mailer prelude parse error: {e}");
            return;
        }
    };
    if let Err(e) = interpreter.interpret(&program) {
        eprintln!("mailer prelude execute error: {e}");
        return;
    }

    // Register __MailDelivery in the mode-independent job registry so the
    // /_jobs/run/:name callback can dispatch background mail delivery.
    if let Some(class) = interpreter.global_env().borrow().get("__MailDelivery") {
        crate::interpreter::builtins::jobs::register_job_class_in_registry("__MailDelivery", class);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_display_addresses() {
        assert_eq!(parse_address("a@b.com"), (None, "a@b.com".to_string()));
        assert_eq!(
            parse_address("Alice <alice@example.com>"),
            (Some("Alice".to_string()), "alice@example.com".to_string())
        );
        assert_eq!(envelope_address("Bob <bob@x.io>"), "bob@x.io");
    }

    #[test]
    fn snake_cases_mailer_class_names() {
        assert_eq!(class_name_to_snake("UserMailer"), "user_mailer");
        assert_eq!(
            class_name_to_snake("OrderReceiptMailer"),
            "order_receipt_mailer"
        );
    }

    #[test]
    fn builds_mime_with_html_and_subject() {
        let mut mail: HashPairs = HashPairs::default();
        mail.insert(
            HashKey::String("from".into()),
            Value::String("from@x.io".into()),
        );
        mail.insert(
            HashKey::String("to".into()),
            Value::String("to@x.io".into()),
        );
        mail.insert(
            HashKey::String("subject".into()),
            Value::String("Hi".into()),
        );
        mail.insert(
            HashKey::String("html".into()),
            Value::String("<b>Hi</b>".into()),
        );
        let mime = build_mime(&mail).unwrap();
        assert!(mime.contains("Subject: Hi"));
        assert!(mime.contains("to@x.io"));
    }
}
