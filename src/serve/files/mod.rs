//! "File mode" — serving a plain directory as a website.
//!
//! `soli serve <dir>` assumes an MVC app. When the folder has no `app/`
//! controllers and no `config/routes.sl`, there is nothing to route to, and
//! the server used to refuse to start. Instead it now falls back to this
//! mode, which turns any directory into a browsable site:
//!
//! * files are served straight off disk (MIME, ETag, Range, `HEAD`);
//! * `.md` is rendered to a styled HTML page;
//! * `.slv` / `.erb` templates are executed by the template engine;
//! * every folder gets a generated index, and every page gets a sidebar
//!   carrying the whole tree.
//!
//! The mode is *read-only and code-adjacent*: it never loads `.env`, never
//! initializes a DB connection, and never executes controllers. Templates are
//! the one exception — they are code, by definition, so file mode is only for
//! directories whose contents you trust.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use bytes::Bytes;
use hyper::{HeaderMap, Response, StatusCode};

use super::{full, server_constants, ResponseBody};

mod assets;
mod index;
mod markdown;
mod preview;
mod render;
mod shell;
mod tree;

pub(crate) use assets::{handle_files_css, handle_files_js};
pub(crate) use render::render as render_template;

/// How `soli serve` should treat the target folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServeMode {
    /// A Soli MVC app: controllers, routes, models, the whole framework.
    App,
    /// A plain directory: static files, Markdown, templates.
    Files,
}

/// Root of the served directory, set once at boot when running in file mode.
///
/// A process-global rather than another parameter threaded through
/// `handle_hyper_request`: that function already carries nine arguments, and
/// the root is fixed for the lifetime of the server.
static FILES_ROOT: OnceLock<PathBuf> = OnceLock::new();

pub(crate) fn set_files_root(root: PathBuf) {
    let _ = FILES_ROOT.set(root);
}

/// The served root, or `None` when this process is serving an MVC app.
pub(crate) fn files_root() -> Option<&'static Path> {
    FILES_ROOT.get().map(PathBuf::as_path)
}

/// Extra read-only static roots — `soli serve <dir> --assets <dir>`.
///
/// A folder of documents often points at assets that live outside it: the
/// pages under `www/docs/` embed `/images/blog/x.svg`, and those files sit in
/// `www/public/images/`. Serving `www/docs` on its own answers every page and
/// 404s every picture, because in file mode the served folder is the whole
/// static root — there is no `public/` sub-root to fall back to.
///
/// Each root is consulted in the order given, and only after the served root
/// has failed, so the primary root always wins.
static ASSETS_ROOTS: OnceLock<Vec<PathBuf>> = OnceLock::new();

/// Pin the extra static roots at boot. Paths must already be absolute — the
/// server may daemonize (and change directory) after this point.
pub fn set_assets_roots(roots: Vec<PathBuf>) {
    let _ = ASSETS_ROOTS.set(roots);
}

/// The extra static roots, empty when none were requested.
pub(crate) fn assets_roots() -> &'static [PathBuf] {
    ASSETS_ROOTS.get().map(Vec::as_slice).unwrap_or(&[])
}

/// Mode pinned on the command line, if any.
static REQUESTED_MODE: OnceLock<ServeMode> = OnceLock::new();

/// Pin the mode explicitly — `soli serve --static` / `--app`. Overrides
/// detection, so `--app` on a folder with no controllers still fails with the
/// MVC structure error rather than quietly serving files.
pub fn request_mode(mode: ServeMode) {
    let _ = REQUESTED_MODE.set(mode);
}

/// How this folder should be served: what the operator asked for, or what the
/// folder looks like.
pub fn resolve_mode(folder: &Path) -> ServeMode {
    if let Some(mode) = REQUESTED_MODE.get() {
        return *mode;
    }
    if looks_like_soli_app(folder) {
        ServeMode::App
    } else {
        ServeMode::Files
    }
}

