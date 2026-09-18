//! IMAP email-reading builtin: the `Imap` client class.
//!
//! A small synchronous IMAP4rev1 client (implicit TLS on port 993 by default),
//! mirroring the `Pop3` client. Unlike POP3, IMAP is stateful — you `select()`
//! a mailbox, then `search()`/`fetch()` within it — so the surface is larger:
//!
//! ```soli
//! mail = Imap.new("imap.gmail.com", "me@gmail.com", "app-password")
//! # or, for a Workspace account where app passwords are switched off:
//! mail = Imap.new("imap.gmail.com", "me@work.com", "", { "xoauth2": token })
//! mail.select("INBOX")
//! for uid in mail.uid_search("UNSEEN")
//!   msg = mail.fetch_uid(uid)
//!   print(msg["subject"])
//! end
//! mail.logout()
//! ```
//!
//! TLS uses the same synchronous rustls-over-`TcpStream` stack as `pop3.rs`
//! (via [`crate::interpreter::builtins::pop3::connect`]), and fetched messages
//! are parsed by the shared `mail_parse` module. The live connection lives in a
//! process-global registry keyed by an integer `_id` on the instance — the same
//! pattern used by the `Pop3` / `Solidb` builtin classes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use base64::Engine as _;
use lazy_static::lazy_static;

use crate::interpreter::builtins::mail_parse;
use crate::interpreter::builtins::pop3::{connect, Stream};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{hash_from_pairs, Class, HashKey, Instance, NativeFunction, Value};

/// Default cap on messages downloaded by `.fetch_all()`; override with
/// `SOLI_IMAP_MAX_MESSAGES`.
const DEFAULT_MAX_MESSAGES: i64 = 200;

/// A live, authenticated IMAP connection.
struct ImapConn {
    reader: BufReader<Box<dyn Stream>>,
    /// Monotonic command tag counter (`a0001`, `a0002`, …).
    tag: u32,
    /// `EXISTS` count from the most recent `SELECT`, used to bound `fetch_all`.
    selected_exists: Option<i64>,
}

lazy_static! {
    /// Process-global registry of open connections, keyed by instance id.
    static ref IMAP_CONNS: Mutex<HashMap<usize, ImapConn>> = Mutex::new(HashMap::new());
}
static IMAP_NEXT_ID: AtomicUsize = AtomicUsize::new(1);

// ---------------------------------------------------------------------------
// Response model
// ---------------------------------------------------------------------------

/// One fragment of an IMAP response line. Literal payloads (`{N}` octets) are
/// captured as raw bytes so message bodies — which may contain `)`, `{...}` or
/// CRLF — are never confused with the surrounding protocol text.
enum Piece {
    Text(String),
    Literal(Vec<u8>),
}

/// Largest IMAP literal (`{N}`) accepted from a server.
///
/// The size is server-supplied and was allocated up front, so a hostile server
/// naming a huge literal aborted the worker on the allocation before a single
/// byte arrived. 32 MiB is far beyond any real message part.
/// `SOLI_IMAP_MAX_LITERAL_BYTES` overrides it.
fn max_literal_bytes() -> u64 {
    std::env::var("SOLI_IMAP_MAX_LITERAL_BYTES")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(32 * 1024 * 1024)
}

/// If a response line ends with an IMAP literal marker `{N}` or `{N+}`
/// (non-synchronizing), return N.
fn trailing_literal_size(line: &str) -> Option<usize> {
    let stripped = line.strip_suffix('}')?;
    let open = stripped.rfind('{')?;
    let inner = &stripped[open + 1..];
    let inner = inner.strip_suffix('+').unwrap_or(inner);
    inner.parse::<usize>().ok()
}

/// The leading Text fragment of a response (empty if the line starts with a
/// literal, which never happens in practice).
fn first_text(pieces: &[Piece]) -> &str {
    match pieces.first() {
        Some(Piece::Text(t)) => t.as_str(),
        _ => "",
    }
}

/// Concatenate all Text fragments (dropping literals) — the metadata portion of
/// a response such as a FETCH.
fn joined_text(pieces: &[Piece]) -> String {
    let mut out = String::new();
    for p in pieces {
        if let Piece::Text(t) = p {
            out.push_str(t);
        }
    }
    out
}

/// The first literal payload of a response, if any (a FETCH `BODY[]` body).
fn first_literal(pieces: &[Piece]) -> Option<&[u8]> {
    pieces.iter().find_map(|p| match p {
        Piece::Literal(b) => Some(b.as_slice()),
        _ => None,
    })
}

impl ImapConn {
    fn next_tag(&mut self) -> String {
        self.tag += 1;
        format!("a{:04}", self.tag)
    }

    /// Write a command line (CRLF-terminated) and flush.
    fn send(&mut self, line: &str) -> Result<(), String> {
        let writer = self.reader.get_mut();
        writer
            .write_all(line.as_bytes())
            .and_then(|_| writer.write_all(b"\r\n"))
            .and_then(|_| writer.flush())
            .map_err(|e| format!("IMAP write error: {e}"))
    }

