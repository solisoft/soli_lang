//! PDF generation builtins, backed by the `soli-pdf` crate.
//!
//!   * `pdf_render(template_json, data_json, options?)` — a plain PDF.
//!   * `pdf_facturx(template_json, data_json, facturx_xml, options?)` — a
//!     PDF/A-3b Factur-X (EN 16931) electronic invoice with the CII XML embedded.
//!   * `pdf_facturx_from_invoice(template_json, invoice_json, options?)` — same,
//!     but the CII XML (and the visual totals/VAT breakdown) are **generated**
//!     from a typed invoice document, so the PDF and the XML can never disagree.
//!   * `pdf_preview(template_json, data_json, options?)` — the same document as
//!     one **base64 PNG per page**, for thumbnails and previews. With
//!     `out_dir` it writes the files and returns their paths instead.
//!   * `pdf_preview_from_markdown(markdown, options?)` — the Markdown
//!     counterpart.
//!   * `pdf_preview_response(template, data, options?)` — one page as a ready
//!     `image/png` response hash.
//!
//! Previews rasterise the layout engine's own draw model, not PDF bytes, so
//! they cover what the renderer produces — not a merged, filled or stamped
//! PDF, which exists only as bytes. Raster-only options: `dpi` (default 96),
//! `width`/`height` in px (either overrides `dpi`), `pages` (the 1-based
//! selection `pdf_pages` takes), `out_dir`/`prefix`, and `page` for the
//! response helper. `stationery`, `attachments`, `password`, `pdfa` and `sign`
//! are accepted but have no raster meaning, so one options hash can drive both
//! the PDF and its preview; the two that change what you would see —
//! `stationery` and `sign` — warn once per process.
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
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use base64::Engine as _;
use sha2::{Digest, Sha256};
use soli_pdf::{FacturxMetadata, Profile, RenderOptions, SignMeta};
use time::OffsetDateTime;

use crate::interpreter::builtins::{pades, pdf_markdown};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, HashPairs, NativeFunction, Value};

/// Register the PDF builtins.
/// Teach the PDF renderer which image sources it may load.
///
/// Its loader took any `http(s)` URL through a bare client (redirects followed,
/// no blocklist — a blind SSRF, reachable from `pdf_from_markdown` via
/// `![](http://169.254.169.254/…)`) and read any other string straight off
/// disk, outside the app-root jail, embedding another user's private upload in
/// the generated document. Both now go through the same policies as the rest of
/// the runtime.
fn install_pdf_image_source_guards() {
    soli_pdf::images::set_image_source_guards(
        crate::interpreter::builtins::http_class::validate_url_for_ssrf,
        |path| crate::interpreter::builtins::file::resolve_readable_path(path, "PDF image"),
    );
}