/// Does this folder look like a Soli MVC app?
///
/// Two markers, either of which is enough: `app/controllers/` (what the server
/// has always required) or `config/routes.sl` (an app that declares its routes
/// explicitly but keeps handlers elsewhere). Anything else is a plain
/// directory.
pub fn looks_like_soli_app(folder: &Path) -> bool {
    folder.join("app").join("controllers").is_dir()
        || folder.join("config").join("routes.sl").is_file()
}

/// A folder's index document, in priority order.
///
/// `index.*` means "this *is* the page for this directory", so it replaces the
/// generated listing entirely — the same convention every web server and
/// static-site generator uses. Each one is then served by its own rule:
/// `.html`/`.htm` as bytes, `.md` rendered, `.slv`/`.erb` executed.
const INDEX_NAMES: &[&str] = &[
    "index.html",
    "index.htm",
    "index.md",
    "index.html.slv",
    "index.slv",
    "index.html.erb",
    "index.erb",
];

/// A folder's own documentation, rendered *above* its listing.
///
/// Distinct from [`INDEX_NAMES`] on purpose: a `README` describes a directory,
/// an `index` replaces it. Same split as GitHub and every static host.
const README_NAMES: &[&str] = &["README.md", "readme.md", "Readme.md"];

/// Extensions rendered as Markdown.
const MARKDOWN_EXTENSIONS: &[&str] = &[".md", ".markdown"];

/// Template suffixes handed to the template engine. Longest first — `.html.slv`
/// must win over `.slv` when stripping the extension.
const TEMPLATE_SUFFIXES: &[&str] = &[".html.slv", ".html.erb", ".slv", ".erb"];

/// Extensions tried for an extension-less request path (`/about` → `about.md`).
const IMPLICIT_EXTENSIONS: &[&str] = &[".md", ".html", ".html.slv", ".slv", ".html.erb", ".erb"];

/// What file mode decided to do with a request.
pub(crate) enum Outcome {
    /// Answer it right here on the async side.
    Response(Response<ResponseBody>),
    /// Render this template on a worker (path relative to the root, template
    /// suffix stripped — the engine re-resolves the extension chain itself).
    Template(String),
}

/// Where a request path landed inside the root.
enum Located {
    Found(PathBuf),
    /// Absent, or deliberately invisible (dotfiles).
    Missing,
    /// Escaped the root — a traversal or symlink attempt.
    Forbidden,
}

/// Resolve a URL path inside the served root.
///
/// Mirrors the jail in [`super::resolve_static_file`]: canonicalize both sides
/// and compare with segment-aware `Path::starts_with`, then serve the
/// *canonical* path so a symlink planted between check and open cannot escape.
/// Unlike that function it also accepts directories, and it hides dotfiles.
fn locate(rel: &str, root: &Path) -> Located {
    if rel.contains("..") {
        return Located::Missing;
    }
    let candidate = if rel.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };

    let (canonical, canonical_root) = match (
        std::fs::canonicalize(&candidate),
        std::fs::canonicalize(root),
    ) {
        (Ok(f), Ok(r)) => (f, r),
        _ => return Located::Missing,
    };

    if !canonical.starts_with(&canonical_root) {
        return Located::Forbidden;
    }

    // Hide dotfiles — `.env`, `.git/`, `.ssh/`. Checked on the *canonical*
    // path so a symlink named `notes` pointing at `.env` is hidden too. A 404
    // rather than a 403: a 403 would confirm the file exists.
    if let Ok(sub) = canonical.strip_prefix(&canonical_root) {
        if sub
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        {
            return Located::Missing;
        }
    }

    Located::Found(canonical)
}

/// Decide what to serve for `path`.
///
/// Returns [`Outcome::Template`] when the answer needs an interpreter; every
/// other case is answered here, on the async side, without touching a worker.
pub(crate) fn handle(
    path: &str,
    method: &str,
    root: &Path,
    headers: &HeaderMap,
    query: Option<&str>,
    dev_mode: bool,
) -> Outcome {
    if method != "GET" && method != "HEAD" {
        return Outcome::Response(
            Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .header("Allow", "GET, HEAD")
                .header("Content-Type", "text/plain; charset=utf-8")
                .body(full(Bytes::from_static(
                    b"File mode serves GET and HEAD only",
                )))
                .unwrap(),
        );
    }

    let outcome = route(path, root, headers, query, dev_mode);

    // HEAD is the same response minus the bytes. Done centrally so every
    // branch above gets it right — Content-Length is set explicitly on every
    // response we build, so it survives the swap.
    match (method, outcome) {
        ("HEAD", Outcome::Response(resp)) => {
            let (parts, _) = resp.into_parts();
            Outcome::Response(Response::from_parts(parts, full(Bytes::new())))
        }
        (_, other) => other,
    }
}

