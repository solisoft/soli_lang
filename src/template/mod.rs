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
pub mod declared_props;
pub mod helpers;
pub mod layout;
pub mod parser;
pub mod renderer;
pub mod response_cache;
pub mod vm_template;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use web_time::Instant;

use crate::interpreter::value::{SoliStr, Value};
use crate::serve::{vfs_exists, vfs_read_to_string};
use parser::parse_template;
use renderer::render_with_interpreter;
use std::rc::Rc;
use vm_template::ViewRender;

/// A cached template's parsed AST. Hot reload clears the cache; nothing
/// re-checks a file's mtime.
#[derive(Debug, Clone)]
struct CachedTemplate {
    nodes: Arc<Vec<parser::TemplateNode>>,
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

/// Maximum nesting depth for partial/component includes. A partial that
/// renders itself (directly or through a cycle) would otherwise recurse
/// until the stack overflows — which aborts the process without unwinding,
/// beyond the reach of the server's `catch_unwind` fault isolation.
const MAX_INCLUDE_DEPTH: usize = 64;

thread_local! {
    /// This thread's resolved includes, by cache, kind and the name as written
    /// (`"card"`, `"posts/row"`). A page of fifty components asked for the same
    /// name fifty times, and each time formatted the on-disk name, took two
    /// locks, hashed a path and re-tested its extension.
    static INCLUDES: RefCell<IncludeCache> = RefCell::new(IncludeCache::default());
}

crate::request_local! {
    static INCLUDE_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Bumped by [`TemplateCache::clear`]: every thread drops its [`INCLUDES`] on
/// its next include, as a hot reload must.
static INCLUDE_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Tells two `TemplateCache`s apart in [`INCLUDES`]: an engine's views and the
/// app's resolve the same name to different files.
static NEXT_CACHE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Default)]
struct IncludeCache {
    generation: u64,
    /// By `(cache id, is a component include)`, then by name: a hit looks the
    /// name up as written, with nothing allocated.
    entries: HashMap<(u64, bool), IncludeNames, ahash::RandomState>,
}

type IncludeNames = HashMap<Box<str>, Rc<IncludeEntry>, ahash::RandomState>;

/// What an include name resolves to, worked out once per thread.
struct IncludeEntry {
    /// The app/views-relative name: `components/card`, `posts/_row`.
    resolved_name: String,
    path: Arc<PathBuf>,
    /// `path` as the renderer's error messages print it.
    path_str: String,
    nodes: Arc<Vec<parser::TemplateNode>>,
    is_component: bool,
    markdown: bool,
    /// The template reads its data hash whole ([`reads_its_data`]), so the
    /// controller's `@ivars` must be entries of it, not only names in scope.
    reads_data: bool,
}

/// Whether a template reads its data hash as a whole rather than name by
/// name: `locals` (or `@locals`), or a default `<%= yield %>`, which writes
/// the data's `content`. An include whose template does gets the
/// controller's `@ivars` as entries of a copy of its data, as every include
/// used to; any other reads them through its scope, with nothing copied.
fn reads_its_data(nodes: &[parser::TemplateNode]) -> bool {
    use crate::ast::expr::{Expr, ExprKind};
    use parser::TemplateNode;

    fn expr_reads(expr: &Expr) -> bool {
        let mut found = false;
        crate::ast::walk::walk_expr(expr, &mut |_| {}, &mut |e| found |= names_locals(e));
        found
    }
    fn names_locals(expr: &Expr) -> bool {
        match &expr.kind {
            ExprKind::Variable(name) => name == "locals",
            ExprKind::Member { object, name } | ExprKind::SafeMember { object, name } => {
                name == "locals" && matches!(object.kind, ExprKind::This)
            }
            _ => false,
        }
    }
    nodes.iter().any(|node| match node {
        TemplateNode::Literal(_) | TemplateNode::Yield(Some(_)) => false,
        TemplateNode::Yield(None) => true,
        TemplateNode::If {
            condition,
            body,
            else_body,
            ..
        } => {
            expr_reads(condition)
                || reads_its_data(body)
                || else_body.as_deref().is_some_and(reads_its_data)
        }
        TemplateNode::For { iterable, body, .. } => expr_reads(iterable) || reads_its_data(body),
        TemplateNode::ContentFor { body, .. } => reads_its_data(body),
        TemplateNode::FormWith { parts, body, .. } => {
            expr_reads(&parts.builder_expr)
                || expr_reads(&parts.open_expr)
                || expr_reads(&parts.close_expr)
                || reads_its_data(body)
        }
        TemplateNode::Component { parts, body, .. } => {
            expr_reads(&parts.name)
                || parts.props.as_ref().is_some_and(expr_reads)
                || reads_its_data(body)
        }
        // Without a context the tag passes the data on, and the partial it
        // renders inherits the `@ivars` itself.
        TemplateNode::Partial { context, .. } => context.as_ref().is_some_and(expr_reads),
        TemplateNode::CoreCodeBlock { stmts, .. } => {
            let mut found = false;
            crate::ast::walk::walk_stmts(stmts, &mut |_| {}, &mut |e| found |= names_locals(e));
            found
        }
        TemplateNode::CoreOutput { expr, .. } => expr_reads(expr),
    })
}

crate::request_local! {
    /// The `TemplateCache` whose render (a view with its layout, a partial or
    /// a component, on either engine) is running on this thread. `partial()`
    /// and `component()` render through it: the tenant's cache cell behind
    /// `get_template_cache` takes a process-wide lock and bumps a shared
    /// refcount on every call, which under concurrent requests cost ~0.6 µs
    /// per include.
    static CURRENT_CACHE: std::cell::Cell<*const TemplateCache> =
        const { std::cell::Cell::new(std::ptr::null()) };
}

/// Sets [`CURRENT_CACHE`] for a render and puts back, on drop, the value it
/// replaced: an include rendered inside a view ends before the view does.
struct CurrentCache(*const TemplateCache);

impl CurrentCache {
    fn set(cache: &TemplateCache) -> Self {
        CurrentCache(CURRENT_CACHE.with(|current| current.replace(cache as *const _)))
    }
}

impl Drop for CurrentCache {
    fn drop(&mut self) {
        CURRENT_CACHE.with(|current| current.set(self.0));
    }
}

/// Run `render` with the cache rendering on this thread, or with the
/// tenant's when nothing is rendering (a controller's `render_partial`).
pub(crate) fn with_rendering_cache<T>(
    render: impl FnOnce(&TemplateCache) -> Result<T, String>,
) -> Result<T, String> {
    let current = CURRENT_CACHE.with(|current| current.get());
    if current.is_null() {
        let cache = crate::interpreter::builtins::template::get_template_cache()?;
        return render(cache.as_ref());
    }
    // SAFETY: set by `CurrentCache::set` from a `&TemplateCache` that outlives
    // the render it is set for, put back when that render returns, and read
    // only on this thread: a call made after the render ended reads the value
    // from before it, null or an enclosing render's.
    render(unsafe { &*current })
}

/// Template cache that stores parsed templates and tracks file changes.
///
/// Shared across worker threads via `Arc<TemplateCache>`; a template parsed by
/// any worker is visible to the others, avoiding per-worker reparsing.
pub struct TemplateCache {
    /// Base directory for views (e.g., app/views)
    views_dir: PathBuf,
    /// This cache's key in the per-thread include cache ([`INCLUDES`]).
    id: u64,
    /// Cached parsed templates (PathBuf -> nodes).
    cache: RwLock<HashMap<PathBuf, CachedTemplate, ahash::RandomState>>,
    /// Cached path resolutions (template_name -> resolved_path).
    /// Arc so cache hits are pointer increments, not heap clones.
    path_cache: RwLock<HashMap<String, Arc<PathBuf>, ahash::RandomState>>,
    /// Negative path resolutions (template_name -> the "not found" error), so
    /// a name that resolves to nothing — an optional per-controller layout,
    /// say — stops costing a filesystem probe per extension on every render.
    /// Production only: under `--dev` a view created while the server runs
    /// must be found on the next request, so misses are never remembered.
    /// Bounded like `path_cache`; emptied by [`TemplateCache::clear`].
    missing_cache: RwLock<HashMap<String, String, ahash::RandomState>>,
}

/// Which kind of nested include is being rendered — selects the path-resolution
/// rule (partials get a `_` prefix; components resolve under `components/`
/// verbatim) and the dev-bar label / span kind.
#[derive(Clone, Copy)]
enum IncludeKind {
    Partial,
    Component,
}

impl IncludeKind {
    /// The app/views-relative name an include of this kind resolves `name` to.
    fn resolve_name(self, name: &str) -> String {
        match self {
            IncludeKind::Component => {
                // A `/`- or `.`-bearing name is app/views-relative and used
                // verbatim; a bare name resolves under components/.
                if name.contains('/') || name.contains('.') {
                    name.to_string()
                } else {
                    format!("components/{}", name)
                }
            }
            IncludeKind::Partial => {
                // Partials start with `_`, except components/ paths (kept clean
                // so the block form's `components/<name>` resolves like the
                // function-call `component()` helper).
                if name.starts_with("components/") || name.contains("/components/") {
                    name.to_string()
                } else if let Some((dir, file)) = name.rsplit_once('/') {
                    // e.g., "users/card" -> "users/_card"
                    format!("{}/_{}", dir, file)
                } else {
                    format!("_{}", name)
                }
            }
        }
    }
}