pub fn register_pdf_builtins(env: &mut Environment) {
    install_pdf_image_source_guards();
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

    // The same document as a page image. Previews rasterise the layout
    // engine's own draw model rather than the PDF, so they cover what
    // `pdf_render` produces — not a merged, filled or stamped PDF, which
    // exists only as bytes.
    env.define(
        "pdf_preview".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_preview", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "pdf_preview() expects 2 or 3 arguments (template, data, options?), got {}",
                    args.len()
                ));
            }
            let template = arg_string(&args[0], "pdf_preview", "template")?;
            let data = arg_string(&args[1], "pdf_preview", "data")?;
            let (pages, format) = preview_pages(
                template.as_bytes(),
                data.as_bytes(),
                args.get(2),
                "pdf_preview",
            )?;
            preview_result(pages, format, args.get(2), "pdf_preview")
        })),
    );

    // Where every element landed. A visual editor cannot work this out itself:
    // a flowing element's position depends on everything before it, so only the
    // layout engine knows it. Returns one hash per drawn element — a `repeat`
    // row yields several boxes sharing one `path`, because it is authored once
    // and drawn per item.
    env.define(
        "pdf_layout_map".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_layout_map", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "pdf_layout_map() expects 2 or 3 arguments (template, data, options?), got {}",
                    args.len()
                ));
            }
            let template = arg_string(&args[0], "pdf_layout_map", "template")?;
            let data = arg_string(&args[1], "pdf_layout_map", "data")?;
            let opts = render_options(args.get(2))?;
            let boxes = soli_pdf::layout_boxes(template.as_bytes(), data.as_bytes(), &opts)
                .map_err(|e| format!("pdf_layout_map() failed: {e}"))?;
            let out: Vec<Value> = boxes
                .into_iter()
                .map(|b| {
                    let mut h = HashPairs::default();
                    h.insert(HashKey::String("path".into()), Value::String(b.path.into()));
                    h.insert(HashKey::String("kind".into()), Value::String(b.kind.into()));
                    h.insert(HashKey::String("page".into()), Value::Int(b.page as i64));
                    h.insert(HashKey::String("x".into()), Value::Float(b.x as f64));
                    h.insert(HashKey::String("y".into()), Value::Float(b.y as f64));
                    h.insert(HashKey::String("w".into()), Value::Float(b.w as f64));
                    h.insert(HashKey::String("h".into()), Value::Float(b.h as f64));
                    Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(h)))
                })
                .collect();
            Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(out))))
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

    // Markdown straight to page images — the counterpart of
    // `pdf_from_markdown`. A separate builtin rather than an option because
    // the argument shape differs: there is no `data` document.
    env.define(
        "pdf_preview_from_markdown".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "pdf_preview_from_markdown",
            None,
            |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(format!(
                        "pdf_preview_from_markdown() expects 1 or 2 arguments \
                         (markdown, options?), got {}",
                        args.len()
                    ));
                }
                let md = arg_string(&args[0], "pdf_preview_from_markdown", "markdown")?;
                let template_json =
                    markdown_template_json(&md, args.get(1), "pdf_preview_from_markdown")?;
                let (pages, format) = preview_pages(
                    &template_json,
                    b"{}",
                    args.get(1),
                    "pdf_preview_from_markdown",
                )?;
                preview_result(pages, format, args.get(1), "pdf_preview_from_markdown")
            },
        )),
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

    // Digitally sign an EXISTING PDF (path or base64) — the standalone sibling
    // of the `sign` render option. Options are the sign config directly
    // (`{ cert, key, chain?, reason?, tsa?, appearance? }`). Composes with the
    // toolkit: merge → sign, fill → sign.
    env.define(
        "pdf_sign".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_sign", Some(2), |args| {
            let source = arg_string(&args[0], "pdf_sign", "pdf")?;
            let pdf = load_pdf_source(&source).map_err(|e| format!("pdf_sign(): {e}"))?;
            let Value::Hash(opts) = &args[1] else {
                return Err(
                    "pdf_sign() expects an options hash { cert, key, … } as the 2nd argument"
                        .to_string(),
                );
            };
            let cfg = sign_config_from_hash(&opts.borrow())?;
            let signed = apply_signature(pdf, Some(&cfg))?;
            Ok(b64(signed))
        })),
    );

    // Verify the digital signatures embedded in a PDF (path or base64). Returns
    // an array of `{ field, valid, covers_document, signer, reason?, signed_at? }`
    // — the reverse of pdf_sign. `valid` = the CMS verifies AND the ByteRange
    // digest matches; it does not assert certificate trust.
    env.define(
        "pdf_verify".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_verify", Some(1), |args| {
            let source = arg_string(&args[0], "pdf_verify", "pdf")?;
            let pdf = load_pdf_source(&source).map_err(|e| format!("pdf_verify(): {e}"))?;
            let sigs =
                soli_pdf::extract_signatures(&pdf).map_err(|e| format!("pdf_verify(): {e}"))?;
            let items: Vec<Value> = sigs
                .into_iter()
                .map(|s| {
                    let outcome = pades::verify_cms(&s.cms_der, &s.signed_bytes);
                    let mut h = HashPairs::default();
                    h.insert(
                        HashKey::String("field".into()),
                        Value::String(s.field.into()),
                    );
                    h.insert(
                        HashKey::String("valid".into()),
                        Value::Bool(outcome.as_ref().map(|o| o.valid).unwrap_or(false)),
                    );
                    h.insert(
                        HashKey::String("covers_document".into()),
                        Value::Bool(s.covers_document),
                    );
                    let signer = outcome.as_ref().ok().and_then(|o| o.signer.clone());
                    h.insert(
                        HashKey::String("signer".into()),
                        signer
                            .map(|c| Value::String(c.into()))
                            .unwrap_or(Value::Null),
                    );
                    if let Some(r) = s.reason {
                        h.insert(HashKey::String("reason".into()), Value::String(r.into()));
                    }
                    if let Some(t) = s.signing_time {
                        h.insert(HashKey::String("signed_at".into()), Value::String(t.into()));
                    }
                    if let Err(e) = &outcome {
                        h.insert(
                            HashKey::String("error".into()),
                            Value::String(e.clone().into()),
                        );
                    }
                    Value::Hash(Rc::new(RefCell::new(h)))
                })
                .collect();
            Ok(Value::Array(Rc::new(RefCell::new(items))))
        })),
    );

    // Read the embedded Factur-X / ZUGFeRD / XRechnung invoice XML out of a
    // *received* PDF — the reverse of pdf_facturx. Returns the XML string, or
    // null when the PDF carries no e-invoice payload.
    env.define(
        "pdf_extract_facturx".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "pdf_extract_facturx",
            Some(1),
            |args| {
                let source = arg_string(&args[0], "pdf_extract_facturx", "pdf")?;
                let pdf =
                    load_pdf_source(&source).map_err(|e| format!("pdf_extract_facturx(): {e}"))?;
                match soli_pdf::extract_facturx(&pdf)
                    .map_err(|e| format!("pdf_extract_facturx() failed: {e}"))?
                {
                    Some(xml) => Ok(Value::String(
                        String::from_utf8_lossy(&xml).into_owned().into(),
                    )),
                    None => Ok(Value::Null),
                }
            },
        )),
    );

    // List every embedded file in a PDF: `[{ name, mime, size, base64 }]`.
    env.define(
        "pdf_attachments".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_attachments", Some(1), |args| {
            let source = arg_string(&args[0], "pdf_attachments", "pdf")?;
            let pdf = load_pdf_source(&source).map_err(|e| format!("pdf_attachments(): {e}"))?;
            let atts = soli_pdf::extract_attachments(&pdf)
                .map_err(|e| format!("pdf_attachments() failed: {e}"))?;
            let items: Vec<Value> = atts
                .into_iter()
                .map(|a| {
                    let mut h = HashPairs::default();
                    h.insert(HashKey::String("name".into()), Value::String(a.name.into()));
                    h.insert(HashKey::String("mime".into()), Value::String(a.mime.into()));
                    h.insert(
                        HashKey::String("size".into()),
                        Value::Int(a.bytes.len() as i64),
                    );
                    h.insert(
                        HashKey::String("base64".into()),
                        Value::String(
                            base64::engine::general_purpose::STANDARD
                                .encode(&a.bytes)
                                .into(),
                        ),
                    );
                    Value::Hash(Rc::new(RefCell::new(h)))
                })
                .collect();
            Ok(Value::Array(Rc::new(RefCell::new(items))))
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

    // One page as a ready `image/png` response — the preview mirror of
    // `pdf_response`.
    env.define(
        "pdf_preview_response".to_string(),
        Value::NativeFunction(NativeFunction::new("pdf_preview_response", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "pdf_preview_response() expects 2 or 3 arguments (template, data, options?), got {}",
                    args.len()
                ));
            }
            let template = arg_string(&args[0], "pdf_preview_response", "template")?;
            let data = arg_string(&args[1], "pdf_preview_response", "data")?;
            let opts = args.get(2);

            // A response carries one image, so the plural forms cannot mean
            // anything here. Say so rather than silently picking one.
            if let Some(Value::Hash(h)) = opts {
                let h = h.borrow();
                if h.get(&HashKey::String("pages".into())).is_some() {
                    return Err("pdf_preview_response(): use `page` (a single page) rather than `pages` — the response carries one image".to_string());
                }
                if h.get(&HashKey::String("out_dir".into())).is_some() {
                    return Err("pdf_preview_response(): `out_dir` is meaningless here — the response carries the image; use pdf_preview() to write files".to_string());
                }
            }

            let page_no = match opts {
                Some(Value::Hash(h)) => match h.borrow().get(&HashKey::String("page".into())) {
                    Some(Value::Int(n)) if *n >= 1 => *n as usize,
                    Some(v) => {
                        return Err(format!(
                            "pdf_preview_response(): `page` must be a positive integer, got {}",
                            v.type_name()
                        ))
                    }
                    None => 1,
                },
                _ => 1,
            };

            warn_ignored_preview_options(opts, "pdf_preview_response");
            let render = base_render_options(opts);
            let raster = raster_options(opts, "pdf_preview_response")?;
            let (format, quality) = preview_format(opts, "pdf_preview_response")?;
            let session =
                soli_pdf::RasterSession::open(template.as_bytes(), data.as_bytes(), &render)
                    .map_err(|e| format!("pdf_preview_response() failed: {e}"))?;
            if page_no > session.page_count() {
                return Err(format!(
                    "pdf_preview_response(): page {page_no} is past the end of the document ({} page(s))",
                    session.page_count()
                ));
            }
            let page = session
                .page_pixels(page_no - 1, &raster)
                .map_err(|e| format!("pdf_preview_response() failed: {e}"))?;
            let body = encode_preview(&page, format, quality, "pdf_preview_response")?;

            let mut headers = HashPairs::default();
            headers.insert(
                HashKey::String("Content-Type".into()),
                Value::String(format.mime().into()),
            );
            if let Some(filename) = opt_str(opts, "filename") {
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
            response.insert(HashKey::String("body_base64".into()), b64(body));
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
/// Resolve a font search directory, confined to the app root.
///
/// This used to join a relative path onto the jail root and hand an *absolute*
/// one straight back, with no containment check — so a `fonts` option (and the
/// letterhead/attachment paths that share this helper) read anywhere the
/// process could. Falling back to the app root on a rejected path keeps
/// rendering working rather than failing the document over a font search path.
fn resolve_font_dir(dir: PathBuf) -> PathBuf {
    let raw = dir.to_string_lossy().to_string();
    match crate::interpreter::builtins::file::resolve_readable_path(&raw, "PDF font directory") {
        Ok(resolved) => resolved,
        Err(message) => {
            eprintln!("[WARN] ignoring font directory {raw:?}: {message}");
            crate::interpreter::builtins::file::jail_root().unwrap_or(dir)
        }
    }
}

// ---------------------------------------------------------------------------
// PNG page previews
// ---------------------------------------------------------------------------

/// SEC: a preview's `dpi`/`width` typically comes straight from a request, so
/// every knob that drives an allocation is capped. An A4 at 600 dpi is already
/// a 140 MB pixmap; at 20 000 dpi it is hundreds of gigabytes.
///
/// * `SOLI_PDF_PREVIEW_MAX_DPI` — default 600.
/// * `SOLI_PDF_PREVIEW_MAX_DIMENSION_PX` — per axis, default 8192.
/// * `SOLI_PDF_PREVIEW_MAX_PAGES` — default 64.
/// * `SOLI_PDF_PREVIEW_MAX_PIXELS` — per page, default 40M (≈160 MB RGBA).
fn env_cap<T: std::str::FromStr + Copy>(var: &'static str, default: T) -> T {
    std::env::var(var)
        .ok()
        .and_then(|s| s.parse::<T>().ok())
        .unwrap_or(default)
}

fn preview_max_dpi() -> f32 {
    static CAP: OnceLock<f32> = OnceLock::new();
    *CAP.get_or_init(|| env_cap("SOLI_PDF_PREVIEW_MAX_DPI", 600.0f32))
}

fn preview_max_dimension_px() -> u32 {
    static CAP: OnceLock<u32> = OnceLock::new();
    *CAP.get_or_init(|| env_cap("SOLI_PDF_PREVIEW_MAX_DIMENSION_PX", 8192u32))
}

fn preview_max_pages() -> usize {
    static CAP: OnceLock<usize> = OnceLock::new();
    *CAP.get_or_init(|| env_cap("SOLI_PDF_PREVIEW_MAX_PAGES", 64usize))
}

fn preview_max_pixels() -> u64 {
    static CAP: OnceLock<u64> = OnceLock::new();
    *CAP.get_or_init(|| env_cap("SOLI_PDF_PREVIEW_MAX_PIXELS", 40_000_000u64))
}

/// Read a positive pixel dimension, refusing anything over the cap.
fn preview_dimension(opts: Option<&Value>, key: &str, func: &str) -> Result<Option<u32>, String> {
    let Some(Value::Hash(h)) = opts else {
        return Ok(None);
    };
    let raw = match h.borrow().get(&HashKey::String(key.into())) {
        Some(v) => match opt_num(v) {
            Some(n) => n,
            None => return Err(format!("{func}(): `{key}` must be a number")),
        },
        None => return Ok(None),
    };
    if !raw.is_finite() || raw < 1.0 {
        return Err(format!(
            "{func}(): `{key}` must be at least 1 pixel, got {raw}"
        ));
    }
    let cap = preview_max_dimension_px();
    if raw > cap as f32 {
        return Err(format!(
            "{func}(): `{key}` of {raw}px exceeds the {cap}px cap \
             (raise SOLI_PDF_PREVIEW_MAX_DIMENSION_PX)"
        ));
    }
    Ok(Some(raw as u32))
}

/// Build the raster options from the Soli options hash.
fn raster_options(opts: Option<&Value>, func: &str) -> Result<soli_pdf::RasterOptions, String> {
    let mut raster = soli_pdf::RasterOptions {
        max_pixels: preview_max_pixels(),
        ..Default::default()
    };

    if let Some(Value::Hash(h)) = opts {
        if let Some(v) = h.borrow().get(&HashKey::String("dpi".into())) {
            let dpi = opt_num(v).ok_or_else(|| format!("{func}(): `dpi` must be a number"))?;
            if !dpi.is_finite() || dpi <= 0.0 {
                return Err(format!("{func}(): `dpi` must be positive, got {dpi}"));
            }
            let cap = preview_max_dpi();
            if dpi > cap {
                return Err(format!(
                    "{func}(): `dpi` of {dpi} exceeds the {cap} cap \
                     (raise SOLI_PDF_PREVIEW_MAX_DPI)"
                ));
            }
            raster.dpi = dpi;
        }
    }
    raster.width = preview_dimension(opts, "width", func)?;
    raster.height = preview_dimension(opts, "height", func)?;

    // `pages`: the same 1-based selection `pdf_pages` takes, converted to the
    // 0-based indices the backend uses.
    if let Some(Value::Hash(h)) = opts {
        let sel = h.borrow().get(&HashKey::String("pages".into())).cloned();
        if let Some(v) = sel {
            let pages = parse_page_selection(&v).map_err(|e| format!("{func}(): {e}"))?;
            let cap = preview_max_pages();
            if pages.len() > cap {
                return Err(format!(
                    "{func}(): `pages` selects {} pages, over the {cap} cap \
                     (raise SOLI_PDF_PREVIEW_MAX_PAGES)",
                    pages.len()
                ));
            }
            raster.pages =
                soli_pdf::PageSelection::Indices(pages.iter().map(|p| *p as usize - 1).collect());
        }
    }
    Ok(raster)
}

/// The image format a preview is encoded in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PreviewFormat {
    Png,
    Webp,
    Jpeg,
}

impl PreviewFormat {
    fn extension(self) -> &'static str {
        match self {
            PreviewFormat::Png => "png",
            PreviewFormat::Webp => "webp",
            PreviewFormat::Jpeg => "jpg",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            PreviewFormat::Png => "image/png",
            PreviewFormat::Webp => "image/webp",
            PreviewFormat::Jpeg => "image/jpeg",
        }
    }

    fn image_format(self) -> image::ImageFormat {
        match self {
            PreviewFormat::Png => image::ImageFormat::Png,
            PreviewFormat::Webp => image::ImageFormat::WebP,
            PreviewFormat::Jpeg => image::ImageFormat::Jpeg,
        }
    }
}