fn route(
    path: &str,
    root: &Path,
    headers: &HeaderMap,
    query: Option<&str>,
    dev_mode: bool,
) -> Outcome {
    let raw = path.trim_start_matches('/');
    let decoded = urlencoding::decode(raw)
        .map(|d| d.into_owned())
        .unwrap_or_else(|_| raw.to_string());
    let rel = decoded.trim_end_matches('/');

    match locate(rel, root) {
        Located::Forbidden => Outcome::Response(forbidden()),
        Located::Found(target) => {
            if target.is_dir() {
                // Without a trailing slash every relative link in the folder's
                // own README would resolve one level too high.
                if !path.ends_with('/') {
                    return Outcome::Response(redirect(&format!("{}/", path)));
                }
                // A hand-written index document wins over the generated one,
                // and goes through the same per-extension rules as any other
                // file — so `index.md` renders and `index.slv` executes.
                for name in INDEX_NAMES {
                    let candidate = target.join(name);
                    if candidate.is_file() {
                        let candidate_rel = if rel.is_empty() {
                            (*name).to_string()
                        } else {
                            format!("{}/{}", rel, name)
                        };
                        return serve_file(
                            root,
                            &candidate,
                            &candidate_rel,
                            headers,
                            query,
                            dev_mode,
                        );
                    }
                }
                Outcome::Response(index::page(root, &target, rel, dev_mode))
            } else {
                serve_file(root, &target, rel, headers, query, dev_mode)
            }
        }
        Located::Missing => {
            // Nice URLs: `/about` finds `about.md`, `about.html`, `about.slv`.
            if !rel.is_empty() && !rel.ends_with('/') {
                for ext in IMPLICIT_EXTENSIONS {
                    let candidate = format!("{}{}", rel, ext);
                    if let Located::Found(target) = locate(&candidate, root) {
                        if target.is_file() {
                            return serve_file(root, &target, &candidate, headers, query, dev_mode);
                        }
                    }
                }
            }
            // Only once the served root has had every chance: the extra
            // `--assets` roots, which exist for exactly the paths it misses.
            if let Some(response) = asset_bytes(rel, assets_roots(), headers, dev_mode) {
                return Outcome::Response(response);
            }
            Outcome::Response(shell::not_found(root, rel, dev_mode))
        }
    }
}

/// Serve one resolved file: Markdown page, template, viewer, or raw bytes.
fn serve_file(
    root: &Path,
    target: &Path,
    rel: &str,
    headers: &HeaderMap,
    query: Option<&str>,
    dev_mode: bool,
) -> Outcome {
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    if MARKDOWN_EXTENSIONS.iter().any(|ext| name.ends_with(ext)) {
        return Outcome::Response(markdown::page(root, target, rel, dev_mode));
    }

    if let Some(suffix) = TEMPLATE_SUFFIXES.iter().find(|s| name.ends_with(*s)) {
        let stripped = &rel[..rel.len() - suffix.len()];
        return Outcome::Template(stripped.to_string());
    }

    let mime = server_constants::get_mime_type(target);

    // HTML is already a page — wrapping someone's own document in our chrome
    // would be presumptuous. Everything else that a browser navigated to gets
    // a viewer, so clicking a picture in a listing keeps you in the site
    // instead of dumping you on a blank page with the back button as the only
    // way home. Subresource requests (`<img src>`) still get the bytes.
    if !mime.starts_with("text/html") && preview::wants_viewer(headers, query) {
        return Outcome::Response(preview::page(root, target, rel, mime, dev_mode));
    }

    Outcome::Response(serve_static(target, headers, dev_mode))
}