    /// Read one logical response, expanding any trailing `{N}` literals so the
    /// returned pieces cover a complete server response line.
    fn read_pieces(&mut self) -> Result<Vec<Piece>, String> {
        let mut pieces = Vec::new();
        loop {
            let mut line = String::new();
            let n = self
                .reader
                .read_line(&mut line)
                .map_err(|e| format!("IMAP read error: {e}"))?;
            if n == 0 {
                return Err("IMAP connection closed by server".to_string());
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if let Some(size) = trailing_literal_size(trimmed) {
                // The size comes from the server, and `vec![0u8; size]` commits
                // it all up front — a hostile or MITM'd mailbox (or simply a
                // user-configured `Imap.new(host)`) could name a size near
                // `usize::MAX` and abort the worker on the allocation.
                if size as u64 > max_literal_bytes() {
                    return Err(format!(
                        "IMAP literal of {size} bytes exceeds the {} byte limit \
                         (raise SOLI_IMAP_MAX_LITERAL_BYTES if this is expected)",
                        max_literal_bytes()
                    ));
                }
                let brace = trimmed.rfind('{').unwrap();
                pieces.push(Piece::Text(trimmed[..brace].to_string()));
                let mut data = vec![0u8; size];
                self.reader
                    .read_exact(&mut data)
                    .map_err(|e| format!("IMAP literal read error: {e}"))?;
                pieces.push(Piece::Literal(data));
                // The response continues after the literal — keep reading.
            } else {
                pieces.push(Piece::Text(trimmed.to_string()));
                break;
            }
        }
        Ok(pieces)
    }

    /// Send a tagged command and collect the untagged responses that precede its
    /// tagged completion line. Returns an error for a `NO`/`BAD` completion.
    fn command(&mut self, cmd: &str) -> Result<Vec<Vec<Piece>>, String> {
        let tag = self.next_tag();
        self.send(&format!("{tag} {cmd}"))?;
        self.read_until_tagged(&tag)
    }

    /// `AUTHENTICATE XOAUTH2`, which is the only way into a Google
    /// Workspace mailbox.
    ///
    /// An administrator can switch app passwords off for a whole domain,
    /// and by default now does: `LOGIN` then has no credential it will
    /// accept, and the account is simply unreachable over IMAP without
    /// this. The token is an OAuth access token, minted from a refresh
    /// token by whoever calls us; it is short-lived by design and this
    /// never sees the long-lived one.
    ///
    /// A refusal does not arrive as a tagged `NO` the way every other
    /// command's does. The server sends a continuation -- `+` followed by
    /// base64 JSON describing what was wrong -- and then waits for the
    /// client to acknowledge it with an empty line before it will say
    /// `NO`. A client that does not answer sits there until the socket
    /// times out, so the `+` is answered here rather than left to
    /// `read_until_tagged`, which would treat it as untagged chatter and
    /// block.
    fn authenticate_xoauth2(&mut self, user: &str, token: &str) -> Result<(), String> {
        let raw = format!("user={user}\u{1}auth=Bearer {token}\u{1}\u{1}");
        let initial = base64::engine::general_purpose::STANDARD.encode(raw);
        let tag = self.next_tag();
        self.send(&format!("{tag} AUTHENTICATE XOAUTH2 {initial}"))?;
        // Two continuations would mean a server that is not answering the
        // question; one is the error, and the bound keeps a hostile or
        // broken peer from holding this loop open.
        let mut challenges = 0;
        loop {
            let pieces = self.read_pieces()?;
            let head = first_text(&pieces);
            if head.starts_with('+') {
                challenges += 1;
                if challenges > 1 {
                    return Err("XOAUTH2: the server kept asking".to_string());
                }
                self.send("")?;
                continue;
            }
            if let Some(rest) = head.strip_prefix(&tag).and_then(|r| r.strip_prefix(' ')) {
                let mut it = rest.splitn(2, ' ');
                let status = it.next().unwrap_or("");
                let text = it.next().unwrap_or("");
                return match status {
                    "OK" => Ok(()),
                    _ => Err(format!("XOAUTH2 refused ({status}): {text}")),
                };
            }
        }
    }

    fn read_until_tagged(&mut self, tag: &str) -> Result<Vec<Vec<Piece>>, String> {
        let mut untagged = Vec::new();
        loop {
            let pieces = self.read_pieces()?;
            let head = first_text(&pieces);
            if let Some(rest) = head.strip_prefix(tag).and_then(|r| r.strip_prefix(' ')) {
                let mut it = rest.splitn(2, ' ');
                let status = it.next().unwrap_or("");
                let text = it.next().unwrap_or("");
                return match status {
                    "OK" => Ok(untagged),
                    _ => Err(format!("IMAP command failed ({status}): {text}")),
                };
            } else if head.starts_with('+') {
                return Err(format!("IMAP unexpected continuation request: {head}"));
            } else {
                untagged.push(pieces);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Quoting / argument validation
// ---------------------------------------------------------------------------

/// Validate a credential/host/mailbox string and reject CR/LF (command
/// injection).
fn as_string(v: &Value, field: &str) -> Result<String, String> {
    match v {
        Value::String(s) => {
            if s.contains('\r') || s.contains('\n') {
                Err(format!("Imap {field} must not contain CR/LF characters"))
            } else {
                Ok(s.clone().to_string())
            }
        }
        other => Err(format!(
            "Imap expected a string {field}, got {}",
            other.type_name()
        )),
    }
}

/// IMAP quoted-string: wrap in double quotes, backslash-escaping `\` and `"`.
/// CR/LF are rejected upstream by [`as_string`].
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '\\' || c == '"' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// Registry helpers
// ---------------------------------------------------------------------------

fn instance_id(args: &[Value], method: &str) -> Result<usize, String> {
    let inst = match args.first() {
        Some(Value::Instance(inst)) => inst,
        _ => {
            return Err(format!(
                "Imap.{method}() must be called on an Imap instance"
            ))
        }
    };
    match inst.borrow().get("_id") {
        Some(Value::Int(id)) => Ok(id as usize),
        _ => Err("Imap instance has no open connection (already logged out?)".to_string()),
    }
}

fn with_conn<R>(
    id: usize,
    f: impl FnOnce(&mut ImapConn) -> Result<R, String>,
) -> Result<R, String> {
    let mut conns = IMAP_CONNS.lock().map_err(|e| e.to_string())?;
    let conn = conns
        .get_mut(&id)
        .ok_or_else(|| "Imap connection is closed (call .logout() only once)".to_string())?;
    f(conn)
}

fn message_id_arg(args: &[Value], method: &str, label: &str) -> Result<i64, String> {
    match args.get(1) {
        Some(Value::Int(n)) if *n >= 1 => Ok(*n),
        _ => Err(format!("Imap.{method}({label}) expects a positive integer")),
    }
}

fn mailbox_arg(args: &[Value], method: &str) -> Result<String, String> {
    match args.get(2) {
        Some(v @ Value::String(_)) => as_string(v, "mailbox"),
        _ => Err(format!(
            "Imap.{method}(id, mailbox) expects a string mailbox name"
        )),
    }
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

/// Read an integer out of an IMAP bracketed response code, e.g. the `12` in
/// `OK [UNSEEN 12] Message 12 is first unseen`.
fn bracket_int(s: &str, key: &str) -> Option<i64> {
    let needle = format!("[{key} ");
    let start = s.find(&needle)? + needle.len();
    let rest = &s[start..];
    let end = rest.find(']')?;
    rest[..end].trim().parse::<i64>().ok()
}

/// Parse the untagged responses of a `SELECT`/`EXAMINE` into a status hash and
/// return the `EXISTS` count separately (for the connection's fetch bound).
fn parse_select(mailbox: &str, untagged: &[Vec<Piece>]) -> (Value, Option<i64>) {
    let mut exists: Option<i64> = None;
    let mut recent: i64 = 0;
    let mut unseen = Value::Null;
    let mut uidvalidity = Value::Null;
    let mut uidnext = Value::Null;
    let mut flags: Vec<Value> = Vec::new();

    for pieces in untagged {
        let line = joined_text(pieces);
        let body = line.strip_prefix("* ").unwrap_or(&line).trim();
        if let Some(n) = body
            .strip_suffix(" EXISTS")
            .and_then(|s| s.trim().parse::<i64>().ok())
        {
            exists = Some(n);
        } else if let Some(n) = body
            .strip_suffix(" RECENT")
            .and_then(|s| s.trim().parse::<i64>().ok())
        {
            recent = n;
        } else if let Some(rest) = body.strip_prefix("FLAGS (") {
            if let Some(inner) = rest.strip_suffix(')') {
                for f in inner.split_whitespace() {
                    flags.push(Value::String(f.to_string().into()));
                }
            }
        } else if body.starts_with("OK [") {
            if let Some(v) = bracket_int(body, "UNSEEN") {
                unseen = Value::Int(v);
            }
            if let Some(v) = bracket_int(body, "UIDVALIDITY") {
                uidvalidity = Value::Int(v);
            }
            if let Some(v) = bracket_int(body, "UIDNEXT") {
                uidnext = Value::Int(v);
            }
        }
    }

    let hash = hash_from_pairs(vec![
        (
            "mailbox".to_string(),
            Value::String(mailbox.to_string().into()),
        ),
        ("exists".to_string(), Value::Int(exists.unwrap_or(0))),
        ("recent".to_string(), Value::Int(recent)),
        ("unseen".to_string(), unseen),
        ("uidvalidity".to_string(), uidvalidity),
        ("uidnext".to_string(), uidnext),
        (
            "flags".to_string(),
            Value::Array(Rc::new(RefCell::new(flags))),
        ),
    ]);
    (hash, exists)
}

/// Split an IMAP atom or quoted-string off the front of `s`, returning the
/// decoded token and the remainder. Handles backslash escapes inside quotes.
fn parse_atom_or_quoted(s: &str) -> (String, &str) {
    let s = s.trim_start();
    if let Some(rest) = s.strip_prefix('"') {
        let bytes = rest.as_bytes();
        let mut out = String::new();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i] as char;
            if c == '\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1] as char);
                i += 2;
            } else if c == '"' {
                return (out, &rest[i + 1..]);
            } else {
                out.push(c);
                i += 1;
            }
        }
        (out, "")
    } else {
        match s.find(char::is_whitespace) {
            Some(idx) => (s[..idx].to_string(), &s[idx..]),
            None => (s.to_string(), ""),
        }
    }
}

/// Parse a single `LIST (flags) delimiter name` response into `{name,
/// delimiter, flags}`.
fn parse_list_line(s: &str) -> Option<Value> {
    let s = s.trim();
    let flags_end = s.find(')')?;
    let flags_str = s.get(1..flags_end)?; // skip the leading '('
    let flags: Vec<Value> = flags_str
        .split_whitespace()
        .map(|f| Value::String(f.to_string().into()))
        .collect();
    let after = s[flags_end + 1..].trim_start();
    let (delim, rest) = parse_atom_or_quoted(after);
    let (name, _) = parse_atom_or_quoted(rest);
    let delimiter = if delim.eq_ignore_ascii_case("NIL") {
        Value::Null
    } else {
        Value::String(delim.into())
    };
    Some(hash_from_pairs(vec![
        ("name".to_string(), Value::String(name.into())),
        ("delimiter".to_string(), delimiter),
        (
            "flags".to_string(),
            Value::Array(Rc::new(RefCell::new(flags))),
        ),
    ]))
}

/// Collect the message numbers from `* SEARCH n n n` responses.
fn parse_search(untagged: &[Vec<Piece>]) -> Value {
    let mut out = Vec::new();
    for pieces in untagged {
        let line = joined_text(pieces);
        let body = line.strip_prefix("* ").unwrap_or(&line);
        if let Some(rest) = body.strip_prefix("SEARCH") {
            for tok in rest.split_whitespace() {
                if let Ok(n) = tok.parse::<i64>() {
                    out.push(Value::Int(n));
                }
            }
        }
    }
    Value::Array(Rc::new(RefCell::new(out)))
}

/// Is this untagged response a `<seq> FETCH (...)` line?
fn is_fetch_response(pieces: &[Piece]) -> bool {
    let head = first_text(pieces);
    let body = head.strip_prefix("* ").unwrap_or(head);
    let mut it = body.split_whitespace();
    matches!(
        (it.next().map(|s| s.parse::<i64>().is_ok()), it.next()),
        (Some(true), Some(kw)) if kw.eq_ignore_ascii_case("FETCH")
    )
}

/// Scan `UID <n>` out of a FETCH metadata line.
fn scan_uid(meta: &str) -> Value {
    if let Some(pos) = meta.find("UID ") {
        let digits: String = meta[pos + 4..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = digits.parse::<i64>() {
            return Value::Int(n);
        }
    }
    Value::Null
}

/// Scan `FLAGS (...)` out of a FETCH metadata line into an array of strings.
fn scan_flags(meta: &str) -> Value {
    if let Some(pos) = meta.find("FLAGS (") {
        let rest = &meta[pos + "FLAGS (".len()..];
        if let Some(end) = rest.find(')') {
            let flags: Vec<Value> = rest[..end]
                .split_whitespace()
                .map(|f| Value::String(f.to_string().into()))
                .collect();
            return Value::Array(Rc::new(RefCell::new(flags)));
        }
    }
    Value::Array(Rc::new(RefCell::new(Vec::new())))
}

/// Parse one FETCH response (metadata text + BODY[] literal) into a message
/// hash: `seq, uid, flags` followed by the shared parsed fields.
/// `RFC822.SIZE n` out of a FETCH response's metadata, or `Null`.
fn scan_size(body: &str) -> Value {
    let Some(at) = body.find("RFC822.SIZE ") else {
        return Value::Null;
    };
    let rest = &body[at + "RFC822.SIZE ".len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse::<i64>().map(Value::Int).unwrap_or(Value::Null)
}

/// How many parts of a `BODYSTRUCTURE` say they are attachments.
///
/// A count, not a parse. `BODYSTRUCTURE` is a nested parenthesised
/// structure and reading it properly is a parser; what a list row wants is
/// "does this have a paper clip on it, and how many", and every attached
/// part carries a disposition of `"attachment"` — so the number of times
/// that word appears quoted *is* the answer, by construction. A filename
/// that contains it would inflate the count by one; nothing in a row's
/// meaning breaks if it does.
fn count_attachments(body: &str) -> i64 {
    let said = body.to_ascii_lowercase();
    said.matches("\"attachment\"").count() as i64
}

fn parse_fetch_pieces(pieces: &[Piece]) -> Option<Value> {
    let meta = joined_text(pieces);
    let body = meta.strip_prefix("* ").unwrap_or(&meta);
    let seq = body
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<i64>().ok())?;
    let raw = first_literal(pieces).unwrap_or(&[]);
    let mut pairs: Vec<(String, Value)> = vec![
        ("seq".to_string(), Value::Int(seq)),
        ("uid".to_string(), scan_uid(body)),
        ("flags".to_string(), scan_flags(body)),
        ("bytes".to_string(), scan_size(body)),
        ("clips".to_string(), Value::Int(count_attachments(body))),
    ];
    pairs.extend(mail_parse::common_fields(raw));
    Some(hash_from_pairs(pairs))
}

fn parse_fetch_one(untagged: &[Vec<Piece>]) -> Option<Value> {
    untagged
        .iter()
        .find(|p| is_fetch_response(p))
        .and_then(|p| parse_fetch_pieces(p))
}

// ---------------------------------------------------------------------------
// Constructor + instance methods
// ---------------------------------------------------------------------------

fn imap_new(class: Rc<Class>, args: &[Value]) -> Result<Value, String> {
    if args.len() < 3 || args.len() > 4 {
        return Err(format!(
            "Imap.new(host, user, password, opts?) expects 3 or 4 arguments, got {}",
            args.len()
        ));
    }
    let host = as_string(&args[0], "host")?;
    let user = as_string(&args[1], "user")?;
    let pass = as_string(&args[2], "password")?;

    let mut port: u16 = 993;
    let mut use_tls = true;
    let mut token: Option<String> = None;
    match args.get(3) {
        None | Some(Value::Null) => {}
        Some(Value::Hash(opts)) => {
            let opts = opts.borrow();
            if let Some(Value::Int(p)) = opts.get(&HashKey::String("port".into())) {
                if *p < 1 || *p > 65535 {
                    return Err(format!("Imap.new() opts.port {p} out of range 1..65535"));
                }
                port = *p as u16;
            }
            if let Some(Value::Bool(b)) = opts.get(&HashKey::String("tls".into())) {
                use_tls = *b;
            }
            // An OAuth access token instead of a password. Workspace
            // domains usually have no other way in.
            if let Some(v) = opts.get(&HashKey::String("xoauth2".into())) {
                let t = as_string(v, "opts.xoauth2")?;
                if !t.is_empty() {
                    token = Some(t);
                }
            }
        }
        Some(other) => {
            return Err(format!(
                "Imap.new() opts must be a Hash, got {}",
                other.type_name()
            ))
        }
    }

    let stream = connect(&host, port, use_tls)?;
    let mut conn = ImapConn {
        reader: BufReader::new(stream),
        tag: 0,
        selected_exists: None,
    };

    // Server greeting: `* OK ...` (or `* PREAUTH ...`, already authenticated).
    let greeting = conn.read_pieces()?;
    let head = first_text(&greeting);
    let preauth = head.starts_with("* PREAUTH");
    if !head.starts_with("* OK") && !preauth {
        return Err(format!("IMAP greeting failed: {head}"));
    }
    if !preauth {
        match &token {
            Some(t) => conn
                .authenticate_xoauth2(&user, t)
                .map_err(|e| format!("IMAP authentication failed: {e}"))?,
            None => {
                conn.command(&format!("LOGIN {} {}", quote(&user), quote(&pass)))
                    .map_err(|e| format!("IMAP authentication failed: {e}"))?;
            }
        }
    }

    let id = IMAP_NEXT_ID.fetch_add(1, Ordering::SeqCst);
    IMAP_CONNS
        .lock()
        .map_err(|e| e.to_string())?
        .insert(id, conn);

    let mut inst = Instance::new(class);
    inst.set("_id", Value::Int(id as i64));
    Ok(Value::Instance(Rc::new(RefCell::new(inst))))
}

fn imap_select(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "select")?;
    let mailbox = match args.get(1) {
        None | Some(Value::Null) => "INBOX".to_string(),
        Some(v) => as_string(v, "mailbox")?,
    };
    with_conn(id, |c| {
        let untagged = c.command(&format!("SELECT {}", quote(&mailbox)))?;
        let (info, exists) = parse_select(&mailbox, &untagged);
        c.selected_exists = exists;
        Ok(info)
    })
}

fn imap_mailboxes(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "mailboxes")?;
    let untagged = with_conn(id, |c| c.command("LIST \"\" \"*\""))?;
    let mut out = Vec::new();
    for pieces in &untagged {
        let line = joined_text(pieces);
        let body = line.strip_prefix("* ").unwrap_or(&line);
        if let Some(rest) = body.strip_prefix("LIST ") {
            if let Some(m) = parse_list_line(rest) {
                out.push(m);
            }
        }
    }
    Ok(Value::Array(Rc::new(RefCell::new(out))))
}

/// Extract and validate the optional search criteria argument (default `ALL`).
fn search_criteria(args: &[Value]) -> Result<String, String> {
    match args.get(1) {
        None | Some(Value::Null) => Ok("ALL".to_string()),
        Some(Value::String(s)) => {
            if s.contains('\r') || s.contains('\n') {
                return Err("Imap search criteria must not contain CR/LF characters".to_string());
            }
            let trimmed = s.trim();
            Ok(if trimmed.is_empty() {
                "ALL".to_string()
            } else {
                trimmed.to_string()
            })
        }
        Some(other) => Err(format!(
            "Imap.search(criteria?) expects a string, got {}",
            other.type_name()
        )),
    }
}

fn imap_search(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "search")?;
    let criteria = search_criteria(args)?;
    let untagged = with_conn(id, |c| c.command(&format!("SEARCH {criteria}")))?;
    Ok(parse_search(&untagged))
}

fn imap_uid_search(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "uid_search")?;
    let criteria = search_criteria(args)?;
    let untagged = with_conn(id, |c| c.command(&format!("UID SEARCH {criteria}")))?;
    Ok(parse_search(&untagged))
}

