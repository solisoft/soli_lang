//! First-class file attachments: disk and S3 blob storage.
//!
//! `has_one_attached` / `has_many_attached` default to the `disk` service
//! (`SOLI_ATTACHMENTS_PATH`, default `./storage/attachments`). `s3` uses
//! `SOLI_ATTACHMENTS_BUCKET` plus the same AWS/S3 credentials as `S3.*`.
//! The existing `uploader(...)` DSL still defaults to SoliDB blobs.

use std::fs;
use std::path::PathBuf;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, HashPairs, NativeFunction, Value};
use base64::{engine::general_purpose::STANDARD, Engine as _};
// The S3 half of this module is the `cloud` feature: a build without it
// keeps disk attachments and answers plainly for the other service rather
// than pretending to store anything.
#[cfg(feature = "cloud")]
use super::s3_client::{metadata_value, run as run_s3, ObjectInfo, S3Client};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;

use super::args::hash_str;

const DEFAULT_DISK_ROOT: &str = "./storage/attachments";

/// What a build without the `cloud` feature says when an application asks
/// for the `s3` service.
#[cfg(not(feature = "cloud"))]
const NO_CLOUD: &str =
    "this build of soli has no S3 attachment service: it was built without the `cloud` feature";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BlobMeta {
    filename: String,
    content_type: String,
    size: u64,
}

fn disk_root() -> PathBuf {
    crate::platform::env::var("SOLI_ATTACHMENTS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_DISK_ROOT))
}

fn sanitize_part(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .take(128)
        .collect();
    // `.` survives the filter, so `..` survived sanitisation whole: an id taken
    // from a request param and handed to `read_attachment` / `delete_attachment`
    // resolved to `<root>/<collection>/../{data,meta.json}`, and a collection of
    // `".."` climbed another level. The built-in controller never does this, but
    // a function named `sanitize_part` must not hand back a traversal.
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '.') {
        "blob".to_string()
    } else {
        cleaned
    }
}

fn disk_paths(collection: &str, id: &str) -> (PathBuf, PathBuf) {
    let dir = disk_root()
        .join(sanitize_part(collection))
        .join(sanitize_part(id));
    (dir.join("data"), dir.join("meta.json"))
}

pub fn store_bytes(
    service: &str,
    collection: &str,
    filename: &str,
    content_type: &str,
    data: Vec<u8>,
) -> Result<String, String> {
    match service {
        #[cfg(feature = "cloud")]
        "s3" => store_s3(collection, filename, content_type, data),
        #[cfg(not(feature = "cloud"))]
        "s3" => Err(NO_CLOUD.to_string()),
        "disk" => store_disk(collection, filename, content_type, data),
        other => Err(format!(
            "unknown attachment service {other:?} (use disk, s3, or solidb)"
        )),
    }
}

/// Move a finished tus upload into the attachment store without reading it into
/// memory. The upload is consumed on success.
///
/// The id is the caller's word, so it is resolved exactly as `tus_take` does:
/// the upload must belong to the current session, be complete and unexpired.
/// Name and type come from the upload itself, not from the hash that carried
/// the id.
fn store_from_tus(service: &str, collection: &str, tus_id: &str) -> Result<String, String> {
    let owner = crate::interpreter::builtins::session::get_current_session_id();
    let done = crate::serve::tus::finished(tus_id, owner.as_deref())?;
    let (path, filename, content_type) = (&done.path, &done.filename, &done.content_type);
    let id = match service {
        "disk" => store_disk_from_path(collection, filename, content_type, path)?,
        #[cfg(feature = "cloud")]
        "s3" => store_s3_from_path(collection, filename, content_type, path)?,
        #[cfg(not(feature = "cloud"))]
        "s3" => return Err(NO_CLOUD.to_string()),
        other => {
            return Err(format!(
                "a resumable upload can be attached to a disk or s3 attachment, not {other:?}"
            ))
        }
    };
    crate::serve::tus::discard(tus_id);
    Ok(id)
}

fn store_disk_from_path(
    collection: &str,
    filename: &str,
    content_type: &str,
    source: &std::path::Path,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let (data_path, meta_path) = disk_paths(collection, &id);
    if let Some(parent) = data_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("attachment disk mkdir: {e}"))?;
    }
    let size = fs::metadata(source)
        .map_err(|e| format!("attachment disk stat: {e}"))?
        .len();
    // A rename is free on one filesystem; across two it falls back to a copy.
    if fs::rename(source, &data_path).is_err() {
        fs::copy(source, &data_path).map_err(|e| format!("attachment disk copy: {e}"))?;
        let _ = fs::remove_file(source);
    }
    let meta = BlobMeta {
        filename: filename.to_string(),
        content_type: content_type.to_string(),
        size,
    };
    fs::write(
        &meta_path,
        serde_json::to_vec(&meta).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("attachment disk meta: {e}"))?;
    Ok(id)
}

