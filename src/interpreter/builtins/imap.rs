//! IMAP email-reading builtin: the `Imap` client class.
//!
//! A small synchronous IMAP4rev1 client (implicit TLS on port 993 by default),
//! mirroring the `Pop3` client. Unlike POP3, IMAP is stateful — you `select()`
//! a mailbox, then `search()`/`fetch()` within it — so the surface is larger:
//!
//! ```soli
//! mail = Imap.new("imap.gmail.com", "me@gmail.com", "app-password")
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

fn imap_new(class: Rc<Class>, args: Vec<Value>) -> Result<Value, String> {
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
        conn.command(&format!("LOGIN {} {}", quote(&user), quote(&pass)))
            .map_err(|e| format!("IMAP authentication failed: {e}"))?;
    }

    let id = IMAP_NEXT_ID.fetch_add(1, Ordering::SeqCst);
    IMAP_CONNS
        .lock()
        .map_err(|e| e.to_string())?
        .insert(id, conn);

    let mut inst = Instance::new(class);
    inst.set("_id".to_string(), Value::Int(id as i64));
    Ok(Value::Instance(Rc::new(RefCell::new(inst))))
}

fn imap_select(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "select")?;
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

fn imap_mailboxes(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "mailboxes")?;
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

fn imap_search(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "search")?;
    let criteria = search_criteria(&args)?;
    let untagged = with_conn(id, |c| c.command(&format!("SEARCH {criteria}")))?;
    Ok(parse_search(&untagged))
}

fn imap_uid_search(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "uid_search")?;
    let criteria = search_criteria(&args)?;
    let untagged = with_conn(id, |c| c.command(&format!("UID SEARCH {criteria}")))?;
    Ok(parse_search(&untagged))
}

fn imap_fetch(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "fetch")?;
    let seq = message_id_arg(&args, "fetch", "seq")?;
    let untagged = with_conn(id, |c| {
        c.command(&format!("FETCH {seq} (UID FLAGS BODY.PEEK[])"))
    })?;
    parse_fetch_one(&untagged).ok_or_else(|| format!("Imap.fetch({seq}): no such message"))
}

fn imap_fetch_uid(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "fetch_uid")?;
    let uid = message_id_arg(&args, "fetch_uid", "uid")?;
    let untagged = with_conn(id, |c| {
        c.command(&format!("UID FETCH {uid} (UID FLAGS BODY.PEEK[])"))
    })?;
    parse_fetch_one(&untagged).ok_or_else(|| format!("Imap.fetch_uid({uid}): no such message"))
}

fn imap_fetch_all(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "fetch_all")?;
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
fn store_flag(args: Vec<Value>, method: &str, op: char, flag: &str) -> Result<Value, String> {
    let id = instance_id(&args, method)?;
    let seq = message_id_arg(&args, method, "seq")?;
    with_conn(id, |c| {
        c.command(&format!("STORE {seq} {op}FLAGS ({flag})"))
    })?;
    Ok(Value::Bool(true))
}

fn imap_mark_seen(args: Vec<Value>) -> Result<Value, String> {
    store_flag(args, "mark_seen", '+', "\\Seen")
}

fn imap_mark_unseen(args: Vec<Value>) -> Result<Value, String> {
    store_flag(args, "mark_unseen", '-', "\\Seen")
}

fn imap_delete(args: Vec<Value>) -> Result<Value, String> {
    store_flag(args, "delete", '+', "\\Deleted")
}

fn imap_expunge(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "expunge")?;
    with_conn(id, |c| c.command("EXPUNGE"))?;
    Ok(Value::Bool(true))
}

fn imap_copy(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "copy")?;
    let seq = message_id_arg(&args, "copy", "seq")?;
    let mailbox = mailbox_arg(&args, "copy")?;
    with_conn(id, |c| {
        c.command(&format!("COPY {seq} {}", quote(&mailbox)))
    })?;
    Ok(Value::Bool(true))
}

fn imap_move(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "move")?;
    let seq = message_id_arg(&args, "move", "seq")?;
    let mailbox = mailbox_arg(&args, "move")?;
    // Uses the RFC 6851 MOVE extension (supported by Gmail, Dovecot, …). Servers
    // without it return NO/BAD, surfaced as an error to the caller.
    with_conn(id, |c| {
        c.command(&format!("MOVE {seq} {}", quote(&mailbox)))
    })?;
    Ok(Value::Bool(true))
}

fn imap_logout(args: Vec<Value>) -> Result<Value, String> {
    let id = instance_id(&args, "logout")?;
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
    f: fn(Vec<Value>) -> Result<Value, String>,
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
        method("mark_seen", Some(1), imap_mark_seen),
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

    fn conn_from(bytes: &[u8]) -> ImapConn {
        let stream: Box<dyn Stream> = Box::new(Cursor::new(bytes.to_vec()));
        ImapConn {
            reader: BufReader::new(stream),
            tag: 0,
            selected_exists: None,
        }
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