impl TemplateCache {
    /// Create a new template cache for the given views directory.
    pub fn new(views_dir: impl Into<PathBuf>) -> Self {
        Self {
            views_dir: views_dir.into(),
            id: NEXT_CACHE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            // ahash: every render and include looks a path or a name up here,
            // and the default SipHash of a path was ~5% of a page of components.
            cache: RwLock::new(HashMap::default()),
            path_cache: RwLock::new(HashMap::default()),
            missing_cache: RwLock::new(HashMap::default()),
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
        let layout_name: Option<&str> = match layout {
            Some(Some(name)) => Some(name),
            _ => None,
        };
        let template_path = self.resolve_template_path(template_name)?;

        let Some(data_sig) = Self::cache_signature(&template_path, layout_name, data) else {
            return self.render_uncached(template_name, data, layout, &template_path, layout_name);
        };
        if let Some(cached) =
            response_cache::get(template_path.clone(), layout_name, data_sig, false)
        {
            return Ok(cached.body.to_string());
        }
        let body =
            self.render_uncached(template_name, data, layout, &template_path, layout_name)?;
        // Store the freshly-rendered body. On the next identical
        // request the cache lookup above short-circuits to this body.
        response_cache::put(
            template_path,
            layout_name,
            data_sig,
            false,
            SoliStr::from(body.as_str()),
            SoliStr::new(),
        );
        Ok(body)
    }

    /// `render` followed by [`html_response`], for a controller's
    /// `render(...)`. In production the finished response — scripts injected,
    /// ETag computed — is what gets cached, so a hit neither re-renders nor
    /// re-scans or re-hashes the page, and hands out the body without copying
    /// it. The `render` builtin keeps dev mode on `render` + `html_response`.
    pub fn render_html_response(
        &self,
        template_name: &str,
        data: &Value,
        layout: Option<Option<&str>>,
        status: i64,
    ) -> Result<Value, String> {
        let layout_name: Option<&str> = match layout {
            Some(Some(name)) => Some(name),
            _ => None,
        };
        let template_path = self.resolve_template_path(template_name)?;

        let Some(data_sig) = Self::cache_signature(&template_path, layout_name, data) else {
            let body =
                self.render_uncached(template_name, data, layout, &template_path, layout_name)?;
            return Ok(html_response(body, status));
        };
        if let Some(cached) =
            response_cache::get(template_path.clone(), layout_name, data_sig, true)
        {
            return Ok(html_response_parts(cached.body, cached.etag, status));
        }
        let body =
            self.render_uncached(template_name, data, layout, &template_path, layout_name)?;
        let (body, etag) = finish_html(body);
        let (body, etag) = (SoliStr::from(body), SoliStr::from(etag));
        response_cache::put(
            template_path,
            layout_name,
            data_sig,
            true,
            body.clone(),
            etag.clone(),
        );
        Ok(html_response_parts(body, etag, status))
    }

    /// The response-cache signature of this render, or `None` when the site
    /// must not be cached.
    ///
    /// Render sites already known to be uncacheable skip the cache machinery
    /// entirely. `data_signature` is a full recursive walk that hashes every
    /// byte of every string in `data`, so on a list page it is proportional to
    /// the whole result set — and for a layout containing `csrf_meta_tag()`
    /// the result is discarded every time. See
    /// `response_cache::is_known_uncacheable`. A request with a session skips
    /// it too: the layout can read the session (`current_user`, `signed_in?`),
    /// which the data signature cannot see, so one person's page would be
    /// served to the next. And so does data the signature cannot stand for —
    /// a model record, a query not yet run: see
    /// `response_cache::data_signature`.
    fn cache_signature(
        template_path: &Arc<PathBuf>,
        layout_name: Option<&str>,
        data: &Value,
    ) -> Option<u64> {
        if response_cache::request_has_session()
            || response_cache::is_known_uncacheable(template_path, layout_name)
        {
            return None;
        }
        response_cache::data_signature(data)
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

        // The cache a `partial()` or `component()` inside renders through.
        let _current_cache = CurrentCache::set(self);

        // Per-template span for the dev-bar flamegraph + flat per-template
        // duration log. Both early-out when --dev is off.
        let mut _span = crate::serve::span_log::SpanGuard::start(
            template_name,
            crate::serve::span_log::SpanKind::View,
        );
        let view_start = crate::serve::view_log::is_enabled().then(web_time::Instant::now);
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

        // One interpreter for the view and the layout. It goes back to the
        // pool when this render returns — the layout still reads it until then.
        // A view has the controller's `@ivars` in its data already.
        let mut interpreter = core_eval::template_interpreter(data, None);

        // Render the view: compiled to the VM in production when it can be
        // (`vm_template`), else on the tree-walker with the shared interpreter.
        // What the compiled view assigned goes into that interpreter, where the
        // layout reads it.
        let content_before = vm_template::check_enabled().then(content_store::snapshot);
        let content = match vm_template::render_view(
            &nodes,
            data,
            None,
            &interpreter.environment,
            self,
            template_path,
        ) {
            ViewRender::Rendered(rendered) if vm_template::check_enabled() => {
                // The tree-walker renders the view again: its `content_for`
                // blocks must not be captured a second time.
                content_store::restore(content_before.flatten());
                // Diagnostic (`SOLI_VM_VIEWS_CHECK=1`): render on the
                // tree-walker too, report any difference, serve its output.
                let tree = render_with_interpreter(
                    &mut interpreter,
                    &nodes,
                    data,
                    Some(&partial_renderer),
                    Some(&template_path.to_string_lossy()),
                )?;
                vm_template::report_difference(
                    &template_path.to_string_lossy(),
                    &rendered.html,
                    &tree,
                );
                tree
            }
            ViewRender::Rendered(rendered) => {
                for (name, value) in rendered.assigned {
                    core_eval::define_var(&mut interpreter, &name, value);
                }
                rendered.html
            }
            ViewRender::TreeWalker(vm_failure) => {
                let html = render_with_interpreter(
                    &mut interpreter,
                    &nodes,
                    data,
                    Some(&partial_renderer),
                    Some(&template_path.to_string_lossy()),
                )?;
                // It rendered where the VM failed: it keeps the tree-walker.
                if let Some(vm_failure) = vm_failure {
                    vm_failure.tree_walker_rendered();
                }
                html
            }
        };

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

    /// Render a partial template (no layout). Partials live at `_name` (or
    /// `dir/_name`), *except* `components/` paths which stay clean.
    pub fn render_partial(&self, name: &str, data: &Value) -> Result<String, String> {
        self.render_include(name, data, IncludeKind::Partial)
    }

    /// Render a component template (no layout). Components are looked up under
    /// `components/` (or an explicit `/`- or `.`-bearing path) and do NOT get a
    /// leading `_` prefix (unlike partials).
    pub fn render_component(&self, name: &str, data: &Value) -> Result<String, String> {
        self.render_include(name, data, IncludeKind::Component)
    }

    /// Shared implementation behind [`render_partial`] and [`render_component`].
    /// The two differ only in on-disk name resolution and the dev-bar label /
    /// span kind; everything else (content_for frame join, markdown conversion,
    /// view-log + metrics) is identical.
    fn render_include(
        &self,
        name: &str,
        data: &Value,
        kind: IncludeKind,
    ) -> Result<String, String> {
        // Guard against include cycles (`_self.slv` rendering itself) — stack
        // overflows abort rather than unwind, so containment can't save us.
        let depth = INCLUDE_DEPTH.with(|d| {
            d.set(d.get() + 1);
            d.get()
        });
        if depth > MAX_INCLUDE_DEPTH {
            INCLUDE_DEPTH.with(|d| d.set(depth - 1));
            return Err(format!(
                "partials/components nested too deeply (max depth {MAX_INCLUDE_DEPTH}) at \"{name}\""
            ));
        }
        let result = self.render_include_inner(name, data, kind);
        INCLUDE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        result
    }

    fn render_include_inner(
        &self,
        name: &str,
        data: &Value,
        kind: IncludeKind,
    ) -> Result<String, String> {
        let entry = self.include_entry(name, kind)?;
        let is_component = entry.is_component;
        let (span_kind, marker) = if is_component {
            (crate::serve::span_log::SpanKind::Component, "component")
        } else {
            (crate::serve::span_log::SpanKind::Partial, "partial")
        };

        // Per-include span + view-log entry, matching `render` above.
        let mut _span = crate::serve::span_log::SpanGuard::start(name, span_kind);
        let view_start = crate::serve::view_log::is_enabled().then(web_time::Instant::now);
        let view_id = view_start.map(|_| crate::serve::view_log::next_id());
        if let Some(id) = view_id {
            _span.set_render_id(id);
        }

        // Timing for production Prometheus metrics (Phase A), gated on SOLI_METRICS.
        let render_start = crate::metrics::metrics_enabled().then(Instant::now);

        // Join the surrounding render's content_for store so captures inside
        // the include reach the layout. A standalone render (no view in
        // progress) owns a throwaway frame instead — captures are dropped.
        let _content_frame = content_store::ensure_frame();

        // The cache a `partial()` or `component()` inside renders through.
        let _current_cache = CurrentCache::set(self);

        let template_path = &*entry.path;
        let nodes = &entry.nodes;

        // A component with a class (`app/components/card_component.sl` defining
        // `CardComponent`) renders with an instance bound as `this`.
        let component_data;
        let data = match is_component
            .then(|| instantiate_component_class(&entry.resolved_name, data))
            .transpose()?
            .flatten()
        {
            Some(with_instance) => {
                component_data = with_instance;
                &component_data
            }
            None => data,
        };

        // The current controller's `@ivars` reach the include (Rails-style),
        // as they reach the view: explicit locals win, framework fields
        // (req/params/session/headers) stay out, and outside a class
        // controller's request (function handlers, tests) there are none. They
        // used to be copied into the include's data, which was the caller's own
        // hash, at ~55 ns a field per include; now a name the data lacks reads
        // the controller's field, on both engines. A template that reads its
        // data whole (`locals`, a default `yield`) still gets them as entries,
        // of a copy.
        let with_fields;
        let mut inherited = None;
        let data = match data {
            Value::Hash(locals) => {
                match crate::interpreter::builtins::template::current_controller_instance() {
                    Some(controller) if entry.reads_data => {
                        let mut pairs = locals.borrow().clone();
                        crate::interpreter::builtins::template::inherit_controller_fields(
                            &mut pairs,
                            &controller.borrow(),
                        );
                        with_fields = Value::Hash(Rc::new(RefCell::new(pairs)));
                        &with_fields
                    }
                    controller => {
                        inherited = controller;
                        data
                    }
                }
            }
            // Nothing was ever copied into a context that is not a hash.
            _ => data,
        };

        // In --dev, record the props a component declares (`props(...)`) so we can
        // warn about any it wasn't given. Component includes only; a stack frame
        // keeps nested components isolated. Zero cost in production.
        let props_frame = (is_component && crate::serve::template_warnings::is_enabled())
            .then(crate::template::declared_props::push_frame);

        let partial_renderer =
            |n: &str, ctx: &Value| -> Result<String, String> { self.render_partial(n, ctx) };

        // On the VM like a view (`vm_template`), but only inside a view's
        // render: there an undefined name reads nil on both engines, while a
        // partial rendered on its own (a controller's `render_partial`) raises
        // for it on the tree-walker.
        let mut interpreter = core_eval::template_interpreter(data, inherited.clone());
        let mut vm_failure = None;
        let on_vm = if crate::interpreter::executor::template_lenient_vars_enabled() {
            let content_before = vm_template::check_enabled().then(content_store::snapshot);
            match vm_template::render_view(
                nodes,
                data,
                inherited.as_ref(),
                &interpreter.environment,
                self,
                template_path,
            ) {
                ViewRender::Rendered(rendered) if vm_template::check_enabled() => {
                    content_store::restore(content_before.flatten());
                    let tree = render_with_interpreter(
                        &mut interpreter,
                        nodes,
                        data,
                        Some(&partial_renderer),
                        Some(&entry.path_str),
                    )?;
                    vm_template::report_difference(&entry.path_str, &rendered.html, &tree);
                    Some(tree)
                }
                ViewRender::Rendered(rendered) => Some(rendered.html),
                ViewRender::TreeWalker(failure) => {
                    vm_failure = failure;
                    None
                }
            }
        } else {
            vm_template::note_interpreted("view", template_path, self, "rendered outside a view");
            None
        };
        let content = match on_vm {
            Some(html) => html,
            None => {
                let html = render_with_interpreter(
                    &mut interpreter,
                    nodes,
                    data,
                    Some(&partial_renderer),
                    Some(&entry.path_str),
                )?;
                // It rendered where the VM failed: it keeps the tree-walker.
                if let Some(vm_failure) = vm_failure {
                    vm_failure.tree_walker_rendered();
                }
                html
            }
        };

        // Diff declared props against the data the component received; a declared
        // prop absent from the data (including inherited @ivars) is likely a bug.
        if props_frame.is_some() {
            let declared = crate::template::declared_props::current();
            if !declared.is_empty() {
                let mut present: std::collections::HashSet<String> = match data {
                    Value::Hash(h) => h
                        .borrow()
                        .iter()
                        .filter_map(|(k, _)| match k {
                            crate::interpreter::value::HashKey::String(s) => Some(s.to_string()),
                            _ => None,
                        })
                        .collect(),
                    _ => std::collections::HashSet::new(),
                };
                if let Some(controller) = &inherited {
                    present.extend(
                        controller
                            .borrow()
                            .fields
                            .keys()
                            .filter(|name| {
                                crate::interpreter::environment::is_inherited_field(name)
                            })
                            .map(|name| name.to_string()),
                    );
                }
                let mut missing: Vec<&String> = declared.difference(&present).collect();
                missing.sort();
                for prop in missing {
                    let msg = format!("component \"{}\": missing prop \"{}\"", name, prop);
                    eprintln!("[soli] {}", msg);
                    crate::serve::template_warnings::record(msg);
                }
            }
        }

        // If the include is a markdown file, convert to HTML. URL-neutralizing
        // converter — the ERB pass already escaped interpolated data (see the
        // matching note in `render`).
        let result = if entry.markdown {
            markdown_to_html_safe_urls(&content)
        } else {
            content
        };
        let result = if let Some(id) = view_id {
            wrap_dev_marker(marker, id, name, &result)
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

    /// Resolve an include once per thread: name, path, parsed nodes and the
    /// facts about them every render of it asks again. Failures are not kept,
    /// so a missing partial is looked for afresh, as before.
    fn include_entry(&self, name: &str, kind: IncludeKind) -> Result<Rc<IncludeEntry>, String> {
        let generation = INCLUDE_GENERATION.load(std::sync::atomic::Ordering::Acquire);
        let is_component_kind = matches!(kind, IncludeKind::Component);
        let found = INCLUDES.with(|includes| {
            let mut includes = includes.borrow_mut();
            if includes.generation != generation {
                includes.entries.clear();
                includes.generation = generation;
            }
            includes
                .entries
                .get(&(self.id, is_component_kind))
                .and_then(|names| names.get(name))
                .cloned()
        });
        if let Some(entry) = found {
            return Ok(entry);
        }

        let resolved_name = kind.resolve_name(name);
        // Anything resolving under components/ is a component for the dev bar,
        // regardless of which syntax reached us (the block form arrives here as
        // a Partial include with a `components/<name>` path).
        let is_component = is_component_kind
            || resolved_name.starts_with("components/")
            || resolved_name.contains("/components/");
        let path = self.resolve_template_path(&resolved_name)?;
        let nodes = self.get_or_load_template(&path)?;
        let entry = Rc::new(IncludeEntry {
            markdown: is_markdown_template(&path),
            path_str: path.to_string_lossy().into_owned(),
            reads_data: reads_its_data(&nodes),
            resolved_name,
            path,
            nodes,
            is_component,
        });
        INCLUDES.with(|includes| {
            let mut includes = includes.borrow_mut();
            if includes.generation == generation {
                let names = includes
                    .entries
                    .entry((self.id, is_component_kind))
                    .or_default();
                if names.len() >= PATH_CACHE_MAX_SIZE {
                    names.clear();
                }
                names.insert(name.into(), entry.clone());
            }
        });
        Ok(entry)
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
        let view_start = crate::serve::view_log::is_enabled().then(web_time::Instant::now);
        let view_id = view_start.map(|_| crate::serve::view_log::next_id());
        if let Some(id) = view_id {
            _span.set_render_id(id);
        }

        let result = match self.resolve_template_path(layout_key) {
            Ok(layout_path) => {
                let layout_nodes = self.get_or_load_template(&layout_path)?;
                vm_template::note_interpreted(
                    "layout",
                    &layout_path,
                    self,
                    "layouts are not compiled",
                );
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

    /// Whether `name` resolves to a template file.
    pub fn template_exists(&self, name: &str) -> bool {
        self.resolve_template_path(name).is_ok()
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

        let remember_misses = !crate::interpreter::builtins::template::is_dev_mode();
        if remember_misses {
            if let Some(err) = self
                .missing_cache
                .read()
                .ok()
                .and_then(|c| c.get(name).cloned())
            {
                return Err(err);
            }
        }

        // Cache miss - do file system lookup
        let resolved = match self.do_resolve_template_path(name) {
            Ok(path) => Arc::new(path),
            Err(err) => {
                if remember_misses {
                    if let Ok(mut missing) = self.missing_cache.write() {
                        if missing.len() >= PATH_CACHE_MAX_SIZE {
                            missing.clear();
                        }
                        missing.insert(name.to_string(), err.clone());
                    }
                }
                return Err(err);
            }
        };

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
                },
            );
        }

        Ok(nodes)
    }

    /// Clear the template cache (useful for hot reload).
    pub fn clear(&self) {
        INCLUDE_GENERATION.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if let Ok(mut c) = self.cache.write() {
            c.clear();
        }
        if let Ok(mut c) = self.path_cache.write() {
            c.clear();
        }
        if let Ok(mut c) = self.missing_cache.write() {
            c.clear();
        }
        vm_template::reset_thread();
    }
}

/// Create a response hash for rendered HTML content.
pub fn html_response(body: String, status: i64) -> Value {
    let (body, etag) = finish_html(body);
    html_response_parts(SoliStr::from(body), SoliStr::from(etag), status)
}

/// The bytes an HTML page goes out as, and their ETag: the rendered body with
/// every script the server injects. A pure function of the body and of
/// process-wide settings, which is what lets a finished response be cached.
fn finish_html(body: String) -> (String, String) {
    // Inject live reload script if enabled
    let body = if crate::serve::live_reload::is_live_reload_enabled() {
        crate::serve::live_reload::inject_live_reload_script(&body)
    } else {
        body
    };

    // The instant-navigation script (body swap + pushState) on every page, or
    // hover prefetching with `SOLI_NAV=off`; then the native bridge, camera,
    // motion sensors and proof of work, each only for the pages whose markup
    // asks for it. One forward pass over the page answers what each script
    // asks, one backward pass finds where the tags go: see `serve::inject`.
    let body = crate::serve::inject::add_framework_scripts(
        body,
        crate::serve::inject::Navigation::from_env(),
    );

    // Compute a content-derived ETag so the shipped hover-prefetch feature
    // actually delivers "instant navigation": Chrome reuses the prefetched
    // body on the actual click as long as the server returns 304 on the
    // revalidation (see `Cache-Control: no-cache` below). The hash runs
    // after all script injections so the ETag reflects the exact bytes we
    // send over the wire.
    let etag = etag_for_body(&body);
    (body, etag)
}

/// The response hash for a finished HTML body and its ETag.
fn html_response_parts(body: SoliStr, etag: SoliStr, status: i64) -> Value {
    use crate::interpreter::value::{HashKey, HashPairs};

    let mut headers: HashPairs = HashPairs::with_capacity(3);
    headers.insert(
        HashKey::String("Content-Type".into()),
        Value::String("text/html; charset=utf-8".into()),
    );
    headers.insert(HashKey::String("ETag".into()), Value::String(etag));
    // `private`: browser may cache, shared caches (CDN, reverse proxy) may not.
    // `no-cache`: cache entry must be revalidated with If-None-Match before
    // reuse — so any prefetched response survives, but a stale one doesn't.
    headers.insert(
        HashKey::String("Cache-Control".into()),
        Value::String("private, no-cache".into()),
    );

    let mut result: HashPairs = HashPairs::with_capacity(3);
    result.insert(HashKey::String("status".into()), Value::Int(status));
    result.insert(
        HashKey::String("headers".into()),
        Value::Hash(Rc::new(RefCell::new(headers))),
    );
    result.insert(HashKey::String("body".into()), Value::String(body));

    Value::Hash(Rc::new(RefCell::new(result)))
}

/// Compute a deterministic ETag for an HTML response body.
///
/// Deterministic within AND across processes — no random seed, no dependence
/// on CPU features — so a prefetch stored by one worker (or host) can be
/// revalidated against another's render without unnecessary body re-delivery.
/// The hash is not cryptographically strong; that's fine: the ETag is a cache
/// validator, not an auth token. It consumes the body eight bytes at a time
/// (one multiply + xor-shift per word, both bijections of the state, so two
/// bodies that differ in a single word never collide) and finishes with the
/// MurmurHash3 64-bit avalanche. Collision probability between two different
/// bodies is on the order of 2^-64 — far below anything that matters for
/// navigation caching. The previous byte-at-a-time FNV-1a cost one multiply
/// per byte, which dominated for large pages.
///
/// Format: `W/` weak validator with quoted 16-hex-digit body (RFC 7232 §2.3).
/// Weak (not strong) so the header survives content-encoding transformations
/// applied by CDNs in front of the app — Cloudflare and friends strip strong
/// ETags when they re-encode (Brotli/gzip) because the byte stream the client
/// receives no longer matches what the origin hashed. Weak validators assert
/// semantic equivalence rather than byte-identity, which is exactly what we
/// need: the same render is "the same response" whether compressed or not.
pub fn etag_for_body(body: &str) -> String {
    format!("W/\"{:016x}\"", body_hash_64(body.as_bytes()))
}

/// Word-at-a-time 64-bit hash behind [`etag_for_body`]. Seed and constants are
/// fixed so the value is stable across processes and machines.
fn body_hash_64(bytes: &[u8]) -> u64 {
    const SEED: u64 = 0xcbf2_9ce4_8422_2325;
    const MUL: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut hash = SEED ^ (bytes.len() as u64).wrapping_mul(MUL);
    let (chunks, tail) = bytes.as_chunks::<8>();
    for chunk in chunks {
        hash = (hash ^ u64::from_le_bytes(*chunk)).wrapping_mul(MUL);
        hash ^= hash >> 29;
    }
    if !tail.is_empty() {
        let mut buf = [0u8; 8];
        buf[..tail.len()].copy_from_slice(tail);
        hash = (hash ^ u64::from_le_bytes(buf)).wrapping_mul(MUL);
        hash ^= hash >> 29;
    }
    // MurmurHash3 fmix64 finalizer.
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xff51_afd7_ed55_8ccd);
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    hash ^= hash >> 33;
    hash
}

/// `components/stat_block` → `StatBlockComponent`: the class a component
/// template may be paired with.
fn component_class_name(resolved_name: &str) -> String {
    let base = resolved_name.rsplit('/').next().unwrap_or(resolved_name);
    let base = base.split('.').next().unwrap_or(base);
    let mut class = String::new();
    for word in base.split(['_', '-']).filter(|w| !w.is_empty()) {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            class.extend(first.to_uppercase());
            class.push_str(chars.as_str());
        }
    }
    class.push_str("Component");
    class
}

/// If `app/components/` defines the class for this component, build an
/// instance — the props become its fields — run its `before_render` hook, and
/// return the props with the instance added as `this`. `None` means there is no
/// class and the template renders from the props alone, as it always did.
///
/// The props hash is copied, not mutated: the caller may reuse it.
fn instantiate_component_class(resolved_name: &str, data: &Value) -> Result<Option<Value>, String> {
    use crate::interpreter::value::{HashKey, Instance};

    let Some(Value::Class(class)) = crate::interpreter::builtins::template::get_view_helper(
        &component_class_name(resolved_name),
    ) else {
        return Ok(None);
    };
    let Value::Hash(props) = data else {
        return Ok(None);
    };

    let mut copy = props.borrow().clone();
    let mut instance = Instance::new(class.clone());
    // A declared field the caller did not pass reads as nil, not as an error.
    let mut declaring = Some(class.clone());
    while let Some(current) = declaring {
        for field in current.fields.keys() {
            instance.set(field.to_string(), Value::Null);
        }
        declaring = current.superclass.clone();
    }
    for (key, value) in copy.iter() {
        if let HashKey::String(name) = key {
            instance.set(name.to_string(), value.clone());
        }
    }
    let instance = Rc::new(RefCell::new(instance));

    if let Some(hook) = class.find_method("before_render") {
        let bound = crate::interpreter::executor::access::member::bind_user_method_to_receiver(
            Value::Instance(instance.clone()),
            hook,
        );
        // The hook runs in the class's own closure environment; a bare
        // interpreter over an empty one avoids registering every builtin again
        // on each render of the component.
        crate::interpreter::executor::Interpreter::with_environment(Rc::new(RefCell::new(
            crate::interpreter::environment::Environment::new(),
        )))
        .call_value(bound, Vec::new(), crate::span::Span::default())
        .map_err(|e| format!("{}#before_render: {}", class.name, e))?;
    }

    copy.insert(HashKey::String("this".into()), Value::Instance(instance));
    Ok(Some(Value::Hash(Rc::new(RefCell::new(copy)))))
}

/// Check if a template path is a markdown file.
fn is_markdown_template(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.ends_with(".md")
}

crate::request_local! {
    /// Whether dev-bar marker comments are suppressed for the current render.
    /// See [`without_dev_markers`].
    static SUPPRESS_DEV_MARKERS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Restores the flag on the way out, so a `?` or a panic can't leave marker
/// suppression stuck on for the rest of the worker's life.
struct SuppressMarkersGuard(bool);

impl Drop for SuppressMarkersGuard {
    fn drop(&mut self) {
        SUPPRESS_DEV_MARKERS.with(|s| s.set(self.0));
    }
}

/// Render inside `f` without the dev bar's `<!--solidev:…-->` marker comments.
/// For output that leaves the process — mailer bodies — where the markers would
/// ride along into a real email. Nesting is safe; production is unaffected
/// (markers are dev-only to begin with).
pub fn without_dev_markers<T>(f: impl FnOnce() -> T) -> T {
    let _guard = SuppressMarkersGuard(SUPPRESS_DEV_MARKERS.with(|s| s.replace(true)));
    f()
}

/// Wrap rendered template output in HTML comment markers used by the dev
/// bar's hover overlay. Only emitted when `view_log` is enabled (i.e.
/// dev mode); production HTML is unchanged.
///
/// `kind` is `"view"`, `"partial"`, or `"layout"`. `id` matches the
/// id stored alongside the entry in `view_log`, which the dev bar emits
/// as `data-solidev-view-idx` on the matching sub-row.
fn wrap_dev_marker(kind: &str, id: u32, name: &str, body: &str) -> String {
    // Some rendered output leaves the process — mailer bodies above all. Dev
    // instrumentation has no business in an email, so a suppressed render emits
    // the body verbatim (view_log still records the timing either way).
    if SUPPRESS_DEV_MARKERS.with(std::cell::Cell::get) {
        return body.to_string();
    }
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

/// Render markdown as **plain text** a person can read and act on.
///
/// The third face of one source: `markdown_to_html` for the readers that
/// draw, this for the readers that do not. The rule throughout is that
/// nothing a reader would need may be dropped — which `strip_html` on the
/// HTML does drop: it loses a list's bullets and, worse, every link's
/// address, leaving the words of a link and no way to reach it.
///
/// So: headings keep their text, list items keep a marker (`- ` and `1. `,
/// indented two spaces per level), a link becomes `text <url>` unless the
/// text already is the url, an image becomes its alt text, a quote keeps
/// its `> `, a code fence keeps its lines verbatim, a table's cells are
/// joined by ` | `, and a rule is `---`. Emphasis loses its markers,
/// because this is the face for someone who wants no markers.
pub fn markdown_to_text(markdown: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut out = String::new();
    // The line being built, and the prefix every line of the current block
    // opens with (quote markers and list indentation).
    let mut line = String::new();
    let mut prefix: Vec<String> = Vec::new();
    // One entry per open list: `None` for a bullet list, `Some(n)` for an
    // ordered one, counting.
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut links: Vec<String> = Vec::new();
    let mut link_text = String::new();
    let mut in_link = false;
    let mut in_code = false;
    let mut cells: Vec<String> = Vec::new();
    let mut in_table = false;

    fn flush(out: &mut String, line: &mut String, prefix: &[String]) {
        let said = line.trim_end();
        if !said.is_empty() {
            out.push_str(&prefix.concat());
            out.push_str(said);
        }
        out.push('\n');
        line.clear();
    }

    for ev in Parser::new_ext(markdown, options) {
        match ev {
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                flush(&mut out, &mut line, &prefix);
                out.push('\n');
            }
            Event::Start(Tag::Heading { .. }) => {}
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut out, &mut line, &prefix);
                out.push('\n');
            }
            Event::Start(Tag::List(first)) => {
                lists.push(first);
                if lists.len() > 1 {
                    prefix.push("  ".to_string());
                }
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                if !lists.is_empty() {
                    prefix.pop();
                }
                if lists.is_empty() {
                    out.push('\n');
                }
            }
            Event::Start(Tag::Item) => {
                let mark = match lists.last_mut() {
                    Some(Some(n)) => {
                        let said = format!("{n}. ");
                        *n += 1;
                        said
                    }
                    _ => "- ".to_string(),
                };
                line.push_str(&mark);
            }
            Event::End(TagEnd::Item) => flush(&mut out, &mut line, &prefix),
            Event::Start(Tag::BlockQuote(_)) => prefix.push("> ".to_string()),
            Event::End(TagEnd::BlockQuote(_)) => {
                prefix.pop();
            }
            Event::Start(Tag::CodeBlock(_)) => in_code = true,
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                flush(&mut out, &mut line, &prefix);
                out.push('\n');
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                links.push(dest_url.to_string());
                link_text.clear();
                in_link = true;
            }
            Event::End(TagEnd::Link) => {
                in_link = false;
                let url = links.pop().unwrap_or_default();
                // `text <url>`, unless the text is the url already — which is
                // what an autolink is, and `<https://…> <https://…>` helps
                // nobody.
                if link_text.trim() == url.trim() || url.is_empty() {
                    line.push_str(&link_text);
                } else {
                    line.push_str(&link_text);
                    line.push_str(" <");
                    line.push_str(&url);
                    line.push('>');
                }
                link_text.clear();
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                links.push(dest_url.to_string());
                link_text.clear();
                in_link = true;
            }
            Event::End(TagEnd::Image) => {
                in_link = false;
                let url = links.pop().unwrap_or_default();
                let alt = if link_text.trim().is_empty() {
                    "image".to_string()
                } else {
                    link_text.clone()
                };
                line.push_str(&format!("[{alt}: {url}]"));
                link_text.clear();
            }
            Event::Start(Tag::Table(_)) => in_table = true,
            Event::End(TagEnd::Table) => {
                in_table = false;
                out.push('\n');
            }
            Event::Start(Tag::TableCell) => {}
            Event::End(TagEnd::TableCell) => {
                cells.push(std::mem::take(&mut line).trim().to_string());
            }
            Event::End(TagEnd::TableRow) | Event::End(TagEnd::TableHead) => {
                line = cells.join(" | ");
                cells.clear();
                flush(&mut out, &mut line, &prefix);
            }
            Event::Rule => {
                flush(&mut out, &mut line, &prefix);
                out.push_str("---\n\n");
            }
            Event::Text(t) | Event::Code(t) => {
                if in_link {
                    link_text.push_str(&t);
                } else if in_code {
                    // A fence keeps its own newlines.
                    for (i, said) in t.split('\n').enumerate() {
                        if i > 0 {
                            flush(&mut out, &mut line, &prefix);
                        }
                        line.push_str(said);
                    }
                } else {
                    line.push_str(&t);
                }
            }
            Event::SoftBreak => {
                if in_link {
                    link_text.push(' ');
                } else if in_table {
                    line.push(' ');
                } else {
                    flush(&mut out, &mut line, &prefix);
                }
            }
            Event::HardBreak => flush(&mut out, &mut line, &prefix),
            _ => {}
        }
    }
    flush(&mut out, &mut line, &prefix);
    // At most one blank line anywhere, and none at either end.
    let mut tidy = String::new();
    let mut blanks = 0;
    for said in out.lines() {
        if said.trim().is_empty() {
            blanks += 1;
            if blanks > 1 || tidy.is_empty() {
                continue;
            }
            tidy.push('\n');
        } else {
            blanks = 0;
            tidy.push_str(said);
            tidy.push('\n');
        }
    }
    tidy.trim_end().to_string()
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

/// Neutralize a Markdown link/image URL that a browser would treat as a
/// script or off-origin sink. `pub(crate)` so the file-mode Markdown renderer
/// (`serve::files::markdown`), which builds its own event stream to highlight
/// fenced Soli code, applies the same SEC-022 policy as the view pipeline.
pub(crate) fn safe_markdown_url(url: pulldown_cmark::CowStr<'_>) -> pulldown_cmark::CowStr<'_> {
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
    fn markdown_to_text_keeps_what_a_reader_needs() {
        let md = "# Le point\n\nBonjour **Ana**,\n\n- un premier point\n- un second, avec [un lien](https://example.com)\n\n> du texte cité\n\nÀ bientôt.";
        let said = markdown_to_text(md);
        assert!(said.contains("Le point"), "{said}");
        assert!(
            said.contains("Bonjour Ana,"),
            "emphasis loses its markers: {said}"
        );
        assert!(
            said.contains("- un premier point"),
            "a list keeps its bullets: {said}"
        );
        // The one `strip_html` drops, and the one that matters most.
        assert!(
            said.contains("un lien <https://example.com>"),
            "a link keeps its address: {said}"
        );
        assert!(
            said.contains("> du texte cité"),
            "a quote stays quoted: {said}"
        );
        assert!(!said.contains("**"), "no markers left: {said}");
    }

    #[test]
    fn markdown_to_text_numbers_an_ordered_list_and_leaves_no_double_blanks() {
        let said = markdown_to_text("1. un\n2. deux\n3. trois\n\n\n\nfin");
        assert!(said.contains("1. un"), "{said}");
        assert!(said.contains("3. trois"), "{said}");
        assert!(!said.contains("\n\n\n"), "at most one blank line: {said:?}");
        assert!(!said.ends_with('\n'), "no trailing blank: {said:?}");
    }

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

    /// An include is resolved once per thread; a hot reload (`clear`) must
    /// still reach it.
    #[test]
    fn a_cached_include_follows_a_reload() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let views = dir.path().join("views");
        fs::create_dir_all(&views)?;
        let cache = TemplateCache::new(&views);
        let data = Value::Hash(Rc::new(RefCell::new(Default::default())));

        fs::write(views.join("_row.html.slv"), "first")?;
        assert_eq!(cache.render_partial("row", &data)?, "first");

        fs::write(views.join("_row.html.slv"), "second")?;
        assert_eq!(cache.render_partial("row", &data)?, "first");
        cache.clear();
        assert_eq!(cache.render_partial("row", &data)?, "second");

        // Another cache over other views resolves the same name to its own file.
        let other_views = dir.path().join("other");
        fs::create_dir_all(&other_views)?;
        fs::write(other_views.join("_row.html.slv"), "other")?;
        let other = TemplateCache::new(&other_views);
        assert_eq!(other.render_partial("row", &data)?, "other");
        assert_eq!(cache.render_partial("row", &data)?, "second");
        Ok(())
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
        let _engine_guard = crate::serve::engine_loader::lock_engine_context_for_test();
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
        // Component-wise, not a `/`-joined substring: the separator is `\` on Windows.
        assert!(result.unwrap().starts_with(&engine_views));

        crate::serve::engine_loader::reset_engine_context();
    }

    #[test]
    fn test_resolve_main_view_preferred_over_engine() {
        let _engine_guard = crate::serve::engine_loader::lock_engine_context_for_test();
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
        // Main views dir should win. Component-wise: the separator is `\` on Windows.
        assert!(result.starts_with(&views));
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
    fn component_class_names_follow_the_template_name() {
        assert_eq!(component_class_name("components/badge"), "BadgeComponent");
        assert_eq!(
            component_class_name("components/stat_block"),
            "StatBlockComponent"
        );
        assert_eq!(
            component_class_name("components/cards/stat"),
            "StatComponent"
        );
        assert_eq!(
            component_class_name("components/user-card"),
            "UserCardComponent"
        );
        assert_eq!(component_class_name("shared/nav.html"), "NavComponent");
    }

    #[test]
    fn a_component_without_a_class_is_left_alone() {
        let data = Value::Hash(Rc::new(RefCell::new(
            crate::interpreter::value::HashPairs::default(),
        )));
        assert!(matches!(
            instantiate_component_class("components/no_such_thing", &data),
            Ok(None)
        ));
    }

    #[test]
    fn component_renders_end_to_end_both_forms() {
        // Exercises the *real* cache (not a mock renderer) so a component file
        // on disk, prop passing, and the default slot all work together.
        use crate::interpreter::builtins::template::clear_view_helpers;
        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();

        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        let components = views.join("components");
        fs::create_dir_all(&components).unwrap();
        // A component reads a prop (title) and the default slot (yield).
        fs::write(
            components.join("card.html.slv"),
            "<div class=\"card\"><h3><%= title %></h3><%= yield %></div>",
        )
        .unwrap();

        let cache = TemplateCache::new(&views);

        // Block form: named-arg props + a body captured into the default slot.
        fs::write(
            views.join("index.html.slv"),
            "<%- component \"card\", title: \"Stats\" do %>body<% end %>",
        )
        .unwrap();
        let block = cache.render("index", &Value::Null, Some(None)).unwrap();
        assert_eq!(block.trim(), "<div class=\"card\"><h3>Stats</h3>body</div>");

        // Function form: props hash + an explicit `content` key for the slot.
        let mut data = HashPairs::default();
        data.insert(
            HashKey::String("title".into()),
            Value::String("Stats".into()),
        );
        data.insert(
            HashKey::String("content".into()),
            Value::String("body".into()),
        );
        let data = Value::Hash(Rc::new(RefCell::new(data)));
        let func = cache.render_component("card", &data).unwrap();
        assert_eq!(func.trim(), "<div class=\"card\"><h3>Stats</h3>body</div>");

        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();
    }

    #[test]
    fn number_delimiter_and_ternary_render_in_template() {
        // Verifies number_with_delimiter() is registered as a view helper AND
        // that a ternary evaluates inside <%= %> (both go through the core
        // expression parser in the real render pipeline).
        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        fs::create_dir_all(&views).unwrap();
        fs::write(
            views.join("p.html.slv"),
            "n=<%= number_with_delimiter(1234567) %>|t=<%= 1 == 1 ? \"yes\" : \"no\" %>",
        )
        .unwrap();
        let cache = TemplateCache::new(&views);
        let out = cache.render("p", &Value::Null, Some(None)).unwrap();
        assert_eq!(out.trim(), "n=1,234,567|t=yes");
    }

    #[test]
    fn component_slot_builder_named_slots() {
        // The slot-builder `do |c| … c.slot("x") do … end` desugars to
        // content_for and feeds `yield "x"` — same result as the content_for
        // form, and folds correctly from a raw `<%- … do %>` tag.
        use crate::interpreter::builtins::template::clear_view_helpers;
        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();

        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        let components = views.join("components");
        fs::create_dir_all(&components).unwrap();
        fs::write(
            components.join("panel.html.slv"),
            "<header><%= yield \"header\" %></header><main><%= yield %></main>",
        )
        .unwrap();
        fs::write(
            views.join("index.html.slv"),
            "<%- component \"panel\" do |c| %><%- c.slot(\"header\") do %>HEAD<% end %>BODY<%- end %>",
        )
        .unwrap();

        let cache = TemplateCache::new(&views);
        let out = cache.render("index", &Value::Null, Some(None)).unwrap();
        assert_eq!(out.trim(), "<header>HEAD</header><main>BODY</main>");

        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();
    }

    #[test]
    fn component_named_slot_via_content_for() {
        // A `content_for "x"` inside the component block feeds `yield "x"` in
        // the component template (named slots); the remaining body feeds the
        // default `yield`.
        use crate::interpreter::builtins::template::clear_view_helpers;
        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();

        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        let components = views.join("components");
        fs::create_dir_all(&components).unwrap();
        fs::write(
            components.join("panel.html.slv"),
            "<header><%= yield \"header\" %></header><main><%= yield %></main>",
        )
        .unwrap();
        fs::write(
            views.join("index.html.slv"),
            "<%- component \"panel\" do %><% content_for \"header\" do %>HEAD<% end %>BODY<%- end %>",
        )
        .unwrap();

        let cache = TemplateCache::new(&views);
        let out = cache.render("index", &Value::Null, Some(None)).unwrap();
        assert_eq!(out.trim(), "<header>HEAD</header><main>BODY</main>");

        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();
    }

    #[test]
    fn component_missing_prop_records_warning() {
        use crate::interpreter::builtins::template::clear_view_helpers;
        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();
        crate::serve::template_warnings::clear();
        crate::serve::template_warnings::set_enabled(true);

        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        let components = views.join("components");
        fs::create_dir_all(&components).unwrap();
        // Declares two props; the render is only given `title`.
        fs::write(
            components.join("card.html.slv"),
            "<% props(\"title\", \"value\") %><h3><%= title %></h3>",
        )
        .unwrap();

        let cache = TemplateCache::new(&views);
        let mut data = HashPairs::default();
        data.insert(HashKey::String("title".into()), Value::String("Hi".into()));
        let data = Value::Hash(Rc::new(RefCell::new(data)));
        let _ = cache.render_component("card", &data).unwrap();

        let warnings = crate::serve::template_warnings::snapshot();
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("missing prop \"value\"")),
            "expected a missing-prop warning, got {:?}",
            warnings
        );
        // `title` was provided, so it must not warn.
        assert!(!warnings.iter().any(|w| w.contains("\"title\"")));

        crate::serve::template_warnings::set_enabled(false);
        crate::serve::template_warnings::clear();
        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();
    }

