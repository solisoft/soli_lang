//! Multipart form handling and file upload utilities

use std::collections::HashMap;
use std::sync::OnceLock;

use base64::{engine::general_purpose, Engine as _};
use bytes::Bytes;

use crate::interpreter::value::{HashKey, HashPairs, Value};
use crate::serve::UploadedFile;
use std::cell::RefCell;
use std::rc::Rc;

/// SEC-031: maximum number of files accepted per multipart request.
/// A single 8 MiB body packed with ~1-byte parts could otherwise spawn
/// hundreds of thousands of `UploadedFile` allocations (~200 bytes each
/// once the Soli `file_map` is built) and OOM the worker before the
/// handler ever runs. Operator override: `SOLI_MAX_UPLOAD_FILES`.
fn max_upload_files() -> usize {
    static CAP: OnceLock<usize> = OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("SOLI_MAX_UPLOAD_FILES")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(32)
    })
}

/// Parse multipart form data into form fields and files.
pub async fn parse_multipart_body(
    body_bytes: &[u8],
    content_type: &str,
) -> (HashMap<String, String>, Vec<UploadedFile>) {
    let mut form_fields = HashMap::new();
    let mut files = Vec::new();

    // Extract boundary from content-type header
    let boundary = content_type.split(';').find_map(|part| {
        let part = part.trim();
        if part.starts_with("boundary=") {
            Some(
                part.trim_start_matches("boundary=")
                    .trim_matches('"')
                    .to_string(),
            )
        } else {
            None
        }
    });

    let boundary = match boundary {
        Some(b) => b,
        None => return (form_fields, files),
    };

    // Use multer to parse the multipart data
    let stream = futures_util::stream::once(async move {
        Ok::<_, std::io::Error>(Bytes::copy_from_slice(body_bytes))
    });

    let mut multipart = multer::Multipart::new(stream, boundary);

    let file_cap = max_upload_files();
    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().map(|s| s.to_string()).unwrap_or_default();
        let filename = field.file_name().map(|s| s.to_string());
        let content_type = field
            .content_type()
            .map(|m| m.to_string())
            .unwrap_or_default();

        if let Ok(data) = field.bytes().await {
            if let Some(fname) = filename {
                // SEC-031: refuse to accept additional files past the cap so
                // a body packed with thousands of tiny parts can't allocate
                // a per-file Soli hash for each one.
                if files.len() >= file_cap {
                    break;
                }
                files.push(UploadedFile {
                    name: name.clone(),
                    filename: fname,
                    content_type,
                    data: data.to_vec(),
                });
            } else {
                // This is a regular form field
                let value = String::from_utf8_lossy(&data).to_string();
                form_fields.insert(name, value);
            }
        }
    }

    (form_fields, files)
}

/// Convert uploaded files to Soli Value array.
///
/// SEC-031: `data` is exposed as a base64-encoded `Value::String` rather
/// than `Value::Array(Vec<Value::Int>)`. Each `Value::Int` is a 16-byte
/// tagged enum, so the old shape inflated an 8 MiB upload into ~130 MiB
/// per worker — concurrent uploads would OOM the worker pool. The
/// base64 string is ~1.33× the raw bytes (UTF-8 safe, 4-bytes-per-3-bytes)
/// and is the form `Image.from_buffer` and `solidb_store_blob` already
/// expect, so most callers don't need to re-encode.
pub fn uploaded_files_to_value(files: &[UploadedFile]) -> Value {
    let file_values: Vec<Value> = files
        .iter()
        .map(|f| {
            let mut file_map: HashPairs = HashPairs::default();
            file_map.insert(
                HashKey::String("name".to_string()),
                Value::String(f.name.clone()),
            );
            file_map.insert(
                HashKey::String("filename".to_string()),
                Value::String(f.filename.clone()),
            );
            file_map.insert(
                HashKey::String("content_type".to_string()),
                Value::String(f.content_type.clone()),
            );
            file_map.insert(
                HashKey::String("size".to_string()),
                Value::Int(f.data.len() as i64),
            );
            file_map.insert(
                HashKey::String("data".to_string()),
                Value::String(general_purpose::STANDARD.encode(&f.data)),
            );
            Value::Hash(Rc::new(RefCell::new(file_map)))
        })
        .collect();
    Value::Array(Rc::new(RefCell::new(file_values)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(data: &[u8]) -> UploadedFile {
        UploadedFile {
            name: "f".to_string(),
            filename: "x.bin".to_string(),
            content_type: "application/octet-stream".to_string(),
            data: data.to_vec(),
        }
    }

    /// SEC-031: `data` is base64-encoded so an 8 MiB upload no longer
    /// inflates into ~130 MiB of `Value::Int`-tagged enum boxes.
    #[test]
    fn uploaded_data_is_base64_string() {
        // Use bytes that aren't valid UTF-8 to also confirm the result
        // is a String (only possible because we base64 the bytes first).
        let v = uploaded_files_to_value(&[file(&[0xff, 0xfe, 0x00, 0x42, 0x00])]);
        let arr = match v {
            Value::Array(a) => a,
            other => panic!("expected array, got {other:?}"),
        };
        let arr_borrow = arr.borrow();
        assert_eq!(arr_borrow.len(), 1);
        let file_hash = match &arr_borrow[0] {
            Value::Hash(h) => h.clone(),
            other => panic!("expected hash, got {other:?}"),
        };
        let file_borrow = file_hash.borrow();

        // Size reflects raw byte count, not base64 length.
        assert!(matches!(
            file_borrow.get(&HashKey::String("size".to_string())),
            Some(Value::Int(5))
        ));

        // Data is a base64-encoded string and round-trips back to the
        // original bytes.
        let data = match file_borrow.get(&HashKey::String("data".to_string())) {
            Some(Value::String(s)) => s.clone(),
            other => panic!("expected string data, got {other:?}"),
        };
        let decoded = general_purpose::STANDARD.decode(&data).unwrap();
        assert_eq!(decoded, vec![0xff, 0xfe, 0x00, 0x42, 0x00]);
    }
}
