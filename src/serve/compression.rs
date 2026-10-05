//! Response compression: `SOLI_COMPRESS=gzip` gzips what an application
//! answers, the way `Rack::Deflater` does in a Rails app's `config.ru`, and
//! keeps what it compressed so a page served again is not deflated again.
//!
//! Off by default, like the proxy's `[compression]`: it costs CPU on every
//! miss, it changes what caches see (`Vary`, weak ETags), and compressing a
//! page that reflects request input next to a secret is what BREACH
//! exploits. Turn it on when the app is served without a compressing front
//! server, or when it can compress more cheaply than one: an app's pages repeat
//! (a cached page is the same string from one request to the next), and a
//! front server, which sees each response for the first time, cannot know it.
//!
//! - [`settings`] — the environment: `SOLI_COMPRESS`, `SOLI_COMPRESS_LEVEL`
//!   (1–9, default 6, zlib's and so `Rack::Deflater`'s), `SOLI_COMPRESS_MIN_BYTES`
//!   (default 1024), `SOLI_COMPRESS_CACHE_MB` (default 64, 0 keeps nothing).
//! - [`accepts_gzip`] — `Accept-Encoding` negotiation, q-values honoured.
//! - [`compressible`] — which responses are compressed at all.
//! - [`gzip`] — the compressed body, from the cache when it has it.
//!
//! The cache is shared by every worker. A body is found first by its buffer:
//! a worker that serves a page it keeps returns the same string each time, and
//! the cache holds a reference to that buffer, so while the entry lives the
//! buffer cannot be freed, reused or changed (a shared `EcoString` is copied on
//! write). Otherwise by a hash of the bytes, and a hit is then compared byte
//! for byte with what was compressed: a hash collision costs a compression,
//! never someone else's page.

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, LazyLock, Mutex};

use bytes::Bytes;
use flate2::write::GzEncoder;
use flate2::Compression;

/// Compression as the environment configures it; `None` when it is off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Settings {
    pub level: u32,
    pub min_bytes: usize,
    pub cache_bytes: usize,
}

static SETTINGS: LazyLock<Option<Settings>> =
    LazyLock::new(|| settings_from(|name| std::env::var(name).ok()));

/// The process's settings, read once.
pub(crate) fn settings() -> Option<&'static Settings> {
    #[cfg(test)]
    if let Some(overridden) = TEST_SETTINGS.with(|t| t.get()) {
        return overridden;
    }
    SETTINGS.as_ref()
}

#[cfg(test)]
thread_local! {
    static TEST_SETTINGS: std::cell::Cell<Option<Option<&'static Settings>>> =
        const { std::cell::Cell::new(None) };
}

/// Tests: the settings this thread sees, in place of the environment's.
#[cfg(test)]
pub(crate) fn set_for_test(settings: Option<Settings>) {
    let leaked = settings.map(|s| &*Box::leak(Box::new(s)));
    TEST_SETTINGS.with(|t| t.set(Some(leaked)));
}

fn settings_from(var: impl Fn(&str) -> Option<String>) -> Option<Settings> {
    let mode = var("SOLI_COMPRESS")?;
    if !matches!(
        mode.trim().to_ascii_lowercase().as_str(),
        "gzip" | "1" | "true" | "on" | "yes"
    ) {
        return None;
    }
    let number = |name: &str| var(name).and_then(|v| v.trim().parse::<usize>().ok());
    Some(Settings {
        level: number("SOLI_COMPRESS_LEVEL").map_or(6, |l| l.clamp(1, 9) as u32),
        min_bytes: number("SOLI_COMPRESS_MIN_BYTES").unwrap_or(1024),
        cache_bytes: number("SOLI_COMPRESS_CACHE_MB").unwrap_or(64) << 20,
    })
}

