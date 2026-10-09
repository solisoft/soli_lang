//! Whole responses an application marks as cacheable, answered again without
//! a worker while the SQLite database has not changed.
//!
//! The application opts in per response with a `Soli-Response-Cache: <seconds>`
//! header (stripped before the response leaves). The server then keeps the
//! response under a key made of the request line and every request header the
//! response could depend on — `Cookie` among them, so a response is only ever
//! replayed to a request carrying the same cookies, and so the same session. A
//! later request with the same key is answered from memory, before any Soli
//! code runs, as long as:
//!
//! - **the database has not changed.** A read-only connection watches the
//!   app's SQLite file with `PRAGMA data_version`, which moves on every commit
//!   from any other connection: a message, a sign-out, a ban, a rename all
//!   retire every entry at once. The version is read before the request goes to
//!   a worker and again when its response comes back; a commit in between keeps
//!   the response out of the cache, so it can never be filed under a version
//!   newer than the data it was rendered from. This is the Rust and C ports'
//!   response cache, which follow the same rule.
//! - **its time-to-live has not run out** (the header's seconds, at most
//!   five minutes), for what a page shows that no commit changes: relative
//!   dates, expiring links.
//!
//! Only a `GET` without a body, `Range` or `Cache-Control: no-cache` is looked
//! up; only a `200` of at most 1 MiB, without `Cache-Control: no-store` and
//! without a new framework session cookie (a flash consumed, a CSRF token
//! rotated) is kept. Conditional GET, compression and prefetch handling run on
//! a hit as on a miss: the entry is the worker's reply, before `assemble`.
//!
//! Off with no SQLite file behind the default connection (another adapter, or
//! `:memory:`), in `--dev`, and with `SOLI_RESPONSE_CACHE_MB=0`; 64 MiB by
//! default. Entries of an older version are dropped as soon as a newer one is
//! seen; past the size bound, the oldest go first.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use bytes::Bytes;

use super::ResponseData;

/// The response header an application sets to have a response kept.
pub(crate) const HEADER: &str = "soli-response-cache";

const MAX_BODY: usize = 1 << 20;
const MAX_KEY: usize = 16 * 1024;
const MAX_TTL: Duration = Duration::from_secs(300);
const DEFAULT_CAPACITY_MB: usize = 64;

/// Request headers left out of the key: validators and compression are applied
/// to the kept reply on every hit, prefetch marks only change its
/// `Cache-Control` there, and request ids differ on every request.
const UNKEYED: &[&str] = &[
    "if-none-match",
    "if-modified-since",
    "accept-encoding",
    "purpose",
    "sec-purpose",
    "x-moz",
    "x-request-id",
    "x-request-start",
    "traceparent",
    "tracestate",
];

/// A request that may be answered from the cache, or whose response may be
/// kept: its key, and the database version it was read under.
pub(crate) struct Ticket {
    key: String,
    generation: u64,
}

struct Entry {
    status: u16,
    headers: Arc<Vec<(String, String)>>,
    body: Bytes,
    expires: Instant,
    size: usize,
}

struct Entries {
    generation: u64,
    map: HashMap<String, Entry>,
    order: VecDeque<String>,
    bytes: usize,
}

#[cfg(feature = "sqlite")]
struct Observer {
    conn: rusqlite::Connection,
    data_version: i64,
    generation: u64,
}

pub(crate) struct Store {
    #[cfg(feature = "sqlite")]
    observer: Mutex<Observer>,
    entries: RwLock<Entries>,
    capacity: usize,
}

static STORE: OnceLock<Option<Store>> = OnceLock::new();

/// The process's cache, opened on first use; `None` when it is off.
fn store() -> Option<&'static Store> {
    STORE.get_or_init(open).as_ref()
}

fn capacity_from(raw: Option<String>) -> usize {
    raw.and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(DEFAULT_CAPACITY_MB)
        .saturating_mul(1 << 20)
}

#[cfg(feature = "sqlite")]
fn open() -> Option<Store> {
    let capacity = capacity_from(std::env::var("SOLI_RESPONSE_CACHE_MB").ok());
    if capacity == 0 {
        return None;
    }
    let registry = crate::db::registry::registry();
    let spec = registry.default_spec();
    if spec.adapter != crate::db::Adapter::Sqlite {
        return None;
    }
    let path = match crate::db::sqlite::parse_target(spec.url.as_deref()?) {
        crate::db::sqlite::Target::File(path) => path,
        crate::db::sqlite::Target::Memory => return None,
    };
    match Store::open(&path, capacity) {
        Ok(store) => Some(store),
        Err(e) => {
            eprintln!(
                "[soli] response cache off: cannot watch {}: {e}",
                path.display()
            );
            None
        }
    }
}