    #[test]
    fn instance_vars_propagate_into_partials_and_components() {
        // A controller's @ivars reach partials AND components (Rails-style),
        // resolving through the same `@foo -> foo local` lenient fallback the
        // main view uses. Explicit locals still win (covered elsewhere).
        use crate::interpreter::builtins::controller::registry::{
            clear_current_controller, set_current_controller,
        };
        use crate::interpreter::builtins::template::clear_view_helpers;
        use crate::interpreter::value::{Class, Instance};

        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();

        // Fake controller exposing @greeting = "Bonjour".
        let class = Rc::new(Class {
            name: "GreetController".to_string(),
            ..Default::default()
        });
        let mut inst = Instance::new(class);
        inst.fields
            .insert("greeting".into(), Value::String("Bonjour".into()));
        set_current_controller(Value::Instance(Rc::new(RefCell::new(inst))));

        let dir = tempfile::tempdir().unwrap();
        let views = dir.path().join("views");
        let components = views.join("components");
        fs::create_dir_all(&components).unwrap();
        // Both read the inherited @greeting with no explicit local passed.
        fs::write(views.join("_hello.html.slv"), "partial:<%= @greeting %>").unwrap();
        fs::write(
            components.join("hello.html.slv"),
            "component:<%= @greeting %>",
        )
        .unwrap();

        let cache = TemplateCache::new(&views);
        let empty = || Value::Hash(Rc::new(RefCell::new(HashPairs::default())));
        // `@greeting` resolves through the view's lenient-vars fallback.
        let _lenient = crate::interpreter::executor::enter_template_lenient_vars();
        let partial_out = cache.render_partial("hello", &empty()).unwrap();
        let component_out = cache.render_component("hello", &empty()).unwrap();
        drop(_lenient);
        assert_eq!(partial_out, "partial:Bonjour");
        assert_eq!(component_out, "component:Bonjour");

        clear_current_controller();
        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();
    }