/// Whether an `Accept-Encoding` value lets the response be gzipped: `gzip`
/// (or `x-gzip`) with a non-zero q-value, or else `*` with one.
pub(crate) fn accepts_gzip(accept_encoding: &str) -> bool {
    let mut star = None;
    for item in accept_encoding.split(',') {
        let mut parts = item.split(';');
        let coding = parts.next().unwrap_or("").trim();
        let q = parts
            .filter_map(|p| {
                p.trim()
                    .strip_prefix("q=")
                    .or_else(|| p.trim().strip_prefix("Q="))
            })
            .find_map(|q| q.trim().parse::<f32>().ok())
            .unwrap_or(1.0);
        if coding.eq_ignore_ascii_case("gzip") || coding.eq_ignore_ascii_case("x-gzip") {
            return q > 0.0;
        }
        if coding == "*" {
            star = Some(q > 0.0);
        }
    }
    star.unwrap_or(false)
}

/// Whether a response is one compression applies to, whatever the client
/// accepts (those get `Vary: Accept-Encoding`): a status with a body, a text
/// type, no encoding yet, no `Cache-Control: no-transform`, and a body of at
/// least `min_bytes`.
pub(crate) fn compressible(
    status: u16,
    headers: &[(String, String)],
    body_len: usize,
    settings: &Settings,
) -> bool {
    if body_len == 0 || body_len < settings.min_bytes {
        return false;
    }
    if (100..200).contains(&status) || matches!(status, 204 | 206 | 304) {
        return false;
    }
    let mut text = false;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("content-encoding") {
            return false;
        }
        if name.eq_ignore_ascii_case("cache-control")
            && value.to_ascii_lowercase().contains("no-transform")
        {
            return false;
        }
        if name.eq_ignore_ascii_case("content-type") {
            text = compressible_type(value);
        }
    }
    text
}

fn compressible_type(content_type: &str) -> bool {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if mime == "text/event-stream" {
        return false;
    }
    mime.starts_with("text/")
        || mime.ends_with("+json")
        || mime.ends_with("+xml")
        || matches!(
            mime.as_str(),
            "application/json"
                | "application/javascript"
                | "application/x-javascript"
                | "application/xml"
                | "application/wasm"
                | "image/svg+xml"
                | "image/x-icon"
                | "image/vnd.microsoft.icon"
                | "font/ttf"
                | "font/otf"
        )
}

/// Whether a `Vary` value already covers `Accept-Encoding` (or everything).
pub(crate) fn varies_on_encoding(vary: &str) -> bool {
    vary.split(',')
        .map(str::trim)
        .any(|v| v == "*" || v.eq_ignore_ascii_case("accept-encoding"))
}

/// What the cache keeps for one compressed body.
struct Entry {
    /// Buffers known to hold exactly the bytes that were compressed: a body
    /// found at one of them is a hit without hashing or comparing. Several,
    /// because each worker keeps its own copy of the same page.
    originals: Vec<Bytes>,
    gzipped: Bytes,
}

/// At most this many buffers are remembered per entry (one per worker,
/// typically).
const MAX_ORIGINALS: usize = 32;

impl Entry {
    fn cost(&self) -> usize {
        self.gzipped.len() + self.originals.iter().map(Bytes::len).sum::<usize>()
    }
}

struct Cache {
    /// Entries by the hash of their bytes, least recently used first out.
    entries: lru::LruCache<u64, Arc<Entry>>,
    /// Buffers' (address, length) to the hash of their entry.
    by_buffer: HashMap<(usize, usize), u64>,
    bytes: usize,
}

fn buffer_id(body: &Bytes) -> (usize, usize) {
    (body.as_ptr() as usize, body.len())
}

impl Cache {
    fn new() -> Self {
        Cache {
            entries: lru::LruCache::unbounded(),
            by_buffer: HashMap::new(),
            bytes: 0,
        }
    }

    /// The compressed body for `body` if `body` is one of the buffers an entry
    /// holds.
    fn by_buffer(&mut self, body: &Bytes) -> Option<Bytes> {
        let id = buffer_id(body);
        let hash = *self.by_buffer.get(&id)?;
        let entry = self.entries.get(&hash)?;
        entry
            .originals
            .iter()
            .any(|o| buffer_id(o) == id)
            .then(|| entry.gzipped.clone())
    }