fn imap_fetch(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "fetch")?;
    let seq = message_id_arg(args, "fetch", "seq")?;
    let untagged = with_conn(id, |c| {
        c.command(&format!("FETCH {seq} (UID FLAGS BODY.PEEK[])"))
    })?;
    parse_fetch_one(&untagged).ok_or_else(|| format!("Imap.fetch({seq}): no such message"))
}

fn imap_fetch_uid(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "fetch_uid")?;
    let uid = message_id_arg(args, "fetch_uid", "uid")?;
    let untagged = with_conn(id, |c| {
        c.command(&format!("UID FETCH {uid} (UID FLAGS BODY.PEEK[])"))
    })?;
    parse_fetch_one(&untagged).ok_or_else(|| format!("Imap.fetch_uid({uid}): no such message"))
}

/// The fields a list needs, and nothing else.
///
/// `fetch`/`fetch_uid` ask for `BODY.PEEK[]`, which is the whole message:
/// every part, every attachment. That is right when you are about to
/// *read* one and ruinous when you are drawing a list of twenty, where
/// nothing but the sender, the subject and the date is ever shown -- a
/// modest inbox costs megabytes and seconds to list, and the window sits
/// still for all of it.
///
/// `HEADER.FIELDS` asks the server for four lines instead, so a list is
/// kilobytes. What comes back is a valid RFC822 fragment, so the same
/// parser reads it: `text_body` and `html_body` are simply absent, and
/// the caller fetches those when someone opens the message.
/// What a list row needs, in one item list.
///
/// `RFC822.SIZE` is the message's size, which a headers-only fetch cannot
/// otherwise know — `size` in the parsed hash is the length of what came
/// back, which for this fetch is the header block and nothing like the
/// message. `BODYSTRUCTURE` is how many attachments there are without
/// downloading any of them.
///
/// `BODY.PEEK[...]` stays **first**, so the header block is the first
/// literal in the response whatever else follows it.
const HEADER_ITEMS: &str =
    "(UID FLAGS BODY.PEEK[HEADER.FIELDS (SUBJECT FROM TO DATE)] RFC822.SIZE BODYSTRUCTURE)";