#[cfg(not(feature = "sqlite"))]
fn open() -> Option<Store> {
    None
}

impl Store {
    #[cfg(feature = "sqlite")]
    fn open(path: &std::path::Path, capacity: usize) -> rusqlite::Result<Store> {
        use rusqlite::OpenFlags;
        let conn = rusqlite::Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_millis(50))?;
        let data_version = conn.query_row("PRAGMA data_version", [], |row| row.get(0))?;
        Ok(Store {
            observer: Mutex::new(Observer {
                conn,
                data_version,
                generation: 0,
            }),
            entries: RwLock::new(Entries {
                generation: 0,
                map: HashMap::new(),
                order: VecDeque::new(),
                bytes: 0,
            }),
            capacity,
        })
    }

    /// The database's version as this process counts it: one more for every
    /// change of `data_version` it has seen. `None` when it cannot be read,
    /// which keeps the request away from the cache.
    #[cfg(feature = "sqlite")]
    fn version(&self) -> Option<u64> {
        let mut observer = self.observer.lock().unwrap_or_else(|e| e.into_inner());
        let current: i64 = observer
            .conn
            .query_row("PRAGMA data_version", [], |row| row.get(0))
            .ok()?;
        if current != observer.data_version {
            observer.data_version = current;
            observer.generation += 1;
        }
        Some(observer.generation)
    }

    #[cfg(not(feature = "sqlite"))]
    fn version(&self) -> Option<u64> {
        None
    }

    fn get(&self, ticket: &Ticket) -> Option<ResponseData> {
        let entries = self.entries.read().unwrap_or_else(|e| e.into_inner());
        if entries.generation != ticket.generation {
            return None;
        }
        let entry = entries.map.get(&ticket.key)?;
        if entry.expires <= Instant::now() {
            return None;
        }
        Some(ResponseData {
            status: entry.status,
            headers: (*entry.headers).clone(),
            body: entry.body.clone(),
        })
    }

    fn put(&self, ticket: Ticket, response: &ResponseData, ttl: Duration) {
        let size = response.body.len()
            + ticket.key.len()
            + response
                .headers
                .iter()
                .map(|(k, v)| k.len() + v.len())
                .sum::<usize>();
        if size > self.capacity {
            return;
        }
        let mut entries = self.entries.write().unwrap_or_else(|e| e.into_inner());
        if entries.generation != ticket.generation {
            if ticket.generation < entries.generation {
                return;
            }
            entries.generation = ticket.generation;
            entries.map.clear();
            entries.order.clear();
            entries.bytes = 0;
        }
        if let Some(old) = entries.map.remove(&ticket.key) {
            entries.bytes -= old.size;
            entries.order.retain(|k| k != &ticket.key);
        }
        while entries.bytes + size > self.capacity {
            let Some(oldest) = entries.order.pop_front() else {
                break;
            };
            if let Some(old) = entries.map.remove(&oldest) {
                entries.bytes -= old.size;
            }
        }
        entries.bytes += size;
        entries.order.push_back(ticket.key.clone());
        entries.map.insert(
            ticket.key,
            Entry {
                status: response.status,
                headers: Arc::new(response.headers.clone()),
                body: response.body.clone(),
                expires: Instant::now() + ttl,
                size,
            },
        );
    }
}

/// The ticket for a request the cache may answer, or `None` when it may not:
/// the cache is off, the request is not a plain `GET`, the client asked for a
/// fresh copy, or the database version cannot be read.
pub(crate) fn begin(
    method: &str,
    path: &str,
    raw_query: Option<&str>,
    headers: &hyper::HeaderMap,
) -> Option<Ticket> {
    let store = store()?;
    if !eligible(method, headers) {
        return None;
    }
    let key = request_key(path, raw_query, headers)?;
    let generation = store.version()?;
    Some(Ticket { key, generation })
}

/// The kept reply for this ticket, if there is a live one.
pub(crate) fn lookup(ticket: &Ticket) -> Option<ResponseData> {
    store()?.get(ticket)
}

/// Strip the application's cache header from a reply, and keep the reply when
/// it asked to be kept and the database did not change while it was made.
pub(crate) fn admit(ticket: Option<Ticket>, response: &mut ResponseData) {
    let Some(ttl) = take_ttl(&mut response.headers) else {
        return;
    };
    let (Some(ticket), Some(store)) = (ticket, store()) else {
        return;
    };
    if !keepable(response) {
        return;
    }
    if store.version() == Some(ticket.generation) {
        store.put(ticket, response, ttl);
    }
}

