//! Resumable uploads: the tus 1.0.0 protocol (creation, termination,
//! expiration), for files too big to send in one request.
//!
//! Off until the app calls `resumable_uploads()` in `config/routes.sl`. The
//! endpoint is not a route: it is answered before routing, like
//! `/live/upload`, because a `PATCH` carries raw bytes that the controller
//! layer's request model (a UTF-8 `body`) cannot hold.
//!
//! **What is bounded.** Bytes go to disk as each `PATCH` arrives, so memory
//! is one chunk (the client chooses its size; the request body cap still
//! applies to each), never the whole file. The disk is bounded instead: a
//! per-upload size cap, a per-session and a global cap on stored uploads
//! (finished ones included, until `tus_take` consumes them), and expiry of
//! anything left for a day.
//!
//! An upload belongs to the session that created it: a session the store
//! knows, not merely a cookie that looks like one, so rotating the cookie
//! value neither dodges the per-session cap nor reaches another's upload. A finished upload is
//! turned into an attachment with `tus_take(id)` + `attach_<field>`, which
//! moves the file into the attachment store without loading it into memory
//! (disk and S3 services, no image transform).

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use super::tenant::TenantValue;
use super::{header_str, RequestData, ResponseData};

pub const TUS_VERSION: &str = "1.0.0";
const OFFSET_CONTENT_TYPE: &str = "application/offset+octet-stream";

const DEFAULT_PREFIX: &str = "/tus";
const DEFAULT_MAX_SIZE: u64 = 1024 * 1024 * 1024;
const DEFAULT_MAX_ACTIVE: usize = 200;
const DEFAULT_MAX_PER_OWNER: usize = 20;
const DEFAULT_EXPIRES_SECS: i64 = 24 * 3600;

/// Settings from `resumable_uploads({...})`.
#[derive(Clone, Debug)]
pub struct Config {
    /// URL prefix the protocol answers on.
    pub prefix: String,
    /// Largest file accepted (bytes).
    pub max_size: u64,
    /// Stored uploads (unfinished, or finished but not yet taken) across all
    /// sessions.
    pub max_active: usize,
    /// Stored uploads per session.
    pub max_per_owner: usize,
    /// Seconds an upload is kept.
    pub expires_secs: i64,
    /// Where the bytes live.
    pub dir: PathBuf,
    /// Refuse callers without a session cookie.
    pub require_session: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            prefix: DEFAULT_PREFIX.to_string(),
            max_size: DEFAULT_MAX_SIZE,
            max_active: DEFAULT_MAX_ACTIVE,
            max_per_owner: DEFAULT_MAX_PER_OWNER,
            expires_secs: DEFAULT_EXPIRES_SECS,
            dir: std::env::var("SOLI_TUS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("./storage/tus")),
            require_session: true,
        }
    }
}

/// Per application, so one app's opt-in never opens another's endpoint.
static CONFIG: TenantValue<Option<Config>> = TenantValue::new(|| None);

/// Uploads with a `PATCH` in flight. A second concurrent writer to the same
/// upload gets `423 Locked` instead of interleaving bytes.
static BUSY: TenantValue<HashSet<String>> = TenantValue::new(HashSet::new);

pub fn enable(config: Config) {
    CONFIG.write(|slot| *slot = Some(config));
}