    /// The controller of a test's request: `fields` as its `@ivars`, until
    /// the guard drops.
    struct CurrentController;

    impl CurrentController {
        fn set(fields: &[(&str, Value)]) -> Self {
            use crate::interpreter::value::{Class, Instance};
            let class = Rc::new(Class {
                name: "RowsController".to_string(),
                ..Default::default()
            });
            let mut instance = Instance::new(class);
            for (name, value) in fields {
                instance.fields.insert((*name).into(), value.clone());
            }
            crate::interpreter::builtins::controller::registry::set_current_controller(
                Value::Instance(Rc::new(RefCell::new(instance))),
            );
            CurrentController
        }
    }

    impl Drop for CurrentController {
        fn drop(&mut self) {
            crate::interpreter::builtins::controller::registry::clear_current_controller();
        }
    }

    fn string(text: &str) -> Value {
        Value::String(text.into())
    }

    fn hash_of(pairs: &[(&str, Value)]) -> Value {
        let mut map = HashPairs::default();
        for (key, value) in pairs {
            map.insert(HashKey::String((*key).into()), value.clone());
        }
        Value::Hash(Rc::new(RefCell::new(map)))
    }

    /// A view helper `shout(x)`: `X!`.
    fn register_shout_helper() {
        use crate::interpreter::builtins::template::register_view_helper;
        use crate::interpreter::value::NativeFunction;
        register_view_helper(
            "shout".to_string(),
            Value::NativeFunction(NativeFunction::new("shout", Some(1), |args| {
                Ok(Value::String(format!("{}!", args[0]).to_uppercase().into()))
            })),
        );
    }

