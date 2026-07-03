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
//!   * `pdfa` — bool: emit PDF/A-3b (archival) output without a Factur-X
//!     payload. Incompatible with `password` (PDF/A forbids encryption) and with
//!     `pdf_facturx*` (already PDF/A). Composes with a tagged template — the
//!     output then declares both PDF/A-3b and PDF/UA-1 (accessible + archival).
//!   * `filename` (pdf_response) — sets `Content-Disposition: attachment`.
//!
//! `pdf_response(template, data, options?)` renders and returns a ready
//! response hash (`application/pdf`, binary body via `body_base64`) — return
//! it straight from a controller action, no `file_write_base64` dance.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use base64::Engine as _;
use sha2::{Digest, Sha256};
use soli_pdf::{FacturxMetadata, Profile, RenderOptions, SignMeta};
use time::OffsetDateTime;

use crate::interpreter::builtins::{pades, pdf_markdown};
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
            if opts.pdfa && opts.encrypt.is_some() {
                return Err("pdf_render(): `pdfa` is incompatible with password protection (PDF/A forbids encryption); drop `password`".to_string());
            }
            let sign = build_sign_config(args.get(2))?;
            if sign.is_some() && opts.encrypt.is_some() {
                return Err("pdf_render(): `sign` is incompatible with password protection (a signed PDF must not be encrypted); drop `password`".to_string());
            }
            let pdf = soli_pdf::render_to_bytes(template.as_bytes(), data.as_bytes(), &opts)
                .map_err(|e| format!("pdf_render() failed: {e}"))?;
            let pdf = apply_signature(pdf, sign.as_ref())?;
            Ok(b64(pdf))
        })),
    );

    // Markdown → PDF: fold a Markdown document into the layout engine's
    // template and render it. "Write prose, get a designed PDF." Accepts the
    // same render options as `pdf_render` (font_dirs, sign, pdfa, …) plus theme
    // overrides (fonts, fontSize, lineHeight, headingColor, textColor,
    // linkColor, codeColor).
    env.define(
        "pdf_from_markdown".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_from_markdown", None, |args| {
            if args.is_empty() || args.len() > 2 {
                return Err(format!(
                    "pdf_from_markdown() expects 1 or 2 arguments (markdown, options?), got {}",
                    args.len()
                ));
            }
            let md = arg_string(&args[0], "pdf_from_markdown", "markdown")?;
            let opts = render_options(args.get(1))?;
            let sign = build_sign_config(args.get(1))?;
            if sign.is_some() && opts.encrypt.is_some() {
                return Err("pdf_from_markdown(): `sign` is incompatible with password protection; drop `password`".to_string());
            }
            let theme = theme_from_options(args.get(1));
            let template = pdf_markdown::markdown_to_template(&md, &theme);
            let template_json = serde_json::to_vec(&template)
                .map_err(|e| format!("pdf_from_markdown(): building template: {e}"))?;
            let pdf = soli_pdf::render_to_bytes(&template_json, b"{}", &opts)
                .map_err(|e| format!("pdf_from_markdown() failed: {e}"))?;
            let pdf = apply_signature(pdf, sign.as_ref())?;
            Ok(b64(pdf))
        })),
    );

    // Fill an existing PDF's AcroForm fields from a `{ field => value }` hash —
    // the "take a government/enterprise form and fill it" workflow. `pdf` is an
    // app-root relative path or base64 PDF bytes; `options.flatten` bakes the
    // values in and locks the fields.
    env.define(
        "pdf_fill".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_fill", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "pdf_fill() expects 2 or 3 arguments (pdf, data, options?), got {}",
                    args.len()
                ));
            }
            let source = arg_string(&args[0], "pdf_fill", "pdf")?;
            let pdf_bytes = load_pdf_source(&source)?;
            let values = parse_field_values(&args[1])?;
            let flatten = matches!(
                args.get(2).and_then(|o| match o {
                    Value::Hash(h) => h.borrow().get(&HashKey::String("flatten".into())).cloned(),
                    _ => None,
                }),
                Some(Value::Bool(true))
            );
            let filled = soli_pdf::fill_form(&pdf_bytes, &values, flatten)
                .map_err(|e| format!("pdf_fill() failed: {e}"))?;
            Ok(b64(filled))
        })),
    );

    // Merge several PDFs into one (each source is a path or base64 bytes).
    env.define(
        "pdf_merge".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_merge", Some(1), |args| {
            let Value::Array(arr) = &args[0] else {
                return Err("pdf_merge() expects an array of PDFs (paths or base64)".to_string());
            };
            let mut pdfs = Vec::new();
            for v in arr.borrow().iter() {
                let s = match v {
                    Value::String(s) => s.to_string(),
                    other => {
                        return Err(format!(
                            "pdf_merge(): each entry must be a string, got {}",
                            other.type_name()
                        ))
                    }
                };
                pdfs.push(load_pdf_source(&s).map_err(|e| format!("pdf_merge(): {e}"))?);
            }
            let merged = soli_pdf::merge(&pdfs).map_err(|e| format!("pdf_merge() failed: {e}"))?;
            Ok(b64(merged))
        })),
    );

    // Keep a subset of pages: `pdf_pages(pdf, "1-3,7")` or `pdf_pages(pdf, [1,3])`.
    env.define(
        "pdf_pages".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_pages", Some(2), |args| {
            let source = arg_string(&args[0], "pdf_pages", "pdf")?;
            let pdf = load_pdf_source(&source).map_err(|e| format!("pdf_pages(): {e}"))?;
            let pages = parse_page_selection(&args[1]).map_err(|e| format!("pdf_pages(): {e}"))?;
            let out = soli_pdf::select_pages(&pdf, &pages)
                .map_err(|e| format!("pdf_pages() failed: {e}"))?;
            Ok(b64(out))
        })),
    );

    // Stamp text (label / diagonal watermark) onto an existing PDF's pages.
    env.define(
        "pdf_stamp".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_stamp", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "pdf_stamp() expects 2 or 3 arguments (pdf, text, options?), got {}",
                    args.len()
                ));
            }
            let source = arg_string(&args[0], "pdf_stamp", "pdf")?;
            let pdf = load_pdf_source(&source).map_err(|e| format!("pdf_stamp(): {e}"))?;
            let text = arg_string(&args[1], "pdf_stamp", "text")?;
            let opts = build_stamp_options(text, args.get(2));
            let out =
                soli_pdf::stamp(&pdf, &opts).map_err(|e| format!("pdf_stamp() failed: {e}"))?;
            Ok(b64(out))
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
            if opts.pdfa && opts.encrypt.is_some() {
                return Err("pdf_response(): `pdfa` is incompatible with password protection (PDF/A forbids encryption); drop `password`".to_string());
            }
            let sign = build_sign_config(args.get(2))?;
            if sign.is_some() && opts.encrypt.is_some() {
                return Err("pdf_response(): `sign` is incompatible with password protection (a signed PDF must not be encrypted); drop `password`".to_string());
            }
            let pdf = soli_pdf::render_to_bytes(template.as_bytes(), data.as_bytes(), &opts)
                .map_err(|e| format!("pdf_response() failed: {e}"))?;
            let pdf = apply_signature(pdf, sign.as_ref())?;

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
            if opts.pdfa {
                return Err("pdf_facturx(): PDF/A is implied by Factur-X; drop the `pdfa` option".to_string());
            }
            let sign = build_sign_config(args.get(3))?;
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
            let pdf = apply_signature(pdf, sign.as_ref())?;
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
                if opts.pdfa {
                    return Err("pdf_facturx_from_invoice(): PDF/A is implied by Factur-X; drop the `pdfa` option".to_string());
                }
                let sign = build_sign_config(args.get(2))?;
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
                let pdf = apply_signature(pdf, sign.as_ref())?;
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
    let mut pdfa = false;
    if let Some(Value::Hash(h)) = opts {
        let h = h.borrow();
        if let Some(Value::Bool(b)) = h.get(&HashKey::String("fetch_images".into())) {
            fetch_images = *b;
        }
        if let Some(Value::Bool(b)) = h.get(&HashKey::String("pdfa".into())) {
            pdfa = *b;
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

    // `pdfa` is set explicitly (not via `..Default::default()`) so a future
    // field reorder can't silently drop it.
    Ok(RenderOptions {
        font_dirs: dirs.into_iter().map(resolve_font_dir).collect(),
        fetch_images,
        title: opt_str(opts, "title"),
        author: opt_str(opts, "author"),
        subject: opt_str(opts, "subject"),
        stationery,
        attachments,
        encrypt,
        pdfa,
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

/// A parsed `sign` option: the signer material, the human-facing signature
/// dictionary metadata, and the signing time (shared by the `/M` entry and the
/// CMS signing-time attribute so they can't drift).
struct SignConfig {
    material: pades::SignerMaterial,
    meta: SignMeta,
    signing_time: OffsetDateTime,
    /// RFC 3161 TSA URL for a PAdES-B-T timestamp; `None` = B-B (no timestamp).
    tsa: Option<String>,
}

/// Read a `sign` sub-key that is either an inline PEM string or an app-root
/// relative path to a PEM file. Keys/certs are read here (never from request
/// data by convention) so the render itself stays IO-free.
fn load_pem(value: &str, what: &str) -> Result<String, String> {
    if value.contains("-----BEGIN") {
        return Ok(value.to_string());
    }
    let resolved = resolve_font_dir(PathBuf::from(value));
    std::fs::read_to_string(&resolved).map_err(|e| {
        format!(
            "sign: could not read {what} '{value}' ({}): {e}",
            resolved.display()
        )
    })
}

fn sign_str(hash: &HashPairs, key: &str) -> Option<String> {
    match hash.get(&HashKey::String(key.into())) {
        Some(Value::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// Format an `OffsetDateTime` (UTC) as a PDF date string `D:YYYYMMDDHHmmSS+00'00'`.
fn pdf_date(dt: OffsetDateTime) -> String {
    let dt = dt.to_offset(time::UtcOffset::UTC);
    format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}+00'00'",
        dt.year(),
        dt.month() as u8,
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    )
}

/// Parse the optional `sign` hash into a [`SignConfig`]. Returns `Ok(None)` when
/// no `sign` key is present. `cert` and `key` are required; `chain` (array),
/// `reason`, `location`, `name`, `contact` are optional.
fn build_sign_config(opts: Option<&Value>) -> Result<Option<SignConfig>, String> {
    let sign_val = match opts {
        Some(Value::Hash(h)) => h.borrow().get(&HashKey::String("sign".into())).cloned(),
        _ => None,
    };
    let sign = match sign_val {
        None => return Ok(None),
        Some(Value::Hash(s)) => s,
        Some(_) => return Err("sign: option must be a hash".to_string()),
    };
    let sb = sign.borrow();

    let cert_in =
        sign_str(&sb, "cert").ok_or("sign: `cert` (PEM string or path) is required".to_string())?;
    let key_in =
        sign_str(&sb, "key").ok_or("sign: `key` (PEM string or path) is required".to_string())?;
    let cert_der = pades::cert_to_der(&load_pem(&cert_in, "cert")?)
        .map_err(|e| format!("sign: certificate: {e}"))?;
    let key = pades::parse_private_key(&load_pem(&key_in, "key")?)
        .map_err(|e| format!("sign: private key: {e}"))?;

    let mut chain_der = Vec::new();
    if let Some(Value::Array(arr)) = sb.get(&HashKey::String("chain".into())) {
        for entry in arr.borrow().iter() {
            if let Value::String(pem) = entry {
                chain_der.push(
                    pades::cert_to_der(&load_pem(pem, "chain")?)
                        .map_err(|e| format!("sign: chain certificate: {e}"))?,
                );
            }
        }
    }

    let signing_time = now_odt();
    let meta = SignMeta {
        reason: sign_str(&sb, "reason"),
        location: sign_str(&sb, "location"),
        name: sign_str(&sb, "name"),
        contact: sign_str(&sb, "contact"),
        signing_time: Some(pdf_date(signing_time)),
    };
    Ok(Some(SignConfig {
        material: pades::SignerMaterial {
            cert_der,
            chain_der,
            key,
        },
        meta,
        signing_time,
        tsa: sign_str(&sb, "tsa"),
    }))
}

/// Reserve a signature slot, digest the ByteRange, build the CMS, and splice it
/// in. No-op when `cfg` is `None`. The placeholder scales to the embedded
/// certificates so a full chain never overflows it.
fn apply_signature(pdf: Vec<u8>, cfg: Option<&SignConfig>) -> Result<Vec<u8>, String> {
    let Some(cfg) = cfg else {
        return Ok(pdf);
    };
    let cert_bytes = cfg.material.cert_der.len()
        + cfg
            .material
            .chain_der
            .iter()
            .map(|c| c.len())
            .sum::<usize>();
    let mut placeholder = (cert_bytes + 4096).max(8192);
    // A PAdES-B-T timestamp token embeds the TSA's own certificate + CMS —
    // reserve extra room so it fits the placeholder.
    if cfg.tsa.is_some() {
        placeholder += 8192;
    }

    let prepared = soli_pdf::prepare_signature(&pdf, &cfg.meta, placeholder)
        .map_err(|e| format!("sign: {e}"))?;
    let digest = Sha256::digest(prepared.signed_bytes());
    let cms = pades::build_cms(&digest, &cfg.material, cfg.signing_time, cfg.tsa.as_deref())
        .map_err(|e| format!("sign: {e}"))?;
    soli_pdf::embed_cms(prepared, &cms).map_err(|e| format!("sign: {e}"))
}

/// Load a PDF source that is either an app-root relative path to an existing
/// file, or base64-encoded PDF bytes (what `pdf_render` returns).
fn load_pdf_source(s: &str) -> Result<Vec<u8>, String> {
    let resolved = resolve_font_dir(PathBuf::from(s));
    if resolved.is_file() {
        return std::fs::read(&resolved)
            .map_err(|e| format!("could not read PDF '{s}' ({}): {e}", resolved.display()));
    }
    base64::engine::general_purpose::STANDARD
        .decode(s.trim().as_bytes())
        .map_err(|_| "a PDF source is neither a readable path nor valid base64".to_string())
}

/// Parse a page selection: a range string like `"1-3,7,9-11"` or an array of
/// integers, into a list of 1-based page numbers.
fn parse_page_selection(v: &Value) -> Result<Vec<u32>, String> {
    match v {
        Value::String(s) => {
            let mut out = Vec::new();
            for part in s.split(',') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                if let Some((a, b)) = part.split_once('-') {
                    match (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                        (Ok(a), Ok(b)) if a >= 1 && b >= a => out.extend(a..=b),
                        _ => return Err(format!("invalid page range '{part}'")),
                    }
                } else {
                    out.push(
                        part.parse::<u32>()
                            .map_err(|_| format!("invalid page number '{part}'"))?,
                    );
                }
            }
            Ok(out)
        }
        Value::Array(arr) => Ok(arr
            .borrow()
            .iter()
            .filter_map(|v| match v {
                Value::Int(n) if *n >= 1 => Some(*n as u32),
                _ => None,
            })
            .collect()),
        other => Err(format!(
            "pages must be a range string or an array of ints, got {}",
            other.type_name()
        )),
    }
}

/// Parse a hex color (`"888888"`, optional leading `#`) into `(r, g, b)` in 0..1.
fn hex_rgb(s: &str) -> (f32, f32, f32) {
    let h = s.trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
    if h.len() >= 6 {
        (
            byte(0) as f32 / 255.0,
            byte(2) as f32 / 255.0,
            byte(4) as f32 / 255.0,
        )
    } else {
        (0.6, 0.6, 0.6)
    }
}

/// Build [`soli_pdf::StampOptions`] from the stamp text and an options hash.
fn build_stamp_options(text: String, opts: Option<&Value>) -> soli_pdf::StampOptions {
    let mut s = soli_pdf::StampOptions {
        text,
        ..Default::default()
    };
    let Some(Value::Hash(h)) = opts else {
        return s;
    };
    let h = h.borrow();
    if let Some(n) = h.get(&HashKey::String("size".into())).and_then(opt_num) {
        s.size = n;
    }
    if let Some(n) = h.get(&HashKey::String("rotation".into())).and_then(opt_num) {
        s.rotation = n;
    }
    if let Some(n) = h.get(&HashKey::String("opacity".into())).and_then(opt_num) {
        s.opacity = n;
    }
    if let Some(n) = h.get(&HashKey::String("x".into())).and_then(opt_num) {
        s.x = Some(n);
    }
    if let Some(n) = h.get(&HashKey::String("y".into())).and_then(opt_num) {
        s.y = Some(n);
    }
    if let Some(Value::String(c)) = h.get(&HashKey::String("color".into())) {
        s.color = hex_rgb(c);
    }
    if let Some(pages) = h.get(&HashKey::String("pages".into())) {
        if let Ok(list) = parse_page_selection(pages) {
            if !list.is_empty() {
                s.pages = Some(list);
            }
        }
    }
    s
}

/// Convert a `{ field => value }` hash into `(name, string)` pairs. Scalars are
/// stringified; a bool becomes `"true"`/`"false"` (checkboxes read that).
fn parse_field_values(v: &Value) -> Result<Vec<(String, String)>, String> {
    let Value::Hash(h) = v else {
        return Err("pdf_fill(): data must be a hash of field => value".to_string());
    };
    let mut out = Vec::new();
    for (k, val) in h.borrow().iter() {
        if let HashKey::String(key) = k {
            let s = match val {
                Value::String(s) => s.to_string(),
                Value::Int(n) => n.to_string(),
                Value::Float(f) => f.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            out.push((key.to_string(), s));
        }
    }
    Ok(out)
}

/// Build a Markdown [`pdf_markdown::Theme`] from the `options` hash, overriding
/// only the keys that are present.
fn theme_from_options(opts: Option<&Value>) -> pdf_markdown::Theme {
    let mut t = pdf_markdown::Theme::default();
    if let Some(s) = opt_str(opts, "headingColor") {
        t.heading_color = Some(s);
    }
    if let Some(s) = opt_str(opts, "textColor") {
        t.text_color = Some(s);
    }
    if let Some(s) = opt_str(opts, "linkColor") {
        t.link_color = s;
    }
    if let Some(s) = opt_str(opts, "codeColor") {
        t.code_color = s;
    }
    if let Some(Value::Hash(h)) = opts {
        let h = h.borrow();
        if let Some(n) = h.get(&HashKey::String("fontSize".into())).and_then(opt_num) {
            t.body_size = n;
        }
        if let Some(n) = h
            .get(&HashKey::String("lineHeight".into()))
            .and_then(opt_num)
        {
            t.line_height = n;
        }
        if let Some(Value::Array(arr)) = h.get(&HashKey::String("fonts".into())) {
            let fonts: Vec<String> = arr
                .borrow()
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect();
            if !fonts.is_empty() {
                t.fonts = fonts;
            }
        }
    }
    t
}

fn opt_num(v: &Value) -> Option<f32> {
    match v {
        Value::Int(n) => Some(*n as f32),
        Value::Float(f) => Some(*f as f32),
        _ => None,
    }
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