/// `format` and `quality`. PNG is the default because it is lossless and
/// universal; `"webp"` is the one to reach for when size matters — a text page
/// is 3-5x smaller as lossy WebP, and libwebp (not the `image` crate's
/// lossless-only encoder) is what makes that true.
fn preview_format(opts: Option<&Value>, func: &str) -> Result<(PreviewFormat, u8), String> {
    let format = match opt_str(opts, "format") {
        None => PreviewFormat::Png,
        Some(f) => match f.trim().to_ascii_lowercase().as_str() {
            "png" => PreviewFormat::Png,
            "webp" => PreviewFormat::Webp,
            "jpeg" | "jpg" => PreviewFormat::Jpeg,
            other => {
                return Err(format!(
                    "{func}(): unknown `format` {other:?} — use \"png\", \"webp\" or \"jpeg\""
                ))
            }
        },
    };
    let mut quality: u8 = 90;
    if let Some(Value::Hash(h)) = opts {
        if let Some(v) = h.borrow().get(&HashKey::String("quality".into())) {
            let q = opt_num(v).ok_or_else(|| format!("{func}(): `quality` must be a number"))?;
            if !(1.0..=100.0).contains(&q) {
                return Err(format!("{func}(): `quality` must be 1-100, got {q}"));
            }
            quality = q as u8;
        }
    }
    Ok((format, quality))
}

