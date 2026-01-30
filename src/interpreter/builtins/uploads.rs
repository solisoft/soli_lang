use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, NativeFunction, Value};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use indexmap::IndexMap;
use lazy_static::lazy_static;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::RwLock;

lazy_static! {
    static ref SOLIDB_ADDRESS: RwLock<Option<String>> = RwLock::new(None);
}

pub fn get_solidb_address() -> Option<String> {
    SOLIDB_ADDRESS.read().ok().and_then(|s| s.clone())
}

pub fn set_solidb_address(addr: &str) {
    if let Ok(mut guard) = SOLIDB_ADDRESS.write() {
        *guard = Some(addr.to_string());
    }
}

pub fn register_upload_builtins(env: &mut Environment) {
    env.define(
        "parse_multipart".to_string(),
        Value::NativeFunction(NativeFunction::new("parse_multipart", Some(1), |args| {
            let req = &args[0];

            let body = match req {
                Value::Hash(hash) => {
                    let borrowed = hash.borrow();
                    borrowed.iter()
                        .find(|(k, _)| matches!(k, HashKey::String(s) if s == "body"))
                        .map(|(_, v)| {
                            if let Value::String(s) = v { s.clone() } else { String::new() }
                        })
                        .unwrap_or_default()
                }
                _ => return Err("parse_multipart() expects request hash".to_string()),
            };

            let content_type = match req {
                Value::Hash(hash) => {
                    let borrowed = hash.borrow();
                    borrowed.iter()
                        .find(|(k, _)| matches!(k, HashKey::String(s) if s == "headers"))
                        .and_then(|(_, h)| {
                            if let Value::Hash(headers) = h {
                                let h_borrowed = headers.borrow();
                                h_borrowed.iter()
                                    .find(|(k, _)| matches!(k, HashKey::String(s) if s == "content-type" || s == "Content-Type"))
                                    .map(|(_, v)| {
                                        if let Value::String(s) = v { s.clone() } else { String::new() }
                                    })
                            } else { None }
                        })
                        .unwrap_or_default()
                }
                _ => return Err("parse_multipart() expects request hash".to_string()),
            };

            if body.is_empty() || !content_type.contains("multipart/form-data") {
                return Ok(Value::Array(Rc::new(RefCell::new(vec![]))));
            }

            let boundary = extract_boundary(&content_type)
                .ok_or_else(|| "No boundary found in Content-Type".to_string())?;

            match multer::parse_boundary(&boundary) {
                Ok(_) => {}
                Err(e) => return Err(format!("Invalid boundary: {}", e)),
            }

            let files = parse_multipart_data(&body, &boundary)?;

            let file_values: Vec<Value> = files.into_iter().map(|file| {
                let mut file_map: IndexMap<HashKey, Value> = IndexMap::new();
                file_map.insert(HashKey::String("filename".to_string()), Value::String(file.filename));
                file_map.insert(HashKey::String("content_type".to_string()), Value::String(file.content_type));
                file_map.insert(HashKey::String("size".to_string()), Value::Int(file.size as i64));
                file_map.insert(HashKey::String("data_base64".to_string()), Value::String(file.data_base64));
                file_map.insert(HashKey::String("field_name".to_string()), Value::String(file.field_name));
                Value::Hash(Rc::new(RefCell::new(file_map)))
            }).collect();

            Ok(Value::Array(Rc::new(RefCell::new(file_values))))
        })),
    );

    env.define(
        "set_solidb_address".to_string(),
        Value::NativeFunction(NativeFunction::new("set_solidb_address", Some(1), |args| {
            let addr = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "set_solidb_address() expects string address, got {}",
                        other.type_name()
                    ))
                }
            };
            set_solidb_address(&addr);
            Ok(Value::Null)
        })),
    );

    env.define(
        "upload_to_solidb".to_string(),
        Value::NativeFunction(NativeFunction::new("upload_to_solidb", Some(3), |args| {
            let req = &args[0];
            let collection = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "upload_to_solidb() expects string collection, got {}",
                        other.type_name()
                    ))
                }
            };
            let field_name = match &args[2] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "upload_to_solidb() expects string field_name, got {}",
                        other.type_name()
                    ))
                }
            };
            let solidb_addr = get_solidb_address().ok_or_else(|| {
                "SoliDB address not configured. Use set_solidb_address() first.".to_string()
            })?;

            let files = match parse_multipart_from_req(req) {
                Ok(f) => f,
                Err(e) => return Err(e),
            };

            let target_file = files
                .into_iter()
                .find(|f| f.field_name == field_name)
                .ok_or_else(|| format!("Field '{}' not found in multipart data", field_name))?;

            let result = upload_blob_to_solidb(&solidb_addr, &collection, &target_file)?;

            let mut result_hash: IndexMap<HashKey, Value> = IndexMap::new();
            result_hash.insert(
                HashKey::String("blob_id".to_string()),
                Value::String(result.blob_id),
            );
            result_hash.insert(
                HashKey::String("filename".to_string()),
                Value::String(target_file.filename),
            );
            result_hash.insert(
                HashKey::String("size".to_string()),
                Value::Int(target_file.size as i64),
            );
            result_hash.insert(
                HashKey::String("content_type".to_string()),
                Value::String(target_file.content_type),
            );

            Ok(Value::Hash(Rc::new(RefCell::new(result_hash))))
        })),
    );

    env.define(
        "upload_all_to_solidb".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "upload_all_to_solidb",
            Some(2),
            |args| {
                let req = &args[0];
                let collection = match &args[1] {
                    Value::String(s) => s.clone(),
                    other => {
                        return Err(format!(
                            "upload_all_to_solidb() expects string collection, got {}",
                            other.type_name()
                        ))
                    }
                };
                let solidb_addr = get_solidb_address().ok_or_else(|| {
                    "SoliDB address not configured. Use set_solidb_address() first.".to_string()
                })?;

                let files = match parse_multipart_from_req(req) {
                    Ok(f) => f,
                    Err(e) => return Err(e),
                };

                let mut results: Vec<Value> = Vec::new();
                for file in files {
                    match upload_blob_to_solidb(&solidb_addr, &collection, &file) {
                        Ok(result) => {
                            let mut result_hash: IndexMap<HashKey, Value> = IndexMap::new();
                            result_hash.insert(
                                HashKey::String("blob_id".to_string()),
                                Value::String(result.blob_id),
                            );
                            result_hash.insert(
                                HashKey::String("filename".to_string()),
                                Value::String(file.filename),
                            );
                            result_hash.insert(
                                HashKey::String("size".to_string()),
                                Value::Int(file.size as i64),
                            );
                            result_hash.insert(
                                HashKey::String("content_type".to_string()),
                                Value::String(file.content_type),
                            );
                            result_hash.insert(
                                HashKey::String("field_name".to_string()),
                                Value::String(file.field_name),
                            );
                            results.push(Value::Hash(Rc::new(RefCell::new(result_hash))));
                        }
                        Err(e) => {
                            let mut error_hash: IndexMap<HashKey, Value> = IndexMap::new();
                            error_hash.insert(HashKey::String("error".to_string()), Value::String(e));
                            error_hash.insert(
                                HashKey::String("filename".to_string()),
                                Value::String(file.filename),
                            );
                            error_hash.insert(
                                HashKey::String("field_name".to_string()),
                                Value::String(file.field_name),
                            );
                            results.push(Value::Hash(Rc::new(RefCell::new(error_hash))));
                        }
                    }
                }

                Ok(Value::Array(Rc::new(RefCell::new(results))))
            },
        )),
    );

    env.define(
        "get_blob_url".to_string(),
        Value::NativeFunction(NativeFunction::new("get_blob_url", Some(2), |args| {
            let collection = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "get_blob_url() expects string collection, got {}",
                        other.type_name()
                    ))
                }
            };
            let blob_id = match &args[1] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "get_blob_url() expects string blob_id, got {}",
                        other.type_name()
                    ))
                }
            };
            let base_url = match args.get(2) {
                Some(Value::String(s)) => s.clone(),
                _ => get_solidb_address().ok_or_else(|| {
                    "SoliDB address not configured. Use set_solidb_address() or pass base_url."
                        .to_string()
                })?,
            };
            let _expires_in = match args.get(3) {
                Some(Value::Int(i)) => *i as u64,
                _ => 3600,
            };

            let url = format!(
                "{}/_api/database/solidb/document/{}/{}",
                base_url.trim_end_matches('/'),
                collection,
                blob_id
            );
            Ok(Value::String(url))
        })),
    );
}

