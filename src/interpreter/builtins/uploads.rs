use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, HashPairs, NativeFunction, Value};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use lazy_static::lazy_static;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::RwLock;

lazy_static! {
    static ref SOLIDB_ADDRESS: RwLock<Option<String>> = RwLock::new(None);
}

/// Sanitize filename to prevent path traversal and other security issues.
/// Removes directory components, leading/trailing dots and spaces, and dangerous characters.
fn sanitize_filename(filename: &str) -> String {
    // Get just the filename component (remove any path)
    let filename = Path::new(filename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(filename);

    // Remove any remaining path components (in case of .. traversal)
    let mut result = filename.replace("..", "").replace(['/', '\\'], "");

    // Remove leading/trailing dots and spaces
    result = result
        .trim()
        .trim_start_matches('.')
        .trim_start_matches(' ')
        .trim_end_matches(' ')
        .to_string();

    // If empty after sanitization, use a default
    if result.is_empty() {
        result = "unnamed".to_string();
    }

    result
}

pub fn get_solidb_address() -> Option<String> {
    if let Some(addr) = SOLIDB_ADDRESS.read().ok().and_then(|s| s.clone()) {
        return Some(addr);
    }
    std::env::var("SOLIDB_HOST").ok()
}

pub fn set_solidb_address(addr: &str) {
    if let Ok(mut guard) = SOLIDB_ADDRESS.write() {
        *guard = Some(addr.to_string());
    }
}

pub fn register_upload_builtins(env: &mut Environment) {
    register_uploader_helpers(env);
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
                let mut file_map: HashPairs = HashPairs::default();
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
                "SoliDB address not configured. Set SOLIDB_HOST env var or call set_solidb_address().".to_string()
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

            let mut result_hash: HashPairs = HashPairs::default();
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
                    "SoliDB address not configured. Set SOLIDB_HOST env var or call set_solidb_address().".to_string()
                })?;

                let files = match parse_multipart_from_req(req) {
                    Ok(f) => f,
                    Err(e) => return Err(e),
                };

                let mut results: Vec<Value> = Vec::new();
                for file in files {
                    match upload_blob_to_solidb(&solidb_addr, &collection, &file) {
                        Ok(result) => {
                            let mut result_hash: HashPairs = HashPairs::default();
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
                            let mut error_hash: HashPairs = HashPairs::default();
                            error_hash
                                .insert(HashKey::String("error".to_string()), Value::String(e));
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
                    "SoliDB address not configured. Set SOLIDB_HOST env var, call set_solidb_address(), or pass base_url."
                        .to_string()
                })?,
            };
            let _expires_in = match args.get(3) {
                Some(Value::Int(i)) => *i as u64,
                _ => 3600,
            };

            let database =
                std::env::var("SOLIDB_DATABASE").unwrap_or_else(|_| "solidb".to_string());
            let url = format!(
                "{}/_api/database/{}/document/{}/{}",
                base_url.trim_end_matches('/'),
                database,
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
                            filename = sanitize_filename(&header[fname_start + 10..end]);
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

    let mut client = SoliDBClient::connect(solidb_addr)
        .map_err(|e| format!("Failed to connect to SoliDB: {}", e))?;

    if let Ok(db) = std::env::var("SOLIDB_DATABASE") {
        client.set_database(&db);
    }

    // Auth priority matches the model layer: API key > cached JWT (from
    // SOLIDB_USERNAME/PASSWORD login) > basic auth fallback.
    use crate::interpreter::builtins::model::core::{get_api_key, get_jwt_token};
    if let Some(api_key) = get_api_key() {
        client = client.with_api_key(api_key);
    } else if let Some(jwt) = get_jwt_token() {
        client = client.with_jwt_token(jwt);
    } else if let (Ok(user), Ok(pass)) = (
        std::env::var("SOLIDB_USERNAME"),
        std::env::var("SOLIDB_PASSWORD"),
    ) {
        client = client.with_basic_auth(&user, &pass);
    }

    let raw_bytes = STANDARD
        .decode(&file.data_base64)
        .map_err(|e| format!("Failed to decode multipart data: {}", e))?;

    let blob_id = client
        .store_blob(collection, &raw_bytes, &file.filename, &file.content_type)
        .map_err(|e| format!("Failed to store blob: {}", e))?;

    Ok(UploadResult { blob_id })
}