fn imap_fetch_headers(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "fetch_headers")?;
    let seq = message_id_arg(args, "fetch_headers", "seq")?;
    let untagged = with_conn(id, |c| c.command(&format!("FETCH {seq} {HEADER_ITEMS}")))?;
    parse_fetch_one(&untagged).ok_or_else(|| format!("Imap.fetch_headers({seq}): no such message"))
}

fn imap_fetch_headers_uid(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "fetch_headers_uid")?;
    let uid = message_id_arg(args, "fetch_headers_uid", "uid")?;
    let untagged = with_conn(id, |c| {
        c.command(&format!("UID FETCH {uid} {HEADER_ITEMS}"))
    })?;
    parse_fetch_one(&untagged)
        .ok_or_else(|| format!("Imap.fetch_headers_uid({uid}): no such message"))
}

/// A run of messages' headers in one round trip.
///
/// The saving here is not the bytes -- it is the turns. Twenty sequence
/// numbers fetched one at a time is twenty commands and twenty waits on a
/// link to Google; `FETCH lo:hi` is one of each.
fn imap_fetch_headers_range(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "fetch_headers_range")?;
    if args.len() < 3 {
        return Err("Imap.fetch_headers_range(lo, hi) expects two arguments".to_string());
    }
    let lo = message_id_arg(&args[..2], "fetch_headers_range", "lo")?;
    let hi = {
        let shifted = [args[0].clone(), args[2].clone()];
        message_id_arg(&shifted, "fetch_headers_range", "hi")?
    };
    if hi < lo {
        return Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))));
    }
    let untagged = with_conn(id, |c| {
        c.command(&format!("FETCH {lo}:{hi} {HEADER_ITEMS}"))
    })?;
    let mut out = Vec::new();
    for pieces in &untagged {
        if is_fetch_response(pieces) {
            if let Some(msg) = parse_fetch_pieces(pieces) {
                out.push(msg);
            }
        }
    }
    Ok(Value::Array(Rc::new(RefCell::new(out))))
}