struct ParsedFile {
    filename: String,
    content_type: String,
    size: usize,
    data_base64: String,
    field_name: String,
}

struct UploadResult {
    blob_id: String,
}

fn extract_boundary(content_type: &str) -> Option<String> {
    content_type
        .split("boundary=")
        .nth(1)
        .map(|s| s.trim().to_string())
}

fn parse_multipart_data(body: &str, boundary: &str) -> Result<Vec<ParsedFile>, String> {
    let mut files = Vec::new();

    let boundary_line = format!("--{}", boundary);
    let parts: Vec<&str> = body.split(&boundary_line).collect();

    for part in parts {
        let part = part.trim();
        if part.is_empty() || part == "--" {
            continue;
        }

        if let Some(headers_end) = part.find("\r\n\r\n") {
            let headers_section = &part[..headers_end];
            let data = &part[headers_end + 4..];

            let clean_data = data.trim().trim_end_matches("--").trim();

            let mut filename = String::new();
            let mut content_type = String::new();
            let mut field_name = String::new();

            for header in headers_section.lines() {
                let header = header.trim();
                if header.to_lowercase().starts_with("content-disposition:") {
                    if let Some(name_start) = header.find("name=\"") {
                        let name_end = header[name_start + 6..]
                            .find('"')
                            .map(|i| name_start + 6 + i);
                        if let Some(end) = name_end {
                            field_name = header[name_start + 6..end].to_string();
                        }
                    }
                    if let Some(fname_start) = header.find("filename=\"") {
                        let fname_end = header[fname_start + 10..]
                            .find('"')
                            .map(|i| fname_start + 10 + i);
                        if let Some(end) = fname_end {
                            filename = header[fname_start + 10..end].to_string();
                        }
                    }
                }
                if header.to_lowercase().starts_with("content-type:") {
                    content_type = header[14..].trim().to_string();
                }
            }

            if !filename.is_empty() {
                let data_bytes = clean_data.as_bytes();
                let size = data_bytes.len();
                let data_base64 = STANDARD.encode(data_bytes);

                files.push(ParsedFile {
                    filename,
                    content_type,
                    size,
                    data_base64,
                    field_name,
                });
            }
        }
    }

    Ok(files)
}