/// Look `rel` up in the extra `--assets` roots and return its bytes.
///
/// Deliberately narrower than [`serve_file`]: an assets root holds data, not a
/// second site. Only an existing file answers — a directory gets no generated
/// listing (it is not part of the sidebar tree either), Markdown is not wrapped
/// in the shell, and a `.slv`/`.erb` is neither executed nor dumped as source,
/// so pointing `--assets` at a folder that happens to contain templates can
/// leak neither their output nor their text.
///
/// [`locate`] jails each lookup to its own root, so an assets root widens what
/// is reachable by exactly one directory tree and no more.
fn asset_bytes(
    rel: &str,
    roots: &[PathBuf],
    headers: &HeaderMap,
    dev_mode: bool,
) -> Option<Response<ResponseBody>> {
    if rel.is_empty() {
        return None;
    }
    for root in roots {
        let Located::Found(target) = locate(rel, root) else {
            continue;
        };
        if !target.is_file() {
            continue;
        }
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if TEMPLATE_SUFFIXES.iter().any(|s| name.ends_with(*s)) {
            continue;
        }
        return Some(serve_static(&target, headers, dev_mode));
    }
    None
}

/// Serve raw file bytes with MIME, ETag/304 and Range support.
///
/// A compact sibling of the app-mode static block in `serve::mod`: file mode
/// has no production asset cache to consult and no `public/` sub-root, so it
/// reads from disk and leans on the same
/// [`server_constants`] helpers for MIME, ETag and Range parsing.
fn serve_static(file_path: &Path, headers: &HeaderMap, dev_mode: bool) -> Response<ResponseBody> {
    let mime_type = server_constants::get_mime_type(file_path);

    let etag = std::fs::metadata(file_path)
        .and_then(|m| m.modified())
        .map(server_constants::generate_etag)
        .ok();

    if let Some(etag) = &etag {
        if let Some(client) = headers.get("if-none-match").and_then(|v| v.to_str().ok()) {
            if client == etag || client == format!("W/{}", etag) {
                return Response::builder()
                    .status(StatusCode::NOT_MODIFIED)
                    .header("ETag", etag)
                    .body(full(Bytes::new()))
                    .unwrap();
            }
        }
    }

    let content = match std::fs::read(file_path) {
        Ok(c) => c,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .header("Content-Type", "text/plain; charset=utf-8")
                .body(full(Bytes::from_static(b"Could not read file")))
                .unwrap()
        }
    };
    let total = content.len() as u64;

    if let Some(range) = headers.get("range").and_then(|v| v.to_str().ok()) {
        return match server_constants::parse_range_header(range, total) {
            Some((start, end)) => {
                let slice = &content[start as usize..=(end as usize).min(content.len() - 1)];
                Response::builder()
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header("Content-Type", mime_type)
                    .header(
                        "Content-Range",
                        format!("bytes {}-{}/{}", start, end, total),
                    )
                    .header("Content-Length", (end - start + 1).to_string())
                    .header("Accept-Ranges", "bytes")
                    .body(full(Bytes::copy_from_slice(slice)))
                    .unwrap()
            }
            None => Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header("Content-Range", format!("bytes */{}", total))
                .body(full(Bytes::new()))
                .unwrap(),
        };
    }

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", mime_type)
        .header("Content-Length", total.to_string())
        .header("Accept-Ranges", "bytes");
    if let Some(etag) = etag {
        builder = builder.header("ETag", etag);
        // No immutable max-age here: file mode serves a directory the operator
        // is actively editing, and the URLs carry no content hash. ETag gives
        // the bandwidth win without pinning a stale copy in the browser.
        if !dev_mode {
            builder = builder.header("Cache-Control", "public, max-age=0, must-revalidate");
        }
    }
    builder.body(full(Bytes::from(content))).unwrap()
}

fn forbidden() -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::FORBIDDEN)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(full(Bytes::from_static(b"Forbidden")))
        .unwrap()
}

fn redirect(location: &str) -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::MOVED_PERMANENTLY)
        .header("Location", location)
        .header("Content-Length", "0")
        .body(full(Bytes::new()))
        .unwrap()
}