/// Encode one rasterised page. PNG goes through the `image` crate; WebP goes
/// through libwebp via the same encoder `Image.format("webp")` uses, so a
/// preview and a thumbnail of it compress identically.
fn encode_preview(
    page: &soli_pdf::RasterPixels,
    format: PreviewFormat,
    quality: u8,
    func: &str,
) -> Result<Vec<u8>, String> {
    let buffer = image::RgbaImage::from_raw(page.width, page.height, page.rgba.clone())
        .ok_or_else(|| format!("{func}(): rasterised page has an inconsistent pixel buffer"))?;
    let mut dynamic = image::DynamicImage::ImageRgba8(buffer);
    if format == PreviewFormat::Jpeg {
        // JPEG has no alpha channel; flatten rather than emit a black page.
        dynamic = image::DynamicImage::ImageRgb8(dynamic.to_rgb8());
    }
    crate::interpreter::builtins::image::encode_dynamic_image(
        &dynamic,
        quality,
        format.image_format(),
    )
    .map_err(|e| format!("{func}(): {e}"))
}

/// Options that mean something for a PDF but nothing for a page image. Warn
/// rather than reject: sharing one hash between `pdf_render` and
/// `pdf_preview` is the natural way to write this, and refusing it would buy
/// no safety.
///
/// Warned **once per option per process**. A preview endpoint calls this on
/// every request, so warning each time would put three lines in the log for
/// every thumbnail served — and, in `soli test`, repaint over the progress
/// dashboard. The condition is a property of the caller's options hash, not
/// of the request, so saying it once is saying it. Same shape as the
/// warn-once in `serve::otel`.
fn warn_ignored_preview_options(opts: Option<&Value>, func: &str) {
    let Some(Value::Hash(h)) = opts else {
        return;
    };
    let h = h.borrow();
    let has = |k: &str| h.get(&HashKey::String(k.into())).is_some();

    let warn = |option: &'static str, why: &str| {
        static SEEN: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
        let seen = SEEN.get_or_init(|| Mutex::new(HashSet::new()));
        if let Ok(mut set) = seen.lock() {
            if set.insert(option) {
                eprintln!("[WARN] {func}(): ignoring `{option}` — {why}");
            }
        }
    };

    // Only the two that change what the reader would see. `attachments`,
    // `password` and `pdfa` are equally inert here, but they cannot alter a
    // single pixel, so announcing them is noise — the options table documents
    // them. A warning earns its line by describing a *difference*.
    if has("stationery") {
        warn(
            "stationery",
            "a letterhead is composited onto emitted PDF bytes, so the preview \
             shows the content without it",
        );
    }
    if has("sign") {
        warn(
            "sign",
            "a signature is applied to emitted PDF bytes, so a visible \
             signature appearance will be missing from the preview",
        );
    }
}

