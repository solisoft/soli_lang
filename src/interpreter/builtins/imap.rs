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

use crate::interpreter::builtins::bodystructure;
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
/// A mailbox name as a person reads it: modified UTF-7 decoded to UTF-8.
///
/// RFC 3501 §5.1.3. A server names its mailboxes in US-ASCII, and anything
/// outside it is shifted into a modified BASE64 of UTF-16 between `&` and
/// `-`: "Messages envoyés" arrives as `Messages envoy&AOK-s`, which is
/// what a folder list shows when nobody decodes it. `&-` is a literal
/// ampersand, and the alphabet is BASE64's with `,` in place of `/`
/// because `/` is a hierarchy delimiter.
///
/// What comes back is for *reading*. The name to put in a command is the
/// one the server sent — this returns the display form beside it, never
/// instead of it.
fn decode_modified_utf7(said: &str) -> String {
    let mut out = String::new();
    let b: Vec<char> = said.chars().collect();
    let mut i = 0usize;
    while i < b.len() {
        let Some(&c) = b.get(i) else { break };
        if c != '&' {
            out.push(c);
            i += 1;
            continue;
        }
        // `&-` is how a name says it contains an ampersand.
        if b.get(i + 1) == Some(&'-') {
            out.push('&');
            i += 2;
            continue;
        }
        let Some(end) = b
            .get(i + 1..)
            .and_then(|rest| rest.iter().position(|c| *c == '-'))
        else {
            // A shift that never ends is not a shift. Take it as text
            // rather than losing the rest of the name.
            out.push(c);
            i += 1;
            continue;
        };
        let chunk: String = b
            .get(i + 1..i + 1 + end)
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        match decode_utf7_chunk(&chunk) {
            Some(said) => out.push_str(&said),
            None => {
                out.push('&');
                out.push_str(&chunk);
                out.push('-');
            }
        }
        i += end + 2;
    }
    out
}