/// Remove every `Soli-Response-Cache` header, returning the time-to-live the
/// last one asked for (`None` without one, or for zero or garbage).
pub(crate) fn take_ttl(headers: &mut Vec<(String, String)>) -> Option<Duration> {
    let mut ttl = None;
    headers.retain(|(k, v)| {
        if !k.eq_ignore_ascii_case(HEADER) {
            return true;
        }
        ttl = v
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|s| *s > 0)
            .map(|s| Duration::from_secs(s).min(MAX_TTL));
        false
    });
    ttl
}

fn eligible(method: &str, headers: &hyper::HeaderMap) -> bool {
    method == "GET"
        && !headers.contains_key(hyper::header::RANGE)
        && !headers.contains_key(hyper::header::UPGRADE)
        && !headers.contains_key(hyper::header::CONTENT_LENGTH)
        && !headers.contains_key(hyper::header::TRANSFER_ENCODING)
        && !directives(
            headers.get_all(hyper::header::CACHE_CONTROL).iter(),
            &["no-cache", "no-store"],
        )
        && !headers
            .get(hyper::header::PRAGMA)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("no-cache"))
}

fn keepable(response: &ResponseData) -> bool {
    if response.status != 200 || response.body.len() > MAX_BODY {
        return false;
    }
    let mut cache_control = Vec::new();
    for (k, v) in &response.headers {
        if k.eq_ignore_ascii_case("cache-control") {
            cache_control.push(v.as_str());
        }
        // A new framework session (a flash consumed, a token rotated) belongs
        // to this one response, not to the requests that would replay it.
        if k.eq_ignore_ascii_case("set-cookie") && cookie_name(v).eq_ignore_ascii_case("session_id")
        {
            return false;
        }
    }
    !cache_control
        .iter()
        .any(|v| has_directive(v, &["no-store", "no-transform"]))
}

fn cookie_name(set_cookie: &str) -> &str {
    set_cookie.split(['=', ';']).next().unwrap_or("").trim()
}

fn directives<'a>(
    values: impl Iterator<Item = &'a hyper::header::HeaderValue>,
    rejected: &[&str],
) -> bool {
    values
        .filter_map(|v| v.to_str().ok())
        .any(|v| has_directive(v, rejected))
}

fn has_directive(value: &str, rejected: &[&str]) -> bool {
    value.split(',').any(|directive| {
        let name = directive.trim().split('=').next().unwrap_or("").trim();
        rejected.iter().any(|r| name.eq_ignore_ascii_case(r))
    })
}