/// A `prefix` must be a plain file-name component. Even inside the jail, a
/// `prefix` of `../avatars/me` would overwrite someone else's file.
fn preview_prefix(opts: Option<&Value>, func: &str) -> Result<String, String> {
    let prefix = opt_str(opts, "prefix").unwrap_or_else(|| "page".to_string());
    if prefix.is_empty() || prefix.len() > 100 {
        return Err(format!(
            "{func}(): `prefix` must be 1-100 characters, got {}",
            prefix.len()
        ));
    }
    if prefix.contains(['/', '\\', '\0'])
        || prefix == "."
        || prefix == ".."
        || prefix.starts_with('.')
    {
        return Err(format!(
            "{func}(): `prefix` must be a plain file name without path separators, got {prefix:?}"
        ));
    }
    Ok(prefix)
}

/// Turn rasterised pages into the builtin's return value: base64 PNGs, or —
/// when `out_dir` is set — the paths they were written to.
///
/// Writes go through `file::write_bytes_jailed`, so SEC-006 containment and
/// SEC-050's `O_NOFOLLOW` apply at open time rather than being reimplemented
/// here. Paths come back as given, not canonicalised: the useful thing is an
/// `<img src>`, and the absolute path would leak the server's layout.
fn preview_result(
    pages: Vec<EncodedPage>,
    format: PreviewFormat,
    opts: Option<&Value>,
    func: &str,
) -> Result<Value, String> {
    let Some(out_dir) = opt_str(opts, "out_dir") else {
        let encoded: Vec<Value> = pages.into_iter().map(|p| b64(p.bytes)).collect();
        return Ok(Value::Array(Rc::new(RefCell::new(encoded))));
    };

    let prefix = preview_prefix(opts, func)?;
    let dir = out_dir.trim_end_matches('/');
    // A single page keeps the bare name, which is what a gallery wants;
    // several are suffixed with their 1-based page number.
    let single = pages.len() == 1;
    let width = pages
        .iter()
        .map(|p| (p.index + 1).to_string().len())
        .max()
        .unwrap_or(1);

    let mut written = Vec::with_capacity(pages.len());
    for page in pages {
        let ext = format.extension();
        let name = if single {
            format!("{prefix}.{ext}")
        } else {
            format!("{prefix}-{:0width$}.{ext}", page.index + 1, width = width)
        };
        let path = format!("{dir}/{name}");
        crate::interpreter::builtins::file::write_bytes_jailed(&path, func, &page.bytes)?;
        written.push(Value::String(path.into()));
    }
    Ok(Value::Array(Rc::new(RefCell::new(written))))
}