    /// A partial or component reads the controller's `@ivars`, after its own
    /// locals and before the helpers, on the tree-walker and on the VM, and
    /// nothing is written into the hash the caller passed. `partial()` used to
    /// copy every view helper and every controller field into that hash, at
    /// ~60 ns each per include; a page printed `opts.length` as 4 here.
    #[test]
    fn an_include_reads_the_controller_ivars_without_copying_them(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use crate::interpreter::builtins::template::clear_view_helpers;
        clear_view_helpers();
        register_shout_helper();
        crate::template::core_eval::reset_builtins_rc();
        let _controller = CurrentController::set(&[
            ("title", string("Posts")),
            ("post", string("from @post")),
            ("params", string("the request's")),
            ("_note", string("private")),
            ("content", string("from @content")),
        ]);

        let dir = tempfile::tempdir()?;
        let views = dir.path().join("views");
        fs::create_dir_all(views.join("rows"))?;
        fs::create_dir_all(views.join("components"))?;
        let row = "[<%= post %>|<%= @title %>|<%= title %>|<%= shout(\"x\") %>|<%= params.nil? %>|<%= _note %>]";
        fs::write(views.join("rows").join("_row.html.slv"), row)?;
        // The same row in a loop: worth compiling, so it renders on the VM.
        fs::write(
            views.join("rows").join("_looped.html.slv"),
            format!("<% for i in [1] %>{row}<% end %>"),
        )?;
        // Reads its data whole: the `@ivars` are entries of a copy of it.
        fs::write(
            views.join("rows").join("_whole.html.slv"),
            "{<%= locals[\"title\"] %>|<%= locals[\"shout\"].nil? %>|<%= locals.length %>}",
        )?;
        fs::write(
            views.join("components").join("badge.html.slv"),
            "(<%= label %> <%= @title %>)",
        )?;
        fs::write(
            views.join("components").join("panel.html.slv"),
            "<p><%= yield %></p>",
        )?;
        fs::write(
            views.join("page.html.slv"),
            "<% for i in [1] %>\
             <%- partial(\"rows/row\", opts) %>\
             <%- partial(\"rows/looped\", opts) %>\
             <%= render \"rows/row\", opts %>\
             <%- partial(\"rows/whole\", opts) %>\
             <%- component(\"badge\", {\"label\": \"b\"}) %>\
             <%- component(\"panel\", {}) %>\
             <% end %>|<%= opts.length %>",
        )?;

        let cache = TemplateCache::new(&views);
        let row_out = "[passed|Posts|Posts|X!|true|private]";
        let expected =
            format!("{row_out}{row_out}{row_out}{{Posts|true|4}}(b Posts)<p>from @content</p>|1");
        // A different `n` each time: the response cache never answers.
        for n in 0..2 {
            let data = hash_of(&[
                ("opts", hash_of(&[("post", string("passed"))])),
                ("n", Value::Int(n)),
            ]);
            assert_eq!(cache.render("page", &data, Some(None))?, expected);
        }

        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();
        Ok(())
    }

