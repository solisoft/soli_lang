//! The `--dev` component and mailer preview catalogs (`/__soli/components`
//! and `/__soli/mailers`): list every `.html.slv` view, read its declared
//! props and leading `<%# preview %>` header, and render a gallery of iframe
//! cards, plus the single-mailer preview (`/__soli/mailers/<rel>`) the
//! gallery's iframes point at. Extracted from the serve god-module. Dev-only.

use hyper::Response;

use super::operator_shell::{self, Section};
use super::{dev_bar, html_ok, vfs_read_to_string, vfs_walk_dir, ResponseBody};

/// A preview page is rendered two ways: standalone (it's directly linkable) and
/// inside the gallery's iframe cards. The catalogs append `?framed=1` to their
/// iframe srcs so only the standalone view carries navigation chrome — a back
/// link repeated inside every card would be noise, not navigation.
fn preview_is_framed(query: Option<&str>) -> bool {
    query.is_some_and(|q| q.split('&').any(|pair| pair == "framed=1"))
}

/// The back link for a preview page, empty when the page is framed. Previews
/// render on white, so the link is styled for a light page.
/// Shared with the mailer preview, which lives in the serve module.
pub(crate) fn preview_back_link(query: Option<&str>) -> &'static str {
    if preview_is_framed(query) {
        return "";
    }
    "<p style=\"margin:0 0 1rem;font:12px ui-monospace,monospace;\">\
<a href=\"/\" style=\"color:#0969da;text-decoration:none;\">&larr; back to the app</a></p>"
}

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

const PREVIEW_NOTE: &str = "Dev-only. Previews render with the built-in helpers and any \
<code>&lt;%# preview: {...} %&gt;</code> data at the top of the file; app-defined view helpers \
and request context aren\u{2019}t available here.";

fn catalog_shell(section: Section, heading: &str, body: &str) -> String {
    operator_shell::page(section, heading, PREVIEW_NOTE, body)
}

/// Dev-only component catalog index (`GET /__soli/components`).
pub(crate) fn handle_component_catalog() -> Response<ResponseBody> {
    let cache = match crate::interpreter::builtins::template::get_template_cache() {
        Ok(c) => c,
        Err(e) => {
            return html_ok(catalog_shell(
                Section::Components,
                "Components",
                &format!(
                    "<p class=\"notice bad\">Template cache unavailable: {}</p>",
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
            Section::Components,
            "Components",
            "<div class=\"empty\"><b>No components yet.</b><span>Add one under \
<code>app/views/components/</code>, or run <code>soli generate component card</code>.</span></div>",
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
                "<span class=\"muted mono\">props: {}</span>",
                dev_bar::html_escape(&declared.join(", "))
            )
        };
        cards.push_str(&format!(
            "<div class=\"card\"><div class=\"card-head\">\
<a href=\"/__soli/components/{esc}\">{esc}</a>{declared_html}</div>\
<iframe src=\"/__soli/components/{esc}?framed=1\" style=\"height:190px;\" title=\"{esc}\" loading=\"lazy\"></iframe>\
</div>",
        ));
    }
    html_ok(catalog_shell(
        Section::Components,
        "Components",
        &format!(
            "<p class=\"summary\">{} component(s)</p><div class=\"gallery\">{}</div>",
            names.len(),
            cards
        ),
    ))
}

/// Dev-only single-component preview (`GET /__soli/components/<name>`), used by
/// the catalog iframes and directly linkable.
pub(crate) fn handle_component_preview(name: &str, query: Option<&str>) -> Response<ResponseBody> {
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
</head><body>{back}{inner}</body></html>",
        back = preview_back_link(query),
        inner = inner
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
                Section::Mailers,
                "Mailers",
                &format!(
                    "<p class=\"notice bad\">Template cache unavailable: {}</p>",
                    dev_bar::html_escape(&e)
                ),
            ))
        }
    };
    let views_dir = cache.views_dir().to_path_buf();
    let dir_str = views_dir.to_string_lossy().to_string();
    let prefix = format!("{}/", dir_str.trim_end_matches('/'));
    let names = mailer_view_names(&vfs_walk_dir(&dir_str).unwrap_or_default(), &prefix);

    // Templates with fake data live here; what the app actually sent lives in
    // the dev inbox, so cross-link the two.
    let inbox_link = "<p class=\"summary\">Templates with example data. \
Mail the app really sent is in the <a href=\"/__soli/inbox\">Inbox</a>.</p>";

    if names.is_empty() {
        return html_ok(catalog_shell(
            Section::Mailers,
            "Mailers",
            &format!(
                "{inbox_link}<div class=\"empty\"><b>No mailer views yet.</b><span>Generate one with \
<code>soli generate mailer user welcome</code>.</span></div>"
            ),
        ));
    }

    let mut cards = String::new();
    for rel in &names {
        let esc = dev_bar::html_escape(rel);
        cards.push_str(&format!(
            "<div class=\"card\"><div class=\"card-head\">\
<a href=\"/__soli/mailers/{esc}\">{esc}</a></div>\
<iframe src=\"/__soli/mailers/{esc}?framed=1\" style=\"height:320px;\" title=\"{esc}\" loading=\"lazy\"></iframe>\
</div>",
        ));
    }
    html_ok(catalog_shell(
        Section::Mailers,
        "Mailers",
        &format!("{inbox_link}<div class=\"gallery wide\">{}</div>", cards),
    ))
}

/// Dev-only single mailer preview (`GET /__soli/mailers/<mailer>/<action>`),
/// used by the gallery iframes and directly linkable. Renders the HTML body
/// only (no layout), with example data from a `<%# preview: {json} %>` header.
pub(crate) fn handle_mailer_preview(rel: &str, query: Option<&str>) -> Response<ResponseBody> {
    if !crate::template::is_safe_template_name(rel) {
        return html_ok("<!doctype html><p>invalid mailer template</p>".to_string());
    }
    let inner = match crate::interpreter::builtins::template::get_template_cache() {
        Ok(cache) => {
            let raw = view_raw_source(cache.views_dir(), rel).unwrap_or_default();
            let data = component_preview_data(&raw);
            match cache.render(rel, &data, Some(None)) {
                Ok(html) => html,
                Err(e) => format!(
                    "<pre style=\"color:#b00\">render error: {}</pre>",
                    dev_bar::html_escape(&e)
                ),
            }
        }
        Err(e) => format!("template cache unavailable: {}", dev_bar::html_escape(&e)),
    };
    // Mailer bodies bring their own markup; render them on a plain white page.
    // The back link is suppressed when the gallery frames this page (?framed=1).
    html_ok(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<style>body{{margin:0;padding:1rem;font-family:system-ui,sans-serif;background:#fff;color:#111;}}</style>\
</head><body>{back}{inner}</body></html>",
        back = preview_back_link(query),
        inner = inner
    ))
}