/// The request line and every keyed header, each length-prefixed so no two
/// requests can spell the same key. `None` past 16 KiB.
fn request_key(path: &str, raw_query: Option<&str>, headers: &hyper::HeaderMap) -> Option<String> {
    fn field(key: &mut String, value: &str) {
        key.push_str(&value.len().to_string());
        key.push(':');
        key.push_str(value);
    }
    let mut key = String::with_capacity(512);
    field(&mut key, path);
    field(&mut key, raw_query.unwrap_or(""));
    let mut names: Vec<&str> = headers
        .keys()
        .map(|name| name.as_str())
        .filter(|name| !UNKEYED.contains(name))
        .collect();
    names.sort_unstable();
    for name in names {
        field(&mut key, name);
        for value in headers.get_all(name) {
            field(&mut key, &String::from_utf8_lossy(value.as_bytes()));
        }
        key.push(';');
    }
    (key.len() <= MAX_KEY).then_some(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> hyper::HeaderMap {
        let mut map = hyper::HeaderMap::new();
        for (k, v) in pairs {
            map.append(
                hyper::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                hyper::header::HeaderValue::from_str(v).unwrap(),
            );
        }
        map
    }

    fn reply(status: u16, extra: &[(&str, &str)]) -> ResponseData {
        ResponseData {
            status,
            headers: extra
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: Bytes::from_static(b"<html>page</html>"),
        }
    }

    #[test]
    fn the_key_holds_cookies_and_leaves_out_validators_and_encoding() {
        let a = request_key(
            "/rooms/1",
            None,
            &headers(&[
                ("cookie", "s=1"),
                ("accept-encoding", "gzip"),
                ("if-none-match", "\"x\""),
            ]),
        );
        let b = request_key("/rooms/1", None, &headers(&[("cookie", "s=1")]));
        let other_user = request_key("/rooms/1", None, &headers(&[("cookie", "s=2")]));
        let no_cookie = request_key("/rooms/1", None, &headers(&[]));
        assert_eq!(a, b);
        assert_ne!(a, other_user);
        assert_ne!(a, no_cookie);
    }

    #[test]
    fn the_key_cannot_be_spelled_two_ways() {
        let a = request_key("/a", Some("b=1"), &headers(&[]));
        let b = request_key("/a?b=1", None, &headers(&[]));
        assert_ne!(a, b);
        let c = request_key(
            "/",
            None,
            &headers(&[("turbo-frame", "x"), ("accept", "y")]),
        );
        let d = request_key("/", None, &headers(&[("turbo-frame", "x;accept:y")]));
        assert_ne!(c, d);
    }

    #[test]
    fn only_a_plain_get_is_eligible() {
        assert!(eligible("GET", &headers(&[("cookie", "s=1")])));
        assert!(!eligible("HEAD", &headers(&[])));
        assert!(!eligible("POST", &headers(&[])));
        assert!(!eligible("GET", &headers(&[("range", "bytes=0-1")])));
        assert!(!eligible(
            "GET",
            &headers(&[("cache-control", "max-age=0, no-cache")])
        ));
        assert!(!eligible("GET", &headers(&[("pragma", "no-cache")])));
        assert!(!eligible("GET", &headers(&[("content-length", "3")])));
    }

    #[test]
    fn the_header_is_always_stripped_and_read_as_a_bounded_ttl() {
        let mut h = vec![
            ("Content-Type".to_string(), "text/html".to_string()),
            ("Soli-Response-Cache".to_string(), "15".to_string()),
        ];
        assert_eq!(take_ttl(&mut h), Some(Duration::from_secs(15)));
        assert_eq!(h.len(), 1);
        let mut long = vec![("soli-response-cache".to_string(), "99999".to_string())];
        assert_eq!(take_ttl(&mut long), Some(MAX_TTL));
        let mut zero = vec![("soli-response-cache".to_string(), "0".to_string())];
        assert_eq!(take_ttl(&mut zero), None);
        assert!(zero.is_empty());
    }

    #[test]
    fn only_a_reusable_200_is_kept() {
        assert!(keepable(&reply(
            200,
            &[("Set-Cookie", "last_room=1; Path=/")]
        )));
        assert!(!keepable(&reply(302, &[])));
        assert!(!keepable(&reply(
            200,
            &[("Set-Cookie", "session_id=abc; Path=/")]
        )));
        assert!(!keepable(&reply(
            200,
            &[("Cache-Control", "private, no-store")]
        )));
        let mut big = reply(200, &[]);
        big.body = Bytes::from(vec![b'x'; MAX_BODY + 1]);
        assert!(!keepable(&big));
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn a_commit_retires_every_entry_and_a_racing_one_keeps_a_reply_out() {
        let dir = std::env::temp_dir().join(format!("soli-response-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.sqlite3");
        let _ = std::fs::remove_file(&path);
        let writer = rusqlite::Connection::open(&path).unwrap();
        writer
            .execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t (x INTEGER);")
            .unwrap();
        let store = Store::open(&path, 1 << 20).unwrap();
        let ticket = |key: &str| Ticket {
            key: key.to_string(),
            generation: store.version().unwrap(),
        };

        let first = ticket("k");
        assert!(store.get(&first).is_none());
        store.put(ticket("k"), &reply(200, &[]), Duration::from_secs(60));
        assert_eq!(
            &store.get(&ticket("k")).unwrap().body[..],
            b"<html>page</html>"
        );

        // Another connection commits: the entry no longer answers.
        writer.execute("INSERT INTO t VALUES (1)", []).unwrap();
        assert!(store.get(&ticket("k")).is_none());

        // A reply rendered under the old version is not filed under the new.
        let stale = Ticket {
            key: "k".to_string(),
            generation: first.generation,
        };
        store.put(stale, &reply(200, &[]), Duration::from_secs(60));
        assert!(store.get(&ticket("k")).is_none());

        // An expired entry does not answer either.
        store.put(ticket("e"), &reply(200, &[]), Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(5));
        assert!(store.get(&ticket("e")).is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn past_its_size_the_oldest_entries_go_first() {
        let dir =
            std::env::temp_dir().join(format!("soli-response-cache-size-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("app.sqlite3");
        let _ = std::fs::remove_file(&path);
        rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE t (x INTEGER);")
            .unwrap();
        let store = Store::open(&path, 100).unwrap();
        let generation = store.version().unwrap();
        let t = |key: &str| Ticket {
            key: key.to_string(),
            generation,
        };
        for key in ["a", "b", "c", "d", "e", "f"] {
            store.put(t(key), &reply(200, &[]), Duration::from_secs(60));
        }
        assert!(store.get(&t("a")).is_none());
        assert!(store.get(&t("f")).is_some());
        assert!(store.entries.read().unwrap().bytes <= 100);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