    fn by_hash(&mut self, hash: u64) -> Option<Arc<Entry>> {
        self.entries.get(&hash).cloned()
    }

    /// Remember that `body` holds the bytes of the entry at `hash`.
    fn add_buffer(&mut self, hash: u64, body: &Bytes, limit: usize) {
        let Some(entry) = self.entries.get_mut(&hash) else {
            return;
        };
        if entry.originals.len() >= MAX_ORIGINALS {
            return;
        }
        let mut grown = Entry {
            originals: entry.originals.clone(),
            gzipped: entry.gzipped.clone(),
        };
        grown.originals.push(body.clone());
        self.bytes += body.len();
        *entry = Arc::new(grown);
        self.by_buffer.insert(buffer_id(body), hash);
        self.evict(limit);
    }

    fn insert(&mut self, hash: u64, body: &Bytes, gzipped: Bytes, limit: usize) {
        let entry = Entry {
            originals: vec![body.clone()],
            gzipped,
        };
        if entry.cost() > limit / 4 {
            return;
        }
        self.bytes += entry.cost();
        if let Some(old) = self.entries.put(hash, Arc::new(entry)) {
            self.forget(&old);
        }
        self.by_buffer.insert(buffer_id(body), hash);
        self.evict(limit);
    }

    fn evict(&mut self, limit: usize) {
        while self.bytes > limit {
            match self.entries.pop_lru() {
                Some((_, old)) => self.forget(&old),
                None => break,
            }
        }
    }

    fn forget(&mut self, entry: &Entry) {
        self.bytes -= entry.cost();
        for original in &entry.originals {
            self.by_buffer.remove(&buffer_id(original));
        }
    }
}

static CACHE: LazyLock<Mutex<Cache>> = LazyLock::new(|| Mutex::new(Cache::new()));
static HASHER: LazyLock<ahash::RandomState> = LazyLock::new(ahash::RandomState::new);

fn lock() -> std::sync::MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// `body` gzipped at the configured level, from the cache when it holds it.
pub(crate) fn gzip(body: &Bytes, settings: &Settings) -> std::io::Result<Bytes> {
    if settings.cache_bytes == 0 {
        return deflate(body, settings.level);
    }
    if let Some(hit) = lock().by_buffer(body) {
        return Ok(hit);
    }
    let hash = HASHER.hash_one(&body[..]);
    let candidate = lock().by_hash(hash);
    if let Some(entry) = candidate {
        // Compared outside the lock: a large page takes microseconds.
        if entry.originals[0][..] == body[..] {
            lock().add_buffer(hash, body, settings.cache_bytes);
            return Ok(entry.gzipped.clone());
        }
    }
    let gzipped = deflate(body, settings.level)?;
    lock().insert(hash, body, gzipped.clone(), settings.cache_bytes);
    Ok(gzipped)
}