pub fn config() -> Option<Config> {
    CONFIG.read(|slot| slot.clone())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Meta {
    id: String,
    owner: Option<String>,
    length: u64,
    filename: String,
    content_type: String,
    metadata: BTreeMap<String, String>,
    created_at: i64,
    expires_at: i64,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn part_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.part"))
}

fn meta_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// Ids are minted here (UUIDs); anything else is not ours.
fn valid_id(id: &str) -> bool {
    id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

fn read_meta(dir: &Path, id: &str) -> Option<Meta> {
    if !valid_id(id) {
        return None;
    }
    let bytes = fs::read(meta_path(dir, id)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn offset_of(dir: &Path, id: &str) -> u64 {
    fs::metadata(part_path(dir, id))
        .map(|m| m.len())
        .unwrap_or(0)
}

fn remove_upload(dir: &Path, id: &str) {
    let _ = fs::remove_file(part_path(dir, id));
    let _ = fs::remove_file(meta_path(dir, id));
}

/// All stored metadata, dropping (and deleting) whatever expired.
fn list_uploads(dir: &Path) -> Vec<Meta> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let now = now();
    let mut live = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(id) = name.strip_suffix(".json") else {
            continue;
        };
        match read_meta(dir, id) {
            Some(meta) if meta.expires_at > now => live.push(meta),
            Some(meta) => remove_upload(dir, &meta.id),
            None => {}
        }
    }
    live
}

/// `Upload-Metadata: filename dGVzdC50eHQ=,filetype aW1hZ2UvcG5n`
fn parse_metadata(header: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for pair in header.split(',') {
        let mut parts = pair.trim().splitn(2, ' ');
        let Some(key) = parts.next().filter(|k| !k.is_empty()) else {
            continue;
        };
        let value = parts
            .next()
            .and_then(|v| STANDARD.decode(v.trim()).ok())
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .unwrap_or_default();
        out.insert(key.to_string(), value);
    }
    out
}

fn format_metadata(map: &BTreeMap<String, String>) -> String {
    map.iter()
        .map(|(k, v)| format!("{k} {}", STANDARD.encode(v)))
        .collect::<Vec<_>>()
        .join(",")
}

/// A name safe to keep: no path, no control characters, bounded.
fn safe_filename(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        "upload".to_string()
    } else {
        cleaned
    }
}

fn response(status: u16, headers: Vec<(&str, String)>) -> ResponseData {
    let mut all = vec![("Tus-Resumable".to_string(), TUS_VERSION.to_string())];
    all.extend(headers.into_iter().map(|(k, v)| (k.to_string(), v)));
    ResponseData {
        status,
        headers: all,
        body: Bytes::new(),
    }
}

fn error(status: u16, message: &str) -> ResponseData {
    let mut r = response(
        status,
        vec![("Content-Type", "text/plain; charset=utf-8".to_string())],
    );
    r.body = Bytes::from(message.to_string());
    r
}

/// Answer a tus request, or `None` when it is not one (the feature is off, or
/// the path is not under the prefix).
pub(crate) fn handle(data: &RequestData) -> Option<ResponseData> {
    let config = config()?;
    let path = data.path.as_str();
    let rest = path.strip_prefix(config.prefix.as_str())?;
    let id = match rest {
        "" | "/" => None,
        r if r.starts_with('/') => Some(r[1..].trim_end_matches('/')),
        _ => return None,
    };

    let method = data.method.as_ref();

    if method == "OPTIONS" {
        return Some(response(
            204,
            vec![
                ("Tus-Version", TUS_VERSION.to_string()),
                ("Tus-Max-Size", config.max_size.to_string()),
                (
                    "Tus-Extension",
                    "creation,termination,expiration".to_string(),
                ),
            ],
        ));
    }

    // The protocol version must match on everything but OPTIONS.
    if header_str(&data.headers, "tus-resumable") != Some(TUS_VERSION) {
        return Some(response(
            412,
            vec![("Tus-Version", TUS_VERSION.to_string())],
        ));
    }
    // Resolved against the session store, so only a live session owns
    // uploads: a made-up cookie value is no session at all.
    let owner = crate::interpreter::builtins::session::existing_session_id_from_cookie(
        data.headers.get("cookie").and_then(|v| v.to_str().ok()),
    );
    if config.require_session && owner.is_none() {
        return Some(error(401, "sign in (a session cookie) to upload"));
    }

    Some(match (method, id) {
        ("POST", None) => create(data, &config, owner.as_deref()),
        ("HEAD", Some(id)) => head(&config, id, owner.as_deref()),
        ("PATCH", Some(id)) => patch(data, &config, id, owner.as_deref()),
        ("DELETE", Some(id)) => terminate(&config, id, owner.as_deref()),
        _ => error(405, "method not allowed"),
    })
}

fn owns(meta: &Meta, owner: Option<&str>) -> bool {
    match &meta.owner {
        Some(mine) => owner == Some(mine.as_str()),
        None => true,
    }
}

fn create(data: &RequestData, config: &Config, owner: Option<&str>) -> ResponseData {
    let Some(length) =
        header_str(&data.headers, "upload-length").and_then(|v| v.parse::<u64>().ok())
    else {
        return error(400, "Upload-Length is required");
    };
    if length > config.max_size {
        return error(413, "file is larger than this server accepts");
    }
    let metadata = header_str(&data.headers, "upload-metadata")
        .map(parse_metadata)
        .unwrap_or_default();

    if fs::create_dir_all(&config.dir).is_err() {
        return error(500, "upload storage is unavailable");
    }
    // A finished upload holds its disk until `tus_take` consumes it or it
    // expires, so it counts exactly like one still arriving.
    let live = list_uploads(&config.dir);
    if live.len() >= config.max_active {
        return error(503, "too many uploads in progress, retry shortly");
    }
    if live.iter().filter(|m| m.owner.as_deref() == owner).count() >= config.max_per_owner {
        return error(429, "too many stored uploads for this session");
    }

    let id = uuid::Uuid::new_v4().to_string();
    let created_at = now();
    let filename = safe_filename(metadata.get("filename").map(String::as_str).unwrap_or(""));
    let content_type = metadata
        .get("filetype")
        .or_else(|| metadata.get("content_type"))
        .cloned()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let meta = Meta {
        id: id.clone(),
        owner: owner.map(str::to_string),
        length,
        filename,
        content_type,
        metadata,
        created_at,
        expires_at: created_at + config.expires_secs,
    };
    if fs::File::create(part_path(&config.dir, &id)).is_err()
        || fs::write(
            meta_path(&config.dir, &id),
            serde_json::to_vec(&meta).unwrap_or_default(),
        )
        .is_err()
    {
        remove_upload(&config.dir, &id);
        return error(500, "could not create the upload");
    }

    response(
        201,
        vec![
            ("Location", format!("{}/{}", config.prefix, id)),
            ("Upload-Expires", http_date(meta.expires_at)),
        ],
    )
}

fn lookup(config: &Config, id: &str, owner: Option<&str>) -> Result<Meta, ResponseData> {
    match read_meta(&config.dir, id) {
        // Someone else's upload is indistinguishable from a missing one.
        Some(meta) if owns(&meta, owner) => {
            if meta.expires_at <= now() {
                remove_upload(&config.dir, id);
                Err(error(410, "upload expired"))
            } else {
                Ok(meta)
            }
        }
        _ => Err(error(404, "no such upload")),
    }
}

fn head(config: &Config, id: &str, owner: Option<&str>) -> ResponseData {
    let meta = match lookup(config, id, owner) {
        Ok(meta) => meta,
        Err(response) => return response,
    };
    response(
        200,
        vec![
            ("Upload-Offset", offset_of(&config.dir, id).to_string()),
            ("Upload-Length", meta.length.to_string()),
            ("Upload-Metadata", format_metadata(&meta.metadata)),
            ("Upload-Expires", http_date(meta.expires_at)),
            ("Cache-Control", "no-store".to_string()),
        ],
    )
}

fn patch(data: &RequestData, config: &Config, id: &str, owner: Option<&str>) -> ResponseData {
    let meta = match lookup(config, id, owner) {
        Ok(meta) => meta,
        Err(response) => return response,
    };
    let content_type = header_str(&data.headers, "content-type")
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if content_type != OFFSET_CONTENT_TYPE {
        return error(415, "Content-Type must be application/offset+octet-stream");
    }
    let Some(claimed) =
        header_str(&data.headers, "upload-offset").and_then(|v| v.parse::<u64>().ok())
    else {
        return error(400, "Upload-Offset is required");
    };

    let chunk: Bytes = data
        .multipart_files
        .as_ref()
        .and_then(|files| files.first())
        .map(|file| file.data.clone())
        .unwrap_or_default();

    // One writer at a time; the offset check and the append are then one step.
    let acquired = BUSY.write(|busy| busy.insert(id.to_string()));
    if !acquired {
        return error(423, "another request is writing this upload");
    }
    let outcome = append_chunk(config, &meta, claimed, &chunk);
    BUSY.write(|busy| {
        busy.remove(id);
    });
    match outcome {
        Ok(new_offset) => response(
            204,
            vec![
                ("Upload-Offset", new_offset.to_string()),
                ("Upload-Expires", http_date(meta.expires_at)),
            ],
        ),
        Err(failure) => failure,
    }
}

fn append_chunk(
    config: &Config,
    meta: &Meta,
    claimed: u64,
    chunk: &[u8],
) -> Result<u64, ResponseData> {
    let current = offset_of(&config.dir, &meta.id);
    if claimed != current {
        return Err(response(409, vec![("Upload-Offset", current.to_string())]));
    }
    if current + chunk.len() as u64 > meta.length {
        return Err(error(413, "chunk goes past the declared Upload-Length"));
    }
    if chunk.is_empty() {
        return Ok(current);
    }
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(part_path(&config.dir, &meta.id))
        .map_err(|_| error(500, "could not open the upload"))?;
    file.write_all(chunk)
        .map_err(|_| error(500, "could not write the upload"))?;
    Ok(current + chunk.len() as u64)
}

fn terminate(config: &Config, id: &str, owner: Option<&str>) -> ResponseData {
    if let Err(response) = lookup(config, id, owner) {
        return response;
    }
    remove_upload(&config.dir, id);
    response(204, vec![])
}

/// RFC 7231 date for `Upload-Expires`.
fn http_date(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|dt| dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// What a finished upload looks like to application code.
// ---------------------------------------------------------------------------

/// A finished upload, as `tus_take` describes it.
#[derive(Clone, Debug)]
pub struct Finished {
    pub id: String,
    pub filename: String,
    pub content_type: String,
    pub size: u64,
    pub path: PathBuf,
}

/// The finished upload `id`, if it exists, belongs to `owner`, and every byte
/// has arrived.
pub fn finished(id: &str, owner: Option<&str>) -> Result<Finished, String> {
    let config = config().ok_or("resumable_uploads() is not enabled")?;
    let meta = read_meta(&config.dir, id).ok_or("no such upload")?;
    if !owns(&meta, owner) {
        return Err("no such upload".to_string());
    }
    if meta.expires_at <= now() {
        return Err("upload expired".to_string());
    }
    let size = offset_of(&config.dir, id);
    if size != meta.length {
        return Err(format!(
            "upload is not finished ({size} of {} bytes)",
            meta.length
        ));
    }
    Ok(Finished {
        id: meta.id,
        filename: meta.filename,
        content_type: meta.content_type,
        size,
        path: part_path(&config.dir, id),
    })
}

/// Delete an upload's bytes and metadata.
pub fn discard(id: &str) {
    if let Some(config) = config() {
        if valid_id(id) {
            remove_upload(&config.dir, id);
        }
    }
}

// ---------------------------------------------------------------------------
// Builtins: `resumable_uploads`, `tus_take`, `tus_discard`.
// ---------------------------------------------------------------------------

/// Bytes `tus_take(id, true)` will load into memory for a service that cannot
/// stream (SoliDB blobs) or an image transform. Bigger files use disk or S3.
const MAX_INLINE_BYTES: u64 = 64 * 1024 * 1024;

fn option_i64(
    opts: &crate::interpreter::value::HashPairs,
    key: &str,
) -> Result<Option<i64>, String> {
    use crate::interpreter::value::{HashKey, Value};
    match opts.get(&HashKey::String(key.into())) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Int(n)) if *n > 0 => Ok(Some(*n)),
        Some(_) => Err(format!(
            "resumable_uploads: {key} must be a positive integer"
        )),
    }
}

