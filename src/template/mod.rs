//! ERB-style template engine for Soli MVC.
//!
//! Supports:
//! - `<%= expr %>` - HTML-escaped output
//! - `<%- expr %>` - Raw/unescaped output (no HTML escaping)
//! - `<% if/for/end %>` - Control flow
//! - `<%= yield %>` - Layout content insertion point
//! - `<% content_for "name" do %>...<% end %>` / `<%= yield "name" %>` - Named content blocks
//! - `<%= render 'partial' %>` - Partial rendering

pub mod content_store;
pub mod core_eval;
pub mod helpers;
pub mod layout;
pub mod parser;
pub mod renderer;
pub mod response_cache;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Instant, SystemTime};

use crate::interpreter::value::Value;
use crate::serve::{vfs_exists, vfs_read_to_string};
use parser::parse_template;
use renderer::{render_nodes_with_path, render_with_interpreter};
use std::rc::Rc;

/// A cached template with its parsed AST and modification time.
#[derive(Debug, Clone)]
struct CachedTemplate {
    nodes: Arc<Vec<parser::TemplateNode>>,
    modified: SystemTime,
}

/// Reject template names that could escape the views directory or contain
/// path-shaping bytes. We only allow `Component::Normal` segments — `..`,
/// absolute roots, drive prefixes, and `\0` are all refused before any
/// `dir.join(name)` call. Forward slashes between normal segments are still
/// fine, so `users/show` still resolves.
pub fn is_safe_template_name(name: &str) -> bool {
    if name.is_empty() || name.contains('\0') || name.contains('\\') {
        return false;
    }
    if name.starts_with('/') {
        return false;
    }
    Path::new(name)
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// Maximum size for path cache to prevent unbounded memory growth.
const PATH_CACHE_MAX_SIZE: usize = 1000;

/// Maximum size for template cache to prevent unbounded memory growth.
const TEMPLATE_CACHE_MAX_SIZE: usize = 500;

/// Template cache that stores parsed templates and tracks file changes.
///
/// Shared across worker threads via `Arc<TemplateCache>`; a template parsed by
/// any worker is visible to the others, avoiding per-worker reparsing.
pub struct TemplateCache {
    /// Base directory for views (e.g., app/views)
    views_dir: PathBuf,
    /// Cached parsed templates (PathBuf -> nodes).
    cache: RwLock<HashMap<PathBuf, CachedTemplate>>,
    /// Cached path resolutions (template_name -> resolved_path).
    /// Arc so cache hits are pointer increments, not heap clones.
    path_cache: RwLock<HashMap<String, Arc<PathBuf>>>,
}

impl TemplateCache {
    /// Create a new template cache for the given views directory.
    pub fn new(views_dir: impl Into<PathBuf>) -> Self {
        Self {
            views_dir: views_dir.into(),
            cache: RwLock::new(HashMap::new()),
            path_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Get the views directory path.
    pub fn views_dir(&self) -> &Path {
        &self.views_dir
    }

    /// Render a template with the given data.
    ///
    /// # Arguments
    /// * `template_name` - Template path relative to views dir (e.g., "users/index")
    /// * `data` - Data context for the template
    /// * `layout` - Optional layout name (defaults to "application"), or None to skip layout
    pub fn render(
        &self,
        template_name: &str,
        data: &Value,
        layout: Option<Option<&str>>,
    ) -> Result<String, String> {
        // Cached path: if `(template, layout, data_sig)` is in the
        // response cache and the request isn't dirty, return the cached
        // body directly. The cached body is the raw (pre-ETag) body
        // produced by the template AST walk; the FNV-1a ETag is
        // recomputed in `html_response` on every request.
        let data_sig = response_cache::data_signature(data);
        let layout_name: Option<&str> = match layout {
            Some(Some(name)) => Some(name),
            _ => None,
        };
        let template_path = self.resolve_template_path(template_name)?;
        if let Some(cached) = response_cache::get(template_path.clone(), layout_name, data_sig) {
            return Ok(cached.body);
        }
        let body =
            self.render_uncached(template_name, data, layout, &template_path, layout_name)?;
        // Store the freshly-rendered body. On the next identical
        // request the cache lookup above short-circuits to this body.
        response_cache::put(
            template_path,
            layout_name,
            data_sig,
            body.clone(),
            String::new(),
        );
        Ok(body)
    }

    /// Internal: do the actual template render. Wrapped by `render`
    /// which adds the static-page response cache on top.
    fn render_uncached(
        &self,
        template_name: &str,
        data: &Value,
        layout: Option<Option<&str>>,
        template_path: &std::path::Path,
        _layout_name: Option<&str>,
    ) -> Result<String, String> {
        // Treat undefined variable lookups as Null throughout template
        // rendering — controllers commonly omit optional locals like
        // `flash`, and the scaffolded layout references them directly.
        let _guard = crate::interpreter::executor::enter_template_lenient_vars();

        // content_for store for this render: the view captures named blocks,
        // the layout reads them back. Dropped at the end of the render so
        // captures never leak into the next request on this worker thread.
        let _content_frame = content_store::ensure_frame();

        // Per-template span for the dev-bar flamegraph + flat per-template
        // duration log. Both early-out when --dev is off.
        let mut _span = crate::serve::span_log::SpanGuard::start(
            template_name,
            crate::serve::span_log::SpanKind::View,
        );
        let view_start = crate::serve::view_log::is_enabled().then(std::time::Instant::now);
        let view_id = view_start.map(|_| crate::serve::view_log::next_id());
        if let Some(id) = view_id {
            _span.set_render_id(id);
        }

        // Timing for production Prometheus metrics (Phase A visibility), gated on
        // `SOLI_METRICS` so the `Instant::now()` is skipped when nobody is scraping.
        // This captures the full cost of view + layout + all partials executed during render.
        let render_start = crate::metrics::metrics_enabled().then(Instant::now);

        // Get template from cache
        let nodes = self.get_or_load_template(template_path)?;

        // Create partial renderer closure
        let partial_renderer =
            |name: &str, ctx: &Value| -> Result<String, String> { self.render_partial(name, ctx) };

        // Create ONE interpreter for both view and layout rendering
        let mut interpreter = core_eval::create_template_interpreter(data);

        // Render the template content with shared interpreter
        let template_path_str = template_path.to_string_lossy();
        let content = render_with_interpreter(
            &mut interpreter,
            &nodes,
            data,
            Some(&partial_renderer),
            Some(&template_path_str),
        )?;

        // If the template is a markdown file, convert to HTML. Use the
        // URL-neutralizing converter: the ERB pass above already escaped
        // interpolated data, so this only needs to stop markdown link/image
        // syntax from smuggling a `javascript:` URL past that escaping.
        let content = if is_markdown_template(template_path) {
            markdown_to_html_safe_urls(&content)
        } else {
            content
        };

        // Wrap the view's own content in dev-bar marker comments so the
        // hover overlay can find this template's region in the page. Only
        // wraps the inner view (not the layout that wraps it later).
        let content = if let Some(id) = view_id {
            wrap_dev_marker("view", id, template_name, &content)
        } else {
            content
        };

        // Apply layout if specified, reusing the same interpreter
        let result = match layout {
            Some(Some(layout_name)) => self.render_layout_with_shared_interpreter(
                &mut interpreter,
                &content,
                data,
                layout_name,
                &partial_renderer,
            ),
            Some(None) => {
                // Explicitly no layout (layout: false)
                Ok(content)
            }
            None => {
                // Default layout: application
                // But if the template name starts with underscore (partial), skip layout
                if template_name.contains("/_") || template_name.starts_with('_') {
                    Ok(content)
                } else {
                    self.render_layout_with_shared_interpreter(
                        &mut interpreter,
                        &content,
                        data,
                        "application",
                        &partial_renderer,
                    )
                }
            }
        };
        if let (Some(start), Some(id)) = (view_start, view_id) {
            crate::serve::view_log::record(id, template_name, start.elapsed().as_micros() as u64);
        }
        // Record for production Prometheus metrics (gated on SOLI_METRICS).
        if let Some(render_start) = render_start {
            crate::metrics::Metrics::global().record_template_render(render_start.elapsed());
        }
        result
    }

    /// Render a partial template (no layout).
    pub fn render_partial(&self, name: &str, data: &Value) -> Result<String, String> {
        // Per-partial span + view-log entry, matching `render` above.
        let mut _span = crate::serve::span_log::SpanGuard::start(
            name,
            crate::serve::span_log::SpanKind::Partial,
        );
        let view_start = crate::serve::view_log::is_enabled().then(std::time::Instant::now);
        let view_id = view_start.map(|_| crate::serve::view_log::next_id());
        if let Some(id) = view_id {
            _span.set_render_id(id);
        }

        // Timing for production Prometheus metrics (Phase A), gated on SOLI_METRICS.
        let render_start = crate::metrics::metrics_enabled().then(Instant::now);

        // Join the surrounding render's content_for store so captures inside
        // the partial reach the layout. A standalone partial render (no view
        // in progress) owns a throwaway frame instead — captures are dropped.
        let _content_frame = content_store::ensure_frame();

        // Partials start with underscore
        let partial_name = if name.contains('/') {
            // e.g., "users/card" -> "users/_card"
            let parts: Vec<&str> = name.rsplitn(2, '/').collect();
            if parts.len() == 2 {
                format!("{}/_{}", parts[1], parts[0])
            } else {
                format!("_{}", name)
            }
        } else {
            format!("_{}", name)
        };

        let template_path = self.resolve_template_path(&partial_name)?;
        let nodes = self.get_or_load_template(&template_path)?;

        let partial_renderer =
            |n: &str, ctx: &Value| -> Result<String, String> { self.render_partial(n, ctx) };

        let template_path_str = template_path.to_string_lossy();
        let content = render_nodes_with_path(
            &nodes,
            data,
            Some(&partial_renderer),
            Some(&template_path_str),
        )?;

        // If the partial is a markdown file, convert to HTML. URL-neutralizing
        // converter — the ERB pass already escaped interpolated data (see the
        // matching note in `render`).
        let result = if is_markdown_template(&template_path) {
            markdown_to_html_safe_urls(&content)
        } else {
            content
        };
        let result = if let Some(id) = view_id {
            wrap_dev_marker("partial", id, name, &result)
        } else {
            result
        };
        if let (Some(start), Some(id)) = (view_start, view_id) {
            crate::serve::view_log::record(id, name, start.elapsed().as_micros() as u64);
        }
        // Record for production Prometheus metrics (gated on SOLI_METRICS).
        if let Some(render_start) = render_start {
            crate::metrics::Metrics::global().record_template_render(render_start.elapsed());
        }
        Ok(result)
    }

    /// Render content with a named layout, reusing an existing interpreter.
    fn render_layout_with_shared_interpreter(
        &self,
        interpreter: &mut crate::interpreter::executor::Interpreter,
        content: &str,
        data: &Value,
        layout_name: &str,
        partial_renderer: &dyn Fn(&str, &Value) -> Result<String, String>,
    ) -> Result<String, String> {
        // Strip "layouts/" prefix if present to avoid double prefixing
        let layout_name = layout_name.trim_start_matches("layouts/");
        let layout_name = layout_name.trim_start_matches("layouts");

        // Fast path: avoid format! allocation for the most common layout name
        let layout_template;
        let layout_key = if layout_name == "application" {
            "layouts/application"
        } else {
            layout_template = format!("layouts/{}", layout_name);
            &layout_template
        };

        // Per-layout span + view-log entry, matching `render` and
        // `render_partial`. The recorded name is the resolved layout key
        // (e.g. "layouts/application") so it's distinguishable from the
        // top-level template in the dev-bar sub-row list.
        let mut _span = crate::serve::span_log::SpanGuard::start(
            layout_key,
            crate::serve::span_log::SpanKind::View,
        );
        let view_start = crate::serve::view_log::is_enabled().then(std::time::Instant::now);
        let view_id = view_start.map(|_| crate::serve::view_log::next_id());
        if let Some(id) = view_id {
            _span.set_render_id(id);
        }

        let result = match self.resolve_template_path(layout_key) {
            Ok(layout_path) => {
                let layout_nodes = self.get_or_load_template(&layout_path)?;
                let layout_path_str = layout_path.to_string_lossy();
                layout::render_layout_with_interpreter(
                    interpreter,
                    &layout_nodes,
                    content,
                    data,
                    Some(partial_renderer),
                    Some(&layout_path_str),
                )
            }
            Err(_) => {
                // No layout file, return content as-is
                Ok(content.to_string())
            }
        };

        let result = match (result, view_id) {
            (Ok(html), Some(id)) => Ok(wrap_dev_marker("layout", id, layout_key, &html)),
            (other, _) => other,
        };

        if let (Some(start), Some(id)) = (view_start, view_id) {
            crate::serve::view_log::record(id, layout_key, start.elapsed().as_micros() as u64);
        }

        result
    }

    /// Resolve a template name to a file path (cached).
    /// Returns Arc<PathBuf> so cache hits are pointer increments, not heap clones.
    fn resolve_template_path(&self, name: &str) -> Result<Arc<PathBuf>, String> {
        // Check path cache first
        if let Some(path) = self
            .path_cache
            .read()
            .ok()
            .and_then(|c| c.get(name).cloned())
        {
            return Ok(path);
        }

        // Cache miss - do file system lookup
        let resolved = Arc::new(self.do_resolve_template_path(name)?);

        // Cache the result (with eviction if cache is too large)
        if let Ok(mut path_cache) = self.path_cache.write() {
            if path_cache.len() >= PATH_CACHE_MAX_SIZE {
                path_cache.clear();
            }
            path_cache.insert(name.to_string(), Arc::clone(&resolved));
        }

        Ok(resolved)
    }

    /// Actually resolve the template path (file system lookup).
    fn do_resolve_template_path(&self, name: &str) -> Result<PathBuf, String> {
        if !is_safe_template_name(name) {
            return Err(format!("Template '{}' not found", name));
        }

        // Try main views directory first
        if let Ok(path) = self.try_resolve_in_dir(&self.views_dir, name) {
            return Ok(path);
        }

        // If template name starts with an engine name, resolve from the engine's views dir.
        // e.g. render("shop/index") → engines/shop/app/views/shop/index.html.slv
        if let Some(slash_pos) = name.find('/') {
            let candidate = &name[..slash_pos];
            if crate::serve::engine_loader::is_engine_name(candidate) {
                if let Some(engine_views_dir) = self
                    .views_dir
                    .parent() // app/views -> app
                    .and_then(|p| p.parent()) // app -> project root
                    .map(|root| {
                        root.join("engines")
                            .join(candidate)
                            .join("app")
                            .join("views")
                    })
                {
                    let view_path = &name[slash_pos + 1..];
                    if let Ok(path) = self.try_resolve_in_dir(&engine_views_dir, view_path) {
                        return Ok(path);
                    }
                    if let Ok(path) = self.try_resolve_in_dir(&engine_views_dir, name) {
                        return Ok(path);
                    }
                }
            }
        }

        Err(format!(
            "Template '{}' not found in {}",
            name,
            self.views_dir.display()
        ))
    }

    /// Try to resolve a template path in a specific directory with various extensions.
    fn try_resolve_in_dir(&self, dir: &Path, name: &str) -> Result<PathBuf, String> {
        if !is_safe_template_name(name) {
            return Err(format!("Template not found in {}", dir.display()));
        }

        let extensions = [
            ".html.slv",
            ".slv",
            ".html.md",
            ".md",
            ".html.erb",
            ".erb",
            "",
        ];

        for ext in extensions {
            let path = if ext.is_empty() {
                dir.join(name)
            } else {
                dir.join(format!("{}{}", name, ext))
            };
            let path_str = path.to_string_lossy().to_string();
            if vfs_exists(&path_str) {
                return Ok(path);
            }
        }

        Err(format!("Template not found in {}", dir.display()))
    }

    /// Get a template from cache or load and parse it.
    fn get_or_load_template(&self, path: &Path) -> Result<Arc<Vec<parser::TemplateNode>>, String> {
        // Check cache first (fast path - shared read lock)
        if let Some(nodes) = self
            .cache
            .read()
            .ok()
            .and_then(|c| c.get(path).map(|entry| entry.nodes.clone()))
        {
            return Ok(nodes);
        }

        // Cache miss - load and parse template
        let path_str = path.to_string_lossy().to_string();
        let source = vfs_read_to_string(&path_str)
            .map_err(|e| format!("Failed to read template '{}': {}", path.display(), e))?;

        let modified = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        // Surface the .slv path on parse failures. Without this, errors from
        // the template parser (e.g. a reserved keyword used as a `<% for ... %>`
        // loop variable) bubble up through the controller's `render(...)` call,
        // get tagged with the controller's file:line by the executor's frame
        // tracking, and leave the user hunting a non-existent bug in the
        // controller. Stamping the view path on the message keeps the
        // diagnostic pointed at the offending template.
        let nodes =
            Arc::new(parse_template(&source).map_err(|e| format!("{} in {}", e, path.display()))?);

        // Update cache (with eviction if cache is too large)
        if let Ok(mut cache) = self.cache.write() {
            if cache.len() >= TEMPLATE_CACHE_MAX_SIZE {
                cache.clear();
            }
            cache.insert(
                path.to_path_buf(),
                CachedTemplate {
                    nodes: nodes.clone(),
                    modified,
                },
            );
        }

        Ok(nodes)
    }

    /// Clear the template cache (useful for hot reload).
    pub fn clear(&self) {
        if let Ok(mut c) = self.cache.write() {
            c.clear();
        }
        if let Ok(mut c) = self.path_cache.write() {
            c.clear();
        }
    }

    /// Check if any tracked templates have changed.
    pub fn has_changes(&self) -> bool {
        let Ok(cache) = self.cache.read() else {
            return false;
        };
        for (path, cached) in cache.iter() {
            let path_str = path.to_string_lossy().to_string();
            if vfs_exists(&path_str) {
                if let Ok(metadata) = std::fs::metadata(path) {
                    if let Ok(modified) = metadata.modified() {
                        if modified != cached.modified {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

/// Create a response hash for rendered HTML content.
pub fn html_response(body: String, status: i64) -> Value {
    use crate::interpreter::value::{HashKey, HashPairs};
    use ahash::RandomState as AHasher;

    // Inject live reload script if enabled
    let body = if crate::serve::live_reload::is_live_reload_enabled() {
        crate::serve::live_reload::inject_live_reload_script(&body)
    } else {
        body
    };

    // Inject the instant-navigation script (body swap + pushState) unless
    // disabled via `SOLI_NAV=off`. Nav subsumes hover prefetching with its own
    // in-memory cache (a fetch() can't consume `<link rel="prefetch">`
    // entries), so the two scripts are mutually exclusive: prefetch.js is only
    // injected when nav is off, restoring the previous behavior unchanged.
    let nav_on = crate::serve::nav::is_enabled();
    let body = if nav_on {
        crate::serve::nav::inject_nav_tag(&body)
    } else {
        body
    };
    let body = if !nav_on && crate::serve::prefetch::is_enabled() {
        crate::serve::prefetch::inject_prefetch_tag(&body)
    } else {
        body
    };

    // Compute a content-derived ETag so the shipped hover-prefetch feature
    // actually delivers "instant navigation": Chrome reuses the prefetched
    // body on the actual click as long as the server returns 304 on the
    // revalidation (see `Cache-Control: no-cache` below). The hash runs
    // after all script injections so the ETag reflects the exact bytes we
    // send over the wire.
    let etag = etag_for_body(&body);

    let mut headers: HashPairs = HashPairs::with_capacity_and_hasher(3, AHasher::default());
    headers.insert(
        HashKey::String("Content-Type".into()),
        Value::String("text/html; charset=utf-8".into()),
    );
    headers.insert(HashKey::String("ETag".into()), Value::String(etag.into()));
    // `private`: browser may cache, shared caches (CDN, reverse proxy) may not.
    // `no-cache`: cache entry must be revalidated with If-None-Match before
    // reuse — so any prefetched response survives, but a stale one doesn't.
    headers.insert(
        HashKey::String("Cache-Control".into()),
        Value::String("private, no-cache".into()),
    );

    let mut result: HashPairs = HashPairs::with_capacity_and_hasher(3, AHasher::default());
    result.insert(HashKey::String("status".into()), Value::Int(status));
    result.insert(
        HashKey::String("headers".into()),
        Value::Hash(Rc::new(RefCell::new(headers))),
    );
    result.insert(HashKey::String("body".into()), Value::String(body.into()));

    Value::Hash(Rc::new(RefCell::new(result)))
}

/// Compute a deterministic ETag for an HTML response body using FNV-1a 64-bit.
///
/// Deterministic within AND across processes — no random seed — so a prefetch
/// stored by one worker can be revalidated against another worker's render
/// without unnecessary body re-delivery. FNV is not cryptographically strong;
/// that's fine: the ETag is a cache validator, not an auth token. Collision
/// probability between two different bodies is ~2^-32, which means one false
/// 304 per ~4 billion distinct renders — far below anything that matters for
/// navigation caching.
///
/// Format: `W/` weak validator with quoted 16-hex-digit body (RFC 7232 §2.3).
/// Weak (not strong) so the header survives content-encoding transformations
/// applied by CDNs in front of the app — Cloudflare and friends strip strong
/// ETags when they re-encode (Brotli/gzip) because the byte stream the client
/// receives no longer matches what the origin hashed. Weak validators assert
/// semantic equivalence rather than byte-identity, which is exactly what we
/// need: the same render is "the same response" whether compressed or not.
/// Compute a deterministic ETag for an HTML response body using FNV-1a 64-bit.
///
/// Deterministic within AND across processes — no random seed — so a prefetch
/// stored by one worker can be revalidated against another worker's render
/// without unnecessary body re-delivery. FNV is not cryptographically strong;
/// that's fine: the ETag is a cache validator, not an auth token. Collision
/// probability between two different bodies is ~2^-32, which means one false
/// 304 per ~4 billion distinct renders — far below anything that matters for
/// navigation caching.
///
/// Format: `W/` weak validator with quoted 16-hex-digit body (RFC 7232 §2.3).
/// Weak (not strong) so the header survives content-encoding transformations
/// applied by CDNs in front of the app — Cloudflare and friends strip strong
/// ETags when they re-encode (Brotli/gzip) because the byte stream the client
/// receives no longer matches what the origin hashed. Weak validators assert
/// semantic equivalence rather than byte-identity, which is exactly what we
/// need: the same render is "the same response" whether compressed or not.
pub fn etag_for_body(body: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET;
    for &b in body.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("W/\"{:016x}\"", hash)
}

/// Check if a template path is a markdown file.
fn is_markdown_template(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.ends_with(".md")
}

/// Wrap rendered template output in HTML comment markers used by the dev
/// bar's hover overlay. Only emitted when `view_log` is enabled (i.e.
/// dev mode); production HTML is unchanged.
///
/// `kind` is `"view"`, `"partial"`, or `"layout"`. `id` matches the
/// id stored alongside the entry in `view_log`, which the dev bar emits
/// as `data-solidev-view-idx` on the matching sub-row.
fn wrap_dev_marker(kind: &str, id: u32, name: &str, body: &str) -> String {
    // Sanitize the template name for inclusion inside an HTML comment:
    // strip `--` (which would terminate the comment early) and any
    // angle brackets (defensive, names shouldn't contain them).
    let safe_name: String = name
        .chars()
        .filter(|c| *c != '<' && *c != '>')
        .collect::<String>()
        .replace("--", "__");
    let mut out = String::with_capacity(body.len() + safe_name.len() + 80);
    out.push_str("<!--solidev:");
    out.push_str(kind);
    out.push_str(":start id=");
    out.push_str(&id.to_string());
    out.push_str(" name=");
    out.push_str(&safe_name);
    out.push_str("-->");
    out.push_str(body);
    out.push_str("<!--solidev:");
    out.push_str(kind);
    out.push_str(":end id=");
    out.push_str(&id.to_string());
    out.push_str("-->");
    out
}

/// Convert markdown text to HTML using pulldown-cmark.
pub fn markdown_to_html(markdown: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(markdown, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);
    html_output
}

/// One inline span produced by [`markdown_to_spans`]: a run of text plus the
/// inline styles that were active over it. Maps onto a PDF `StyledSpan`.
#[derive(Debug, Clone, PartialEq)]
pub struct MdSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub mono: bool,
    pub link: Option<String>,
}

/// Convert **inline** markdown into a flat list of styled spans, suitable for a
/// PDF paragraph's `spans`. Bold (`**`/`__`) → bold, emphasis (`*`/`_`) →
/// italic, inline code (`` `…` ``) → monospace, and links → a clickable span.
/// Block structure is flattened to one inline flow (blocks joined by a space;
/// soft breaks → space, hard breaks → `\n`); strikethrough renders as plain
/// text (there is no struck face). Adjacent same-style runs are merged.
pub fn markdown_to_spans(markdown: &str) -> Vec<MdSpan> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

    fn push(
        spans: &mut Vec<MdSpan>,
        text: &str,
        bold: bool,
        italic: bool,
        mono: bool,
        link: Option<&str>,
    ) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = spans.last_mut() {
            if last.bold == bold
                && last.italic == italic
                && last.mono == mono
                && last.link.as_deref() == link
            {
                last.text.push_str(text);
                return;
            }
        }
        spans.push(MdSpan {
            text: text.to_string(),
            bold,
            italic,
            mono,
            link: link.map(str::to_string),
        });
    }

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut spans: Vec<MdSpan> = Vec::new();
    let mut strong = 0u32;
    let mut emph = 0u32;
    let mut links: Vec<String> = Vec::new();
    let mut blocks_seen = 0u32;

    for ev in Parser::new_ext(markdown, options) {
        let link = links.last().map(|s| s.as_str());
        match ev {
            Event::Start(Tag::Strong) => strong += 1,
            Event::End(TagEnd::Strong) => strong = strong.saturating_sub(1),
            Event::Start(Tag::Emphasis) => emph += 1,
            Event::End(TagEnd::Emphasis) => emph = emph.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => links.push(dest_url.to_string()),
            Event::End(TagEnd::Link) => {
                links.pop();
            }
            // New top-level block after the first → separate with a space so the
            // inline flow doesn't run words together.
            Event::Start(Tag::Paragraph | Tag::Heading { .. } | Tag::Item) => {
                if blocks_seen > 0 {
                    push(&mut spans, " ", false, false, false, None);
                }
                blocks_seen += 1;
            }
            Event::Text(t) => push(&mut spans, &t, strong > 0, emph > 0, false, link),
            Event::Code(t) => push(&mut spans, &t, strong > 0, emph > 0, true, link),
            Event::SoftBreak => push(&mut spans, " ", strong > 0, emph > 0, false, link),
            Event::HardBreak => push(&mut spans, "\n", strong > 0, emph > 0, false, link),
            _ => {}
        }
    }
    spans
}

/// Convert markdown text to HTML while escaping raw HTML and neutralizing unsafe links.
pub fn markdown_to_safe_html(markdown: &str) -> String {
    use pulldown_cmark::{html, Event, Options, Parser, Tag};
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let events = Parser::new_ext(markdown, options).map(|event| match event {
        Event::Html(html) | Event::InlineHtml(html) => Event::Text(html),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: safe_markdown_url(dest_url),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: safe_markdown_url(dest_url),
            title,
            id,
        }),
        other => other,
    });

    let mut html_output = String::new();
    html::push_html(&mut html_output, events);
    html_output
}

/// Convert markdown to HTML, neutralizing unsafe link/image URLs (`javascript:`,
/// `data:`, protocol-relative, etc.) while preserving raw-HTML passthrough.
///
/// This is the converter used for `.md` view templates. By the time this runs,
/// the ERB pass has already HTML-escaped every `<%= %>` interpolation, so raw
/// `<script>` cannot arrive from interpolated data — but markdown link/image
/// syntax (`[x](javascript:…)`, `![x](…)`) survives ERB escaping and would
/// otherwise render as a live `javascript:` sink. Neutralizing the URL closes
/// that while still letting a developer's own inline HTML in the `.md` file
/// render (which full `markdown_to_safe_html` would escape).
pub fn markdown_to_html_safe_urls(markdown: &str) -> String {
    use pulldown_cmark::{html, Event, Options, Parser, Tag};
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let events = Parser::new_ext(markdown, options).map(|event| match event {
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: safe_markdown_url(dest_url),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: safe_markdown_url(dest_url),
            title,
            id,
        }),
        other => other,
    });

    let mut html_output = String::new();
    html::push_html(&mut html_output, events);
    html_output
}

fn safe_markdown_url(url: pulldown_cmark::CowStr<'_>) -> pulldown_cmark::CowStr<'_> {
    let trimmed = url.trim();
    if is_safe_markdown_url(trimmed) {
        url
    } else {
        pulldown_cmark::CowStr::from("#")
    }
}

fn is_safe_markdown_url(url: &str) -> bool {
    // SEC-022: reject backslash-based protocol-relative bypasses.
    // Browsers normalize `\` to `/` in special schemes, so `\\evil`,
    // `/\evil`, and `\/evil` all parse as `//evil` — a redirect off
    // the current origin. Reject:
    //   1. `//` prefix (the canonical protocol-relative form),
    //   2. `/\` prefix (browser-normalized to `//`),
    //   3. any `\` in the scheme/authority section (before the first
    //      `/?#` separator) — covers `\\evil`, `\/evil`, `\foo`, etc.
    if url.starts_with("//") || url.starts_with("/\\") {
        return false;
    }
    let pre_separator_end = url.find(['/', '?', '#']).unwrap_or(url.len());
    if url[..pre_separator_end].contains('\\') {
        return false;
    }

    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with('/')
        || lower.starts_with('#')
        || lower.starts_with('?')
    {
        return true;
    }

    let first_separator = lower.find(['/', '?', '#']).unwrap_or(lower.len());
    !lower[..first_separator].contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::{HashKey, HashPairs};
    use std::fs;

    #[test]
    fn markdown_to_spans_maps_inline_styles() {
        let spans = markdown_to_spans("a **b** _c_ `d` [e](http://x) ~~f~~");
        // Find by a unique character (plain spans merge with adjacent spaces).
        let find = |t: char| {
            spans
                .iter()
                .find(|s| s.text.contains(t))
                .unwrap_or_else(|| panic!("no span containing {t}"))
        };
        let a = find('a');
        assert!(!a.bold && !a.italic && !a.mono && a.link.is_none());
        assert!(find('b').bold);
        assert!(find('c').italic);
        assert!(find('d').mono);
        assert_eq!(find('e').link.as_deref(), Some("http://x"));
        // strikethrough has no struck face → renders as plain text
        let f = find('f');
        assert!(!f.bold && !f.italic && !f.mono && f.link.is_none());
    }

    #[test]
    fn markdown_to_spans_nested_bold_italic() {
        let spans = markdown_to_spans("**_x_**");
        assert_eq!(spans.len(), 1);
        assert!(spans[0].bold && spans[0].italic);
        assert_eq!(spans[0].text, "x");
    }

    #[test]
    fn test_is_markdown_template() {
        assert!(is_markdown_template(Path::new("app/views/docs/index.md")));
        assert!(is_markdown_template(Path::new(
            "app/views/docs/index.html.md"
        )));
        assert!(!is_markdown_template(Path::new(
            "app/views/docs/index.html.slv"
        )));
        assert!(!is_markdown_template(Path::new("app/views/docs/index.slv")));
        assert!(!is_markdown_template(Path::new("app/views/docs/index.erb")));
    }

    #[test]
    fn test_markdown_to_html_basic() {
        let md = "# Hello World\n\nThis is **bold** and *italic*.";
        let html = markdown_to_html(md);
        assert!(html.contains("<h1>Hello World</h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>italic</em>"));
    }

    #[test]
    fn test_markdown_to_html_list() {
        let md = "- Item 1\n- Item 2\n- Item 3";
        let html = markdown_to_html(md);
        assert!(html.contains("<ul>"));
        assert!(html.contains("<li>Item 1</li>"));
        assert!(html.contains("<li>Item 2</li>"));
        assert!(html.contains("<li>Item 3</li>"));
    }

    #[test]
    fn test_markdown_to_html_table() {
        let md = "| Col A | Col B |\n|-------|-------|\n| 1     | 2     |";
        let html = markdown_to_html(md);
        assert!(html.contains("<table>"));
        assert!(html.contains("<th>Col A</th>"));
        assert!(html.contains("<td>1</td>"));
    }

    #[test]
    fn test_markdown_to_html_strikethrough() {
        let md = "This is ~~deleted~~ text.";
        let html = markdown_to_html(md);
        assert!(html.contains("<del>deleted</del>"));
    }

    #[test]
    fn test_markdown_to_html_code_block() {
        let md = "```rust\nfn main() {}\n```";
        let html = markdown_to_html(md);
        assert!(html.contains("<code"));
        assert!(html.contains("fn main() {}"));
    }

    /// SEC-022: backslash-based protocol-relative URL bypasses.
    /// Each input is expected to be neutralized to `#` in the safe
    /// rendered HTML so the link cannot redirect off-origin. Tests
    /// `markdown_to_safe_html` since that's the variant that runs
    /// `is_safe_markdown_url`.
    ///
    /// Note: pulldown_cmark itself unescapes `\\` to `\` and `\/` to `/`
    /// per CommonMark before the URL reaches our hook, so:
    ///   `\\evil.com` arrives as `\evil.com` (still a leading backslash),
    ///   `\/evil.com` arrives as `/evil.com` (same-origin path — safe).
    /// We test the cases that survive pulldown_cmark's own pass.
    #[test]
    fn markdown_rejects_backslash_protocol_relative() {
        for bad in ["//evil.com", "/\\evil.com", "\\\\evil.com", "\\evil.com"] {
            let md = format!("[click]({})", bad);
            let html = markdown_to_safe_html(&md);
            assert!(
                html.contains("href=\"#\""),
                "expected `{}` to be neutralized to `#`, got: {}",
                bad,
                html
            );
            assert!(
                !html.contains("evil.com"),
                "expected `evil.com` to not appear in href for `{}`, got: {}",
                bad,
                html
            );
        }
    }

    /// SEC-022 negative cases: legitimate same-origin and absolute URLs
    /// must still pass through unchanged in the safe variant.
    #[test]
    fn markdown_accepts_legitimate_urls() {
        let cases = [
            ("[click](/foo)", "/foo"),
            ("[click](/foo/bar)", "/foo/bar"),
            ("[click](http://example.com)", "http://example.com"),
            (
                "[click](https://example.com/path)",
                "https://example.com/path",
            ),
            ("[click](#anchor)", "#anchor"),
            ("[click](?q=x)", "?q=x"),
            ("[click](mailto:a@b.c)", "mailto:a@b.c"),
        ];
        for (md, expected_href) in cases {
            let html = markdown_to_safe_html(md);
            assert!(
                html.contains(&format!("href=\"{}\"", expected_href)),
                "expected href `{}` to survive, got: {}",
                expected_href,
                html
            );
        }
    }

    /// BUG-001: parse errors raised while loading a template must mention
    /// the template's path. Without this, the controller's `render(...)`
    /// call propagates the bare parser error, and the executor's frame
    /// tracking tags it with the controller file:line — sending the user
    /// hunting in the wrong file.
    #[test]
    fn parse_error_mentions_template_path() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        let view_path = views.join("foo.html.slv");
        fs::write(&view_path, "<% for fn in items %><%= fn %><% end %>").unwrap();

        let cache = TemplateCache::new(&views);
        let err = cache
            .render("foo", &Value::Null, Some(None))
            .expect_err("template with reserved keyword loop var must fail to parse");
        assert!(
            err.contains("foo.html.slv"),
            "expected template path in error, got: {}",
            err
        );
        assert!(
            err.contains("'fn'") && err.contains("reserved keyword"),
            "expected keyword diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn rejects_traversal_in_template_name() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        // Plant a sibling file outside the views dir that we'd love an
        // attacker not to be able to render.
        let secret = dir.path().join("secret.html.slv");
        fs::write(&secret, "<h1>secret</h1>").unwrap();
        fs::write(views.join("ok.html.slv"), "<h1>ok</h1>").unwrap();

        let cache = TemplateCache::new(&views);

        // Sanity: the legitimate name still resolves.
        assert!(cache.resolve_template_path("ok").is_ok());

        // None of these should resolve, even though `secret.html.slv` exists.
        for bad in [
            "../secret",
            "..",
            "../../etc/passwd",
            "users/../../secret",
            "/etc/passwd",
            "./secret",
            "",
            "foo\0bar",
            "foo\\..\\secret",
        ] {
            assert!(
                cache.resolve_template_path(bad).is_err(),
                "expected rejection for {:?}",
                bad
            );
        }
    }

    #[test]
    fn rejects_traversal_in_partial_name() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        let secret = dir.path().join("_secret.html.slv");
        fs::write(&secret, "<h1>secret</h1>").unwrap();

        let cache = TemplateCache::new(&views);
        // render_partial turns "../secret" into "../_secret" — the basename
        // gets prefixed with `_`, then resolution must still reject the `..`.
        let err = cache
            .render_partial(
                "../secret",
                &Value::Hash(Rc::new(RefCell::new(Default::default()))),
            )
            .unwrap_err();
        assert!(err.contains("not found"), "got: {}", err);
    }

    #[test]
    fn test_resolve_template_path_md() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        // Create a .html.md file
        fs::write(views.join("page.html.md"), "# Page").unwrap();

        let cache = TemplateCache::new(&views);
        let resolved = cache.resolve_template_path("page").unwrap();
        assert!(resolved.to_string_lossy().ends_with(".html.md"));
    }

    #[test]
    fn test_resolve_template_path_md_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        // Create a .md file (no .html prefix)
        fs::write(views.join("page.md"), "# Page").unwrap();

        let cache = TemplateCache::new(&views);
        let resolved = cache.resolve_template_path("page").unwrap();
        assert!(resolved.to_string_lossy().ends_with(".md"));
    }

    #[test]
    fn test_resolve_template_path_slv_preferred_over_md() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        // Create both .html.slv and .html.md — .slv should win
        fs::write(views.join("page.html.slv"), "<h1>SLV</h1>").unwrap();
        fs::write(views.join("page.html.md"), "# MD").unwrap();

        let cache = TemplateCache::new(&views);
        let resolved = cache.resolve_template_path("page").unwrap();
        assert!(resolved.to_string_lossy().ends_with(".html.slv"));
    }

    #[test]
    fn test_render_md_template() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        fs::write(views.join("test.md"), "# Hello\n\nThis is **bold**.").unwrap();

        let cache = TemplateCache::new(&views);
        let result = cache.render("test", &Value::Null, Some(None)).unwrap();
        assert!(result.contains("<h1>Hello</h1>"));
        assert!(result.contains("<strong>bold</strong>"));
    }

    #[test]
    fn test_render_md_template_with_template_tags() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        fs::write(views.join("greeting.md"), "# Hello <%= name %>").unwrap();

        let mut data = HashPairs::default();
        data.insert(
            HashKey::String("name".into()),
            Value::String("World".into()),
        );
        let data = Value::Hash(Rc::new(RefCell::new(data)));

        let cache = TemplateCache::new(&views);
        let result = cache.render("greeting", &data, Some(None)).unwrap();
        assert!(result.contains("<h1>Hello World</h1>"));
    }

    #[test]
    fn dev_markers_wrap_view_and_partial_when_enabled() {
        // `view_log` gating is thread-local, so flipping it here only
        // affects this test's thread.
        crate::serve::view_log::set_enabled(true);
        crate::serve::view_log::clear();

        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(views.join("things")).unwrap();
        fs::write(
            views.join("things").join("show.html.slv"),
            "BEFORE <%= render 'things/card' %> AFTER",
        )
        .unwrap();
        fs::write(views.join("things").join("_card.html.slv"), "[CARD]").unwrap();

        let cache = TemplateCache::new(&views);
        let out = cache
            .render("things/show", &Value::Null, Some(None))
            .unwrap();

        // The partial's body is wrapped in start/end markers carrying its id.
        assert!(out.contains("<!--solidev:partial:start id="));
        assert!(out.contains(" name=things/card-->[CARD]<!--solidev:partial:end id="));
        // The top-level view is wrapped in view markers, with the partial
        // markers nested inside.
        assert!(out.contains("<!--solidev:view:start id="));
        assert!(out.contains(" name=things/show-->BEFORE "));
        assert!(out.contains(" AFTER<!--solidev:view:end id="));

        crate::serve::view_log::set_enabled(false);
        crate::serve::view_log::clear();
    }

    #[test]
    fn dev_markers_absent_when_disabled() {
        crate::serve::view_log::set_enabled(false);
        crate::serve::view_log::clear();

        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();
        fs::write(views.join("home.html.slv"), "<h1>Hi</h1>").unwrap();

        let cache = TemplateCache::new(&views);
        let out = cache.render("home", &Value::Null, Some(None)).unwrap();

        // Production HTML must stay clean.
        assert!(!out.contains("solidev:"));
        assert_eq!(out, "<h1>Hi</h1>");
    }

    #[test]
    fn content_for_flows_from_view_to_layout() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(views.join("layouts")).unwrap();
        fs::write(
            views.join("layouts").join("application.html.slv"),
            "<head><%= yield \"head\" %></head><body><%= yield %></body>",
        )
        .unwrap();
        fs::write(
            views.join("page.html.slv"),
            "<% content_for \"head\" do %><script src=\"/chart.js\"></script><% end %><h1>Page</h1>",
        )
        .unwrap();

        let cache = TemplateCache::new(&views);
        let out = cache.render("page", &Value::Null, None).unwrap();
        assert_eq!(
            out,
            "<head><script src=\"/chart.js\"></script></head><body><h1>Page</h1></body>"
        );
    }

    #[test]
    fn content_for_in_partial_registers_in_same_store() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(views.join("layouts")).unwrap();
        fs::create_dir_all(views.join("snippets")).unwrap();
        fs::write(
            views.join("layouts").join("application.html.slv"),
            "<head><%= yield \"head\" %></head><%= yield %>",
        )
        .unwrap();
        fs::write(
            views.join("snippets").join("_head_extra.html.slv"),
            "<% content_for \"head\" do %><meta name=\"from-partial\"><% end %>",
        )
        .unwrap();
        fs::write(
            views.join("page.html.slv"),
            "<%= render 'snippets/head_extra' %>Body",
        )
        .unwrap();

        let cache = TemplateCache::new(&views);
        let out = cache.render("page", &Value::Null, None).unwrap();
        assert_eq!(out, "<head><meta name=\"from-partial\"></head>Body");
    }

    #[test]
    fn content_for_does_not_leak_between_renders() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(views.join("layouts")).unwrap();
        fs::write(
            views.join("layouts").join("application.html.slv"),
            "[<%= yield \"head\" %>]<%= yield %>",
        )
        .unwrap();
        fs::write(
            views.join("first.html.slv"),
            "<% content_for \"head\" do %>CAPTURED<% end %>First",
        )
        .unwrap();
        fs::write(views.join("second.html.slv"), "Second").unwrap();

        let cache = TemplateCache::new(&views);
        let first = cache.render("first", &Value::Null, None).unwrap();
        assert_eq!(first, "[CAPTURED]First");
        // The frame guard must have cleared the store: the second render on
        // this same thread starts with an empty slot.
        let second = cache.render("second", &Value::Null, None).unwrap();
        assert_eq!(second, "[]Second");
    }

    #[test]
    fn content_for_predicate_in_layout() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(views.join("layouts")).unwrap();
        fs::write(
            views.join("layouts").join("application.html.slv"),
            "<% if content_for?(\"head\") %><extra><%= yield \"head\" %></extra><% end %><%= yield %>",
        )
        .unwrap();
        fs::write(
            views.join("with.html.slv"),
            "<% content_for \"head\" do %>X<% end %>Body",
        )
        .unwrap();
        fs::write(views.join("without.html.slv"), "Body").unwrap();

        let cache = TemplateCache::new(&views);
        let with = cache.render("with", &Value::Null, None).unwrap();
        assert_eq!(with, "<extra>X</extra>Body");
        let without = cache.render("without", &Value::Null, None).unwrap();
        assert_eq!(without, "Body");
    }

    /// Helper to register a fake mounted engine for template resolution tests.
    fn register_test_engine(name: &str, path: &Path, mounted_at: &str) {
        use crate::serve::engine_loader::{mount_engines, EngineConfig, EngineMount};

        // Create engine manifest so discover_engines finds it
        let engine_dir = path.join("engines").join(name);
        fs::create_dir_all(&engine_dir).unwrap();
        fs::write(
            engine_dir.join("engine.sl"),
            format!(
                "engine \"{}\" {{\n    version: \"1.0.0\",\n    dependencies: []\n}}",
                name
            ),
        )
        .unwrap();

        let config = EngineConfig {
            engines: vec![EngineMount {
                name: name.to_string(),
                mounted_at: mounted_at.to_string(),
            }],
        };
        mount_engines(path, &config).unwrap();
    }

    #[test]
    fn test_resolve_engine_view() {
        // Simulate: project/app/views is the views_dir
        // Engine view at: project/engines/shop/app/views/shop/index.html.slv
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let views = root.join("app/views");
        fs::create_dir_all(&views).unwrap();

        let engine_views = root.join("engines/shop/app/views/shop");
        fs::create_dir_all(&engine_views).unwrap();
        fs::write(engine_views.join("index.html.slv"), "<h1>Shop</h1>").unwrap();

        register_test_engine("shop", root, "/shop");

        let cache = TemplateCache::new(&views);
        // render("shop/index") should find the engine view
        let result = cache.resolve_template_path("shop/index");
        assert!(result.is_ok());
        assert!(result
            .unwrap()
            .to_string_lossy()
            .contains("engines/shop/app/views"));

        crate::serve::engine_loader::reset_engine_context();
    }

    #[test]
    fn test_resolve_main_view_preferred_over_engine() {
        // If a view exists in both main views and engine views, main wins
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let views = root.join("app/views/shop");
        fs::create_dir_all(&views).unwrap();
        fs::write(views.join("index.html.slv"), "<h1>Main Shop</h1>").unwrap();

        let engine_views = root.join("engines/shop/app/views/shop");
        fs::create_dir_all(&engine_views).unwrap();
        fs::write(engine_views.join("index.html.slv"), "<h1>Engine Shop</h1>").unwrap();

        register_test_engine("shop", root, "/shop");

        let cache = TemplateCache::new(root.join("app/views"));
        let result = cache.resolve_template_path("shop/index").unwrap();
        // Main views dir should win
        assert!(result.to_string_lossy().contains("app/views/shop"));
        assert!(!result.to_string_lossy().contains("engines"));

        crate::serve::engine_loader::reset_engine_context();
    }

    #[test]
    fn test_render_md_partial() {
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();

        fs::write(views.join("_note.md"), "**Important:** remember this.").unwrap();

        let cache = TemplateCache::new(&views);
        let result = cache.render_partial("note", &Value::Null).unwrap();
        assert!(result.contains("<strong>Important:</strong>"));
    }

    #[test]
    fn test_render_partial_with_explicit_context_sees_helper() {
        use crate::interpreter::builtins::template::{clear_view_helpers, register_view_helper};
        use crate::interpreter::value::NativeFunction;

        // Isolate this thread's template state.
        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();

        register_view_helper(
            "__spec_partial_shout".to_string(),
            Value::NativeFunction(NativeFunction::new("__spec_partial_shout", None, |args| {
                match args.first() {
                    Some(Value::String(s)) => {
                        Ok(Value::String(format!("{}!", s.to_uppercase()).into()))
                    }
                    _ => Ok(Value::Null),
                }
            })),
        );

        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();
        fs::write(
            views.join("_card.html.slv"),
            "<%= __spec_partial_shout(\"hi\") %>",
        )
        .unwrap();

        // Explicit context is a raw hash with no helper keys — mirrors the
        // real-world `<%= render 'card', user %>` case where the partial
        // used to lose access to user helpers.
        let mut ctx = HashPairs::default();
        ctx.insert(HashKey::String("item_id".into()), Value::Int(42));
        let ctx = Value::Hash(Rc::new(RefCell::new(ctx)));

        let cache = TemplateCache::new(&views);
        let result = cache.render_partial("card", &ctx).unwrap();
        assert_eq!(result.trim(), "HI!");

        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();
    }

    #[test]
    fn test_locals_binding_exposes_partial_context() {
        // The template engine binds `locals` to the partial's hash so
        // reserved words (`class`) and builtin-colliding names (`type`)
        // remain readable from inside the partial. Bare-identifier access
        // stays available for non-reserved keys — both paths must coexist.
        use crate::interpreter::builtins::template::clear_view_helpers;

        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();

        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();
        fs::write(
            views.join("_probe.html.slv"),
            "bare=<%= title %>|\
             locals_title=<%= locals[\"title\"] %>|\
             locals_class=<%= locals[\"class\"] %>|\
             missing_is_nil=<%= locals[\"nope\"].nil? %>",
        )
        .unwrap();

        let mut ctx = HashPairs::default();
        ctx.insert(HashKey::String("title".into()), Value::String("hi".into()));
        ctx.insert(HashKey::String("class".into()), Value::String("red".into()));
        let ctx = Value::Hash(Rc::new(RefCell::new(ctx)));

        let cache = TemplateCache::new(&views);
        let result = cache.render_partial("probe", &ctx).unwrap();
        assert_eq!(
            result.trim(),
            "bare=hi|locals_title=hi|locals_class=red|missing_is_nil=true"
        );

        // `locals` must also exist (as an empty hash) when the partial is
        // rendered without a data hash, so `locals[...]` never blows up.
        fs::write(
            views.join("_no_data.html.slv"),
            "no_data_locals_nil=<%= locals[\"anything\"].nil? %>",
        )
        .unwrap();
        let result = cache.render_partial("no_data", &Value::Null).unwrap();
        assert_eq!(result.trim(), "no_data_locals_nil=true");

        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();
    }

    #[test]
    fn test_html_response_sets_etag_and_cache_headers() {
        // Default HTML responses must carry an ETag + `Cache-Control:
        // private, no-cache`. Without those, the shipped hover-prefetch
        // feature never delivers "instant navigation" — browsers treat an
        // uncached prefetched response as unusable for the real click.
        let resp = html_response("<html><body>hi</body></html>".to_string(), 200);

        let Value::Hash(ref map) = resp else {
            panic!("html_response must return a Hash, got {:?}", resp);
        };
        let Value::Hash(ref hdrs) = map.borrow()[&HashKey::String("headers".into())] else {
            panic!("headers key must be a Hash");
        };
        let hdrs = hdrs.borrow();

        let etag = match &hdrs[&HashKey::String("ETag".into())] {
            Value::String(s) => s.clone(),
            v => panic!("ETag must be a String, got {:?}", v),
        };
        // RFC 7232 weak validator: `W/` followed by a quoted opaque string.
        // Weak (not strong) so the header survives CDN content-encoding
        // transformations — see `etag_for_body` doc comment. Format is
        // `W/"<16 hex>"`, 20 chars total.
        assert_eq!(
            etag.len(),
            20,
            "ETag should be W/\"<16 hex>\", got {}",
            etag
        );
        assert!(
            etag.starts_with("W/\"") && etag.ends_with('"'),
            "ETag must be a weak quoted validator, got {}",
            etag
        );

        let cc = match &hdrs[&HashKey::String("Cache-Control".into())] {
            Value::String(s) => s.clone(),
            v => panic!("Cache-Control must be a String, got {:?}", v),
        };
        assert_eq!(cc.as_str(), "private, no-cache");
    }

    #[test]
    fn test_html_response_etag_is_deterministic_and_content_derived() {
        // Same body ⇒ same ETag. Different bodies ⇒ different ETags.
        // This is the contract that makes 304 revalidation correct.
        let a = html_response("<html><body>X</body></html>".to_string(), 200);
        let b = html_response("<html><body>X</body></html>".to_string(), 200);
        let c = html_response("<html><body>Y</body></html>".to_string(), 200);

        fn etag_of(v: &Value) -> String {
            let Value::Hash(ref m) = v else {
                unreachable!()
            };
            let Value::Hash(ref h) = m.borrow()[&HashKey::String("headers".into())] else {
                unreachable!()
            };
            let Value::String(s) = h.borrow()[&HashKey::String("ETag".into())].clone() else {
                unreachable!()
            };
            s.to_string()
        }

        assert_eq!(etag_of(&a), etag_of(&b), "same body must produce same ETag");
        assert_ne!(etag_of(&a), etag_of(&c), "different body must change ETag");
    }
}