    /// An app helper named like a framework function (once-campfire defines
    /// `button_to`) is the app's in a view, in `partial()` and in a
    /// `<%= render %>` partial, compiled or not. The render tag used to call
    /// the form builder's, a 500 for a helper taking other arguments.
    #[test]
    fn a_helper_named_like_a_framework_function_is_the_apps_everywhere(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use crate::interpreter::builtins::template::{clear_view_helpers, register_view_helper};
        use crate::interpreter::value::NativeFunction;
        clear_view_helpers();
        register_view_helper(
            "button_to".to_string(),
            Value::NativeFunction(NativeFunction::new("button_to", None, |_| {
                Ok(Value::String("USER-button_to".into()))
            })),
        );
        crate::template::core_eval::reset_builtins_rc();

        let dir = tempfile::tempdir()?;
        let views = dir.path().join("views");
        fs::create_dir_all(&views)?;
        fs::write(
            views.join("_btn.html.slv"),
            "<i><%= button_to(\"/x\", {}, \"y\") %></i>",
        )?;
        let body = "view=<%= button_to(\"/x\", {}, \"y\") %>|\
                    partial=<%- partial(\"btn\", {}) %>|\
                    tag=<%= render \"btn\", {\"a\": 1} %>";
        fs::write(views.join("plain.html.slv"), body)?;
        fs::write(
            views.join("looped.html.slv"),
            format!("<% for i in [1] %>{body}<% end %>"),
        )?;

        let cache = TemplateCache::new(&views);
        let expected =
            "view=USER-button_to|partial=<i>USER-button_to</i>|tag=<i>USER-button_to</i>";
        for (page, n) in [("plain", 1), ("looped", 2)] {
            let data = hash_of(&[("n", Value::Int(n))]);
            assert_eq!(cache.render(page, &data, Some(None))?, expected, "{page}");
        }

        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();
        Ok(())
    }