/// One encoded page: the bytes plus the 0-based index they came from.
struct EncodedPage {
    index: usize,
    bytes: Vec<u8>,
}

/// Shared body of `pdf_preview` / `pdf_preview_from_markdown`: lay out once,
/// then paint and encode one page at a time so a long document never holds
/// every pixmap at once.
fn preview_pages(
    template_json: &[u8],
    data_json: &[u8],
    args_opts: Option<&Value>,
    func: &str,
) -> Result<(Vec<EncodedPage>, PreviewFormat), String> {
    warn_ignored_preview_options(args_opts, func);
    let opts = base_render_options(args_opts);
    let raster = raster_options(args_opts, func)?;
    let (format, quality) = preview_format(args_opts, func)?;
    let session = soli_pdf::RasterSession::open(template_json, data_json, &opts)
        .map_err(|e| format!("{func}() failed: {e}"))?;
    let selected = session
        .selected(&raster)
        .map_err(|e| format!("{func}() failed: {e}"))?;
    // The cap is on pages actually painted, not on the document's length: a
    // thumbnail of page 1 of a 200-page report is exactly what this is for.
    if selected.len() > preview_max_pages() {
        return Err(format!(
            "{func}(): would rasterise {} pages, over the {} cap \
             (select fewer with `pages`, or raise SOLI_PDF_PREVIEW_MAX_PAGES)",
            selected.len(),
            preview_max_pages()
        ));
    }
    let mut out = Vec::with_capacity(selected.len());
    for index in selected {
        let page = session
            .page_pixels(index, &raster)
            .map_err(|e| format!("{func}() failed: {e}"))?;
        out.push(EncodedPage {
            index,
            bytes: encode_preview(&page, format, quality, func)?,
        });
    }
    Ok((out, format))
}

