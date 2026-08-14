//! First-class file attachments: disk and S3 blob storage.
//!
//! `has_one_attached` / `has_many_attached` default to the `disk` service
//! (`SOLI_ATTACHMENTS_PATH`, default `./storage/attachments`). `s3` uses
//! `SOLI_ATTACHMENTS_BUCKET` plus the same AWS/S3 credentials as `S3.*`.
//! The existing `uploader(...)` DSL still defaults to SoliDB blobs.

use std::fs;
use std::io::Read;
use std::path::PathBuf;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, HashPairs, NativeFunction, Value};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rusoto_core::Region;
use rusoto_credential::StaticProvider;
use rusoto_s3::{DeleteObjectRequest, GetObjectRequest, PutObjectRequest, S3Client, S3};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;

use crate::serve::get_tokio_handle;

const DEFAULT_DISK_ROOT: &str = "./storage/attachments";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BlobMeta {
    filename: String,
    content_type: String,
    size: u64,
}

fn disk_root() -> PathBuf {
    std::env::var("SOLI_ATTACHMENTS_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_DISK_ROOT))
}

fn sanitize_part(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        .take(128)
        .collect();
    if cleaned.is_empty() {
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
        "s3" => store_s3(collection, filename, content_type, data),
        "disk" => store_disk(collection, filename, content_type, data),
        other => Err(format!(
            "unknown attachment service {other:?} (use disk, s3, or solidb)"
        )),
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

fn s3_bucket() -> Result<String, String> {
    std::env::var("SOLI_ATTACHMENTS_BUCKET")
        .or_else(|_| std::env::var("S3_BUCKET"))
        .map_err(|_| "SOLI_ATTACHMENTS_BUCKET (or S3_BUCKET) is required for service: s3".into())
}

fn s3_key(collection: &str, id: &str) -> String {
    format!("{}/{}", sanitize_part(collection), sanitize_part(id))
}

fn s3_client() -> Result<S3Client, String> {
    let access_key = std::env::var("AWS_ACCESS_KEY_ID")
        .or_else(|_| std::env::var("S3_ACCESS_KEY"))
        .map_err(|_| "S3_ACCESS_KEY or AWS_ACCESS_KEY_ID not set".to_string())?;
    let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY")
        .or_else(|_| std::env::var("S3_SECRET_KEY"))
        .map_err(|_| "S3_SECRET_KEY or AWS_SECRET_ACCESS_KEY not set".to_string())?;
    let region_name = std::env::var("AWS_REGION")
        .or_else(|_| std::env::var("S3_REGION"))
        .unwrap_or_else(|_| "us-east-1".to_string());
    let region = if let Ok(ep) = std::env::var("S3_ENDPOINT") {
        Region::Custom {
            name: region_name,
            endpoint: ep,
        }
    } else {
        region_name.parse().unwrap_or(Region::UsEast1)
    };
    Ok(S3Client::new_with(
        rusoto_core::HttpClient::new().map_err(|e| e.to_string())?,
        StaticProvider::new(access_key, secret_key, None, None),
        region,
    ))
}

fn run_s3<F, T>(future: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    if let Some(rt) = get_tokio_handle() {
        rt.block_on(future)
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?
            .block_on(future)
    }
}

fn store_s3(
    collection: &str,
    filename: &str,
    content_type: &str,
    data: Vec<u8>,
) -> Result<String, String> {
    let bucket = s3_bucket()?;
    let id = uuid::Uuid::new_v4().to_string();
    let key = s3_key(collection, &id);
    let client = s3_client()?;
    let request = PutObjectRequest {
        bucket,
        key,
        body: Some(data.into()),
        content_type: Some(content_type.to_string()),
        metadata: Some(
            [("original-filename".to_string(), filename.to_string())]
                .into_iter()
                .collect(),
        ),
        ..Default::default()
    };
    run_s3(async move {
        client
            .put_object(request)
            .await
            .map_err(|e| format!("attachment s3 put: {e}"))?;
        Ok(id)
    })
}

fn read_s3(collection: &str, id: &str) -> Result<(BlobMeta, Vec<u8>), String> {
    let bucket = s3_bucket()?;
    let key = s3_key(collection, id);
    let client = s3_client()?;
    run_s3(async move {
        let out = client
            .get_object(GetObjectRequest {
                bucket,
                key,
                ..Default::default()
            })
            .await
            .map_err(|_| "attachment not found".to_string())?;
        let mut data = Vec::new();
        if let Some(body) = out.body {
            body.into_blocking_read()
                .read_to_end(&mut data)
                .map_err(|e| e.to_string())?;
        }
        let filename = out
            .metadata
            .as_ref()
            .and_then(|m| m.get("original-filename"))
            .cloned()
            .unwrap_or_else(|| "file".to_string());
        let content_type = out
            .content_type
            .unwrap_or_else(|| "application/octet-stream".to_string());
        Ok((
            BlobMeta {
                filename,
                content_type,
                size: data.len() as u64,
            },
            data,
        ))
    })
}

fn delete_s3(collection: &str, id: &str) -> Result<(), String> {
    let bucket = s3_bucket()?;
    let key = s3_key(collection, id);
    let client = s3_client()?;
    run_s3(async move {
        client
            .delete_object(DeleteObjectRequest {
                bucket,
                key,
                ..Default::default()
            })
            .await
            .map(|_| ())
            .map_err(|e| format!("attachment s3 delete: {e}"))
    })
}

fn hash_str(hash: &HashPairs, key: &str) -> Option<String> {
    hash.get(&HashKey::String(key.into()))
        .and_then(|v| match v {
            Value::String(s) => Some(s.to_string()),
            _ => None,
        })
}

fn file_bytes(file: &HashPairs) -> Result<Vec<u8>, String> {
    let data = hash_str(file, "data").ok_or_else(|| "file is missing data".to_string())?;
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
            let data = file_bytes(&file)?;
            let id = store_bytes(&service, &collection, &filename, &content_type, data)?;
            Ok(Value::String(id.into()))
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
                "s3" => read_s3(&collection, &id),
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
                "s3" => delete_s3(&collection, &id).is_ok(),
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