fn build_config(opts: Option<&crate::interpreter::value::HashPairs>) -> Result<Config, String> {
    use crate::interpreter::value::{HashKey, Value};
    let mut config = Config::default();
    let Some(opts) = opts else {
        return Ok(config);
    };
    for key in opts.keys() {
        let HashKey::String(name) = key else {
            return Err("resumable_uploads: option names must be strings".to_string());
        };
        if ![
            "path",
            "max_size",
            "max_active",
            "max_per_owner",
            "expires_in",
            "dir",
            "require_session",
        ]
        .contains(&&**name)
        {
            return Err(format!(
                "resumable_uploads: unknown option {name:?} (path, max_size, max_active, max_per_owner, expires_in, dir, require_session)"
            ));
        }
    }
    if let Some(Value::String(path)) = opts.get(&HashKey::String("path".into())) {
        let path = path.trim_end_matches('/').to_string();
        if !path.starts_with('/') || path.len() < 2 || path.starts_with("/live") {
            return Err("resumable_uploads: path must be a URL path like \"/tus\"".to_string());
        }
        config.prefix = path;
    }
    if let Some(n) = option_i64(opts, "max_size")? {
        config.max_size = n as u64;
    }
    if let Some(n) = option_i64(opts, "max_active")? {
        config.max_active = n as usize;
    }
    if let Some(n) = option_i64(opts, "max_per_owner")? {
        config.max_per_owner = n as usize;
    }
    if let Some(n) = option_i64(opts, "expires_in")? {
        config.expires_secs = n;
    }
    if let Some(Value::String(dir)) = opts.get(&HashKey::String("dir".into())) {
        config.dir = PathBuf::from(dir.to_string());
    }
    if let Some(Value::Bool(required)) = opts.get(&HashKey::String("require_session".into())) {
        config.require_session = *required;
    }
    Ok(config)
}

