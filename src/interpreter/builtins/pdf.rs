//! PDF generation builtins, backed by the `soli-pdf` crate.
//!
//!   * `pdf_render(template_json, data_json, options?)` — a plain PDF.
//!   * `pdf_facturx(template_json, data_json, facturx_xml, options?)` — a
//!     PDF/A-3b Factur-X (EN 16931) electronic invoice with the CII XML embedded.
//!   * `pdf_facturx_from_invoice(template_json, invoice_json, options?)` — same,
//!     but the CII XML (and the visual totals/VAT breakdown) are **generated**
//!     from a typed invoice document, so the PDF and the XML can never disagree.
//!
//! Both take the layout template and data as JSON strings and return the PDF as
//! a **base64 string** (Soli has no bytes type). Save it with
//! `file_write_base64(path, pdf)`, or send it as an HTTP response body.
//!
//! `options` is an optional hash:
//!   * `font_dirs`  — array of directories to load fonts from (default `["font"]`).
//!   * `fetch_images` — bool, whether to fetch remote `http(s)` images (default true).
//!   * `profile` (pdf_facturx) — `"minimum" | "basicwl" | "basic" | "en16931" | "extended"` (default `en16931`).
//!   * `title` / `author` / `subject` — document metadata (Info dictionary).
//!     For `pdf_render` the title defaults to `"invoice"` when unset.
//!   * `stationery` — path to a letterhead PDF (app-root relative) drawn
//!     beneath every page's content. Page 1 uses the letterhead's first page;
//!     later pages use its second page when present, else the first.
//!   * `attachments` — `[{ "path", "name"?, "mime"? }]` files embedded into the
//!     document (paths app-root relative; missing file is an error).
//!   * `password` / `owner_password` / `permissions` — AES-128 protection.
//!     `permissions` is a subset of `["print","copy","modify","annotate"]`
//!     (empty = allow all). Incompatible with `pdf_facturx*` (PDF/A).
//!   * `filename` (pdf_response) — sets `Content-Disposition: attachment`.
//!
//! `pdf_response(template, data, options?)` renders and returns a ready
//! response hash (`application/pdf`, binary body via `body_base64`) — return
//! it straight from a controller action, no `file_write_base64` dance.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use base64::Engine as _;
use soli_pdf::{FacturxMetadata, Profile, RenderOptions};
use time::OffsetDateTime;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, HashPairs, NativeFunction, Value};

