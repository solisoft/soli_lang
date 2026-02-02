//! Multipart form handling and file upload utilities

use std::collections::HashMap;

use bytes::Bytes;

use crate::interpreter::value::{HashKey, Value};
use crate::serve::UploadedFile;
use indexmap::IndexMap;
use std::cell::RefCell;
use std::rc::Rc;

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

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().map(|s| s.to_string()).unwrap_or_default();
        let filename = field.file_name().map(|s| s.to_string());
        let content_type = field
            .content_type()
            .map(|m| m.to_string())
            .unwrap_or_default();

        if let Ok(data) = field.bytes().await {
            if let Some(fname) = filename {
                // This is a file upload
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
pub fn uploaded_files_to_value(files: &[UploadedFile]) -> Value {
    let file_values: Vec<Value> = files
        .iter()
        .map(|f| {
            let mut file_map: IndexMap<HashKey, Value> = IndexMap::new();
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
                Value::Array(Rc::new(RefCell::new(
                    f.data.iter().map(|&b| Value::Int(b as i64)).collect(),
                ))),
            );
            Value::Hash(Rc::new(RefCell::new(file_map)))
        })
        .collect();
    Value::Array(Rc::new(RefCell::new(file_values)))
}