    /// The other framework names a helper can take: a route helper
    /// (`about_path`, from a named route) and a form-builder function other
    /// than `button_to` (`csrf_field`). Each is the app's in a view, in
    /// `partial()` and in a `<%= render %>` partial, tree-walked or compiled.
    /// The render tag used to call the framework's, with no error: the page
    /// showed `/about` and a CSRF input where the app's helper should have run.
    #[test]
    fn a_helper_named_like_a_route_helper_is_the_apps_everywhere(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use crate::interpreter::builtins::named_routes::rebuild_named_routes;
        use crate::interpreter::builtins::server::{clear_routes, restore_routes, Route};
        use crate::interpreter::builtins::template::{clear_view_helpers, load_view_helpers};

        // `get("/about", "pages#about", name: "about")`, on this thread.
        let routes = vec![Route {
            method: "GET".to_string(),
            path_pattern: "/about".to_string(),
            handler_name: "pages#about".to_string(),
            name: Some("about".to_string()),
            middleware: vec![],
            middleware_names: vec![],
            openapi: None,
        }];
        rebuild_named_routes(&routes);
        restore_routes(routes);

        let dir = tempfile::tempdir()?;
        let views = dir.path().join("views");
        let helpers = dir.path().join("helpers");
        fs::create_dir_all(&views)?;
        fs::create_dir_all(&helpers)?;
        fs::write(
            helpers.join("collide_helper.sl"),
            "def about_path\n  \"USER-about_path\"\nend\n\n\
             def csrf_field\n  \"USER-csrf_field\"\nend\n",
        )?;
        let calls = "<%= about_path() %>,<%- csrf_field() %>";
        fs::write(views.join("_rc.html.slv"), format!("<i>{calls}</i>"))?;
        fs::write(
            views.join("_rc_looped.html.slv"),
            format!("<% for j in [1] %><b>{calls}</b><% end %>"),
        )?;
        let body = format!(
            "view={calls}|\
             partial=<%- partial(\"rc\", {{}}) %><%- partial(\"rc_looped\", {{}}) %>|\
             tag=<%= render \"rc\", {{\"a\": 1}} %><%= render \"rc_looped\", {{\"a\": 1}} %>"
        );
        fs::write(views.join("plain.html.slv"), &body)?;
        fs::write(
            views.join("looped.html.slv"),
            format!("<% for i in [1] %>{body}<% end %>"),
        )?;
        fs::write(views.join("framework.html.slv"), calls)?;
        let cache = TemplateCache::new(&views);

        // Without the app's helpers both names are the framework's.
        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();
        let framework = cache.render("framework", &hash_of(&[]), Some(None))?;
        assert!(
            framework.starts_with("/about,<input type=\"hidden\" name=\"_csrf_token\""),
            "{framework}"
        );

        load_view_helpers(&helpers)?;
        crate::template::core_eval::reset_builtins_rc();
        let own = "USER-about_path,USER-csrf_field";
        let expected =
            format!("view={own}|partial=<i>{own}</i><b>{own}</b>|tag=<i>{own}</i><b>{own}</b>");
        for (page, n) in [("plain", 1), ("looped", 2)] {
            let data = hash_of(&[("n", Value::Int(n))]);
            assert_eq!(cache.render(page, &data, Some(None))?, expected, "{page}");
        }

        clear_view_helpers();
        clear_routes();
        rebuild_named_routes(&[]);
        crate::template::core_eval::reset_builtins_rc();
        Ok(())
    }

    /// `html` with each form's signed render time (`_soli_ts`) cut out: two
    /// renders a second apart differ there.
    fn without_render_time(html: &str) -> String {
        const MARK: &str = "name=\"_soli_ts\" value=\"";
        let mut kept = String::with_capacity(html.len());
        let mut rest = html;
        while let Some(at) = rest.find(MARK) {
            let value = at + MARK.len();
            kept.push_str(&rest[..value]);
            rest = &rest[value..];
            rest = &rest[rest.find('"').unwrap_or(rest.len())..];
        }
        kept.push_str(rest);
        kept
    }