pub fn register(env: &mut crate::interpreter::environment::Environment) {
    use crate::interpreter::value::{hash_from_pairs, NativeFunction, Value};

    env.define(
        "resumable_uploads".to_string(),
        Value::NativeFunction(NativeFunction::new("resumable_uploads", None, |args| {
            let opts = match args.first() {
                Some(Value::Hash(h)) => Some(h.borrow().clone()),
                Some(Value::Null) | None => None,
                Some(_) => return Err("resumable_uploads(options?) expects a hash".to_string()),
            };
            let config = build_config(opts.as_ref())?;
            fs::create_dir_all(&config.dir).map_err(|e| {
                format!(
                    "resumable_uploads: cannot create {}: {e}",
                    config.dir.display()
                )
            })?;
            let prefix = config.prefix.clone();
            enable(config);
            Ok(Value::String(prefix.into()))
        })),
    );

    env.define(
        "tus_take".to_string(),
        Value::NativeFunction(NativeFunction::new("tus_take", None, |args| {
            let id = match args.first() {
                Some(Value::String(id)) => id.to_string(),
                _ => return Err("tus_take(id, load?) expects the upload id".to_string()),
            };
            let load = matches!(args.get(1), Some(Value::Bool(true)));
            let owner = crate::interpreter::builtins::session::get_current_session_id();
            let done = finished(&id, owner.as_deref())?;
            let mut pairs = vec![
                ("filename", Value::String(done.filename.clone().into())),
                ("content_type", Value::String(done.content_type.clone().into())),
                ("size", Value::Int(done.size as i64)),
                ("tus_id", Value::String(done.id.clone().into())),
            ];
            if load {
                if done.size > MAX_INLINE_BYTES {
                    return Err(format!(
                        "upload is {} bytes; only {MAX_INLINE_BYTES} can be loaded into memory (use a disk or s3 attachment, which streams)",
                        done.size
                    ));
                }
                let bytes = fs::read(&done.path).map_err(|e| format!("tus_take: {e}"))?;
                pairs.push(("data", Value::String(STANDARD.encode(bytes).into())));
            }
            Ok(hash_from_pairs(pairs))
        })),
    );

    env.define(
        "tus_discard".to_string(),
        Value::NativeFunction(NativeFunction::new("tus_discard", Some(1), |args| {
            let id = match args.first() {
                Some(Value::String(id)) => id.to_string(),
                _ => return Err("tus_discard(id) expects the upload id".to_string()),
            };
            let owner = crate::interpreter::builtins::session::get_current_session_id();
            let Some(config) = config() else {
                return Ok(Value::Bool(false));
            };
            match read_meta(&config.dir, &id) {
                Some(meta) if owns(&meta, owner.as_deref()) => {
                    remove_upload(&config.dir, &id);
                    Ok(Value::Bool(true))
                }
                _ => Ok(Value::Bool(false)),
            }
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_round_trips_and_tolerates_junk() {
        let map = parse_metadata("filename dGVzdC50eHQ=,filetype aW1hZ2UvcG5n,flag");
        assert_eq!(map["filename"], "test.txt");
        assert_eq!(map["filetype"], "image/png");
        assert_eq!(map["flag"], "");
        assert_eq!(parse_metadata(&format_metadata(&map)), map);
        assert!(parse_metadata("").is_empty());
    }

    #[test]
    fn filenames_lose_paths_and_control_characters() {
        assert_eq!(safe_filename("../../etc/passwd"), "passwd");
        assert_eq!(safe_filename("C:\\Users\\me\\a.png"), "a.png");
        assert_eq!(safe_filename(".htaccess"), "htaccess");
        assert_eq!(safe_filename("a\nb.txt"), "ab.txt");
        assert_eq!(safe_filename(""), "upload");
        assert_eq!(safe_filename("///"), "upload");
    }

    #[test]
    fn only_minted_ids_are_accepted() {
        assert!(valid_id(&uuid::Uuid::new_v4().to_string()));
        assert!(!valid_id("../../etc/passwd"));
        assert!(!valid_id("abc"));
        assert!(!valid_id(&"g".repeat(36)));
    }

    fn scratch_config() -> Config {
        Config {
            dir: std::env::temp_dir().join(format!("soli-tus-{}", uuid::Uuid::new_v4())),
            ..Config::default()
        }
    }

    fn meta_for(config: &Config, length: u64, owner: Option<&str>) -> Meta {
        fs::create_dir_all(&config.dir).unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let meta = Meta {
            id: id.clone(),
            owner: owner.map(str::to_string),
            length,
            filename: "a.bin".into(),
            content_type: "application/octet-stream".into(),
            metadata: BTreeMap::new(),
            created_at: now(),
            expires_at: now() + 60,
        };
        fs::File::create(part_path(&config.dir, &id)).unwrap();
        fs::write(
            meta_path(&config.dir, &id),
            serde_json::to_vec(&meta).unwrap(),
        )
        .unwrap();
        meta
    }

    #[test]
    fn appending_advances_the_offset_and_checks_it() {
        let config = scratch_config();
        let meta = meta_for(&config, 6, None);

        assert_eq!(append_chunk(&config, &meta, 0, b"abc").unwrap(), 3);
        // A stale client retrying offset 0 is told where the server is.
        let conflict = append_chunk(&config, &meta, 0, b"abc").unwrap_err();
        assert_eq!(conflict.status, 409);
        assert!(conflict
            .headers
            .iter()
            .any(|(k, v)| k == "Upload-Offset" && v == "3"));
        // Past the declared length is refused.
        assert_eq!(
            append_chunk(&config, &meta, 3, b"defg").unwrap_err().status,
            413
        );
        assert_eq!(append_chunk(&config, &meta, 3, b"def").unwrap(), 6);
        assert_eq!(
            fs::read(part_path(&config.dir, &meta.id)).unwrap(),
            b"abcdef"
        );
        let _ = fs::remove_dir_all(&config.dir);
    }

    #[test]
    fn someone_elses_upload_is_not_found() {
        let config = scratch_config();
        let meta = meta_for(&config, 3, Some("session-a"));
        assert!(lookup(&config, &meta.id, Some("session-a")).is_ok());
        assert_eq!(
            lookup(&config, &meta.id, Some("session-b"))
                .unwrap_err()
                .status,
            404
        );
        assert_eq!(lookup(&config, &meta.id, None).unwrap_err().status, 404);
        let _ = fs::remove_dir_all(&config.dir);
    }

    #[test]
    fn expired_uploads_are_gone_and_swept() {
        let config = scratch_config();
        let mut meta = meta_for(&config, 3, None);
        meta.expires_at = now() - 1;
        fs::write(
            meta_path(&config.dir, &meta.id),
            serde_json::to_vec(&meta).unwrap(),
        )
        .unwrap();
        assert_eq!(lookup(&config, &meta.id, None).unwrap_err().status, 410);
        assert!(
            !part_path(&config.dir, &meta.id).exists(),
            "the bytes go with it"
        );
        let _ = fs::remove_dir_all(&config.dir);
    }
}
