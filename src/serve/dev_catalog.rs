//! The `--dev` component and mailer preview catalogs (`/__soli/components`
//! and `/__soli/mailers`): list every `.html.slv` view, read its declared
//! props and leading `<%# preview %>` header, and render a gallery of iframe
//! cards. Extracted from the serve god-module. Dev-only. `view_raw_source`
//! and `component_preview_data` are re-exported to `super` for the
//! single-mailer preview endpoint.

use hyper::Response;

use super::{dev_bar, html_ok, vfs_read_to_string, vfs_walk_dir, ResponseBody};

/// Read a view template's raw `.html.slv` source by views-relative path (VFS-aware).
pub(crate) fn view_raw_source(views_dir: &std::path::Path, rel: &str) -> Option<String> {
    let path = views_dir.join(format!("{}.html.slv", rel));
    vfs_read_to_string(&path.to_string_lossy()).ok()
}

/// Read a component's raw `.html.slv` source.
fn component_raw_source(views_dir: &std::path::Path, name: &str) -> Option<String> {
    view_raw_source(views_dir, &format!("components/{}", name))
}

/// Extract example preview data from a leading `<%# preview: {json} %>` header;
/// an empty hash when absent or malformed.
pub(crate) fn component_preview_data(raw: &str) -> crate::interpreter::value::Value {
    use crate::interpreter::value::{HashPairs, Value};
    let empty = || {
        Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(
            HashPairs::default(),
        )))
    };
    let Some(start) = raw.find("<%#") else {
        return empty();
    };
    let after = &raw[start + 3..];
    let Some(end) = after.find("%>") else {
        return empty();
    };
    let Some(json_str) = after[..end].trim().strip_prefix("preview:") else {
        return empty();
    };
    let Ok(j) = serde_json::from_str::<serde_json::Value>(json_str.trim()) else {
        return empty();
    };
    match crate::interpreter::value_json::json_to_value_ref(&j) {
        Ok(v) => {
            crate::interpreter::builtins::template::inject_template_helpers(&v);
            v
        }
        Err(_) => empty(),
    }
}

/// Names declared via `props("a", "b")` in a component's source (display only).
fn component_declared_props(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = raw;
    while let Some(pos) = rest.find("props(") {
        let after = &rest[pos + 6..];
        let end = after.find(')').unwrap_or(after.len());
        let args = &after[..end];
        let mut in_str = false;
        let mut cur = String::new();
        for c in args.chars() {
            if c == '"' {
                if in_str {
                    if !cur.is_empty() && !out.contains(&cur) {
                        out.push(std::mem::take(&mut cur));
                    }
                    cur.clear();
                    in_str = false;
                } else {
                    in_str = true;
                }
            } else if in_str {
                cur.push(c);
            }
        }
        rest = &after[end..];
    }
    out
}