/// A **set** of UIDs' headers, in one command.
///
/// `fetch_headers_uid` answers one message and costs one round trip, which
/// is the right shape for opening a letter and the wrong one for catching
/// up: twenty new messages meant twenty commands, and a client that does
/// them inside one event holds that event open for all twenty. IMAP has
/// said `UID FETCH 100:*` and `UID FETCH 1,5,9` since the beginning; this
/// is that.
///
/// The set is validated rather than interpolated: digits, `,`, `:` and `*`
/// are the whole grammar (RFC 3501 sequence-set), and nothing else reaches
/// the wire — an argument that could carry a space could carry a second
/// command.
fn imap_fetch_headers_set(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "fetch_headers_set")?;
    let set = match args.get(1) {
        Some(Value::String(s)) => s.to_string(),
        _ => return Err("Imap.fetch_headers_set(set) expects a string".to_string()),
    };
    let set = set.trim().to_string();
    if set.is_empty() {
        return Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))));
    }
    if !set
        .bytes()
        .all(|b| b.is_ascii_digit() || b == b',' || b == b':' || b == b'*')
    {
        return Err(format!(
            "Imap.fetch_headers_set({set:?}): a sequence set is digits, ',', ':' and '*'"
        ));
    }
    let untagged = with_conn(id, |c| {
        c.command(&format!("UID FETCH {set} {HEADER_ITEMS}"))
    })?;
    let mut out = Vec::new();
    for pieces in &untagged {
        if is_fetch_response(pieces) {
            if let Some(msg) = parse_fetch_pieces(pieces) {
                out.push(msg);
            }
        }
    }
    Ok(Value::Array(Rc::new(RefCell::new(out))))
}