/// Register the PDF builtins.
pub fn register_pdf_builtins(env: &mut Environment) {
    env.define(
        "pdf_render".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_render", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "pdf_render() expects 2 or 3 arguments (template, data, options?), got {}",
                    args.len()
                ));
            }
            let template = arg_string(&args[0], "pdf_render", "template")?;
            let data = arg_string(&args[1], "pdf_render", "data")?;
            let opts = render_options(args.get(2))?;
            let pdf = soli_pdf::render_to_bytes(template.as_bytes(), data.as_bytes(), &opts)
                .map_err(|e| format!("pdf_render() failed: {e}"))?;
            Ok(b64(pdf))
        })),
    );

    // Render + wrap as a ready HTTP response: return it straight from a
    // controller action. The binary body travels via the `body_base64`
    // response key (decoded server-side in `extract_response`).
    env.define(
        "pdf_response".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_response", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "pdf_response() expects 2 or 3 arguments (template, data, options?), got {}",
                    args.len()
                ));
            }
            let template = arg_string(&args[0], "pdf_response", "template")?;
            let data = arg_string(&args[1], "pdf_response", "data")?;
            let opts = render_options(args.get(2))?;
            let pdf = soli_pdf::render_to_bytes(template.as_bytes(), data.as_bytes(), &opts)
                .map_err(|e| format!("pdf_response() failed: {e}"))?;

            let mut headers = HashPairs::default();
            headers.insert(
                HashKey::String("Content-Type".into()),
                Value::String("application/pdf".into()),
            );
            if let Some(filename) = opt_str(args.get(2), "filename") {
                // Quote-escape so a weird filename can't break the header.
                let safe = filename.replace(['"', '\r', '\n'], "_");
                headers.insert(
                    HashKey::String("Content-Disposition".into()),
                    Value::String(format!("attachment; filename=\"{safe}\"").into()),
                );
            }

            let mut response = HashPairs::default();
            response.insert(HashKey::String("status".into()), Value::Int(200));
            response.insert(
                HashKey::String("headers".into()),
                Value::Hash(Rc::new(RefCell::new(headers))),
            );
            response.insert(HashKey::String("body_base64".into()), b64(pdf));
            Ok(Value::Hash(Rc::new(RefCell::new(response))))
        })),
    );

    env.define(
        "pdf_facturx".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_facturx", None, |args| {
            if args.len() < 3 || args.len() > 4 {
                return Err(format!(
                    "pdf_facturx() expects 3 or 4 arguments (template, data, xml, options?), got {}",
                    args.len()
                ));
            }
            let template = arg_string(&args[0], "pdf_facturx", "template")?;
            let data = arg_string(&args[1], "pdf_facturx", "data")?;
            let xml = arg_string(&args[2], "pdf_facturx", "xml")?;
            let opts = render_options(args.get(3))?;
            if opts.encrypt.is_some() {
                return Err("pdf_facturx(): encryption is incompatible with PDF/A-3b (Factur-X); drop `password`".to_string());
            }
            let profile = opt_str(args.get(3), "profile")
                .and_then(|s| Profile::parse(&s))
                .unwrap_or_default();
            let meta = facturx_meta(args.get(3));
            let pdf = soli_pdf::generate_facturx(
                template.as_bytes(),
                data.as_bytes(),
                xml.as_bytes(),
                profile,
                &meta,
                &opts,
            )
            .map_err(|e| format!("pdf_facturx() failed: {e}"))?;
            Ok(b64(pdf))
        })),
    );

    env.define(
        "pdf_facturx_from_invoice".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "pdf_facturx_from_invoice",
            None,
            |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(format!(
                        "pdf_facturx_from_invoice() expects 2 or 3 arguments (template, invoice, options?), got {}",
                        args.len()
                    ));
                }
                let template = arg_string(&args[0], "pdf_facturx_from_invoice", "template")?;
                let invoice_json = arg_string(&args[1], "pdf_facturx_from_invoice", "invoice")?;
                let invoice = soli_pdf::Invoice::parse(invoice_json.as_bytes())
                    .map_err(|e| format!("pdf_facturx_from_invoice() invalid invoice: {e}"))?;
                let opts = render_options(args.get(2))?;
                if opts.encrypt.is_some() {
                    return Err("pdf_facturx_from_invoice(): encryption is incompatible with PDF/A-3b (Factur-X); drop `password`".to_string());
                }
                let profile = opt_str(args.get(2), "profile")
                    .and_then(|s| Profile::parse(&s))
                    .unwrap_or_default();
                let meta = facturx_meta(args.get(2));
                let pdf = soli_pdf::generate_facturx_from_invoice(
                    template.as_bytes(),
                    &invoice,
                    profile,
                    &meta,
                    &opts,
                )
                .map_err(|e| format!("pdf_facturx_from_invoice() failed: {e}"))?;
                Ok(b64(pdf))
            },
        )),
    );
}

fn b64(bytes: Vec<u8>) -> Value {
    Value::String(
        base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .into(),
    )
}

fn arg_string(value: &Value, fn_name: &str, arg: &str) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.to_string()),
        other => Err(format!(
            "{fn_name}(): expects string {arg}, got {}",
            other.type_name()
        )),
    }
}

/// Resolve a (possibly relative) font directory against the app-root jail, so
/// `"font"` means `<app_root>/font` (matching `slurp`/`File`) rather than the
/// process CWD.
fn resolve_font_dir(dir: PathBuf) -> PathBuf {
    if dir.is_absolute() {
        return dir;
    }
    match crate::interpreter::builtins::file::jail_root() {
        Some(root) => root.join(dir),
        None => dir,
    }
}

