//! Viewer pages for files that are not documents.
//!
//! Clicking a `.jpg` in a listing used to hand the browser the raw bytes: you
//! left the site, lost the tree, and got a picture on a blank background with
//! no way back but the back button. A media file now opens *inside* the shell,
//! with its breadcrumb, its sidebar and its metadata, like every other page.
//!
//! The raw bytes still have to be reachable, or every `<img>` embedded in a
//! Markdown page would render an HTML document instead of a picture. The two
//! are told apart by what the browser asks for: a click is a navigation
//! (`Sec-Fetch-Dest: document`), an `<img>` is a subresource. `?raw` forces the
//! bytes for anything, and is what the viewer's own tags and its download link
//! point at — so the viewer can never recurse into itself.

use std::path::Path;

use hyper::{HeaderMap, Response, StatusCode};

use crate::coverage::reporter::html_highlight_soli;
use crate::template::renderer::html_escape;

use super::{html_page, human_age, human_size, shell, tree, ResponseBody};

/// Text files are read into the page; past this they stay a download. Half a
/// megabyte of source is already far more than anyone reads in a browser.
const MAX_INLINE_TEXT: u64 = 512 * 1024;

/// Extensions shown as text even though the MIME table calls them binary —
/// source files, mostly, which have no registered type but are plainly text.
const TEXT_EXTENSIONS: &[&str] = &[
    "sl",
    "slv",
    "erb",
    "rs",
    "toml",
    "yaml",
    "yml",
    "csv",
    "log",
    "sh",
    "bash",
    "zsh",
    "py",
    "rb",
    "go",
    "ts",
    "tsx",
    "jsx",
    "sql",
    "ini",
    "conf",
    "cfg",
    "lock",
    "gitignore",
    "dockerfile",
    "mk",
    "c",
    "h",
    "cpp",
    "hpp",
    "java",
    "kt",
    "swift",
    "php",
    "lua",
    "vim",
    "el",
];

/// Extensions highlighted with Soli's own lexer.
const SOLI_EXTENSIONS: &[&str] = &["sl", "slv"];

/// What kind of viewer a file gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Image,
    Video,
    Audio,
    Pdf,
    Text,
    /// Anything we cannot show: offered as a download rather than dumped.
    Binary,
}

/// Should this request get a viewer page rather than the file itself?
///
/// True only for a top-level navigation. `Sec-Fetch-Dest` says so directly in
/// every current browser; the `Accept` header is the fallback for older ones
/// and, deliberately, for nothing else — `curl` and `wget` send neither and
/// get the bytes, which is what a tool asking for a URL wants.
pub(crate) fn wants_viewer(headers: &HeaderMap, query: Option<&str>) -> bool {
    if is_raw_request(query) {
        return false;
    }

    if let Some(dest) = headers.get("sec-fetch-dest").and_then(|v| v.to_str().ok()) {
        return dest == "document";
    }

    headers
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|accept| accept.contains("text/html"))
}

/// Does the query string carry the `raw` flag?
pub(crate) fn is_raw_request(query: Option<&str>) -> bool {
    query.is_some_and(|q| {
        q.split('&')
            .any(|part| part == "raw" || part.starts_with("raw="))
    })
}

/// Classify a file by MIME type, falling back to its extension for the source
/// formats that have no registered type.
pub(crate) fn kind_of(file: &Path, mime: &str) -> Kind {
    if mime.starts_with("image/") {
        return Kind::Image;
    }
    if mime.starts_with("video/") {
        return Kind::Video;
    }
    if mime.starts_with("audio/") {
        return Kind::Audio;
    }
    if mime.starts_with("application/pdf") {
        return Kind::Pdf;
    }
    if mime.starts_with("text/")
        || mime.starts_with("application/json")
        || mime.starts_with("application/javascript")
        || mime.starts_with("application/xml")
    {
        return Kind::Text;
    }

    let extension = file
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if TEXT_EXTENSIONS.contains(&extension.as_str()) {
        return Kind::Text;
    }

    Kind::Binary
}

/// An HTML page showing `file` inside the shell.
pub(crate) fn page(
    root: &Path,
    file: &Path,
    rel: &str,
    mime: &str,
    dev_mode: bool,
) -> Response<ResponseBody> {
    let kind = kind_of(file, mime);
    let raw_url = format!("{}?raw", tree::url_path(rel));
    let size = std::fs::metadata(file).map(|m| m.len()).unwrap_or(0);

    let body = match kind {
        Kind::Image => format!(
            "<div class=\"media\"><div class=\"frame\">\
<img src=\"{url}\" alt=\"{name}\" loading=\"lazy\"></div>{actions}</div>",
            url = html_escape(&raw_url),
            name = html_escape(&file_name(file, rel)),
            actions = actions(&raw_url),
        ),
        Kind::Video => format!(
            "<div class=\"media\"><div class=\"frame\">\
<video src=\"{url}\" controls preload=\"metadata\"></video></div>{actions}</div>",
            url = html_escape(&raw_url),
            actions = actions(&raw_url),
        ),
        Kind::Audio => format!(
            "<div class=\"media\"><div class=\"frame\">\
<audio src=\"{url}\" controls preload=\"metadata\"></audio></div>{actions}</div>",
            url = html_escape(&raw_url),
            actions = actions(&raw_url),
        ),
        Kind::Pdf => format!(
            "<div class=\"media\"><iframe src=\"{url}\" title=\"{name}\"></iframe>{actions}</div>",
            url = html_escape(&raw_url),
            name = html_escape(&file_name(file, rel)),
            actions = actions(&raw_url),
        ),
        Kind::Text => text_body(file, size, &raw_url),
        Kind::Binary => format!(
            "<div class=\"state\">\
<p class=\"lead\">Nothing to show for <code>{name}</code>.</p>\
<p class=\"dir\">{mime}, {size}. <a href=\"{url}\" download>Download it</a>.</p>\
</div>",
            name = html_escape(&file_name(file, rel)),
            mime = html_escape(mime),
            size = human_size(size),
            url = html_escape(&raw_url),
        ),
    };

    let meta = match human_age(std::fs::metadata(file).and_then(|m| m.modified()).ok()) {
        age if age.is_empty() => format!("{} · {}", human_size(size), mime),
        age => format!("{} · {} · edited {} ago", human_size(size), mime, age),
    };

    let html = shell::render(
        root,
        shell::Page {
            title: file_name(file, rel),
            current: rel,
            meta,
            body,
            prose: false,
            outline: String::new(),
        },
        dev_mode,
    );
    html_page(html, StatusCode::OK)
}