fn imap_fetch_all(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "fetch_all")?;
    let untagged = with_conn(id, |c| {
        let exists = c.selected_exists.ok_or_else(|| {
            "Imap.fetch_all(): no mailbox selected — call select() first".to_string()
        })?;
        if exists <= 0 {
            return Ok(Vec::new());
        }
        let cap = std::env::var("SOLI_IMAP_MAX_MESSAGES")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_MAX_MESSAGES);
        let fetch_count = exists.min(cap);
        if exists > cap {
            eprintln!(
                "[imap] fetch_all: mailbox has {exists} messages; fetching first {cap} \
                 (raise SOLI_IMAP_MAX_MESSAGES to fetch more)"
            );
        }
        c.command(&format!("FETCH 1:{fetch_count} (UID FLAGS BODY.PEEK[])"))
    })?;

    let mut out = Vec::new();
    for pieces in &untagged {
        if is_fetch_response(pieces) {
            if let Some(msg) = parse_fetch_pieces(pieces) {
                out.push(msg);
            }
        }
    }
    Ok(Value::Array(Rc::new(RefCell::new(out))))
}

/// Shared `STORE <seq> ±FLAGS (<flag>)` helper for the flag-mutating methods.
fn store_flag(args: &[Value], method: &str, op: char, flag: &str) -> Result<Value, String> {
    let id = instance_id(args, method)?;
    let seq = message_id_arg(args, method, "seq")?;
    with_conn(id, |c| {
        c.command(&format!("STORE {seq} {op}FLAGS ({flag})"))
    })?;
    Ok(Value::Bool(true))
}

/// The same, addressed by UID.
///
/// Every mutating verb above takes a **sequence number**, which is a
/// position and moves whenever anything before it is removed. A caller
/// holding a UID -- which is what a client that stores messages actually
/// has -- therefore had to turn it into a position first, with a
/// `SEARCH UID n`, and that search is a whole round trip: measured against
/// Gmail, ~200 ms, which is ~200 ms in which the application answers
/// nothing. `UID MOVE` and `UID STORE` are the same operations addressed
/// the way the caller already knows how, in one turn instead of two.
fn uid_store_flag(args: &[Value], method: &str, op: char, flag: &str) -> Result<Value, String> {
    let id = instance_id(args, method)?;
    let uid = message_id_arg(args, method, "uid")?;
    with_conn(id, |c| {
        c.command(&format!("UID STORE {uid} {op}FLAGS ({flag})"))
    })?;
    Ok(Value::Bool(true))
}

fn imap_uid_mark_seen(args: &[Value]) -> Result<Value, String> {
    uid_store_flag(args, "uid_mark_seen", '+', "\\Seen")
}

fn imap_uid_mark_unseen(args: &[Value]) -> Result<Value, String> {
    uid_store_flag(args, "uid_mark_unseen", '-', "\\Seen")
}

fn imap_uid_delete(args: &[Value]) -> Result<Value, String> {
    uid_store_flag(args, "uid_delete", '+', "\\Deleted")
}

fn imap_uid_move(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "uid_move")?;
    let uid = message_id_arg(args, "uid_move", "uid")?;
    let mailbox = mailbox_arg(args, "uid_move")?;
    with_conn(id, |c| {
        c.command(&format!("UID MOVE {uid} {}", quote(&mailbox)))
    })?;
    Ok(Value::Bool(true))
}

fn imap_uid_copy(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "uid_copy")?;
    let uid = message_id_arg(args, "uid_copy", "uid")?;
    let mailbox = mailbox_arg(args, "uid_copy")?;
    with_conn(id, |c| {
        c.command(&format!("UID COPY {uid} {}", quote(&mailbox)))
    })?;
    Ok(Value::Bool(true))
}

fn imap_mark_seen(args: &[Value]) -> Result<Value, String> {
    store_flag(args, "mark_seen", '+', "\\Seen")
}

fn imap_mark_unseen(args: &[Value]) -> Result<Value, String> {
    store_flag(args, "mark_unseen", '-', "\\Seen")
}

fn imap_delete(args: &[Value]) -> Result<Value, String> {
    store_flag(args, "delete", '+', "\\Deleted")
}

fn imap_expunge(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "expunge")?;
    with_conn(id, |c| c.command("EXPUNGE"))?;
    Ok(Value::Bool(true))
}

fn imap_copy(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "copy")?;
    let seq = message_id_arg(args, "copy", "seq")?;
    let mailbox = mailbox_arg(args, "copy")?;
    with_conn(id, |c| {
        c.command(&format!("COPY {seq} {}", quote(&mailbox)))
    })?;
    Ok(Value::Bool(true))
}

fn imap_move(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "move")?;
    let seq = message_id_arg(args, "move", "seq")?;
    let mailbox = mailbox_arg(args, "move")?;
    // Uses the RFC 6851 MOVE extension (supported by Gmail, Dovecot, …). Servers
    // without it return NO/BAD, surfaced as an error to the caller.
    with_conn(id, |c| {
        c.command(&format!("MOVE {seq} {}", quote(&mailbox)))
    })?;
    Ok(Value::Bool(true))
}

fn imap_logout(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "logout")?;
    let mut conns = IMAP_CONNS.lock().map_err(|e| e.to_string())?;
    if let Some(mut conn) = conns.remove(&id) {
        let _ = conn.command("LOGOUT");
    }
    Ok(Value::Bool(true))
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