/// Image transform query keys that `upload_url` accepts in its options
/// hash, in the canonical order we serialise them. Stable order means
/// identical option sets always produce identical URLs (CDN cache-key
/// stability). Grouping: geometry → orientation → effects → output.
const TRANSFORM_QUERY_ORDER: &[&str] = &[
    "w", "h", "thumb", "square", "crop", "fit", "flipx", "flipy", "rot", "blur", "bright",
    "contrast", "hue", "gray", "invert", "fmt", "q",
];

/// Parse the polymorphic 3rd arg of `upload_url(model, field, opts)` into
/// `(blob_id, transforms)`. The third arg may be:
///   - `String` → `blob_id` (multiple-mode shorthand)
///   - `Hash`   → options `{ blob_id?, w?, h?, thumb?, ..., q? }`
///   - anything else (incl. None / Null) → both empty
///
/// `transforms` are returned in the canonical query order so the URL is
/// deterministic regardless of hash insertion order. Unknown keys are
/// silently dropped, matching the JS-style "be liberal in what you accept"
/// that callers benefit from.
fn parse_upload_url_options(opts: Option<&Value>) -> (Option<String>, Vec<(String, String)>) {
    let mut blob_id_arg: Option<String> = None;
    let mut transform_params: Vec<(&'static str, String)> = Vec::new();

    match opts {
        Some(Value::String(s)) => blob_id_arg = Some(s.clone()),
        Some(Value::Hash(h)) => {
            for (k, v) in h.borrow().iter() {
                let HashKey::String(key_str) = k else {
                    continue;
                };
                if key_str == "blob_id" {
                    if let Value::String(s) = v {
                        blob_id_arg = Some(s.clone());
                    }
                    continue;
                }
                let Some(canonical) = TRANSFORM_QUERY_ORDER
                    .iter()
                    .copied()
                    .find(|&n| n == key_str)
                else {
                    continue;
                };
                // crop accepts an Array of 4 non-negative ints as a
                // shorthand: [10, 20, 300, 200] → "10,20,300,200".
                if canonical == "crop" {
                    if let Value::Array(arr) = v {
                        let arr_ref = arr.borrow();
                        if arr_ref.len() == 4 {
                            let parts: Option<Vec<String>> = arr_ref
                                .iter()
                                .map(|item| match item {
                                    Value::Int(n) if *n >= 0 => Some(n.to_string()),
                                    _ => None,
                                })
                                .collect();
                            if let Some(parts) = parts {
                                transform_params.push((canonical, parts.join(",")));
                            }
                        }
                        continue;
                    }
                }
                let value_str = match v {
                    Value::Int(n) => n.to_string(),
                    Value::Float(f) => {
                        // Strip trailing zeros for clean URLs:
                        // 3.5 → "3.5", 2.0 → "2".
                        if f.fract() == 0.0 && f.is_finite() {
                            format!("{}", *f as i64)
                        } else {
                            format!("{}", f)
                        }
                    }
                    Value::String(s) => s.clone(),
                    Value::Bool(true) => "1".to_string(),
                    _ => continue,
                };
                transform_params.push((canonical, value_str));
            }
        }
        _ => {}
    }

    let mut query_pairs: Vec<(String, String)> = Vec::new();
    for &name in TRANSFORM_QUERY_ORDER {
        if let Some((_, val)) = transform_params.iter().find(|(k, _)| *k == name) {
            query_pairs.push((name.to_string(), val.clone()));
        }
    }
    (blob_id_arg, query_pairs)
}

/// Build a URL by appending `?key=value&...` if `pairs` is non-empty. Values
/// are percent-encoded for query strings; keys are assumed to be the small
/// canonical set defined in `upload_url` and don't need escaping.
fn append_query(base: &str, pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return base.to_string();
    }
    let mut out = String::with_capacity(base.len() + pairs.len() * 12);
    out.push_str(base);
    let separator = if base.contains('?') { '&' } else { '?' };
    let mut first = true;
    for (k, v) in pairs {
        out.push(if first { separator } else { '&' });
        out.push_str(k);
        out.push('=');
        for ch in v.chars() {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
                out.push(ch);
            } else {
                let mut buf = [0u8; 4];
                for &byte in ch.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{:02X}", byte));
                }
            }
        }
        first = false;
    }
    out
}