/// Render `bytes` as an HTML page response with an explicit Content-Length.
fn html_page(html: String, status: StatusCode) -> Response<ResponseBody> {
    let bytes = Bytes::from(html);
    Response::builder()
        .status(status)
        .header("Content-Type", "text/html; charset=utf-8")
        .header("Content-Length", bytes.len().to_string())
        .body(full(bytes))
        .unwrap()
}

/// Find a folder's own page (`README.md` and friends), if it has one.
fn readme_in(dir: &Path) -> Option<PathBuf> {
    README_NAMES
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
}

/// Human-readable byte count: `948 B`, `4.1 KB`, `1.2 MB`.
fn human_size(bytes: u64) -> String {
    const UNITS: &[(u64, &str)] = &[
        (1024 * 1024 * 1024, "GB"),
        (1024 * 1024, "MB"),
        (1024, "KB"),
    ];
    for (scale, unit) in UNITS {
        if bytes >= *scale {
            let value = bytes as f64 / *scale as f64;
            // One decimal below 10 (4.1 MB), none above (128 MB).
            return if value < 10.0 {
                format!("{:.1} {}", value, unit)
            } else {
                format!("{:.0} {}", value, unit)
            };
        }
    }
    format!("{} B", bytes)
}

/// Compact age: `12s`, `5m`, `2h`, `3d`, `2w`, `1y`.
fn human_age(modified: Option<SystemTime>) -> String {
    let Some(modified) = modified else {
        return String::new();
    };
    let Ok(elapsed) = modified.elapsed() else {
        // Clock skew, or a file dated in the future.
        return String::new();
    };
    let secs = elapsed.as_secs();
    const MIN: u64 = 60;
    const HOUR: u64 = 60 * MIN;
    const DAY: u64 = 24 * HOUR;
    const WEEK: u64 = 7 * DAY;
    const YEAR: u64 = 365 * DAY;
    match secs {
        s if s < MIN => format!("{}s", s),
        s if s < HOUR => format!("{}m", s / MIN),
        s if s < DAY => format!("{}h", s / HOUR),
        s if s < WEEK => format!("{}d", s / DAY),
        s if s < YEAR => format!("{}w", s / WEEK),
        s => format!("{}y", s / YEAR),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("soli_files_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_an_mvc_app_by_controllers() {
        let dir = tmpdir("detect_controllers");
        assert!(!looks_like_soli_app(&dir));
        std::fs::create_dir_all(dir.join("app/controllers")).unwrap();
        assert!(looks_like_soli_app(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_an_mvc_app_by_routes_file() {
        let dir = tmpdir("detect_routes");
        std::fs::create_dir_all(dir.join("config")).unwrap();
        assert!(!looks_like_soli_app(&dir));
        std::fs::write(dir.join("config/routes.sl"), "get(\"/\", \"home#index\")").unwrap();
        assert!(looks_like_soli_app(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locate_hides_dotfiles() {
        let dir = tmpdir("dotfiles");
        std::fs::write(dir.join(".env"), "SECRET=1").unwrap();
        assert!(matches!(locate(".env", &dir), Located::Missing));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locate_rejects_traversal() {
        let dir = tmpdir("traversal");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        assert!(matches!(
            locate("sub/../../etc/passwd", &dir),
            Located::Missing
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locate_finds_a_plain_file() {
        let dir = tmpdir("plain");
        std::fs::write(dir.join("a.md"), "# hi").unwrap();
        assert!(matches!(locate("a.md", &dir), Located::Found(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- end-to-end resolution ----------
    //
    // Drives the real `handle` against a real directory, so these cover the
    // whole order-of-resolution table rather than `locate` alone.

    /// A tree with one of everything the resolver has a rule for.
    fn site(name: &str) -> PathBuf {
        let dir = tmpdir(name);
        std::fs::create_dir_all(dir.join("guides/deep")).unwrap();
        std::fs::write(dir.join("README.md"), "# Field notes\n\ntext").unwrap();
        std::fs::write(dir.join("guides/intro.md"), "# Getting started").unwrap();
        std::fs::write(dir.join("guides/deep/notes.txt"), "plain text").unwrap();
        std::fs::write(dir.join("about.html.slv"), "<h1>About</h1>").unwrap();
        std::fs::write(dir.join(".env"), "SECRET=1").unwrap();
        std::fs::write(dir.join("LOGO.PNG"), "PNGDATA").unwrap();
        dir
    }

    /// Headers a browser sends when you click a link, so `get` exercises the
    /// same path a real navigation takes.
    fn navigation() -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert("sec-fetch-dest", "document".parse().unwrap());
        map
    }

    fn get(path: &str, root: &Path) -> Outcome {
        handle(path, "GET", root, &navigation(), None, false)
    }

    fn status(path: &str, root: &Path) -> StatusCode {
        match get(path, root) {
            Outcome::Response(response) => response.status(),
            Outcome::Template(_) => panic!("{} resolved to a template", path),
        }
    }

    #[test]
    fn serves_a_folder_index_and_a_markdown_page() {
        let dir = site("index_and_md");
        assert_eq!(status("/", &dir), StatusCode::OK);
        assert_eq!(status("/guides/", &dir), StatusCode::OK);
        assert_eq!(status("/guides/intro.md", &dir), StatusCode::OK);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_without_a_trailing_slash_redirects() {
        let dir = site("redirect");
        let Outcome::Response(response) = get("/guides", &dir) else {
            panic!("expected a redirect");
        };
        assert_eq!(response.status(), StatusCode::MOVED_PERMANENTLY);
        assert_eq!(response.headers().get("Location").unwrap(), "/guides/");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_template_goes_to_a_worker_with_its_extension_stripped() {
        let dir = site("template");
        match get("/about.html.slv", &dir) {
            Outcome::Template(relative) => assert_eq!(relative, "about"),
            Outcome::Response(_) => panic!("template was answered inline"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_extensionless_path_finds_markdown_then_templates() {
        let dir = site("implicit");
        // `guides/intro.md` exists, so `/guides/intro` is that page.
        assert_eq!(status("/guides/intro", &dir), StatusCode::OK);
        // `about.html.slv` is the only `about`, so `/about` is the template.
        match get("/about", &dir) {
            Outcome::Template(relative) => assert_eq!(relative, "about"),
            Outcome::Response(_) => panic!("expected the template"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dotfiles_are_invisible_rather_than_forbidden() {
        let dir = site("hidden");
        // 404, not 403: a 403 would confirm the file is there.
        assert_eq!(status("/.env", &dir), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_symlink_escaping_the_root_is_forbidden() {
        let dir = site("symlink");
        let outside = tmpdir("symlink_outside");
        std::fs::write(outside.join("secret.txt"), "nope").unwrap();

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.join("secret.txt"), dir.join("leak.txt")).unwrap();
            assert_eq!(status("/leak.txt", &dir), StatusCode::FORBIDDEN);
        }

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_miss_is_a_404_naming_the_nearest_folder() {
        let dir = site("miss");
        let Outcome::Response(response) = get("/guides/nope.md", &dir) else {
            panic!("expected a 404 page");
        };
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn head_keeps_the_headers_and_drops_the_body() {
        let dir = site("head");
        let response = match handle(
            "/guides/deep/notes.txt",
            "HEAD",
            &dir,
            &navigation(),
            Some("raw"),
            false,
        ) {
            Outcome::Response(response) => response,
            Outcome::Template(_) => panic!("unexpected template"),
        };
        assert_eq!(response.status(), StatusCode::OK);
        // Content-Length still describes the resource, per HTTP semantics.
        assert_eq!(response.headers().get("Content-Length").unwrap(), "10");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writes_are_refused() {
        let dir = site("methods");
        let Outcome::Response(response) = handle("/", "POST", &dir, &navigation(), None, false)
        else {
            panic!("expected a 405");
        };
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers().get("Allow").unwrap(), "GET, HEAD");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_uppercase_extension_still_gets_its_mime_type() {
        let dir = site("mime");
        // Asked for as a file rather than navigated to: the bytes, typed.
        let Outcome::Response(response) =
            handle("/LOGO.PNG", "GET", &dir, &HeaderMap::new(), None, false)
        else {
            panic!("expected the file");
        };
        assert_eq!(response.headers().get("Content-Type").unwrap(), "image/png");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clicking_an_image_opens_a_viewer_but_embedding_it_does_not() {
        let dir = site("viewer");

        // A click: the picture arrives wrapped in the shell.
        let Outcome::Response(viewer) = get("/LOGO.PNG", &dir) else {
            panic!("expected a viewer page");
        };
        assert_eq!(
            viewer.headers().get("Content-Type").unwrap(),
            "text/html; charset=utf-8"
        );

        // An `<img>` inside a rendered Markdown page: the bytes. This is the
        // case that must never regress — a viewer here would show an HTML
        // document where the picture should be.
        let mut subresource = HeaderMap::new();
        subresource.insert("sec-fetch-dest", "image".parse().unwrap());
        let Outcome::Response(raw) = handle("/LOGO.PNG", "GET", &dir, &subresource, None, false)
        else {
            panic!("expected the bytes");
        };
        assert_eq!(raw.headers().get("Content-Type").unwrap(), "image/png");

        // `?raw` overrides the navigation signal, which is what the viewer's
        // own <img> and download link use.
        let Outcome::Response(forced) =
            handle("/LOGO.PNG", "GET", &dir, &navigation(), Some("raw"), false)
        else {
            panic!("expected the bytes");
        };
        assert_eq!(forced.headers().get("Content-Type").unwrap(), "image/png");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_markdown_page_is_never_replaced_by_a_viewer() {
        // `.md` and templates are matched before the viewer, so a navigation
        // to one still renders the document.
        let dir = site("md_not_viewer");
        assert_eq!(status("/guides/intro.md", &dir), StatusCode::OK);
        match get("/about.html.slv", &dir) {
            Outcome::Template(relative) => assert_eq!(relative, "about"),
            Outcome::Response(_) => panic!("template was replaced by a viewer"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_explicit_index_html_wins_over_the_generated_one() {
        let dir = site("explicit_index");
        std::fs::write(dir.join("guides/index.html"), "<p>hand written</p>").unwrap();
        let Outcome::Response(response) = get("/guides/", &dir) else {
            panic!("expected the file");
        };
        assert_eq!(response.headers().get("Content-Type").unwrap(), "text/html");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_index_template_is_executed_for_its_folder() {
        let dir = site("index_template");
        std::fs::write(dir.join("guides/index.html.slv"), "<h1>Guides</h1>").unwrap();
        match get("/guides/", &dir) {
            Outcome::Template(relative) => assert_eq!(relative, "guides/index"),
            Outcome::Response(_) => panic!("index template was not executed"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn index_html_outranks_the_other_index_documents() {
        let dir = site("index_priority");
        std::fs::write(dir.join("guides/index.md"), "# From markdown").unwrap();
        std::fs::write(dir.join("guides/index.html"), "<p>from html</p>").unwrap();
        let Outcome::Response(response) = get("/guides/", &dir) else {
            panic!("expected a response");
        };
        // `index.html` is first in INDEX_NAMES, so it wins and is served raw —
        // its own 16 bytes, not the far larger rendered Markdown page.
        assert_eq!(response.headers().get("Content-Type").unwrap(), "text/html");
        assert_eq!(response.headers().get("Content-Length").unwrap(), "16");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_readme_is_not_an_index() {
        // A README describes a folder; an index replaces it. The fixture's
        // root has a README.md and must still produce the generated listing.
        let dir = site("readme_vs_index");
        let Outcome::Response(response) = get("/", &dir) else {
            panic!("expected the generated index");
        };
        assert_eq!(response.status(), StatusCode::OK);
        assert!(readme_in(&dir).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- extra `--assets` roots ----------
    //
    // Driven through `asset_bytes` rather than `handle`, because the roots
    // reach the resolver through a process-global `OnceLock` that a test suite
    // sharing one process can only set once. Precedence is structural: the
    // single call site is the `Located::Missing` arm of `route`, after the
    // implicit-extension probe, so the served root always answers first.

    /// A `public/`-shaped folder living outside the served root.
    fn assets_dir(name: &str) -> PathBuf {
        let dir = tmpdir(name);
        std::fs::create_dir_all(dir.join("images/blog")).unwrap();
        std::fs::write(dir.join("images/blog/post.svg"), "<svg/>").unwrap();
        std::fs::write(dir.join(".env"), "SECRET=1").unwrap();
        dir
    }

    #[test]
    fn an_assets_root_answers_what_the_served_root_misses() {
        let assets = assets_dir("assets_hit");
        let roots = vec![assets.clone()];
        let response = asset_bytes("images/blog/post.svg", &roots, &HeaderMap::new(), false)
            .expect("the assets root should have answered");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get("Content-Type").unwrap(),
            "image/svg+xml"
        );
        // Same conditional-GET treatment as the served root's own files.
        assert!(response.headers().get("ETag").is_some());
        let _ = std::fs::remove_dir_all(&assets);
    }

    #[test]
    fn assets_roots_are_tried_in_order() {
        let first = tmpdir("assets_order_first");
        let second = tmpdir("assets_order_second");
        std::fs::write(first.join("logo.txt"), "first").unwrap();
        std::fs::write(second.join("logo.txt"), "second").unwrap();
        std::fs::write(second.join("only.txt"), "second only").unwrap();
        let roots = vec![first.clone(), second.clone()];

        let response = asset_bytes("logo.txt", &roots, &HeaderMap::new(), false).unwrap();
        assert_eq!(response.headers().get("Content-Length").unwrap(), "5");
        // A path only the second root has is still reachable.
        assert!(asset_bytes("only.txt", &roots, &HeaderMap::new(), false).is_some());

        let _ = std::fs::remove_dir_all(&first);
        let _ = std::fs::remove_dir_all(&second);
    }

    #[test]
    fn an_assets_root_is_data_not_a_second_site() {
        let assets = assets_dir("assets_narrow");
        std::fs::write(assets.join("page.html.slv"), "<h1>Templated</h1>").unwrap();
        std::fs::write(assets.join("notes.md"), "# Notes").unwrap();
        let roots = vec![assets.clone()];
        let headers = HeaderMap::new();

        // A directory gets no generated listing — it is not in the tree.
        assert!(asset_bytes("images", &roots, &headers, false).is_none());
        // A template is neither executed nor dumped as source.
        assert!(asset_bytes("page.html.slv", &roots, &headers, false).is_none());
        // No implicit-extension probe either: only exact files answer.
        assert!(asset_bytes("notes", &roots, &headers, false).is_none());
        assert!(asset_bytes("notes.md", &roots, &headers, false).is_some());

        let _ = std::fs::remove_dir_all(&assets);
    }

    #[test]
    fn each_assets_root_is_jailed_like_the_served_root() {
        let assets = assets_dir("assets_jail");
        let outside = tmpdir("assets_jail_outside");
        std::fs::write(outside.join("secret.txt"), "nope").unwrap();
        let roots = vec![assets.clone()];
        let headers = HeaderMap::new();

        assert!(asset_bytes(".env", &roots, &headers, false).is_none());
        assert!(asset_bytes("images/../../etc/passwd", &roots, &headers, false).is_none());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.join("secret.txt"), assets.join("leak.txt"))
                .unwrap();
            assert!(asset_bytes("leak.txt", &roots, &headers, false).is_none());
        }

        let _ = std::fs::remove_dir_all(&assets);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn no_assets_roots_means_no_extra_lookup() {
        let headers = HeaderMap::new();
        assert!(asset_bytes("images/blog/post.svg", &[], &headers, false).is_none());
        assert!(assets_roots().is_empty());
    }

    #[test]
    fn human_size_scales() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(948), "948 B");
        assert_eq!(human_size(4200), "4.1 KB");
        assert_eq!(human_size(20 * 1024), "20 KB");
        assert_eq!(human_size(3 * 1024 * 1024 / 2), "1.5 MB");
    }
}