/// A text file, shown in the page rather than downloaded. Soli sources are
/// highlighted by the language's own lexer, like fenced blocks in Markdown.
fn text_body(file: &Path, size: u64, raw_url: &str) -> String {
    if size > MAX_INLINE_TEXT {
        return format!(
            "<div class=\"state\">\
<p class=\"lead\">Too big to show &mdash; {size}.</p>\
<p class=\"dir\"><a href=\"{url}\">Open the raw file</a>.</p>\
</div>",
            size = human_size(size),
            url = html_escape(raw_url),
        );
    }

    let Ok(source) = std::fs::read_to_string(file) else {
        return format!(
            "<div class=\"state\">\
<p class=\"lead\">This file is not valid UTF-8.</p>\
<p class=\"dir\"><a href=\"{url}\" download>Download it</a>.</p>\
</div>",
            url = html_escape(raw_url),
        );
    };

    let extension = file
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    let rendered = if SOLI_EXTENSIONS.contains(&extension.as_str()) {
        source
            .lines()
            .map(html_highlight_soli)
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        html_escape(&source).into_owned()
    };

    format!(
        "<div class=\"media\"><pre class=\"code\" data-lang=\"{lang}\"><code>{body}</code></pre>{actions}</div>",
        lang = html_escape(if extension.is_empty() { "text" } else { &extension }),
        body = rendered,
        actions = actions(raw_url),
    )
}

fn actions(raw_url: &str) -> String {
    format!(
        "<p class=\"actions\"><a href=\"{url}\">open raw</a> · \
<a href=\"{url}\" download>download</a></p>",
        url = html_escape(raw_url)
    )
}

fn file_name(file: &Path, rel: &str) -> String {
    file.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| rel.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                hyper::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        map
    }

    #[test]
    fn a_navigation_gets_the_viewer() {
        assert!(wants_viewer(
            &headers(&[("sec-fetch-dest", "document")]),
            None
        ));
    }

    #[test]
    fn an_embedded_image_gets_the_bytes() {
        // This is the case that must not regress: an <img> inside a rendered
        // Markdown page needs the file, not a page about the file.
        assert!(!wants_viewer(
            &headers(&[("sec-fetch-dest", "image")]),
            None
        ));
        assert!(!wants_viewer(
            &headers(&[("sec-fetch-dest", "video")]),
            None
        ));
    }

    #[test]
    fn sec_fetch_dest_outranks_accept() {
        // Chrome sends both on a subresource request; the explicit signal wins.
        let map = headers(&[
            ("sec-fetch-dest", "image"),
            ("accept", "text/html,image/webp,*/*"),
        ]);
        assert!(!wants_viewer(&map, None));
    }

    #[test]
    fn older_browsers_fall_back_to_accept() {
        assert!(wants_viewer(
            &headers(&[("accept", "text/html,application/xhtml+xml")]),
            None
        ));
        assert!(!wants_viewer(
            &headers(&[("accept", "image/webp,*/*")]),
            None
        ));
    }

    #[test]
    fn a_tool_with_no_headers_gets_the_bytes() {
        // `curl https://host/photo.jpg > photo.jpg` must produce a JPEG.
        assert!(!wants_viewer(&HeaderMap::new(), None));
    }

    #[test]
    fn raw_always_wins() {
        let navigation = headers(&[("sec-fetch-dest", "document")]);
        assert!(!wants_viewer(&navigation, Some("raw")));
        assert!(!wants_viewer(&navigation, Some("raw=1")));
        assert!(!wants_viewer(&navigation, Some("a=1&raw")));
        // A parameter that merely starts with the same letters is not the flag.
        assert!(wants_viewer(&navigation, Some("rawr=1")));
    }

    #[test]
    fn classifies_by_mime_then_extension() {
        assert_eq!(kind_of(&PathBuf::from("a.png"), "image/png"), Kind::Image);
        assert_eq!(kind_of(&PathBuf::from("a.mp4"), "video/mp4"), Kind::Video);
        assert_eq!(kind_of(&PathBuf::from("a.mp3"), "audio/mpeg"), Kind::Audio);
        assert_eq!(
            kind_of(&PathBuf::from("a.pdf"), "application/pdf"),
            Kind::Pdf
        );
        assert_eq!(
            kind_of(&PathBuf::from("a.txt"), "text/plain; charset=utf-8"),
            Kind::Text
        );
        // No registered MIME type, but plainly text.
        assert_eq!(
            kind_of(&PathBuf::from("post.sl"), "application/octet-stream"),
            Kind::Text
        );
        assert_eq!(
            kind_of(&PathBuf::from("a.bin"), "application/octet-stream"),
            Kind::Binary
        );
    }
}