#[cfg(feature = "cloud")]
fn store_s3_from_path(
    collection: &str,
    filename: &str,
    content_type: &str,
    source: &std::path::Path,
) -> Result<String, String> {
    let bucket = s3_bucket()?;
    let id = uuid::Uuid::new_v4().to_string();
    let key = s3_key(collection, &id);
    let client = S3Client::from_env()?;
    run_s3(client.put_object_file(
        &bucket,
        &key,
        source,
        content_type,
        &[("original-filename", filename)],
    ))
    .map_err(|e| format!("attachment s3 put: {e}"))?;
    Ok(id)
}

/// What a browser needs to send one file straight to the bucket.
#[cfg(feature = "cloud")]
struct DirectUpload {
    id: String,
    url: String,
    headers: Vec<(String, String)>,
    expires_in: u64,
}

/// Largest lifetime a presigned URL may be asked for.
#[cfg(feature = "cloud")]
const MAX_PRESIGN_SECS: u64 = 3600;

/// A presigned `PUT` for a brand-new blob. The signature covers the host and
/// the `x-amz-meta-original-filename` header, *not* `Content-Type` or
/// `Content-Length`: S3 will accept whatever the browser sends to this URL.
/// The limits are therefore enforced when the upload is finished, against what
/// the bucket reports (`direct_upload_finish`), which deletes an object that
/// breaks them.
#[cfg(feature = "cloud")]
fn presign_direct_upload(
    collection: &str,
    filename: &str,
    content_type: &str,
    _size: u64,
    expires_in: u64,
) -> Result<DirectUpload, String> {
    let bucket = s3_bucket()?;
    let id = uuid::Uuid::new_v4().to_string();
    let key = s3_key(collection, &id);
    let expires_in = expires_in.clamp(1, MAX_PRESIGN_SECS);
    // The browser sends this header verbatim, and a browser cannot send a
    // non-ASCII header value: the name travels as the encoded word S3 uses.
    let filename_header = metadata_value(filename);
    let url = S3Client::from_env()?.presign_put(
        &bucket,
        &key,
        expires_in,
        &[("x-amz-meta-original-filename", &filename_header)],
    );
    Ok(DirectUpload {
        id,
        url,
        headers: vec![
            ("Content-Type".to_string(), content_type.to_string()),
            ("x-amz-meta-original-filename".to_string(), filename_header),
        ],
        expires_in,
    })
}

/// What the bucket actually holds under `id`, or `None` when nothing landed.
#[cfg(feature = "cloud")]
fn head_s3(collection: &str, id: &str) -> Result<Option<BlobMeta>, String> {
    let bucket = s3_bucket()?;
    let key = s3_key(collection, id);
    let head = run_s3(S3Client::from_env()?.head_object(&bucket, &key))
        .map_err(|e| format!("attachment s3 head: {e}"))?;
    Ok(head.map(|info| blob_meta(&info, info.content_length.unwrap_or(0))))
}

/// The blob's name and type as the upload recorded them on the object.
#[cfg(feature = "cloud")]
fn blob_meta(info: &ObjectInfo, size: u64) -> BlobMeta {
    BlobMeta {
        filename: info
            .metadata
            .get("original-filename")
            .cloned()
            .unwrap_or_else(|| "file".to_string()),
        content_type: info
            .content_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_string()),
        size,
    }
}

fn store_disk(
    collection: &str,
    filename: &str,
    content_type: &str,
    data: Vec<u8>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let (data_path, meta_path) = disk_paths(collection, &id);
    if let Some(parent) = data_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("attachment disk mkdir: {e}"))?;
    }
    fs::write(&data_path, &data).map_err(|e| format!("attachment disk write: {e}"))?;
    let meta = BlobMeta {
        filename: filename.to_string(),
        content_type: content_type.to_string(),
        size: data.len() as u64,
    };
    fs::write(
        &meta_path,
        serde_json::to_vec(&meta).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("attachment disk meta: {e}"))?;
    Ok(id)
}

