//! The page shell every generated page shares.
//!
//! Two type families, and the boundary means something: the chrome — top bar,
//! sidebar, breadcrumb, index, metadata — is monospace, because this is a
//! filesystem tool; the prose a Markdown file renders into is serif, because
//! that part was written by a person. There is no third face and no icon set.
//!
//! Everything is inlined or served from this binary: no CDN, no web font, no
//! network request of any kind. `soli serve` has to work on a plane.

use std::path::Path;

use hyper::{Response, StatusCode};

use crate::template::renderer::html_escape;

use super::{html_page, tree, ResponseBody};

/// A sun, matching the mark on the docs site.
const FAVICON: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'%3E%3Ccircle cx='16' cy='16' r='12' fill='%23F59E0B'/%3E%3C/svg%3E";

/// Set `data-theme` before first paint so a dark-mode reader never eats a
/// flash of the light palette (or the other way round).
const THEME_BOOT: &str =
    "try{var t=localStorage.getItem('soli-files-theme');if(t)document.documentElement.dataset.theme=t}catch(e){}";

pub(crate) struct Page<'a> {
    /// Browser tab title.
    pub title: String,
    /// Root-relative path of what is being viewed, for the sidebar rail.
    pub current: &'a str,
    /// Single line of real metadata under the breadcrumb. May be empty.
    pub meta: String,
    /// Main content, already HTML.
    pub body: String,
    /// Wrap the body in the serif reading column.
    pub prose: bool,
    /// Right-hand outline rail, already HTML. Empty for pages that have no
    /// headings worth navigating (folder indexes, error states).
    pub outline: String,
}

/// Build a full HTML page.
pub(crate) fn render(root: &Path, page: Page<'_>, dev_mode: bool) -> String {
    let tree = tree::cached(root);
    let sidebar = tree::render_sidebar(&tree, page.current);
    let crumbs = breadcrumb(&tree.root.name, page.current);

    let meta = if page.meta.is_empty() {
        String::new()
    } else {
        format!("<p class=\"meta\">{}</p>", page.meta)
    };

    let main = if page.prose {
        format!("<article class=\"prose\">{}</article>", page.body)
    } else {
        page.body
    };

    let html = format!(
        "<!doctype html>\n\
<html lang=\"en\">\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>{title}</title>\n\
<link rel=\"icon\" href=\"{favicon}\">\n\
<link rel=\"stylesheet\" href=\"/__soli/files.css\">\n\
<script>{theme_boot}</script>\n\
</head>\n\
<body>\n\
<header class=\"bar\">\n\
<button class=\"ghost menu\" id=\"menu\" aria-label=\"Show files\" aria-expanded=\"false\">files</button>\n\
<a class=\"brand\" href=\"/\"><span class=\"sun\" aria-hidden=\"true\"></span>soli serve</a>\n\
<input id=\"q\" class=\"filter\" type=\"search\" placeholder=\"filter\u{a0}\u{a0}/\" aria-label=\"Filter files\" autocomplete=\"off\" spellcheck=\"false\">\n\
<button class=\"ghost\" id=\"theme\" aria-label=\"Switch theme\">day/night</button>\n\
</header>\n\
<div class=\"shell{shell_class}\">\n\
<aside class=\"side\" id=\"side\">{sidebar}</aside>\n\
<main class=\"main\">\n\
<div class=\"masthead\">{crumbs}{meta}</div>\n\
{main}\n\
</main>\n\
{outline}\n\
</div>\n\
<script src=\"/__soli/files.js\" defer></script>\n\
</body>\n\
</html>",
        title = html_escape(&page.title),
        favicon = FAVICON,
        theme_boot = THEME_BOOT,
        sidebar = sidebar,
        crumbs = crumbs,
        meta = meta,
        main = main,
        // The third column only exists when there is an outline to put in it,
        // so a page without one keeps the full reading width.
        shell_class = if page.outline.is_empty() {
            ""
        } else {
            " has-toc"
        },
        outline = page.outline,
    );

    // In `--dev`, the same SSE channel the MVC server uses refreshes the page
    // when a watched file changes — editing a README and watching the browser
    // catch up is the whole point of the mode.
    if dev_mode && crate::serve::live_reload::is_live_reload_enabled() {
        crate::serve::live_reload::inject_live_reload_script(&html)
    } else {
        html
    }
}