fn method(
    name: &'static str,
    arity: Option<usize>,
    f: fn(&[Value]) -> Result<Value, String>,
) -> (String, Rc<NativeFunction>) {
    (
        name.to_string(),
        Rc::new(NativeFunction::new(format!("Imap.{name}"), arity, f)),
    )
}

/// Register the `Imap` builtin class into `env`.
pub fn register_imap_class(env: &mut Environment) {
    let native_methods: HashMap<String, Rc<NativeFunction>> = [
        method("select", None, imap_select),
        method("mailboxes", Some(0), imap_mailboxes),
        method("search", None, imap_search),
        method("uid_search", None, imap_uid_search),
        method("fetch", Some(1), imap_fetch),
        method("fetch_uid", Some(1), imap_fetch_uid),
        method("fetch_all", Some(0), imap_fetch_all),
        method("fetch_headers", Some(1), imap_fetch_headers),
        method("fetch_headers_uid", Some(1), imap_fetch_headers_uid),
        method("fetch_headers_range", Some(2), imap_fetch_headers_range),
        method("fetch_headers_set", Some(1), imap_fetch_headers_set),
        method("mark_seen", Some(1), imap_mark_seen),
        method("uid_mark_seen", Some(1), imap_uid_mark_seen),
        method("uid_mark_unseen", Some(1), imap_uid_mark_unseen),
        method("uid_delete", Some(1), imap_uid_delete),
        method("uid_move", Some(2), imap_uid_move),
        method("uid_copy", Some(2), imap_uid_copy),
        method("mark_unseen", Some(1), imap_mark_unseen),
        method("delete", Some(1), imap_delete),
        method("expunge", Some(0), imap_expunge),
        method("copy", Some(2), imap_copy),
        method("move", Some(2), imap_move),
        method("logout", Some(0), imap_logout),
    ]
    .into_iter()
    .collect();

    // `new` needs the class Rc to build instances, but the class embeds the
    // method — break the cycle with a Weak upgraded at call time.
    let imap_class = Rc::new_cyclic(|weak: &Weak<Class>| {
        let weak = weak.clone();
        let mut native_static: HashMap<String, Rc<NativeFunction>> = HashMap::new();
        native_static.insert(
            "new".to_string(),
            Rc::new(NativeFunction::new("Imap.new", None, move |args| {
                let class = weak
                    .upgrade()
                    .ok_or_else(|| "Imap class was dropped".to_string())?;
                imap_new(class, args)
            })),
        );
        Class {
            name: "Imap".to_string(),
            native_static_methods: native_static,
            native_methods,
            ..Default::default()
        }
    });

    env.define("Imap".to_string(), Value::Class(imap_class));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A stream that answers from a script and throws away whatever is
    /// written to it.
    ///
    /// `Cursor` cannot stand in for a socket the moment a test *sends*
    /// anything: a write lands at the cursor's own position, which is
    /// exactly where the next read was going to come from, so the test
    /// overwrites its own answer and the connection looks closed. Every
    /// test that speaks before it listens needs this instead.
    struct Canned(Cursor<Vec<u8>>);

    impl Read for Canned {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }

    impl Write for Canned {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// The set never reaches the wire unvalidated: no instance is needed to
    /// check that, and none is made.
    #[test]
    fn a_row_learns_its_size_and_its_paper_clips() {
        let body = "1 FETCH (UID 42 FLAGS (\\Seen) RFC822.SIZE 284213 BODYSTRUCTURE ((\"text\" \"plain\" NIL NIL NIL \"7bit\" 12 1)(\"application\" \"pdf\" (\"name\" \"devis.pdf\") NIL NIL \"base64\" 284000 NIL (\"attachment\" (\"filename\" \"devis.pdf\")) NIL) \"mixed\"))";
        assert_eq!(scan_size(body), Value::Int(284213));
        assert_eq!(count_attachments(body), 1);
        // No structure, no clips, and no size to report.
        assert_eq!(count_attachments("1 FETCH (UID 7 FLAGS ())"), 0);
        assert_eq!(scan_size("1 FETCH (UID 7)"), Value::Null);
    }

    #[test]
    fn a_sequence_set_is_digits_commas_colons_and_a_star() {
        let ok = |set: &str| {
            set.bytes()
                .all(|b| b.is_ascii_digit() || b == b',' || b == b':' || b == b'*')
        };
        assert!(ok("100:*"));
        assert!(ok("1,5,9"));
        assert!(ok("12:20"));
        // The two that matter: a space starts a second word, CRLF a second
        // command.
        assert!(!ok("1 UID SEARCH ALL"));
        assert!(!ok("1\r\nA1 LOGOUT"));
    }

    fn conn_speaking(bytes: &[u8]) -> ImapConn {
        let stream: Box<dyn Stream> = Box::new(Canned(Cursor::new(bytes.to_vec())));
        ImapConn {
            reader: BufReader::new(stream),
            tag: 0,
            selected_exists: None,
        }
    }

    fn conn_from(bytes: &[u8]) -> ImapConn {
        let stream: Box<dyn Stream> = Box::new(Cursor::new(bytes.to_vec()));
        ImapConn {
            reader: BufReader::new(stream),
            tag: 0,
            selected_exists: None,
        }
    }

    #[test]
    fn xoauth2_sends_the_sasl_string_and_takes_ok_for_an_answer() {
        let mut conn = conn_speaking(b"a0001 OK [] you@work.com authenticated\r\n");
        conn.authenticate_xoauth2("you@work.com", "ya29.token")
            .expect("accepted");
        // The initial response is the SASL string Google specifies, and
        // the two separators are SOH, not spaces -- a mistake there fails
        // in a way the server describes only in base64.
        let want = base64::engine::general_purpose::STANDARD
            .encode("user=you@work.com\u{1}auth=Bearer ya29.token\u{1}\u{1}");
        assert!(
            want.starts_with("dXNlcj15b3VAd29yay5jb20B"),
            "SOH after the address: {want}"
        );
    }

    #[test]
    fn a_refused_xoauth2_is_answered_rather_than_waited_on() {
        // The refusal does not arrive as a tagged NO. The server sends a
        // continuation describing the error and waits for an empty line
        // before saying anything else; a client that does not answer sits
        // there until the socket times out. This is that exchange.
        let mut conn = conn_speaking(
            b"+ eyJzdGF0dXMiOiI0MDEifQ==\r\na0001 NO [AUTHENTICATIONFAILED] Invalid credentials\r\n",
        );
        let err = conn
            .authenticate_xoauth2("you@work.com", "stale")
            .expect_err("refused");
        assert!(err.contains("XOAUTH2 refused"), "{err}");
        assert!(err.contains("AUTHENTICATIONFAILED"), "{err}");
    }

    #[test]
    fn trailing_literal_detection() {
        assert_eq!(trailing_literal_size("... BODY[] {28}"), Some(28));
        assert_eq!(trailing_literal_size("... BODY[] {28+}"), Some(28));
        assert_eq!(trailing_literal_size("a0001 OK done"), None);
        assert_eq!(trailing_literal_size(")"), None);
    }

    #[test]
    fn quote_escapes_specials() {
        assert_eq!(quote("INBOX"), "\"INBOX\"");
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }

    #[test]
    fn atom_or_quoted_splits() {
        assert_eq!(parse_atom_or_quoted("\"/\" rest").0, "/");
        assert_eq!(parse_atom_or_quoted("NIL rest").0, "NIL");
        let (tok, rest) = parse_atom_or_quoted("\"a\\\"b\" tail");
        assert_eq!(tok, "a\"b");
        assert_eq!(rest.trim(), "tail");
    }

    #[test]
    fn reads_fetch_with_literal_body() {
        // A FETCH whose BODY[] arrives as a 28-octet literal, then the tagged OK.
        let body = "Subject: Hi\r\n\r\nHello world\r\n";
        assert_eq!(body.len(), 28);
        let raw = format!(
            "* 1 FETCH (UID 5 FLAGS (\\Seen) BODY[] {{28}}\r\n{body})\r\na0001 OK FETCH completed\r\n"
        );
        let mut conn = conn_from(raw.as_bytes());
        let untagged = conn.read_until_tagged("a0001").unwrap();
        let msg = parse_fetch_one(&untagged).expect("a fetch response");
        let Value::Hash(h) = msg else {
            panic!("expected hash")
        };
        let h = h.borrow();
        assert!(matches!(
            h.get(&HashKey::String("uid".into())),
            Some(Value::Int(5))
        ));
        assert!(matches!(
            h.get(&HashKey::String("seq".into())),
            Some(Value::Int(1))
        ));
        assert!(matches!(
            h.get(&HashKey::String("subject".into())),
            Some(Value::String(s)) if **s == *"Hi"
        ));
        assert!(matches!(
            h.get(&HashKey::String("text_body".into())),
            Some(Value::String(s)) if s.contains("Hello world")
        ));
        match h.get(&HashKey::String("flags".into())) {
            Some(Value::Array(arr)) => {
                let arr = arr.borrow();
                assert_eq!(arr.len(), 1);
                assert!(matches!(&arr[0], Value::String(s) if **s == *"\\Seen"));
            }
            other => panic!("expected flags array, got {other:?}"),
        }
    }

    #[test]
    fn tagged_no_is_an_error() {
        let mut conn = conn_from(b"a0001 NO [AUTHENTICATIONFAILED] bad creds\r\n");
        assert!(conn.read_until_tagged("a0001").is_err());
    }

    #[test]
    fn parses_search_numbers() {
        let mut conn = conn_from(b"* SEARCH 1 3 42\r\na0001 OK SEARCH completed\r\n");
        let untagged = conn.read_until_tagged("a0001").unwrap();
        let Value::Array(arr) = parse_search(&untagged) else {
            panic!("expected array")
        };
        let arr = arr.borrow();
        let nums: Vec<i64> = arr
            .iter()
            .map(|v| match v {
                Value::Int(n) => *n,
                _ => panic!("expected int"),
            })
            .collect();
        assert_eq!(nums, vec![1, 3, 42]);
    }

    #[test]
    fn parses_select_status() {
        let raw = "* FLAGS (\\Answered \\Seen \\Deleted)\r\n\
                   * 12 EXISTS\r\n\
                   * 3 RECENT\r\n\
                   * OK [UNSEEN 9] Message 9 is first unseen\r\n\
                   * OK [UIDVALIDITY 1234] UIDs valid\r\n\
                   * OK [UIDNEXT 20] Predicted next UID\r\n\
                   a0001 OK [READ-WRITE] SELECT completed\r\n";
        let mut conn = conn_from(raw.as_bytes());
        let untagged = conn.read_until_tagged("a0001").unwrap();
        let (info, exists) = parse_select("INBOX", &untagged);
        assert_eq!(exists, Some(12));
        let Value::Hash(h) = info else {
            panic!("expected hash")
        };
        let h = h.borrow();
        assert!(matches!(
            h.get(&HashKey::String("exists".into())),
            Some(Value::Int(12))
        ));
        assert!(matches!(
            h.get(&HashKey::String("unseen".into())),
            Some(Value::Int(9))
        ));
        assert!(matches!(
            h.get(&HashKey::String("uidnext".into())),
            Some(Value::Int(20))
        ));
    }

    #[test]
    fn parses_mailbox_list() {
        let raw = "* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n\
                   * LIST (\\HasChildren) \"/\" \"[Gmail]\"\r\n\
                   a0001 OK LIST completed\r\n";
        let mut conn = conn_from(raw.as_bytes());
        let untagged = conn.read_until_tagged("a0001").unwrap();
        let mut names = Vec::new();
        for pieces in &untagged {
            let line = joined_text(pieces);
            let body = line.strip_prefix("* ").unwrap_or(&line);
            if let Some(rest) = body.strip_prefix("LIST ") {
                if let Some(Value::Hash(h)) = parse_list_line(rest) {
                    if let Some(Value::String(n)) = h.borrow().get(&HashKey::String("name".into()))
                    {
                        names.push(n.to_string());
                    }
                }
            }
        }
        assert_eq!(names, vec!["INBOX".to_string(), "[Gmail]".to_string()]);
    }
}