fn read_disk(collection: &str, id: &str) -> Result<(BlobMeta, Vec<u8>), String> {
    let (data_path, meta_path) = disk_paths(collection, id);
    let raw = fs::read(&meta_path).map_err(|_| "attachment not found".to_string())?;
    let meta: BlobMeta = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    let data = fs::read(&data_path).map_err(|_| "attachment not found".to_string())?;
    Ok((meta, data))
}

fn delete_disk(collection: &str, id: &str) -> Result<(), String> {
    let (data_path, meta_path) = disk_paths(collection, id);
    let _ = fs::remove_file(data_path);
    let _ = fs::remove_file(&meta_path);
    if let Some(dir) = meta_path.parent() {
        let _ = fs::remove_dir(dir);
    }
    Ok(())
}

#[cfg(feature = "cloud")]
fn s3_bucket() -> Result<String, String> {
    crate::platform::env::var("SOLI_ATTACHMENTS_BUCKET")
        .or_else(|_| crate::platform::env::var("S3_BUCKET"))
        .map_err(|_| "SOLI_ATTACHMENTS_BUCKET (or S3_BUCKET) is required for service: s3".into())
}

#[cfg(feature = "cloud")]
fn s3_key(collection: &str, id: &str) -> String {
    format!("{}/{}", sanitize_part(collection), sanitize_part(id))
}

#[cfg(feature = "cloud")]
fn store_s3(
    collection: &str,
    filename: &str,
    content_type: &str,
    data: Vec<u8>,
) -> Result<String, String> {
    let bucket = s3_bucket()?;
    let id = uuid::Uuid::new_v4().to_string();
    let key = s3_key(collection, &id);
    let client = S3Client::from_env()?;
    run_s3(client.put_object(
        &bucket,
        &key,
        data.into(),
        content_type,
        &[("original-filename", filename)],
    ))
    .map_err(|e| format!("attachment s3 put: {e}"))?;
    Ok(id)
}

#[cfg(feature = "cloud")]
fn read_s3(collection: &str, id: &str) -> Result<(BlobMeta, Vec<u8>), String> {
    let bucket = s3_bucket()?;
    let key = s3_key(collection, id);
    let (info, data) = run_s3(S3Client::from_env()?.get_object(&bucket, &key))
        .map_err(|_| "attachment not found".to_string())?;
    Ok((blob_meta(&info, data.len() as u64), data.to_vec()))
}

#[cfg(feature = "cloud")]
fn delete_s3(collection: &str, id: &str) -> Result<(), String> {
    let bucket = s3_bucket()?;
    let key = s3_key(collection, id);
    run_s3(S3Client::from_env()?.delete_object(&bucket, &key))
        .map_err(|e| format!("attachment s3 delete: {e}"))
}

/// Borrow a string field instead of copying it.
///
/// [`hash_str`] hands back an owned `String`, which is fine for a filename or a
/// service name but not for `data`: that is the whole base64 payload, so the
/// copy cost ~1.33x the upload on every store, on top of the decode below.
fn hash_str_ref<'a>(hash: &'a HashPairs, key: &str) -> Option<&'a str> {
    hash.get(&HashKey::String(key.into()))
        .and_then(|v| match v {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        })
}

fn file_bytes(file: &HashPairs) -> Result<Vec<u8>, String> {
    let data = hash_str_ref(file, "data").ok_or_else(|| "file is missing data".to_string())?;
    STANDARD
        .decode(data.trim())
        .map_err(|e| format!("file data is not base64: {e}"))
}