/// The template JSON a Markdown preview renders.
fn markdown_template_json(md: &str, opts: Option<&Value>, func: &str) -> Result<Vec<u8>, String> {
    let theme = theme_from_options(opts);
    let template = pdf_markdown::markdown_to_template(md, &theme);
    serde_json::to_vec(&template).map_err(|e| format!("{func}(): building template: {e}"))
}

/// The half of the options that needs no IO: fonts, image fetching and the
/// Info-dictionary metadata.
///
/// Split out because the raster preview must not run the other half —
/// `stationery` and `attachments` read files and hard-error when one is
/// missing, and a preview that refuses to appear over a letterhead it cannot
/// draw anyway would be useless. It also lets one options hash be shared
/// between `pdf_render` and `pdf_preview`.
fn base_render_options(opts: Option<&Value>) -> RenderOptions {
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
    RenderOptions {
        font_dirs: dirs.into_iter().map(resolve_font_dir).collect(),
        fetch_images,
        title: opt_str(opts, "title"),
        author: opt_str(opts, "author"),
        subject: opt_str(opts, "subject"),
        pdfa,
        ..Default::default()
    }
}

/// Build `RenderOptions` from an optional options hash. Defaults the font search
/// path to a `font/` folder at the app root.
fn render_options(opts: Option<&Value>) -> Result<RenderOptions, String> {
    let base = base_render_options(opts);

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
        stationery,
        attachments,
        encrypt,
        ..base
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
    Ok(Some(sign_config_from_hash(&sb)?))
}