fn deflate(body: &[u8], level: u32) -> std::io::Result<Bytes> {
    let mut encoder = GzEncoder::new(
        Vec::with_capacity(body.len() / 4 + 64),
        Compression::new(level),
    );
    encoder.write_all(body)?;
    Ok(encoder.finish()?.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::io::Read;

    fn on() -> Settings {
        Settings {
            level: 6,
            min_bytes: 1024,
            cache_bytes: 1 << 20,
        }
    }

    fn gunzip(b: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        GzDecoder::new(b).read_to_end(&mut out).unwrap();
        out
    }

    fn html() -> Vec<(String, String)> {
        vec![("Content-Type".into(), "text/html; charset=utf-8".into())]
    }

    #[test]
    fn settings_are_off_unless_asked_for() {
        assert_eq!(settings_from(|_| None), None);
        assert_eq!(
            settings_from(|n| (n == "SOLI_COMPRESS").then(|| "off".into())),
            None
        );
        let s = settings_from(|n| match n {
            "SOLI_COMPRESS" => Some("gzip".into()),
            "SOLI_COMPRESS_LEVEL" => Some("12".into()),
            "SOLI_COMPRESS_CACHE_MB" => Some("0".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!((s.level, s.min_bytes, s.cache_bytes), (9, 1024, 0));
    }

    #[test]
    fn accept_encoding_honours_q_values() {
        assert!(accepts_gzip("gzip"));
        assert!(accepts_gzip("deflate, gzip;q=0.5, br"));
        assert!(accepts_gzip("GZIP"));
        assert!(!accepts_gzip("gzip;q=0"));
        assert!(!accepts_gzip("identity"));
        assert!(!accepts_gzip(""));
        assert!(accepts_gzip("*"));
        assert!(!accepts_gzip("*;q=0"));
        assert!(!accepts_gzip("gzip;q=0, *"));
    }

    #[test]
    fn only_text_bodies_large_enough_are_compressible() {
        let s = on();
        assert!(compressible(200, &html(), 2000, &s));
        assert!(!compressible(200, &html(), 100, &s));
        assert!(!compressible(304, &html(), 2000, &s));
        assert!(!compressible(204, &html(), 2000, &s));
        let png = vec![("Content-Type".into(), "image/png".into())];
        assert!(!compressible(200, &png, 2000, &s));
        let json = vec![("content-type".into(), "application/vnd.api+json".into())];
        assert!(compressible(200, &json, 2000, &s));
        let sse = vec![("Content-Type".into(), "text/event-stream".into())];
        assert!(!compressible(200, &sse, 2000, &s));
        let mut encoded = html();
        encoded.push(("Content-Encoding".into(), "br".into()));
        assert!(!compressible(200, &encoded, 2000, &s));
        let mut no_transform = html();
        no_transform.push(("Cache-Control".into(), "private, no-transform".into()));
        assert!(!compressible(200, &no_transform, 2000, &s));
        assert!(!compressible(200, &[], 2000, &s));
    }

    #[test]
    fn a_body_round_trips_and_repeats_come_from_the_cache() {
        let s = on();
        let body = Bytes::from("<p>hello cache</p>".repeat(200));
        let first = gzip(&body, &s).unwrap();
        assert_eq!(gunzip(&first), &body[..]);
        // Same buffer: the very same compressed bytes come back.
        let again = gzip(&body, &s).unwrap();
        assert_eq!(again.as_ptr(), first.as_ptr());
        // Same bytes in another buffer: found by hash, verified, shared.
        let copy = Bytes::from(body.to_vec());
        let other = gzip(&copy, &s).unwrap();
        assert_eq!(other.as_ptr(), first.as_ptr());
        // Different bytes of the same length are compressed on their own.
        let mut changed = body.to_vec();
        changed[10] = b'X';
        let changed = Bytes::from(changed);
        let third = gzip(&changed, &s).unwrap();
        assert_eq!(gunzip(&third), &changed[..]);
    }

    #[test]
    fn the_cache_stays_within_its_budget() {
        let mut cache = Cache::new();
        let limit = 64 * 1024;
        for i in 0..200u64 {
            let body = Bytes::from(vec![i as u8; 4096]);
            let gz = deflate(&body, 1).unwrap();
            cache.insert(i, &body, gz, limit);
            assert!(cache.bytes <= limit);
        }
        assert_eq!(cache.by_buffer.len(), cache.entries.len());
        // An entry larger than a quarter of the budget is not kept.
        let big = Bytes::from(vec![7u8; limit]);
        let gz = deflate(&big, 1).unwrap();
        cache.insert(999, &big, gz, limit);
        assert!(cache.by_hash(999).is_none());
    }

    #[test]
    fn a_disabled_cache_still_compresses() {
        let s = Settings {
            cache_bytes: 0,
            ..on()
        };
        let body = Bytes::from("x".repeat(4096));
        assert_eq!(gunzip(&gzip(&body, &s).unwrap()), &body[..]);
    }
}
