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
                        .find(|(k, _)| matches!(k, HashKey::String(s) if **s == *"body"))
                        .map(|(_, v)| {
                            if let Value::String(s) = v { s.clone() } else { String::new().into() }
                        })
                        .unwrap_or_default()
                }
                _ => return Err("parse_multipart() expects request hash".to_string()),
            };

            let content_type = match req {
                Value::Hash(hash) => {
                    let borrowed = hash.borrow();
                    borrowed.iter()
                        .find(|(k, _)| matches!(k, HashKey::String(s) if **s == *"headers"))
                        .and_then(|(_, h)| {
                            if let Value::Hash(headers) = h {
                                let h_borrowed = headers.borrow();
                                h_borrowed.iter()
                                    .find(|(k, _)| matches!(k, HashKey::String(s) if **s == *"content-type" || **s == *"Content-Type"))
                                    .map(|(_, v)| {
                                        if let Value::String(s) = v { s.clone() } else { String::new().into() }
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
                file_map.insert(HashKey::String("filename".into()), Value::String(file.filename.into()));
                file_map.insert(HashKey::String("content_type".into()), Value::String(file.content_type.into()));
                file_map.insert(HashKey::String("size".into()), Value::Int(file.size as i64));
                file_map.insert(HashKey::String("data_base64".into()), Value::String(file.data_base64.into()));
                file_map.insert(HashKey::String("field_name".into()), Value::String(file.field_name.into()));
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
                .find(|f| *f.field_name == *field_name)
                .ok_or_else(|| format!("Field '{}' not found in multipart data", field_name))?;

            let result = upload_blob_to_solidb(&solidb_addr, &collection, &target_file)?;

            let mut result_hash: HashPairs = HashPairs::default();
            result_hash.insert(
                HashKey::String("blob_id".into()),
                Value::String(result.blob_id.into()),
            );
            result_hash.insert(
                HashKey::String("filename".into()),
                Value::String(target_file.filename.into()),
            );
            result_hash.insert(
                HashKey::String("size".into()),
                Value::Int(target_file.size as i64),
            );
            result_hash.insert(
                HashKey::String("content_type".into()),
                Value::String(target_file.content_type.into()),
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
                                HashKey::String("blob_id".into()),
                                Value::String(result.blob_id.into()),
                            );
                            result_hash.insert(
                                HashKey::String("filename".into()),
                                Value::String(file.filename.into()),
                            );
                            result_hash.insert(
                                HashKey::String("size".into()),
                                Value::Int(file.size as i64),
                            );
                            result_hash.insert(
                                HashKey::String("content_type".into()),
                                Value::String(file.content_type.into()),
                            );
                            result_hash.insert(
                                HashKey::String("field_name".into()),
                                Value::String(file.field_name.into()),
                            );
                            results.push(Value::Hash(Rc::new(RefCell::new(result_hash))));
                        }
                        Err(e) => {
                            let mut error_hash: HashPairs = HashPairs::default();
                            error_hash
                                .insert(HashKey::String("error".into()), Value::String(e.into()));
                            error_hash.insert(
                                HashKey::String("filename".into()),
                                Value::String(file.filename.into()),
                            );
                            error_hash.insert(
                                HashKey::String("field_name".into()),
                                Value::String(file.field_name.into()),
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
                })?.into(),
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
            Ok(Value::String(url.into()))
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
    // SEC-031 / SEC-090: cap at the same `SOLI_MAX_UPLOAD_FILES` limit the
    // server-side `parse_multipart_body` enforces. A multipart body packed
    // with thousands of tiny parts would otherwise allocate one
    // base64-encoded `ParsedFile` per part; the request body cap bounds
    // the input size but not the output amplification.
    let file_cap = crate::serve::file_upload::max_upload_files();
    let mut files = Vec::new();

    let boundary_line = format!("--{}", boundary);

    // Iterate over `body.split(...)` directly instead of materialising the
    // full `Vec<&str>` of parts up front: a hostile body shouldn't get to
    // pre-allocate a parts vector either.
    for part in body.split(&boundary_line) {
        if files.len() >= file_cap {
            break;
        }

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
                .find(|(k, _)| matches!(k, HashKey::String(s) if **s == *"body"))
                .map(|(_, v)| {
                    if let Value::String(s) = v {
                        s.clone()
                    } else {
                        String::new().into()
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
                .find(|(k, _)| matches!(k, HashKey::String(s) if **s == *"headers"))
                .and_then(|(_, h)| {
                    if let Value::Hash(headers) = h {
                        let h_borrowed = headers.borrow();
                        h_borrowed.iter()
                            .find(|(k, _)| matches!(k, HashKey::String(s) if **s == *"content-type" || **s == *"Content-Type"))
                            .map(|(_, v)| {
                                if let Value::String(s) = v { s.clone() } else { String::new().into() }
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
        client = client.with_jwt_token(&jwt);
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
        Some(Value::String(s)) => blob_id_arg = Some(s.clone().to_string()),
        Some(Value::Hash(h)) => {
            for (k, v) in h.borrow().iter() {
                let HashKey::String(key_str) = k else {
                    continue;
                };
                if **key_str == *"blob_id" {
                    if let Value::String(s) = v {
                        blob_id_arg = Some(s.clone().to_string());
                    }
                    continue;
                }
                let Some(canonical) = TRANSFORM_QUERY_ORDER
                    .iter()
                    .copied()
                    .find(|&n| *n == **key_str)
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
                    Value::String(s) => s.clone().to_string(),
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
/// Read a string value from a hash by key, or `None` if absent/non-string.
fn hash_str(h: &HashPairs, key: &str) -> Option<String> {
    match h.get(&HashKey::String(key.into())) {
        Some(Value::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// Read an integer value from a hash by key, or `None` if absent/non-int.
fn hash_int(h: &HashPairs, key: &str) -> Option<i64> {
    match h.get(&HashKey::String(key.into())) {
        Some(Value::Int(n)) => Some(*n),
        _ => None,
    }
}

/// Map an `image::ImageFormat` to its canonical file extension + MIME type.
fn format_ext_and_ct(fmt: image::ImageFormat) -> (&'static str, &'static str) {
    use image::ImageFormat as F;
    match fmt {
        F::Jpeg => ("jpg", "image/jpeg"),
        F::Png => ("png", "image/png"),
        F::WebP => ("webp", "image/webp"),
        F::Gif => ("gif", "image/gif"),
        F::Bmp => ("bmp", "image/bmp"),
        F::Tiff => ("tiff", "image/tiff"),
        F::Ico => ("ico", "image/x-icon"),
        _ => ("bin", "application/octet-stream"),
    }
}

/// Replace a filename's extension (e.g. `photo.png` → `photo.webp`).
fn swap_extension(filename: &str, new_ext: &str) -> String {
    match filename.rfind('.') {
        Some(idx) if idx > 0 => format!("{}.{}", &filename[..idx], new_ext),
        _ => format!("{}.{}", filename, new_ext),
    }
}

/// Storage-time image transform driven by an uploader config. Decodes the
/// uploaded image, optionally downscales it to fit `max_width`/`max_height`
/// (never upscaling, aspect preserved), and re-encodes it to the configured
/// `format`/`quality`. Returns:
///   - `Ok(Some(new_file))` when the bytes were transformed (with updated
///     `data`, `content_type`, `size`, and `filename` extension),
///   - `Ok(None)` when nothing should change — no transform configured, the
///     upload isn't an image (PDF, csv, …), or the bytes aren't a decodable
///     image (in which case the original is stored as-is so an upload is never
///     blocked by a transform failure).
fn transform_upload_file(
    file: &Rc<RefCell<HashPairs>>,
    config: &Rc<RefCell<HashPairs>>,
) -> Result<Option<HashPairs>, String> {
    use crate::interpreter::builtins::image::{encode_dynamic_image, format_from_str};

    let cfg = config.borrow();
    let target_fmt_name = hash_str(&cfg, "format");
    let max_w = hash_int(&cfg, "max_width")
        .filter(|n| *n > 0)
        .map(|n| n as u32);
    let max_h = hash_int(&cfg, "max_height")
        .filter(|n| *n > 0)
        .map(|n| n as u32);
    let quality = hash_int(&cfg, "quality")
        .filter(|n| (1..=100).contains(n))
        .map(|n| n as u8)
        .unwrap_or(82);
    drop(cfg);

    // Nothing requested → no-op.
    if target_fmt_name.is_none() && max_w.is_none() && max_h.is_none() {
        return Ok(None);
    }

    let f = file.borrow();
    let content_type = hash_str(&f, "content_type").unwrap_or_default();
    // Only transform images; PDFs, csv, zip, … pass straight through.
    if !content_type.starts_with("image/") {
        return Ok(None);
    }
    let Some(data_b64) = hash_str(&f, "data") else {
        return Ok(None);
    };
    let filename = hash_str(&f, "filename").unwrap_or_else(|| "upload".to_string());
    drop(f);

    let Ok(bytes) = STANDARD.decode(data_b64.as_bytes()) else {
        return Ok(None);
    };
    let Ok(mut img) = image::load_from_memory(&bytes) else {
        // Not a decodable image → store the original bytes untouched.
        return Ok(None);
    };

    // Target format: explicit config wins, otherwise keep the source format.
    let source_fmt = content_type
        .strip_prefix("image/")
        .and_then(format_from_str)
        .or_else(|| image::guess_format(&bytes).ok());
    let target_fmt = match target_fmt_name.as_deref() {
        Some(name) => format_from_str(name),
        None => source_fmt,
    };
    let Some(target_fmt) = target_fmt else {
        return Ok(None);
    };

    // Downscale only (never upscale), preserving aspect ratio.
    let (w, h) = (img.width(), img.height());
    let mut scale = 1.0f64;
    if let Some(mw) = max_w {
        if w > mw {
            scale = scale.min(mw as f64 / w as f64);
        }
    }
    if let Some(mh) = max_h {
        if h > mh {
            scale = scale.min(mh as f64 / h as f64);
        }
    }
    let resized = scale < 1.0;
    if resized {
        let nw = ((w as f64 * scale).round() as u32).max(1);
        let nh = ((h as f64 * scale).round() as u32).max(1);
        img = img.resize(nw, nh, image::imageops::FilterType::Lanczos3);
    }

    // No explicit format change and no resize actually happened → leave as-is
    // rather than needlessly re-encoding.
    if target_fmt_name.is_none() && !resized {
        return Ok(None);
    }

    let encoded = encode_dynamic_image(&img, quality, target_fmt)?;
    let (ext, new_ct) = format_ext_and_ct(target_fmt);
    let new_b64 = STANDARD.encode(&encoded);

    let mut out = file.borrow().clone();
    out.insert(
        HashKey::String("data".into()),
        Value::String(new_b64.into()),
    );
    out.insert(
        HashKey::String("content_type".into()),
        Value::String(new_ct.into()),
    );
    out.insert(
        HashKey::String("size".into()),
        Value::Int(encoded.len() as i64),
    );
    out.insert(
        HashKey::String("filename".into()),
        Value::String(swap_extension(&filename, ext).into()),
    );
    Ok(Some(out))
}

fn register_uploader_helpers(env: &mut Environment) {
    use crate::interpreter::builtins::model::{get_uploader, get_uploader_field_value_as_string};

    // Storage-time image transform (format conversion + downscale) driven by
    // the uploader's `format`/`quality`/`max_width`/`max_height` options. The
    // attach prelude calls this just before storing the blob.
    env.define(
        "apply_uploader_transform".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "apply_uploader_transform",
            Some(2),
            |args| {
                let Some(Value::Hash(file)) = args.first() else {
                    return Err("apply_uploader_transform() expects a file hash".to_string());
                };
                let Some(Value::Hash(config)) = args.get(1) else {
                    // No config hash → return the file unchanged.
                    return Ok(args.first().cloned().unwrap_or(Value::Null));
                };
                match transform_upload_file(file, config)? {
                    Some(new_file) => Ok(Value::Hash(Rc::new(RefCell::new(new_file)))),
                    None => Ok(Value::Hash(file.clone())),
                }
            },
        )),
    );

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
                return Ok(Value::String(append_query(&path, &query_pairs).into()));
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
            Ok(Value::String(append_query(&base, &all_pairs).into()))
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
                .find(|(k, _)| matches!(k, HashKey::String(s) if **s == *"headers"))
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
                .find(|(k, _)| matches!(k, HashKey::String(s) if **s == *"files"))
                .map(|(_, v)| v.clone());
            let Some(Value::Array(files)) = files_val else {
                return Ok(Value::Null);
            };
            for f in files.borrow().iter() {
                if let Value::Hash(file) = f {
                    let file_ref = file.borrow();
                    let name_match = file_ref.iter().any(|(k, v)| {
                        matches!(k, HashKey::String(s) if **s == *"name")
                            && matches!(v, Value::String(n) if n == &field)
                    });
                    let has_filename = file_ref.iter().any(|(k, v)| {
                        matches!(k, HashKey::String(s) if **s == *"filename")
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
            map.insert(HashKey::String((*k).to_string().into()), v.clone());
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

    // ---- parse_multipart_data (SEC-090) ----

    /// Build a multipart body containing `n` tiny file parts. Each part
    /// carries a 1-byte payload — what an attacker would actually send to
    /// maximise per-part allocations against the worker.
    fn multipart_with_n_files(boundary: &str, n: usize) -> String {
        let mut s = String::new();
        for i in 0..n {
            s.push_str(&format!("--{}\r\n", boundary));
            s.push_str(&format!(
                "Content-Disposition: form-data; name=\"f{}\"; filename=\"a{}.bin\"\r\n",
                i, i
            ));
            s.push_str("Content-Type: application/octet-stream\r\n\r\n");
            s.push('X');
            s.push_str("\r\n");
        }
        s.push_str(&format!("--{}--\r\n", boundary));
        s
    }

    #[test]
    fn parse_multipart_data_caps_at_max_upload_files() {
        // SEC-090: a body packed with > cap parts must allocate at most
        // `max_upload_files()` ParsedFile entries — the server-side parser
        // already enforces this; the helper used to ignore it.
        let cap = crate::serve::file_upload::max_upload_files();
        let boundary = "----testbnd";
        let body = multipart_with_n_files(boundary, cap + 5);
        let files = parse_multipart_data(&body, boundary).unwrap();
        assert!(
            files.len() <= cap,
            "expected at most {} files past the cap, got {}",
            cap,
            files.len()
        );
        assert_eq!(files.len(), cap, "should hit the cap exactly");
    }

    #[test]
    fn parse_multipart_data_under_cap_returns_all_files() {
        // Ensure the cap doesn't regress the common case.
        let boundary = "----testbnd";
        let body = multipart_with_n_files(boundary, 3);
        let files = parse_multipart_data(&body, boundary).unwrap();
        assert_eq!(files.len(), 3);
        for (i, f) in files.iter().enumerate() {
            assert_eq!(f.field_name, format!("f{}", i));
            assert!(f.filename.starts_with(&format!("a{}", i)));
        }
    }

    #[test]
    fn parse_multipart_data_skips_form_field_parts_with_no_filename() {
        // Plain form fields (no filename) are not files — they shouldn't
        // count toward the cap or appear in the result either way.
        let boundary = "----testbnd";
        let body = format!(
            "--{b}\r\n\
             Content-Disposition: form-data; name=\"text\"\r\n\r\n\
             hello\r\n\
             --{b}\r\n\
             Content-Disposition: form-data; name=\"file\"; filename=\"x.bin\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n\
             A\r\n\
             --{b}--\r\n",
            b = boundary
        );
        let files = parse_multipart_data(&body, boundary).unwrap();
        assert_eq!(files.len(), 1, "plain form field must not produce a file");
        assert_eq!(files[0].field_name, "file");
    }
}

#[cfg(test)]
mod transform_tests {
    use super::*;

    fn make_png_b64(w: u32, h: u32) -> String {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        }));
        let mut bytes = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
        STANDARD.encode(&bytes)
    }

    fn file_hash(filename: &str, ct: &str, data_b64: &str) -> Rc<RefCell<HashPairs>> {
        let mut h = HashPairs::default();
        h.insert(
            HashKey::String("filename".into()),
            Value::String(filename.into()),
        );
        h.insert(
            HashKey::String("content_type".into()),
            Value::String(ct.into()),
        );
        h.insert(
            HashKey::String("data".into()),
            Value::String(data_b64.into()),
        );
        h.insert(
            HashKey::String("size".into()),
            Value::Int(data_b64.len() as i64),
        );
        Rc::new(RefCell::new(h))
    }

    fn config_hash(pairs: &[(&str, Value)]) -> Rc<RefCell<HashPairs>> {
        let mut h = HashPairs::default();
        for (k, v) in pairs {
            h.insert(HashKey::String((*k).into()), v.clone());
        }
        Rc::new(RefCell::new(h))
    }

    #[test]
    fn converts_png_to_webp_and_downscales() {
        let b64 = make_png_b64(1200, 800);
        let file = file_hash("photo.png", "image/png", &b64);
        let cfg = config_hash(&[
            ("format", Value::String("webp".into())),
            ("quality", Value::Int(80)),
            ("max_width", Value::Int(600)),
            ("max_height", Value::Int(600)),
        ]);
        let out = transform_upload_file(&file, &cfg)
            .unwrap()
            .expect("should transform");
        assert_eq!(
            hash_str(&out, "content_type").as_deref(),
            Some("image/webp")
        );
        assert_eq!(hash_str(&out, "filename").as_deref(), Some("photo.webp"));
        let bytes = STANDARD.decode(hash_str(&out, "data").unwrap()).unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        // Downscaled to fit 600x600, aspect preserved (1200x800 -> 600x400).
        assert_eq!((img.width(), img.height()), (600, 400));
    }

    #[test]
    fn non_image_uploads_pass_through_untouched() {
        let file = file_hash("doc.pdf", "application/pdf", "JVBERi0xLjQK");
        let cfg = config_hash(&[("format", Value::String("webp".into()))]);
        assert!(transform_upload_file(&file, &cfg).unwrap().is_none());
    }

    #[test]
    fn no_transform_options_is_noop() {
        let b64 = make_png_b64(64, 64);
        let file = file_hash("a.png", "image/png", &b64);
        let cfg = config_hash(&[]);
        assert!(transform_upload_file(&file, &cfg).unwrap().is_none());
    }

    #[test]
    fn size_caps_alone_skip_when_already_small() {
        let b64 = make_png_b64(100, 100);
        let file = file_hash("a.png", "image/png", &b64);
        let cfg = config_hash(&[
            ("max_width", Value::Int(600)),
            ("max_height", Value::Int(600)),
        ]);
        // Smaller than the caps and no format change → nothing to do.
        assert!(transform_upload_file(&file, &cfg).unwrap().is_none());
    }

    #[test]
    fn format_only_reencodes_without_resize() {
        let b64 = make_png_b64(300, 200);
        let file = file_hash("a.png", "image/png", &b64);
        let cfg = config_hash(&[("format", Value::String("jpeg".into()))]);
        let out = transform_upload_file(&file, &cfg)
            .unwrap()
            .expect("format change should transform");
        assert_eq!(
            hash_str(&out, "content_type").as_deref(),
            Some("image/jpeg")
        );
        assert_eq!(hash_str(&out, "filename").as_deref(), Some("a.jpg"));
        let bytes = STANDARD.decode(hash_str(&out, "data").unwrap()).unwrap();
        let img = image::load_from_memory(&bytes).unwrap();
        assert_eq!((img.width(), img.height()), (300, 200));
    }
}