/// One `&…-` run: modified BASE64 of UTF-16BE.
fn decode_utf7_chunk(chunk: &str) -> Option<String> {
    let mut bits: u32 = 0;
    let mut have: u32 = 0;
    let mut units: Vec<u16> = Vec::new();
    for c in chunk.chars() {
        let v = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            ',' => 63,
            _ => return None,
        };
        bits = (bits << 6) | v;
        have += 6;
        if have >= 16 {
            have -= 16;
            units.push(((bits >> have) & 0xFFFF) as u16);
        }
    }
    // Whatever is left over must be zero padding, not a truncated unit.
    if have >= 6 || (bits & ((1 << have) - 1)) != 0 {
        return None;
    }
    String::from_utf16(&units).ok()
}

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
    let shown = decode_modified_utf7(&name);
    Some(hash_from_pairs(vec![
        ("name".to_string(), Value::String(name.into())),
        // The same name, for a person: `Messages envoy&AOK-s` is
        // `Messages envoyés` and nobody should have to know that.
        ("label".to_string(), Value::String(shown.into())),
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

/// `STATUS <mailbox> (MESSAGES UNSEEN RECENT UIDNEXT)` — how much is in a
/// mailbox, without opening it.
///
/// The point is the "without": `SELECT` makes a mailbox *the* mailbox of
/// the connection, so asking how many messages are in five folders with
/// `SELECT` means five folders each becoming the selected one and the
/// sixth command going somewhere unexpected. `STATUS` asks about a
/// mailbox the connection is not in and leaves the selection alone.
///
/// The counts come back as `* STATUS "name" (MESSAGES 231 UNSEEN 4 ...)`.
/// Anything the server leaves out is simply absent from the hash rather
/// than reported as zero: a folder with no `UNSEEN` in its answer is a
/// server that did not say, not a folder with nothing unread.
fn imap_status(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "status")?;
    let mailbox = match args.get(1) {
        None | Some(Value::Null) => "INBOX".to_string(),
        Some(v) => as_string(v, "mailbox")?,
    };
    let untagged = with_conn(id, |c| {
        c.command(&format!(
            "STATUS {} (MESSAGES UNSEEN RECENT UIDNEXT UIDVALIDITY)",
            quote(&mailbox)
        ))
    })?;
    let mut pairs: Vec<(String, Value)> =
        vec![("mailbox".to_string(), Value::String(mailbox.clone().into()))];
    for piece in &untagged {
        let line = joined_text(piece);
        let body = line.strip_prefix("* ").unwrap_or(&line);
        let Some(rest) = body
            .strip_prefix("STATUS ")
            .or_else(|| body.strip_prefix("status "))
        else {
            continue;
        };
        let Some(open) = rest.find('(') else { continue };
        let Some(close) = rest.rfind(')') else {
            continue;
        };
        let Some(inside) = rest.get(open + 1..close) else {
            continue;
        };
        let words: Vec<&str> = inside.split_whitespace().collect();
        let mut i = 0;
        while i + 1 < words.len() {
            let (Some(name), Some(value)) = (words.get(i), words.get(i + 1)) else {
                break;
            };
            if let Ok(n) = value.parse::<i64>() {
                pairs.push((name.to_ascii_lowercase(), Value::Int(n)));
            }
            i += 2;
        }
    }
    Ok(hash_from_pairs(pairs))
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

/// The letter without its freight.
///
/// `fetch_uid` asks for `BODY.PEEK[]`, which is the whole message — and a
/// mail with seventeen photographs on it is two megabytes of which the
/// text is four thousand bytes. It was fetched whole to show the text,
/// and fetched whole *again* when the photographs were wanted, because
/// what the first fetch kept was the list of attachments and not their
/// bytes.
///
/// So: ask the server for the shape first (`BODYSTRUCTURE`, which costs
/// nothing and which it sends with every header anyway), take the numbers
/// of the parts that are the letter's faces, and fetch only those. What
/// comes back is assembled into a message that carries the original
/// headers and the text parts alone, so `mail_parse` reads it exactly as
/// it read the whole thing.
///
/// `attachments` on the result then carries what the structure says about
/// the freight -- name, type and size -- with none of it downloaded.
/// `fetch_uid_parts` is how those bytes are asked for, when they are.
///
/// Anything unexpected falls back to `fetch_uid`: a structure that does
/// not parse, a message with no text part in it, or a server that answers
/// the second fetch with nothing.
fn imap_fetch_uid_text(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "fetch_uid_text")?;
    let uid = message_id_arg(args, "fetch_uid_text", "uid")?;
    let shape = with_conn(id, |c| {
        c.command(&format!(
            "UID FETCH {uid} (UID FLAGS RFC822.SIZE BODYSTRUCTURE)"
        ))
    })?;
    let Some(pieces) = shape.iter().find(|p| is_fetch_response(p)) else {
        return Err(format!("Imap.fetch_uid_text({uid}): no such message"));
    };
    let meta = joined_text(pieces);
    let Some(parts) = bodystructure::parts_of(&meta) else {
        return imap_fetch_uid(args);
    };
    let faces: Vec<&bodystructure::Part> = parts.iter().filter(|p| p.is_text()).collect();
    if faces.is_empty() {
        return imap_fetch_uid(args);
    }
    let mut items = String::from("(UID FLAGS BODY.PEEK[HEADER]");
    for f in &faces {
        items.push_str(&format!(
            " BODY.PEEK[{}.MIME] BODY.PEEK[{}]",
            f.number, f.number
        ));
    }
    items.push(')');
    let got = with_conn(id, |c| c.command(&format!("UID FETCH {uid} {items}")))?;
    let Some(pieces) = got.iter().find(|p| is_fetch_response(p)) else {
        return imap_fetch_uid(args);
    };
    let named = keyed_literals(pieces);
    let Some(head) = literal_named(&named, "HEADER") else {
        return imap_fetch_uid(args);
    };
    let mut faces_in = Vec::new();
    for f in &faces {
        let (Some(mime), Some(body)) = (
            literal_named(&named, &format!("{}.MIME", f.number)),
            literal_named(&named, &f.number),
        ) else {
            continue;
        };
        faces_in.push((mime, body));
    }
    let Some(raw) = assemble(head, &faces_in) else {
        return imap_fetch_uid(args);
    };
    let body = meta.strip_prefix("* ").unwrap_or(&meta);
    let seq = body
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let clips = parts.iter().filter(|p| !p.is_text()).count() as i64;
    let mut pairs: Vec<(String, Value)> = vec![
        ("seq".to_string(), Value::Int(seq)),
        ("uid".to_string(), scan_uid(body)),
        ("flags".to_string(), scan_flags(body)),
        ("bytes".to_string(), scan_size(body)),
        ("clips".to_string(), Value::Int(clips)),
    ];
    pairs.extend(mail_parse::common_fields(&raw));
    // The freight, named and measured, and not one byte of it fetched.
    // This replaces whatever the parse of a text-only message said about
    // attachments, which is necessarily nothing.
    pairs.retain(|(k, _)| k != "attachments");
    pairs.push(("attachments".to_string(), freight(&parts)));
    Ok(hash_from_pairs(pairs))
}

/// The literals of a FETCH response, each keyed by the item that
/// introduced it.
///
/// A response interleaves text and literals — `… BODY[HEADER] {1234}` then
/// twelve hundred bytes, then ` BODY[1.MIME] {56}` then fifty-six — and
/// the pieces arrive in that order. Taking the literals *positionally*,
/// which is what the first version of this did, assumes the server
/// answers with exactly the items that were asked for, in the order they
/// were asked for, and nothing else. A server that adds `FLAGS` between
/// two of them, or answers `BODY[1]` before `BODY[1.MIME]`, then pairs a
/// part's body with the next part's header — and what comes out is a
/// letter that starts in the middle of a stylesheet.
///
/// So each literal is named by the `BODY[…]` in the text just before it,
/// and the caller asks for what it wants by name.
fn keyed_literals(pieces: &[Piece]) -> Vec<(String, &[u8])> {
    let mut out: Vec<(String, &[u8])> = Vec::new();
    let mut last = String::new();
    for p in pieces {
        match p {
            Piece::Text(t) => last.push_str(t),
            Piece::Literal(b) => {
                let key = last
                    .to_ascii_uppercase()
                    .rfind("BODY[")
                    .and_then(|at| {
                        let rest = last.get(at + "BODY[".len()..)?;
                        let end = rest.find(']')?;
                        Some(rest.get(..end)?.to_ascii_uppercase())
                    })
                    .unwrap_or_default();
                out.push((key, b.as_slice()));
                last.clear();
            }
        }
    }
    out
}

/// One named literal, or nothing.
fn literal_named<'a>(named: &'a [(String, &'a [u8])], key: &str) -> Option<&'a [u8]> {
    named
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, b)| *b)
}

/// The freight of a message, as `fetch_uid` would have listed it, from the
/// structure alone: no `base64`, because nothing was downloaded.
fn freight(parts: &[bodystructure::Part]) -> Value {
    let rows: Vec<Value> = parts
        .iter()
        .filter(|p| !p.is_text())
        .map(|p| {
            hash_from_pairs(vec![
                ("name".to_string(), Value::String(p.name.clone().into())),
                (
                    "content_type".to_string(),
                    Value::String(format!("{}/{}", p.kind, p.sub).into()),
                ),
                ("size".to_string(), Value::Int(p.bytes)),
                ("part".to_string(), Value::String(p.number.clone().into())),
                ("base64".to_string(), Value::String(String::new().into())),
            ])
        })
        .collect();
    Value::Array(Rc::new(RefCell::new(rows)))
}

/// Header, then the parts, as one message `mail_parse` can read.
///
/// The literals arrive in the order they were asked for: the header, then
/// a MIME header and a body for each face. One face is spliced straight
/// on to the header with the top `Content-*` lines dropped, because the
/// top one describes a multipart that is no longer there. Several become a
/// `multipart/alternative` of our own making, which is what they were
/// inside the original anyway.
fn assemble(head: &[u8], faces: &[(&[u8], &[u8])]) -> Option<Vec<u8>> {
    if faces.is_empty() {
        return None;
    }
    let mut out: Vec<u8> = Vec::new();
    // Whether the last header line seen is one we are keeping, so its
    // folded continuations follow it either way.
    let mut keeping = true;
    for line in head.split(|b| *b == b'\n') {
        let said = String::from_utf8_lossy(line);
        let trimmed = said.trim_end_matches('\r');
        if trimmed.is_empty() {
            continue;
        }
        // A continuation line belongs to the field above it, so it goes
        // or stays with it.
        if trimmed.starts_with(' ') || trimmed.starts_with('\t') {
            // A folded line belongs to the field above it, and goes or
            // stays with it. Kept as its own line rather than joined,
            // which is what the field meant.
            if keeping {
                out.extend_from_slice(trimmed.as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            continue;
        }
        // The top `Content-*` lines describe a multipart that is not
        // here any more: what follows is one part, or an alternative of
        // this function's own making.
        keeping = !trimmed.to_ascii_lowercase().starts_with("content-");
        if !keeping {
            continue;
        }
        out.extend_from_slice(trimmed.as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    if let [(mime, body)] = faces {
        out.extend_from_slice(mime);
        if !mime.ends_with(b"\n") {
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(body);
        return Some(out);
    }
    let line = "eui-faces-8f3a2b";
    out.extend_from_slice(
        format!("Content-Type: multipart/alternative; boundary=\"{line}\"\r\n\r\n").as_bytes(),
    );
    for (mime, body) in faces {
        out.extend_from_slice(format!("--{line}\r\n").as_bytes());
        out.extend_from_slice(mime);
        if !mime.ends_with(b"\n") {
            out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(body);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(format!("--{line}--\r\n").as_bytes());
    Some(out)
}

/// The freight alone, by part number: `p`'s half of the bargain.
///
/// `parts` is a list of the numbers a previous `fetch_uid_text` reported,
/// and what comes back is one row per part with its `base64`. Nothing else
/// of the message crosses the wire.
fn imap_fetch_uid_parts(args: &[Value]) -> Result<Value, String> {
    let id = instance_id(args, "fetch_uid_parts")?;
    let uid = message_id_arg(args, "fetch_uid_parts", "uid")?;
    let Some(Value::Array(asked)) = args.get(2) else {
        return Err(
            "Imap.fetch_uid_parts(uid, parts) expects an array of part numbers".to_string(),
        );
    };
    let numbers: Vec<String> = asked
        .borrow()
        .iter()
        .filter_map(|v| match v {
            Value::String(s) => Some(s.to_string()),
            _ => None,
        })
        // A part number is digits and dots and nothing else: it is put
        // straight into a command line.
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.'))
        .collect();
    if numbers.is_empty() {
        return Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))));
    }
    let mut items = String::from("(UID");
    for n in &numbers {
        items.push_str(&format!(" BODY.PEEK[{n}]"));
    }
    items.push(')');
    let got = with_conn(id, |c| c.command(&format!("UID FETCH {uid} {items}")))?;
    let Some(pieces) = got.iter().find(|p| is_fetch_response(p)) else {
        return Err(format!("Imap.fetch_uid_parts({uid}): no such message"));
    };
    let named = keyed_literals(pieces);
    let rows: Vec<Value> = numbers
        .iter()
        .filter_map(|n| literal_named(&named, n).map(|raw| (n, raw)))
        .map(|(n, raw)| {
            // What the server sends is the part still encoded the way the
            // message encoded it, which for anything attached is base64
            // with its line breaks. Those are removed and nothing else is
            // touched: the caller writes it with `file_write_base64`.
            let said: String = String::from_utf8_lossy(raw)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            hash_from_pairs(vec![
                ("part".to_string(), Value::String(n.clone().into())),
                ("base64".to_string(), Value::String(said.into())),
            ])
        })
        .collect();
    Ok(Value::Array(Rc::new(RefCell::new(rows))))
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
        method("status", None, imap_status),
        method("fetch_uid_text", Some(1), imap_fetch_uid_text),
        method("fetch_uid_parts", Some(2), imap_fetch_uid_parts),
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

    /// One face: the top header keeps everything but its `Content-*`
    /// lines, and the part's own MIME header takes their place.
    #[test]
    fn one_face_is_spliced_on_to_the_header_it_came_with() {
        let head = b"Subject: les photos\r\nFrom: club@example.com\r\nContent-Type: multipart/mixed; boundary=\"xx\"\r\nContent-Transfer-Encoding: 7bit\r\n";
        let mime = b"Content-Type: text/plain; charset=utf-8\r\n";
        let body = b"bonjour\r\n";
        let out =
            assemble(head.as_slice(), &[(mime.as_slice(), body.as_slice())]).expect("assembled");
        let said = String::from_utf8_lossy(&out);
        assert!(said.contains("Subject: les photos"), "{said}");
        assert!(said.contains("Content-Type: text/plain"), "{said}");
        assert!(
            !said.contains("multipart/mixed"),
            "the freight's wrapper is gone: {said}"
        );
        let fields = mail_parse::common_fields(&out);
        let body = fields
            .iter()
            .find(|(k, _)| k == "text_body")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();
        assert!(body.contains("bonjour"), "{body}");
    }

    /// Two faces become an alternative of our own, which is what they were
    /// inside the original.
    #[test]
    fn two_faces_are_wrapped_in_an_alternative() {
        let head = b"Subject: two\r\nContent-Type: multipart/mixed; boundary=\"xx\"\r\n";
        let out = assemble(
            head.as_slice(),
            &[
                (
                    b"Content-Type: text/plain\r\n".as_slice(),
                    b"plain words\r\n".as_slice(),
                ),
                (
                    b"Content-Type: text/html\r\n".as_slice(),
                    b"<p>rich words</p>\r\n".as_slice(),
                ),
            ],
        )
        .expect("assembled");
        let said = String::from_utf8_lossy(&out);
        assert!(said.contains("multipart/alternative"), "{said}");
        let fields = mail_parse::common_fields(&out);
        let body = fields
            .iter()
            .find(|(k, _)| k == "text_body")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();
        let html = fields
            .iter()
            .find(|(k, _)| k == "html_body")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();
        assert!(body.contains("plain words"), "text face: {body}");
        assert!(html.contains("rich words"), "html face: {html}");
    }

    /// Nothing to splice, or an odd number of pieces, says so rather than
    /// building half a message.
    #[test]
    fn an_incomplete_answer_is_refused() {
        assert!(assemble(b"Subject: x\r\n", &[]).is_none());
    }

    /// The fault that showed as a letter starting in the middle of a
    /// stylesheet: literals taken by position rather than by name.
    #[test]
    fn a_literal_is_named_by_the_item_that_introduced_it() {
        let pieces = vec![
            Piece::Text("* 1 FETCH (UID 42 FLAGS (\\Seen) BODY[HEADER] {12}".to_string()),
            Piece::Literal(b"Subject: hi\n".to_vec()),
            // A server is allowed to answer with more than was asked for,
            // and in whatever order it likes.
            Piece::Text(" BODY[1.2] {6}".to_string()),
            Piece::Literal(b"<p>x</p>".to_vec()),
            Piece::Text(" BODY[1.2.MIME] {10}".to_string()),
            Piece::Literal(b"text/html\n".to_vec()),
            Piece::Text(")".to_string()),
        ];
        let named = keyed_literals(&pieces);
        assert_eq!(
            literal_named(&named, "HEADER"),
            Some(b"Subject: hi\n".as_slice())
        );
        assert_eq!(literal_named(&named, "1.2"), Some(b"<p>x</p>".as_slice()));
        assert_eq!(
            literal_named(&named, "1.2.MIME"),
            Some(b"text/html\n".as_slice())
        );
        assert_eq!(literal_named(&named, "1.1"), None);
    }

    /// A folded `Content-Type` takes its continuation with it, rather
    /// than leaving ` boundary="xx"` behind as a header line of its own.
    #[test]
    fn a_folded_header_that_is_dropped_takes_its_continuation() {
        let head =
            b"Subject: x\r\nContent-Type: multipart/mixed;\r\n boundary=\"xx\"\r\nTo: a@b.c\r\n";
        let out = assemble(
            head.as_slice(),
            &[(
                b"Content-Type: text/plain\r\n".as_slice(),
                b"words\r\n".as_slice(),
            )],
        )
        .expect("assembled");
        let said = String::from_utf8_lossy(&out);
        assert!(!said.contains("boundary=\"xx\""), "{said}");
        assert!(said.contains("To: a@b.c"), "{said}");
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

    /// The encoding that made a French mailbox read as
    /// `Messages envoy&AOK-s` in a folder list.
    #[test]
    fn a_mailbox_name_is_decoded_for_reading_and_kept_for_commands() {
        // `é` is U+00E9, which is `000000 001110 1001` + two bits of
        // padding -- `A`, `O`, `k`. A capital `K` is a different code
        // point and a name that does not mean what it looks like, so it
        // is left alone rather than decoded into the wrong letter.
        assert_eq!(
            decode_modified_utf7("Messages envoy&AOk-s"),
            "Messages envoyés"
        );
        assert_eq!(decode_modified_utf7("INBOX"), "INBOX");
        assert_eq!(
            decode_modified_utf7("[Gmail]/Sent Mail"),
            "[Gmail]/Sent Mail"
        );
        // A literal ampersand, which is the one escape this encoding has.
        assert_eq!(decode_modified_utf7("R&-D"), "R&D");
        // Cyrillic, to prove it is UTF-16 and not Latin-1 with a hat on --
        // and that `,` stands in for BASE64's `/`.
        assert_eq!(
            decode_modified_utf7("&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-"),
            "Отправленные"
        );
        assert_eq!(decode_modified_utf7("Wys&AUI-ane"), "Wysłane");
        // Nonsense stays as it came rather than losing the name.
        assert_eq!(decode_modified_utf7("a&b"), "a&b");
        assert_eq!(decode_modified_utf7("a&!!-b"), "a&!!-b");

        let line = "(\\HasNoChildren \\Sent) \"/\" \"Messages envoy&AOk-s\"";
        let Some(Value::Hash(h)) = parse_list_line(line) else {
            panic!("a mailbox")
        };
        let h = h.borrow();
        // The wire form is what a `SELECT` must be given.
        assert_eq!(
            h.get(&HashKey::String("name".into())),
            Some(&Value::String("Messages envoy&AOk-s".into()))
        );
        assert_eq!(
            h.get(&HashKey::String("label".into())),
            Some(&Value::String("Messages envoyés".into()))
        );
    }

    /// How many messages a folder has, without becoming the folder.
    #[test]
    fn a_status_line_is_read_into_its_counts() {
        let pieces = vec![Piece::Text(
            "* STATUS \"[Gmail]/All Mail\" (MESSAGES 10431 UNSEEN 4 RECENT 0 UIDNEXT 78312)"
                .to_string(),
        )];
        let untagged = vec![pieces];
        // The parse is the body of `imap_status` after the command; run it
        // the same way over a canned response.
        let mut counts: Vec<(String, i64)> = Vec::new();
        for piece in &untagged {
            let line = joined_text(piece);
            let body = line.strip_prefix("* ").unwrap_or(&line);
            let rest = body.strip_prefix("STATUS ").unwrap();
            let open = rest.find('(').unwrap();
            let close = rest.rfind(')').unwrap();
            let inside = &rest[open + 1..close];
            let words: Vec<&str> = inside.split_whitespace().collect();
            let mut i = 0;
            while i + 1 < words.len() {
                if let Ok(n) = words[i + 1].parse::<i64>() {
                    counts.push((words[i].to_ascii_lowercase(), n));
                }
                i += 2;
            }
        }
        assert_eq!(
            counts,
            vec![
                ("messages".to_string(), 10431),
                ("unseen".to_string(), 4),
                ("recent".to_string(), 0),
                ("uidnext".to_string(), 78312),
            ]
        );
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