/// Parse a `{ cert, key, chain?, reason?, …, tsa?, appearance? }` hash into a
/// [`SignConfig`]. Shared by the `sign` render option and the `pdf_sign` builtin.
fn sign_config_from_hash(sb: &HashPairs) -> Result<SignConfig, String> {
    let cert_in =
        sign_str(sb, "cert").ok_or("sign: `cert` (PEM string or path) is required".to_string())?;
    let key_in =
        sign_str(sb, "key").ok_or("sign: `key` (PEM string or path) is required".to_string())?;
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
        reason: sign_str(sb, "reason"),
        location: sign_str(sb, "location"),
        name: sign_str(sb, "name"),
        contact: sign_str(sb, "contact"),
        signing_time: Some(pdf_date(signing_time)),
        appearance: parse_sign_appearance(sb),
    };
    Ok(SignConfig {
        material: pades::SignerMaterial {
            cert_der,
            chain_der,
            key,
        },
        meta,
        signing_time,
        tsa: sign_str(sb, "tsa"),
    })
}

/// Parse the optional `sign.appearance` hash `{ page?, x?, y?, width?, height? }`
/// into a visible signature block (defaults to a box in the bottom-left of
/// page 1). Returns `None` when the key is absent — an invisible signature.
fn parse_sign_appearance(sb: &HashPairs) -> Option<soli_pdf::SignAppearance> {
    let Some(Value::Hash(h)) = sb.get(&HashKey::String("appearance".into())) else {
        return None;
    };
    let a = h.borrow();
    let num = |k: &str, d: f32| {
        a.get(&HashKey::String(k.into()))
            .and_then(opt_num)
            .unwrap_or(d)
    };
    let page = a
        .get(&HashKey::String("page".into()))
        .and_then(|v| match v {
            Value::Int(n) if *n >= 1 => Some(*n as u32),
            _ => None,
        })
        .unwrap_or(1);
    Some(soli_pdf::SignAppearance {
        page,
        x: num("x", 40.0),
        y: num("y", 40.0),
        width: num("width", 190.0),
        height: num("height", 64.0),
    })
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