fn catalog_shell(heading: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Soli \u{b7} {heading}</title>\
<style>body{{margin:0;font-family:'JetBrains Mono',ui-monospace,monospace;background:#08090b;color:#c9d1d9;padding:1.5rem;}}\
h1{{font-size:14px;letter-spacing:0.08em;color:#8b949e;font-weight:600;margin:0 0 0.25rem;}}\
a:hover{{text-decoration:underline;}}</style></head>\
<body><h1>SOLI \u{b7} {heading}</h1>\
<p style=\"font-size:11px;color:#8b949e;margin:0 0 1.25rem;\">Dev-only. Previews render with built-in helpers plus any \
<code>&lt;%# preview: {{...}} %&gt;</code> data; app-defined view helpers and request context aren't available here.</p>\
{body}</body></html>",
        heading = heading,
        body = body,
    )
}

/// Dev-only component catalog index (`GET /__soli/components`).
pub(crate) fn handle_component_catalog() -> Response<ResponseBody> {
    let cache = match crate::interpreter::builtins::template::get_template_cache() {
        Ok(c) => c,
        Err(e) => {
            return html_ok(catalog_shell(
                "COMPONENT CATALOG",
                &format!(
                    "<p style=\"color:#ff6b6b\">Template cache unavailable: {}</p>",
                    dev_bar::html_escape(&e)
                ),
            ))
        }
    };
    let views_dir = cache.views_dir().to_path_buf();
    let comp_dir = views_dir.join("components");
    let dir_str = comp_dir.to_string_lossy().to_string();
    let prefix = format!("{}/", dir_str.trim_end_matches('/'));
    let mut names: Vec<String> = vfs_walk_dir(&dir_str)
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f.ends_with(".html.slv"))
        .map(|f| {
            f.strip_prefix(&prefix)
                .unwrap_or(&f)
                .trim_end_matches(".html.slv")
                .to_string()
        })
        .filter(|n| crate::template::is_safe_template_name(n))
        .collect();
    names.sort();
    names.dedup();

    if names.is_empty() {
        return html_ok(catalog_shell(
            "COMPONENT CATALOG",
            "<p style=\"color:#8b949e\">No components found in <code>app/views/components/</code>.</p>",
        ));
    }

    let mut cards = String::new();
    for name in &names {
        let esc = dev_bar::html_escape(name);
        let raw = component_raw_source(&views_dir, name).unwrap_or_default();
        let declared = component_declared_props(&raw);
        let declared_html = if declared.is_empty() {
            String::new()
        } else {
            format!(
                "<div style=\"font-size:11px;color:#8b949e;margin-top:0.2rem;\">props: {}</div>",
                dev_bar::html_escape(&declared.join(", "))
            )
        };
        cards.push_str(&format!(
            "<div style=\"border:1px solid #30363d;border-radius:6px;overflow:hidden;\">\
<div style=\"padding:0.5rem 0.75rem;border-bottom:1px solid #30363d;background:#0b0d0f;\">\
<a href=\"/__soli/components/{esc}\" style=\"color:#8be9fd;text-decoration:none;font-weight:600;\">{esc}</a>{declared_html}\
</div>\
<iframe src=\"/__soli/components/{esc}\" style=\"width:100%;height:190px;border:0;background:#fff;\" title=\"{esc}\"></iframe>\
</div>",
        ));
    }
    html_ok(catalog_shell(
        "COMPONENT CATALOG",
        &format!(
            "<div style=\"display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));gap:1rem;\">{}</div>",
            cards
        ),
    ))
}

/// Dev-only single-component preview (`GET /__soli/components/<name>`), used by
/// the catalog iframes and directly linkable.
pub(crate) fn handle_component_preview(name: &str) -> Response<ResponseBody> {
    if !crate::template::is_safe_template_name(name) {
        return html_ok("<!doctype html><p>invalid component name</p>".to_string());
    }
    let inner = match crate::interpreter::builtins::template::get_template_cache() {
        Ok(cache) => {
            let raw = component_raw_source(cache.views_dir(), name).unwrap_or_default();
            let data = component_preview_data(&raw);
            match cache.render_component(name, &data) {
                Ok(html) => html,
                Err(e) => format!(
                    "<pre style=\"color:#b00\">render error: {}</pre>",
                    dev_bar::html_escape(&e)
                ),
            }
        }
        Err(e) => format!("template cache unavailable: {}", dev_bar::html_escape(&e)),
    };
    // Bare doc + the app stylesheet (best-effort) so previews approximate reality.
    html_ok(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<link rel=\"stylesheet\" href=\"/css/application.css\">\
<style>body{{margin:0;padding:1rem;font-family:system-ui,sans-serif;}}</style>\
</head><body>{}</body></html>",
        inner
    ))
}

/// From a flat list of view file paths (as returned by `vfs_walk_dir`), pick the
/// sorted, deduped `<mailer>/<action>` names: `.html.slv` files directly under a
/// `*_mailer/` directory. The `.text.slv` companions don't end in `.html.slv`,
/// so they're excluded; unsafe/traversal names are dropped.
pub(crate) fn mailer_view_names(files: &[String], prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = files
        .iter()
        .filter(|f| f.ends_with(".html.slv"))
        .filter_map(|f| {
            let rel = f
                .strip_prefix(prefix)
                .unwrap_or(f)
                .trim_end_matches(".html.slv")
                .to_string();
            let (dir, _action) = rel.split_once('/')?;
            if dir.ends_with("_mailer") {
                Some(rel)
            } else {
                None
            }
        })
        .filter(|n| crate::template::is_safe_template_name(n))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Dev-only mailer preview gallery index (`GET /__soli/mailers`). Lists every
/// `app/views/<name>_mailer/<action>.html.slv` view and previews each in an
/// iframe — the email equivalent of the component catalog.
pub(crate) fn handle_mailer_catalog() -> Response<ResponseBody> {
    let cache = match crate::interpreter::builtins::template::get_template_cache() {
        Ok(c) => c,
        Err(e) => {
            return html_ok(catalog_shell(
                "MAILER PREVIEWS",
                &format!(
                    "<p style=\"color:#ff6b6b\">Template cache unavailable: {}</p>",
                    dev_bar::html_escape(&e)
                ),
            ))
        }
    };
    let views_dir = cache.views_dir().to_path_buf();
    let dir_str = views_dir.to_string_lossy().to_string();
    let prefix = format!("{}/", dir_str.trim_end_matches('/'));
    let names = mailer_view_names(&vfs_walk_dir(&dir_str).unwrap_or_default(), &prefix);

    if names.is_empty() {
        return html_ok(catalog_shell(
            "MAILER PREVIEWS",
            "<p style=\"color:#8b949e\">No mailer views found. Generate one with \
<code>soli generate mailer user welcome</code>.</p>",
        ));
    }

    let mut cards = String::new();
    for rel in &names {
        let esc = dev_bar::html_escape(rel);
        cards.push_str(&format!(
            "<div style=\"border:1px solid #30363d;border-radius:6px;overflow:hidden;\">\
<div style=\"padding:0.5rem 0.75rem;border-bottom:1px solid #30363d;background:#0b0d0f;\">\
<a href=\"/__soli/mailers/{esc}\" style=\"color:#8be9fd;text-decoration:none;font-weight:600;\">{esc}</a>\
</div>\
<iframe src=\"/__soli/mailers/{esc}\" style=\"width:100%;height:320px;border:0;background:#fff;\" title=\"{esc}\"></iframe>\
</div>",
        ));
    }
    html_ok(catalog_shell(
        "MAILER PREVIEWS",
        &format!(
            "<div style=\"display:grid;grid-template-columns:repeat(auto-fill,minmax(360px,1fr));gap:1rem;\">{}</div>",
            cards
        ),
    ))
}