    /// A form's `f.open()` calls the builder's own `csrf_field` past an app
    /// helper of that name, on both engines, while the page's own
    /// `csrf_field()` is the app's. This thread has no worker globals, as a
    /// background job rendering a mailer view has none: the view VM's globals
    /// are then the template env's names, with the app's helpers last, and
    /// the `f.open()` JIT-compiled there called the helper.
    #[test]
    fn a_form_calls_the_builders_csrf_field_past_a_helper_of_that_name(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use crate::interpreter::builtins::template::{clear_view_helpers, load_view_helpers};

        let dir = tempfile::tempdir()?;
        let views = dir.path().join("views");
        let helpers = dir.path().join("helpers");
        fs::create_dir_all(&views)?;
        fs::create_dir_all(&helpers)?;
        fs::write(
            helpers.join("csrf_helper.sl"),
            "def csrf_field\n  \"USER-csrf_field\"\nend\n",
        )?;
        let body = "[<%- csrf_field() %>]\
                    <%- form_with(nil, {\"url\": \"/x\"}) do |f| -%><%- f.label(\"title\") %><%- end -%>";
        let looped = format!("<% for i in [1] %>{body}<% end %>");
        // Compiled: its `f.open()` runs on the VM. The plain page does not.
        let compiles = |source: &str| -> Result<bool, String> {
            Ok(
                vm_template::VmTemplate::compile_for_serve(&parser::parse_template(source)?)
                    .is_ok(),
            )
        };
        assert!(compiles(&looped)? && !compiles(body)?);
        fs::write(views.join("plain.html.slv"), body)?;
        fs::write(views.join("looped.html.slv"), looped)?;

        clear_view_helpers();
        load_view_helpers(&helpers)?;
        crate::template::core_eval::reset_builtins_rc();

        let cache = TemplateCache::new(&views);
        let mut pages = Vec::new();
        for (page, n) in [("plain", 1), ("looped", 2)] {
            // Each render as its own request: the honeypot ids restart.
            crate::interpreter::builtins::spam::reset_honeypot_ids();
            let html = cache.render(page, &hash_of(&[("n", Value::Int(n))]), Some(None))?;
            assert!(
                html.starts_with("[USER-csrf_field]<form "),
                "{page}: {html}"
            );
            assert_eq!(html.matches("USER-csrf_field").count(), 1, "{page}: {html}");
            assert!(html.contains("name=\"_csrf_token\""), "{page}: {html}");
            pages.push(without_render_time(&html));
        }
        assert_eq!(pages[0], pages[1]);

        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();
        Ok(())
    }

    /// An app helper calls the global `partial()`, which gives the partial
    /// `req`, `params` and `cookies` in a copy of the hash: the view's own
    /// `opts` keeps its keys, on the interpreter and on the VM. It used to
    /// gain `req` and `cookies`, so a `json_script(opts)` after the call
    /// printed the request's cookies into the page.
    #[test]
    fn a_helpers_partial_leaves_the_callers_hash_alone() -> Result<(), Box<dyn std::error::Error>> {
        use crate::interpreter::builtins::template::{
            clear_current_request, clear_view_helpers, load_view_helpers, set_current_request,
        };
        let dir = tempfile::tempdir()?;
        let views = dir.path().join("views");
        let helpers = dir.path().join("helpers");
        fs::create_dir_all(&views)?;
        fs::create_dir_all(&helpers)?;
        // Loaded as `soli serve` loads them: closed over the global builtins.
        fs::write(
            helpers.join("cards_helper.sl"),
            "def card(opts)\n  partial(\"card\", opts)\nend\n",
        )?;
        fs::write(
            views.join("_card.html.slv"),
            "{<%= x %> <%= req[\"query\"][\"q\"] %>}",
        )?;
        let keys = "|<%= opts.keys.join(\",\") %>";
        fs::write(
            views.join("plain.html.slv"),
            format!("<% opts = {{\"x\": 1}} %><%- card(opts) %>{keys}"),
        )?;
        fs::write(
            views.join("looped.html.slv"),
            format!("<% opts = {{\"x\": 1}} %><% for i in [1] %><%- card(opts) %><% end %>{keys}"),
        )?;

        clear_view_helpers();
        load_view_helpers(&helpers)?;
        crate::template::core_eval::reset_builtins_rc();
        set_current_request(hash_of(&[
            ("query", hash_of(&[("q", string("a"))])),
            ("cookies", hash_of(&[("sid", string("s3cret"))])),
        ]));

        let cache = TemplateCache::new(&views);
        for (page, n) in [("plain", 1), ("looped", 2)] {
            let data = hash_of(&[("n", Value::Int(n))]);
            assert_eq!(cache.render(page, &data, Some(None))?, "{1 a}|x", "{page}");
        }

        clear_current_request();
        clear_view_helpers();
        crate::template::core_eval::reset_builtins_rc();
        Ok(())
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

    /// A fragment (an htmx swap, a `layout: false` render) has no `</body>`:
    /// its scripts go at the end, in the order the injectors always used.
    #[test]
    fn a_fragment_gets_its_scripts_at_the_end_in_order() -> Result<(), String> {
        let fragment = "<div data-soli-camera></div><input data-soli-pow=\"x\">";
        let response = html_response(fragment.to_string(), 200);
        let Value::Hash(map) = &response else {
            panic!("html_response must return a Hash, got {response:?}");
        };
        let body = match map.borrow().get(&HashKey::String("body".into())) {
            Some(Value::String(body)) => body.to_string(),
            other => panic!("the response has no body: {other:?}"),
        };
        assert!(body.starts_with(fragment), "{body}");
        assert!(body.ends_with("</script>"), "{body}");
        let camera = body.find("/__soli/camera.js").ok_or("no camera.js")?;
        let pow = body.find("/__soli/pow.js").ok_or("no pow.js")?;
        assert!(camera < pow, "{body}");
        // nav.js, or prefetch.js with `SOLI_NAV=off`, whichever this test
        // run's environment selects, goes first.
        for navigation in ["/__soli/nav.js", "/__soli/prefetch.js"] {
            if let Some(at) = body.find(navigation) {
                assert!(at < camera, "{body}");
            }
        }
        Ok(())
    }

    /// A page rendered from an object is rendered again for the next object.
    /// The response cache keyed it on a signature that hashed an object by the
    /// length of its class name, so `Post.all` after an edit (here, the next
    /// card) got the first page back. This held on both cached paths: `render`
    /// (dev mode, auto-render, mailers) and `render_html_response`
    /// (production `render()`).
    #[test]
    fn render_data_holding_an_object_is_rendered_every_time(
    ) -> Result<(), Box<dyn std::error::Error>> {
        use crate::interpreter::value::{Class, Instance};

        response_cache::reset_for_new_request();
        response_cache::clear_cache();
        let dir = tempfile::tempdir()?;
        let views = dir.path().join("views");
        fs::create_dir_all(views.join("cards"))?;
        fs::write(
            views.join("cards/show.html.slv"),
            "<p><%= card.title %></p>",
        )?;
        let cache = TemplateCache::new(&views);

        let data = |card: Value| {
            let mut pairs = HashPairs::default();
            pairs.insert(HashKey::String("card".into()), card);
            Value::Hash(Rc::new(RefCell::new(pairs)))
        };
        let class = Rc::new(Class {
            name: "Card".to_string(),
            ..Default::default()
        });
        let card = |title: &str| {
            let mut instance = Instance::new(class.clone());
            instance.set("title", Value::String(title.into()));
            data(Value::Instance(Rc::new(RefCell::new(instance))))
        };

        let render = |data: &Value| cache.render("cards/show", data, Some(None));
        assert!(render(&card("first"))?.contains("first"));
        let second = render(&card("second"))?;
        assert!(
            second.contains("second"),
            "render() answered with the first card's page: {second}"
        );

        let response_body = |data: &Value| -> Result<String, String> {
            let response = cache.render_html_response("cards/show", data, Some(None), 200)?;
            let Value::Hash(map) = &response else {
                panic!("a response is a hash, got {response:?}");
            };
            let body = map.borrow().get(&HashKey::String("body".into())).cloned();
            match body {
                Some(Value::String(body)) => Ok(body.to_string()),
                other => Err(format!("the response has no body: {other:?}")),
            }
        };
        assert!(response_body(&card("third"))?.contains("third"));
        let fourth = response_body(&card("fourth"))?;
        assert!(
            fourth.contains("fourth"),
            "render_html_response answered with the third card's page: {fourth}"
        );

        // The refusal belongs to those renders, not to the site: the same view
        // rendered from plain values is still kept and found.
        let path = cache.resolve_template_path("cards/show")?;
        assert!(!response_cache::is_known_uncacheable(&path, None));
        let mut fields = HashPairs::default();
        fields.insert(
            HashKey::String("title".into()),
            Value::String("plain".into()),
        );
        let plain = data(Value::Hash(Rc::new(RefCell::new(fields))));
        let signature = response_cache::data_signature(&plain).ok_or("plain data has one")?;
        assert!(render(&plain)?.contains("plain"));
        assert!(
            response_cache::get(path, None, signature, false).is_some(),
            "a page rendered from plain values must still be cached"
        );
        Ok(())
    }
}