fn parse_multipart_from_req(req: &Value) -> Result<Vec<ParsedFile>, String> {
    let body = match req {
        Value::Hash(hash) => {
            let borrowed = hash.borrow();
            borrowed
                .iter()
                .find(|(k, _)| matches!(k, HashKey::String(s) if s == "body"))
                .map(|(_, v)| {
                    if let Value::String(s) = v {
                        s.clone()
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default()
        }
        _ => return Err("parse_multipart_from_req() expects request hash".to_string()),
    };

    let content_type = match req {
        Value::Hash(hash) => {
            let borrowed = hash.borrow();
            borrowed.iter()
                .find(|(k, _)| matches!(k, HashKey::String(s) if s == "headers"))
                .and_then(|(_, h)| {
                    if let Value::Hash(headers) = h {
                        let h_borrowed = headers.borrow();
                        h_borrowed.iter()
                            .find(|(k, _)| matches!(k, HashKey::String(s) if s == "content-type" || s == "Content-Type"))
                            .map(|(_, v)| {
                                if let Value::String(s) = v { s.clone() } else { String::new() }
                            })
                    } else { None }
                })
                .unwrap_or_default()
        }
        _ => return Err("parse_multipart_from_req() expects request hash".to_string()),
    };

    if body.is_empty() || !content_type.contains("multipart/form-data") {
        return Ok(vec![]);
    }

    let boundary = extract_boundary(&content_type)
        .ok_or_else(|| "No boundary found in Content-Type".to_string())?;

    parse_multipart_data(&body, &boundary)
}

fn upload_blob_to_solidb(
    solidb_addr: &str,
    collection: &str,
    file: &ParsedFile,
) -> Result<UploadResult, String> {
    use crate::solidb_http::SoliDBClient;

    let client = SoliDBClient::connect(solidb_addr)
        .map_err(|e| format!("Failed to connect to SoliDB: {}", e))?;

    let blob_id = client
        .store_blob(
            collection,
            file.data_base64.as_bytes(),
            &file.filename,
            &file.content_type,
        )
        .map_err(|e| format!("Failed to store blob: {}", e))?;

    Ok(UploadResult { blob_id })
}