/// Build `RenderOptions` from an optional options hash. Defaults the font search
/// path to a `font/` folder at the app root.
fn render_options(opts: Option<&Value>) -> Result<RenderOptions, String> {
    let mut dirs = vec![PathBuf::from("font")];
    let mut fetch_images = true;
    if let Some(Value::Hash(h)) = opts {
        let h = h.borrow();
        if let Some(Value::Bool(b)) = h.get(&HashKey::String("fetch_images".into())) {
            fetch_images = *b;
        }
        if let Some(Value::Array(arr)) = h.get(&HashKey::String("font_dirs".into())) {
            let provided: Vec<PathBuf> = arr
                .borrow()
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(PathBuf::from(s.to_string())),
                    _ => None,
                })
                .collect();
            if !provided.is_empty() {
                dirs = provided;
            }
        }
    }
    // Letterhead underlay: a path (resolved against the app root, like font
    // dirs) read into bytes here so the render stays IO-free. A missing or
    // unreadable file is a hard error — silently rendering WITHOUT the
    // letterhead would ship a broken document.
    let stationery = match opt_str(opts, "stationery") {
        Some(p) => {
            let resolved = resolve_font_dir(PathBuf::from(&p));
            Some(std::fs::read(&resolved).map_err(|e| {
                format!(
                    "stationery: could not read '{p}' ({}): {e}",
                    resolved.display()
                )
            })?)
        }
        None => None,
    };

    // Attachments: `[{ "path": "...", "name"?: "...", "mime"?: "..." }]`.
    // Paths resolve against the app root; a missing file is a hard error.
    let mut attachments = Vec::new();
    if let Some(Value::Hash(h)) = opts {
        if let Some(Value::Array(arr)) = h.borrow().get(&HashKey::String("attachments".into())) {
            for entry in arr.borrow().iter() {
                let Value::Hash(att) = entry else {
                    return Err("attachments: each entry must be a hash".to_string());
                };
                let att = att.borrow();
                let Some(Value::String(path)) = att.get(&HashKey::String("path".into())) else {
                    return Err("attachments: entry is missing \"path\"".to_string());
                };
                let path = path.to_string();
                let resolved = resolve_font_dir(PathBuf::from(&path));
                let bytes = std::fs::read(&resolved).map_err(|e| {
                    format!(
                        "attachments: could not read '{path}' ({}): {e}",
                        resolved.display()
                    )
                })?;
                let name = match att.get(&HashKey::String("name".into())) {
                    Some(Value::String(n)) => n.to_string(),
                    _ => PathBuf::from(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "attachment".to_string()),
                };
                let mime = match att.get(&HashKey::String("mime".into())) {
                    Some(Value::String(m)) => m.to_string(),
                    _ => guess_mime(&name),
                };
                attachments.push(soli_pdf::Attachment { name, mime, bytes });
            }
        }
    }

    // Password protection: `password` (open) / `owner_password` (unlock) /
    // `permissions` (["print","copy","modify","annotate"]).
    let encrypt = build_encrypt_options(opts);

    Ok(RenderOptions {
        font_dirs: dirs.into_iter().map(resolve_font_dir).collect(),
        fetch_images,
        title: opt_str(opts, "title"),
        author: opt_str(opts, "author"),
        subject: opt_str(opts, "subject"),
        stationery,
        attachments,
        encrypt,
        ..Default::default()
    })
}

/// Build `EncryptOptions` from `password` / `owner_password` / `permissions`.
/// Returns `None` when neither password is set (no encryption).
fn build_encrypt_options(opts: Option<&Value>) -> Option<soli_pdf::EncryptOptions> {
    let user = opt_str(opts, "password").unwrap_or_default();
    let owner = opt_str(opts, "owner_password").unwrap_or_default();
    if user.is_empty() && owner.is_empty() {
        return None;
    }
    let mut allow = Vec::new();
    if let Some(Value::Hash(h)) = opts {
        if let Some(Value::Array(arr)) = h.borrow().get(&HashKey::String("permissions".into())) {
            for v in arr.borrow().iter() {
                if let Value::String(s) = v {
                    allow.push(s.to_string());
                }
            }
        }
    }
    Some(soli_pdf::EncryptOptions {
        user_password: user,
        owner_password: owner,
        allow,
    })
}

/// A minimal extension→MIME map for attachments without an explicit `mime`.
fn guess_mime(name: &str) -> String {
    match name.rsplit('.').next().map(|e| e.to_ascii_lowercase()) {
        Some(ext) => match ext.as_str() {
            "xml" => "text/xml",
            "csv" => "text/csv",
            "json" => "application/json",
            "txt" => "text/plain",
            "pdf" => "application/pdf",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "zip" => "application/zip",
            _ => "application/octet-stream",
        },
        None => "application/octet-stream",
    }
    .to_string()
}

fn opt_str(opts: Option<&Value>, key: &str) -> Option<String> {
    if let Some(Value::Hash(h)) = opts {
        if let Some(Value::String(s)) = h.borrow().get(&HashKey::String(key.into())) {
            return Some(s.to_string());
        }
    }
    None
}

fn facturx_meta(opts: Option<&Value>) -> FacturxMetadata {
    let mut m = FacturxMetadata {
        created: now_odt(),
        ..Default::default()
    };
    if let Some(t) = opt_str(opts, "title") {
        m.title = t;
    }
    if let Some(a) = opt_str(opts, "author") {
        m.author = a;
    }
    if let Some(s) = opt_str(opts, "subject") {
        m.subject = s;
    }
    m
}

/// Current time as an `OffsetDateTime` without requiring time's `clock` feature.
fn now_odt() -> OffsetDateTime {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    OffsetDateTime::from_unix_timestamp(secs).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}