/// Register the read-only uploader helpers (`upload_url`, `find_uploaded_file`)
/// directly in the env so they're available everywhere — including the
/// template-rendering env that doesn't see app-level Soli definitions.
/// The mutation helpers (`attach_upload`, `detach_upload`, etc.) live in the
/// Soli prelude in `serve::uploads_prelude` because they're only invoked from
/// controllers and benefit from staying overridable in plain Soli.
fn register_uploader_helpers(env: &mut Environment) {
    use crate::interpreter::builtins::model::{get_uploader, get_uploader_field_value_as_string};

    env.define(
        "upload_url".to_string(),
        Value::NativeFunction(NativeFunction::new("upload_url", None, |args| {
            // upload_url(model, field_name [, blob_id])
            let inst = match args.first() {
                Some(Value::Instance(inst)) => inst.clone(),
                Some(Value::Null) | None => return Ok(Value::Null),
                Some(other) => {
                    return Err(format!(
                        "upload_url() expects a model instance, got {}",
                        other.type_name()
                    ))
                }
            };
            let field = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                _ => return Err("upload_url() expects a string field name".to_string()),
            };
            // Third arg is polymorphic for ergonomics:
            //   String → blob_id (multiple-mode shorthand)
            //   Hash   → options { blob_id?, w?, h?, thumb?, crop?, fit?,
            //                       fmt?, q?, gray? }
            //   Null   → no options
            let (blob_id_arg, query_pairs) = parse_upload_url_options(args.get(2));

            let inst_ref = inst.borrow();
            let class_name = inst_ref.class.name.clone();
            let key = match inst_ref.get("_key") {
                Some(Value::String(s)) => s,
                _ => return Ok(Value::Null),
            };

            let Some(config) = get_uploader(&class_name, &field) else {
                return Ok(Value::Null);
            };

            let resource = format!("{}s", class_name.to_lowercase());
            let base = format!("/{}/{}/{}", resource, key, field);

            if config.multiple {
                let Some(blob_id) = blob_id_arg else {
                    return Ok(Value::Null);
                };
                // Blob id is in the path → URL is unique per blob without a
                // cache buster. Query string just carries transforms.
                let path = format!("{}/{}", base, blob_id);
                return Ok(Value::String(append_query(&path, &query_pairs)));
            }

            // Single mode: tack `?v=<blob_id>` on so the URL changes whenever
            // the underlying blob is replaced. Browsers / CDNs treat URLs
            // with different query strings as different cache entries —
            // cheapest correct invalidation strategy.
            let Some(stored_id) =
                get_uploader_field_value_as_string(&inst_ref, &format!("{}_blob_id", field))
            else {
                return Ok(Value::Null);
            };
            let mut all_pairs = vec![("v".to_string(), stored_id)];
            all_pairs.extend(query_pairs);
            Ok(Value::String(append_query(&base, &all_pairs)))
        })),
    );

    env.define(
        "find_uploaded_file".to_string(),
        Value::NativeFunction(NativeFunction::new("find_uploaded_file", Some(2), |args| {
            // find_uploaded_file(req, field_name) -> Hash | Null
            let req = match args.first() {
                Some(Value::Hash(h)) => h.clone(),
                _ => return Ok(Value::Null),
            };
            let field = match args.get(1) {
                Some(Value::String(s)) => s.clone(),
                _ => return Err("find_uploaded_file() expects a string field name".to_string()),
            };

            let req_ref = req.borrow();

            // Multipart guard.
            let headers_val = req_ref
                .iter()
                .find(|(k, _)| matches!(k, HashKey::String(s) if s == "headers"))
                .map(|(_, v)| v.clone());
            let is_multipart = match headers_val {
                Some(Value::Hash(h)) => h.borrow().iter().any(|(k, v)| {
                    matches!(k, HashKey::String(s) if s.eq_ignore_ascii_case("content-type"))
                        && matches!(v, Value::String(ct) if ct.contains("multipart/form-data"))
                }),
                _ => false,
            };
            if !is_multipart {
                return Ok(Value::Null);
            }

            // files iteration.
            let files_val = req_ref
                .iter()
                .find(|(k, _)| matches!(k, HashKey::String(s) if s == "files"))
                .map(|(_, v)| v.clone());
            let Some(Value::Array(files)) = files_val else {
                return Ok(Value::Null);
            };
            for f in files.borrow().iter() {
                if let Value::Hash(file) = f {
                    let file_ref = file.borrow();
                    let name_match = file_ref.iter().any(|(k, v)| {
                        matches!(k, HashKey::String(s) if s == "name")
                            && matches!(v, Value::String(n) if n == &field)
                    });
                    let has_filename = file_ref.iter().any(|(k, v)| {
                        matches!(k, HashKey::String(s) if s == "filename")
                            && matches!(v, Value::String(n) if !n.is_empty())
                    });
                    if name_match && has_filename {
                        return Ok(f.clone());
                    }
                }
            }
            Ok(Value::Null)
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::HashPairs;

    fn hash(pairs: &[(&str, Value)]) -> Value {
        let mut map = HashPairs::default();
        for (k, v) in pairs {
            map.insert(HashKey::String((*k).to_string()), v.clone());
        }
        Value::Hash(Rc::new(RefCell::new(map)))
    }

    fn array_int(items: &[i64]) -> Value {
        Value::Array(Rc::new(RefCell::new(
            items.iter().map(|n| Value::Int(*n)).collect(),
        )))
    }

    // ---- append_query ----

    #[test]
    fn append_query_empty_pairs_returns_base() {
        assert_eq!(append_query("/a/b", &[]), "/a/b");
    }

    #[test]
    fn append_query_appends_with_question_mark() {
        let pairs = vec![("v".into(), "abc".into()), ("w".into(), "200".into())];
        assert_eq!(append_query("/a/b", &pairs), "/a/b?v=abc&w=200");
    }

    #[test]
    fn append_query_uses_ampersand_when_base_already_has_query() {
        let pairs = vec![("w".into(), "200".into())];
        assert_eq!(append_query("/a?x=1", &pairs), "/a?x=1&w=200");
    }

    #[test]
    fn append_query_percent_encodes_values() {
        let pairs = vec![("crop".into(), "10,20,300,200".into())];
        assert_eq!(append_query("/a", &pairs), "/a?crop=10%2C20%2C300%2C200",);
    }

    #[test]
    fn append_query_preserves_unreserved_chars() {
        let pairs = vec![("k".into(), "abc-_.~".into())];
        assert_eq!(append_query("/a", &pairs), "/a?k=abc-_.~");
    }

    // ---- parse_upload_url_options ----

    #[test]
    fn parse_options_none_returns_empty() {
        let (id, pairs) = parse_upload_url_options(None);
        assert_eq!(id, None);
        assert!(pairs.is_empty());
    }

    #[test]
    fn parse_options_string_is_blob_id_shorthand() {
        let v = Value::String("blob42".into());
        let (id, pairs) = parse_upload_url_options(Some(&v));
        assert_eq!(id.as_deref(), Some("blob42"));
        assert!(pairs.is_empty());
    }

    #[test]
    fn parse_options_hash_blob_id_extracted() {
        let v = hash(&[("blob_id", Value::String("blob42".into()))]);
        let (id, pairs) = parse_upload_url_options(Some(&v));
        assert_eq!(id.as_deref(), Some("blob42"));
        assert!(pairs.is_empty());
    }

    #[test]
    fn parse_options_canonical_order_independent_of_insertion() {
        // Insert in scrambled order; expect canonical (geometry → effects → output).
        let v = hash(&[
            ("q", Value::Int(80)),
            ("fmt", Value::String("webp".into())),
            ("h", Value::Int(600)),
            ("w", Value::Int(800)),
            ("gray", Value::Bool(true)),
        ]);
        let (_, pairs) = parse_upload_url_options(Some(&v));
        let keys: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, vec!["w", "h", "gray", "fmt", "q"]);
    }

    #[test]
    fn parse_options_unknown_keys_dropped() {
        let v = hash(&[
            ("w", Value::Int(200)),
            ("nope", Value::String("x".into())),
            ("rotate", Value::Int(90)), // not "rot"
        ]);
        let (_, pairs) = parse_upload_url_options(Some(&v));
        assert_eq!(pairs, vec![("w".into(), "200".into())]);
    }

    #[test]
    fn parse_options_crop_array_shorthand() {
        let v = hash(&[("crop", array_int(&[10, 20, 300, 200]))]);
        let (_, pairs) = parse_upload_url_options(Some(&v));
        assert_eq!(pairs, vec![("crop".into(), "10,20,300,200".into())]);
    }

    #[test]
    fn parse_options_crop_array_wrong_arity_dropped() {
        let v = hash(&[("crop", array_int(&[10, 20, 30]))]);
        let (_, pairs) = parse_upload_url_options(Some(&v));
        assert!(pairs.is_empty(), "3-element crop should be dropped");
    }

    #[test]
    fn parse_options_crop_array_negative_dropped() {
        let v = hash(&[("crop", array_int(&[-1, 0, 100, 100]))]);
        let (_, pairs) = parse_upload_url_options(Some(&v));
        assert!(
            pairs.is_empty(),
            "negative crop component should be dropped"
        );
    }

    #[test]
    fn parse_options_crop_string_passthrough() {
        let v = hash(&[("crop", Value::String("10,20,300,200".into()))]);
        let (_, pairs) = parse_upload_url_options(Some(&v));
        assert_eq!(pairs, vec![("crop".into(), "10,20,300,200".into())]);
    }

    #[test]
    fn parse_options_bool_true_emits_one() {
        let v = hash(&[("gray", Value::Bool(true)), ("invert", Value::Bool(true))]);
        let (_, pairs) = parse_upload_url_options(Some(&v));
        assert_eq!(
            pairs,
            vec![("gray".into(), "1".into()), ("invert".into(), "1".into()),]
        );
    }

    #[test]
    fn parse_options_bool_false_dropped() {
        let v = hash(&[("gray", Value::Bool(false))]);
        let (_, pairs) = parse_upload_url_options(Some(&v));
        assert!(pairs.is_empty(), "false flag should not appear in URL");
    }

    #[test]
    fn parse_options_float_strips_trailing_zeros() {
        let v = hash(&[("blur", Value::Float(2.5)), ("contrast", Value::Float(2.0))]);
        let (_, pairs) = parse_upload_url_options(Some(&v));
        // Canonical order puts blur before contrast.
        assert_eq!(
            pairs,
            vec![
                ("blur".into(), "2.5".into()),
                ("contrast".into(), "2".into()),
            ]
        );
    }

    #[test]
    fn parse_options_square_shorthand_emitted_verbatim() {
        // upload_url emits `square=N` as-is — the controller expands the
        // shorthand server-side. Worth pinning so we don't accidentally
        // start emitting `w=N&h=N&fit=cover` from the URL builder, which
        // would change cache keys for every existing app.
        let v = hash(&[("square", Value::Int(200))]);
        let (_, pairs) = parse_upload_url_options(Some(&v));
        assert_eq!(pairs, vec![("square".into(), "200".into())]);
    }

    #[test]
    fn parse_options_blob_id_with_transforms_in_hash() {
        let v = hash(&[
            ("blob_id", Value::String("blob42".into())),
            ("w", Value::Int(200)),
        ]);
        let (id, pairs) = parse_upload_url_options(Some(&v));
        assert_eq!(id.as_deref(), Some("blob42"));
        assert_eq!(pairs, vec![("w".into(), "200".into())]);
    }

    #[test]
    fn parse_options_full_url_assembly_via_append_query() {
        // End-to-end check: a typical multi-option hash, fed through both
        // the parser and `append_query` exactly as upload_url does.
        let v = hash(&[
            ("h", Value::Int(600)),
            ("w", Value::Int(800)),
            ("fmt", Value::String("webp".into())),
            ("q", Value::Int(80)),
            ("gray", Value::Bool(true)),
        ]);
        let (_, pairs) = parse_upload_url_options(Some(&v));
        let url = append_query("/photos/42/img", &pairs);
        assert_eq!(url, "/photos/42/img?w=800&h=600&gray=1&fmt=webp&q=80");
    }
}