/// Register `store_attachment` / `read_attachment` / `delete_attachment`.
pub fn register_attachment_builtins(env: &mut Environment) {
    env.define(
        "store_attachment".to_string(),
        Value::NativeFunction(NativeFunction::new("store_attachment", Some(2), |args| {
            let config = match args.first() {
                Some(Value::Hash(h)) => h.borrow().clone(),
                _ => return Err("store_attachment(config, file) expects a config hash".into()),
            };
            let file = match args.get(1) {
                Some(Value::Hash(h)) => h.borrow().clone(),
                _ => return Err("store_attachment(config, file) expects a file hash".into()),
            };
            let service = hash_str(&config, "service").unwrap_or_else(|| "disk".into());
            if service == "solidb" {
                return Err("store_attachment: use solidb_store_blob for service solidb".into());
            }
            let collection = hash_str(&config, "collection").unwrap_or_else(|| "blobs".into());
            let filename = hash_str(&file, "filename").unwrap_or_else(|| "file".into());
            let content_type = hash_str(&file, "content_type")
                .unwrap_or_else(|| "application/octet-stream".into());
            if let Some(tus_id) = hash_str(&file, "tus_id") {
                if !file.contains_key(&HashKey::String("data".into())) {
                    let id = store_from_tus(&service, &collection, &tus_id)?;
                    return Ok(Value::String(id.into()));
                }
            }
            let data = file_bytes(&file)?;
            let id = store_bytes(&service, &collection, &filename, &content_type, data)?;
            Ok(Value::String(id.into()))
        })),
    );

    // direct_upload(config, {filename, content_type, size}, expires_in?) — a
    // presigned S3 PUT so the browser sends the file to the bucket, not to us.
    env.define(
        "direct_upload".to_string(),
        Value::NativeFunction(NativeFunction::new("direct_upload", None, |args| {
            let config = match args.first() {
                Some(Value::Hash(h)) => h.borrow().clone(),
                _ => return Err("direct_upload(config, file, expires_in?) expects a config hash".into()),
            };
            let file = match args.get(1) {
                Some(Value::Hash(h)) => h.borrow().clone(),
                _ => return Err("direct_upload(config, file, expires_in?) expects a file hash".into()),
            };
            let service = hash_str(&config, "service").unwrap_or_else(|| "disk".into());
            if service != "s3" {
                return Err(format!(
                    "direct_upload needs an s3 attachment service, not {service:?}; use resumable_uploads() for disk"
                ));
            }
            #[cfg(not(feature = "cloud"))]
            {
                let _ = (&file, args);
                Err(NO_CLOUD.to_string())
            }
            #[cfg(feature = "cloud")]
            {
                let collection = hash_str(&config, "collection").unwrap_or_else(|| "blobs".into());
                let filename = hash_str(&file, "filename").unwrap_or_else(|| "file".into());
                let content_type = hash_str(&file, "content_type")
                    .unwrap_or_else(|| "application/octet-stream".into());
                let size = match file.get(&HashKey::String("size".into())) {
                    Some(Value::Int(n)) if *n >= 0 => *n as u64,
                    _ => return Err("direct_upload: file needs a non-negative size".into()),
                };
                let expires_in = match args.get(2) {
                    Some(Value::Int(n)) if *n > 0 => *n as u64,
                    _ => 900,
                };
                let direct =
                    presign_direct_upload(&collection, &filename, &content_type, size, expires_in)?;
                let mut headers = HashPairs::default();
                for (name, value) in direct.headers {
                    headers.insert(HashKey::String(name.into()), Value::String(value.into()));
                }
                let mut pairs = HashPairs::default();
                pairs.insert(HashKey::String("id".into()), Value::String(direct.id.into()));
                pairs.insert(HashKey::String("url".into()), Value::String(direct.url.into()));
                pairs.insert(HashKey::String("method".into()), Value::String("PUT".into()));
                pairs.insert(
                    HashKey::String("headers".into()),
                    Value::Hash(Rc::new(RefCell::new(headers))),
                );
                pairs.insert(
                    HashKey::String("expires_in".into()),
                    Value::Int(direct.expires_in as i64),
                );
                Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
            }
        })),
    );

    // direct_upload_head(config, id) — what the bucket holds under `id`:
    // {filename, content_type, size}, or nil when the browser never finished.
    env.define(
        "direct_upload_head".to_string(),
        Value::NativeFunction(NativeFunction::new("direct_upload_head", Some(2), |args| {
            let config = match args.first() {
                Some(Value::Hash(h)) => h.borrow().clone(),
                _ => return Err("direct_upload_head(config, id) expects a config hash".into()),
            };
            let id = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                _ => return Err("direct_upload_head(config, id) expects a string id".into()),
            };
            #[cfg(not(feature = "cloud"))]
            {
                let _ = (&config, &id);
                Err(NO_CLOUD.to_string())
            }
            #[cfg(feature = "cloud")]
            {
                let collection = hash_str(&config, "collection").unwrap_or_else(|| "blobs".into());
                match head_s3(&collection, &id)? {
                    None => Ok(Value::Null),
                    Some(meta) => {
                        let mut pairs = HashPairs::default();
                        pairs.insert(
                            HashKey::String("filename".into()),
                            Value::String(meta.filename.into()),
                        );
                        pairs.insert(
                            HashKey::String("content_type".into()),
                            Value::String(meta.content_type.into()),
                        );
                        pairs.insert(HashKey::String("size".into()), Value::Int(meta.size as i64));
                        Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
                    }
                }
            }
        })),
    );

    env.define(
        "read_attachment".to_string(),
        Value::NativeFunction(NativeFunction::new("read_attachment", Some(2), |args| {
            let config = match args.first() {
                Some(Value::Hash(h)) => h.borrow().clone(),
                _ => return Err("read_attachment(config, id) expects a config hash".into()),
            };
            let id = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                _ => return Err("read_attachment(config, id) expects a string id".into()),
            };
            let service = hash_str(&config, "service").unwrap_or_else(|| "disk".into());
            let collection = hash_str(&config, "collection").unwrap_or_else(|| "blobs".into());
            let (meta, data) = match service.as_str() {
                #[cfg(feature = "cloud")]
                "s3" => read_s3(&collection, &id),
                #[cfg(not(feature = "cloud"))]
                "s3" => Err(NO_CLOUD.to_string()),
                "disk" => read_disk(&collection, &id),
                _ => return Ok(Value::Null),
            }?;
            let mut pairs = HashPairs::default();
            pairs.insert(
                HashKey::String("filename".into()),
                Value::String(meta.filename.into()),
            );
            pairs.insert(
                HashKey::String("content_type".into()),
                Value::String(meta.content_type.into()),
            );
            pairs.insert(HashKey::String("size".into()), Value::Int(meta.size as i64));
            pairs.insert(
                HashKey::String("data".into()),
                Value::String(STANDARD.encode(data).into()),
            );
            Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
        })),
    );

    env.define(
        "delete_attachment".to_string(),
        Value::NativeFunction(NativeFunction::new("delete_attachment", Some(2), |args| {
            let config = match args.first() {
                Some(Value::Hash(h)) => h.borrow().clone(),
                _ => return Err("delete_attachment(config, id) expects a config hash".into()),
            };
            let id = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                _ => return Err("delete_attachment(config, id) expects a string id".into()),
            };
            let service = hash_str(&config, "service").unwrap_or_else(|| "disk".into());
            let collection = hash_str(&config, "collection").unwrap_or_else(|| "blobs".into());
            let ok = match service.as_str() {
                #[cfg(feature = "cloud")]
                "s3" => delete_s3(&collection, &id).is_ok(),
                #[cfg(not(feature = "cloud"))]
                "s3" => false,
                "disk" => delete_disk(&collection, &id).is_ok(),
                _ => false,
            };
            Ok(Value::Bool(ok))
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_round_trip() {
        let dir = std::env::temp_dir().join(format!("soli-att-{}", uuid::Uuid::new_v4()));
        std::env::set_var("SOLI_ATTACHMENTS_PATH", &dir);
        let id = store_disk("avatars", "me.png", "image/png", b"pngbytes".to_vec()).unwrap();
        let (meta, data) = read_disk("avatars", &id).unwrap();
        assert_eq!(meta.filename, "me.png");
        assert_eq!(meta.content_type, "image/png");
        assert_eq!(data, b"pngbytes");
        delete_disk("avatars", &id).unwrap();
        assert!(read_disk("avatars", &id).is_err());
        let _ = fs::remove_dir_all(&dir);
        std::env::remove_var("SOLI_ATTACHMENTS_PATH");
    }
}

#[cfg(test)]
mod sanitize_part_tests {
    use super::*;

    /// `.` passed the character filter, so `..` came through whole and
    /// `<root>/<collection>/../data` escaped the attachment directory.
    #[test]
    fn dot_segments_never_survive() {
        assert_eq!(sanitize_part(".."), "blob");
        assert_eq!(sanitize_part("."), "blob");
        assert_eq!(sanitize_part("..."), "blob");
        // Separators are stripped first, so this collapses to one harmless
        // segment rather than a traversal — assert it stays a single component.
        let flattened = sanitize_part("../../etc/passwd");
        assert!(!flattened.contains('/'), "{flattened}");
        assert!(!flattened.contains('\\'), "{flattened}");
        assert_ne!(flattened, "..");
    }

    #[test]
    fn ordinary_ids_are_unchanged() {
        assert_eq!(
            sanitize_part("019329ab-7c4d-7e00-8000-1f2b3c4d5e6f"),
            "019329ab-7c4d-7e00-8000-1f2b3c4d5e6f"
        );
        assert_eq!(sanitize_part("avatar.png"), "avatar.png");
        assert_eq!(sanitize_part("posts"), "posts");
    }

    /// Path separators were already stripped; keep it that way.
    #[test]
    fn separators_are_stripped() {
        assert_eq!(sanitize_part("a/b"), "ab");
        assert_eq!(sanitize_part("a\\b"), "ab");
    }
}