/// The masthead: the path itself, one link per segment.
///
/// No hero, no tagline. On a file server the most characteristic thing on the
/// page is where you are, so that is what the page opens with.
fn breadcrumb(root_name: &str, current: &str) -> String {
    let mut out = String::from("<h1 class=\"crumbs\">");
    out.push_str(&format!(
        "<a href=\"/\">{}</a><span class=\"sep\">/</span>",
        html_escape(root_name)
    ));

    let mut walked = String::new();
    let segments: Vec<&str> = current.split('/').filter(|s| !s.is_empty()).collect();
    let last = segments.len().saturating_sub(1);
    for (i, segment) in segments.iter().enumerate() {
        if walked.is_empty() {
            walked.push_str(segment);
        } else {
            walked.push('/');
            walked.push_str(segment);
        }
        if i == last {
            out.push_str(&format!(
                "<span class=\"here\">{}</span>",
                html_escape(segment)
            ));
        } else {
            out.push_str(&format!(
                "<a href=\"{}/\">{}</a><span class=\"sep\">/</span>",
                tree::url_path(&walked),
                html_escape(segment)
            ));
        }
    }
    out.push_str("</h1>");
    out
}

/// A miss, answered with a direction rather than an apology.
pub(crate) fn not_found(root: &Path, rel: &str, dev_mode: bool) -> Response<ResponseBody> {
    let tree = tree::cached(root);

    // Walk back up until something exists, and point there.
    let mut ancestor = rel;
    let nearest = loop {
        match ancestor.rsplit_once('/') {
            Some((parent, _)) => {
                if tree.root.find(parent).is_some_and(|n| n.is_dir) {
                    break parent;
                }
                ancestor = parent;
            }
            None => break "",
        }
    };

    let nearest_node = tree.root.find(nearest);
    let count = nearest_node.map(|n| n.children.len()).unwrap_or(0);
    let label = if nearest.is_empty() {
        format!("{}/", tree.root.name)
    } else {
        format!("{}/", nearest)
    };

    let body = format!(
        "<div class=\"state\">\
<p class=\"lead\">No file at <code>/{path}</code>.</p>\
<p class=\"dir\">Nearest folder is <a href=\"{href}\">{label}</a> — {count} {noun}.</p>\
</div>",
        path = html_escape(rel),
        href = format!("{}/", tree::url_path(nearest)).replace("//", "/"),
        label = html_escape(&label),
        count = count,
        noun = if count == 1 { "entry" } else { "entries" },
    );

    let html = render(
        root,
        Page {
            title: format!("Not found · /{}", rel),
            current: "",
            meta: String::new(),
            body,
            prose: false,
            outline: String::new(),
        },
        dev_mode,
    );
    html_page(html, StatusCode::NOT_FOUND)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breadcrumb_links_every_ancestor_but_the_last() {
        let html = breadcrumb("notes", "docs/api/intro.md");
        assert!(html.contains("<a href=\"/docs/\">docs</a>"));
        assert!(html.contains("<a href=\"/docs/api/\">api</a>"));
        // The page you are on is not a link back to itself.
        assert!(html.contains("<span class=\"here\">intro.md</span>"));
    }

    #[test]
    fn breadcrumb_at_the_root_is_just_the_folder() {
        let html = breadcrumb("notes", "");
        assert!(html.contains("<a href=\"/\">notes</a>"));
        assert!(!html.contains("class=\"here\""));
    }

    #[test]
    fn breadcrumb_escapes_names() {
        let html = breadcrumb("notes", "<script>.md");
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;.md"));
    }
}
