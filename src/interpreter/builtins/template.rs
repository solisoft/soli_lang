//! Template rendering builtins for Soli MVC.
//!
//! Provides the `render()` function for use in controllers to render templates.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use std::path::Path;

use crate::ast::stmt::StmtKind;
use crate::interpreter::builtins::datetime::helpers as datetime_helpers;
use crate::interpreter::builtins::html;
use crate::interpreter::builtins::i18n::helpers as i18n_helpers;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{
    value_to_json, Function, HashKey, HashPairs, NativeFunction, StrKey, Value,
};
use crate::template::{html_response, TemplateCache};

// Process-global template cache, shared across all worker threads so a template
// parsed by one worker is visible to the others.
static TEMPLATE_CACHE: OnceLock<Arc<TemplateCache>> = OnceLock::new();

// Thread-local view context for debugging (stores the data passed to render())
thread_local! {
    static VIEW_DEBUG_CONTEXT: RefCell<Option<Value>> = const { RefCell::new(None) };
}

// Thread-local file mtime cache for public_path() performance
#[derive(Clone)]
struct CachedFileMtime {
    mtime_secs: u64, // Seconds since UNIX_EPOCH
}

/// Maximum size for the file mtime cache to prevent unbounded memory growth.
const FILE_MTIME_CACHE_MAX_SIZE: usize = 1000;

thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    static FILE_MTIME_CACHE: RefCell<HashMap<PathBuf, CachedFileMtime>> = RefCell::new(HashMap::new());
}

static DEV_MODE: AtomicBool = AtomicBool::new(false);

/// Set dev mode for file mtime caching behavior.
/// In production (dev_mode=false), mtimes are cached permanently.
/// In dev mode (dev_mode=true), mtimes are always refreshed.
pub fn set_dev_mode(enabled: bool) {
    DEV_MODE.store(enabled, Ordering::Relaxed);
}

/// Whether the server is running with `--dev`. Process-global, set once at
/// server startup. Used to relax production-only optimisations that hurt
/// debuggability — e.g. `grouped {}` skips read coalescing in dev so each
/// query shows up individually in the dev query log.
#[inline]
pub fn is_dev_mode() -> bool {
    DEV_MODE.load(Ordering::Relaxed)
}

/// Clear the file mtime cache (for hot reload).
pub fn clear_file_mtime_cache() {
    FILE_MTIME_CACHE.with(|cache| cache.borrow_mut().clear());
}

// Thread-local view helpers registry (functions from app/helpers/*.sl)
/// Maximum size for view helpers cache.
const VIEW_HELPERS_MAX_SIZE: usize = 500;

thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    static VIEW_HELPERS: RefCell<HashMap<String, Value>> = RefCell::new(HashMap::new());
}

// Thread-local current request context for views
thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    static CURRENT_REQUEST: RefCell<Option<Value>> = const { RefCell::new(None) };
}

// Closure environment shared by every view helper loaded from app/helpers/*.sl.
// `load_view_helpers` builds it once with builtins + sibling helpers and stashes
// the Rc here so the request lifecycle can re-bind request-scoped names
// (`req`, `params`, `session`, `cookies`, `headers`, `flash`) on it before
// rendering. Without this rebinding, a helper that references `req["current_user"]`
// throws "Undefined variable" because builtins-only env never had those names.
thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    static VIEW_HELPER_ENV: RefCell<Option<Rc<RefCell<Environment>>>> = const { RefCell::new(None) };
}

/// Bind request-scoped names onto the view helpers' shared closure env so user
/// helpers (`app/helpers/*.sl`) can read `req`, `params`, etc. directly.
///
/// Called from the request lifecycle (`setup_controller_context` for OOP
/// controllers and `call_handler` for function-based handlers) so the bindings
/// reflect any mutations already applied by middleware (e.g. `req["current_user"]`).
/// Safe to call before helpers are loaded — it's a no-op until then.
pub fn set_helper_request_context(
    req: &Value,
    params: &Value,
    session: &Value,
    cookies: &Value,
    headers: &Value,
) {
    VIEW_HELPER_ENV.with(|cell| {
        if let Some(env_rc) = cell.borrow().as_ref() {
            let mut env = env_rc.borrow_mut();
            env.define_or_update("req", req.clone());
            env.define_or_update("params", params.clone());
            env.define_or_update("session", session.clone());
            env.define_or_update("cookies", cookies.clone());
            env.define_or_update("headers", headers.clone());
        }
    });
}

/// Clear request-scoped bindings from the helper env. Called on request exit
/// (or interpreter teardown) so a stale request's `req` doesn't bleed into the
/// next one if helpers happen to be re-entered outside a request scope.
pub fn clear_helper_request_context() {
    VIEW_HELPER_ENV.with(|cell| {
        if let Some(env_rc) = cell.borrow().as_ref() {
            let mut env = env_rc.borrow_mut();
            env.define_or_update("req", Value::Null);
            env.define_or_update("params", Value::Null);
            env.define_or_update("session", Value::Null);
            env.define_or_update("cookies", Value::Null);
            env.define_or_update("headers", Value::Null);
        }
    });
}

/// Set the current request context (called before template rendering).
pub fn set_current_request(req: Value) {
    CURRENT_REQUEST.with(|ctx| *ctx.borrow_mut() = Some(req));
}

/// Clear the current request context (called after template rendering).
pub fn clear_current_request() {
    CURRENT_REQUEST.with(|ctx| *ctx.borrow_mut() = None);
}

/// Get the current request context if set.
pub fn get_current_request() -> Option<Value> {
    CURRENT_REQUEST.with(|ctx| ctx.borrow().clone())
}

/// Register a view helper function (only accessible in templates, not controllers).
pub fn register_view_helper(name: String, value: Value) {
    VIEW_HELPERS.with(|helpers| {
        let mut helpers = helpers.borrow_mut();
        // Evict cache if it exceeds the maximum size to prevent unbounded memory growth
        if helpers.len() >= VIEW_HELPERS_MAX_SIZE {
            helpers.clear();
        }
        helpers.insert(name, value);
    });
}

/// Clear all view helpers (for hot reload).
pub fn clear_view_helpers() {
    VIEW_HELPERS.with(|helpers| helpers.borrow_mut().clear());
}

/// Get all registered view helpers.
pub fn get_view_helpers() -> HashMap<String, Value> {
    VIEW_HELPERS.with(|helpers| helpers.borrow().clone())
}

/// Define all registered view helpers into the given environment.
/// Used to seed the template builtins env so helpers resolve via the
/// normal scope chain — available in views, partials, and live-view renders
/// without having to mutate the per-render data hash.
pub fn inject_helpers_into_env(env: &mut Environment) {
    VIEW_HELPERS.with(|helpers| {
        for (name, value) in helpers.borrow().iter() {
            env.define(name.clone(), value.clone());
        }
    });
}

/// Load view helpers from a directory (app/helpers/*.sl).
/// Parses each file and extracts function definitions without executing in interpreter.
pub fn load_view_helpers(helpers_dir: &Path) -> Result<usize, String> {
    let helpers_dir_str = helpers_dir.to_string_lossy().to_string();
    if !crate::serve::vfs_exists(&helpers_dir_str) {
        return Ok(0);
    }

    let mut count = 0;

    // Register the full builtin suite into the helpers' closure so helper
    // functions can call session_get / session_set / redirect / render /
    // JSON / HTTP / Model / argon2_hash / …  Without this, a helper that
    // references any builtin fails with "Undefined variable" at render time.
    // `include_test_builtins: false` — helpers never run under `soli test`.
    let helper_env = Rc::new(RefCell::new(Environment::new()));
    crate::interpreter::builtins::register_builtins(&mut helper_env.borrow_mut(), false);

    let entries = std::fs::read_dir(helpers_dir)
        .map_err(|e| format!("Failed to read helpers directory: {}", e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "sl") {
            let path_str = path.to_string_lossy().to_string();
            let bytes = crate::serve::vfs_read(&path_str)
                .map_err(|e| format!("Failed to read helper file '{}': {}", path.display(), e))?;

            // In a protected bundle the helper file is a serialized AST.
            let program = if crate::bundle::is_ast_blob(&bytes) {
                crate::bundle::deserialize_program(&bytes)
                    .map_err(|e| format!("Failed to load '{}': {}", path.display(), e))?
            } else {
                let source = String::from_utf8(bytes).map_err(|e| {
                    format!("Helper '{}' is not valid UTF-8: {}", path.display(), e)
                })?;

                // Lex
                let tokens = crate::lexer::Scanner::new(&source)
                    .scan_tokens()
                    .map_err(|e| format!("Lexer error in {}: {}", path.display(), e))?;

                // Parse
                crate::parser::Parser::new(tokens)
                    .parse()
                    .map_err(|e| format!("Parser error in {}: {}", path.display(), e))?
            };

            let source_path = path.to_string_lossy().to_string();

            // Extract function definitions and register them
            for stmt in &program.statements {
                if let StmtKind::Function(decl) = &stmt.kind {
                    let func =
                        Function::from_decl(decl, helper_env.clone(), Some(source_path.clone()));
                    register_view_helper(decl.name.clone(), Value::Function(Rc::new(func)));
                    count += 1;
                }
            }
        }
    }

    // Now update the helper environment so helpers can call each other
    VIEW_HELPERS.with(|helpers| {
        let helpers_map = helpers.borrow();
        for (name, value) in helpers_map.iter() {
            helper_env.borrow_mut().define(name.clone(), value.clone());
        }
    });

    // Stash the env so the request lifecycle can rebind `req`/`params`/etc.
    // on it (see `set_helper_request_context`).
    VIEW_HELPER_ENV.with(|cell| {
        *cell.borrow_mut() = Some(helper_env);
    });

    Ok(count)
}

/// Get the current view context for debugging (if any).
/// This is set when render() is called and cleared after rendering completes.
pub fn get_view_debug_context() -> Option<Value> {
    VIEW_DEBUG_CONTEXT.with(|ctx| ctx.borrow().clone())
}

/// Set the view context for debugging.
fn set_view_debug_context(data: Option<Value>) {
    VIEW_DEBUG_CONTEXT.with(|ctx| {
        *ctx.borrow_mut() = data;
    });
}

/// Clear any view debug context left over on this worker thread.
///
/// `render()` only clears the context on a *successful* render; on error it
/// keeps it so the failing locals can be attached to the dev error page. Worker
/// threads are reused across requests, so without a per-request reset a later
/// controller-only error could attach stale `_view_data` from a previous
/// request's view error. The server calls this at the start of each dispatch.
pub fn clear_view_debug_context() {
    set_view_debug_context(None);
}

// Global views directory for initialization
static VIEWS_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

// Global public directory for public_path() helper
static PUBLIC_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Initialize the template system with the views directory.
pub fn init_templates(views_dir: PathBuf) {
    // Store views dir globally
    if let Ok(mut dir) = VIEWS_DIR.lock() {
        *dir = Some(views_dir.clone());
    }

    // Install the shared cache (first call wins; subsequent inits are no-ops
    // unless the views dir changed — in that case we fall back to clear()).
    let _ = TEMPLATE_CACHE.set(Arc::new(TemplateCache::new(views_dir)));
    if let Some(tc) = TEMPLATE_CACHE.get() {
        tc.clear();
    }
}

/// Initialize the public directory for public_path() helper.
pub fn init_public_dir(public_dir: PathBuf) {
    if let Ok(mut dir) = PUBLIC_DIR.lock() {
        *dir = Some(public_dir);
    }
}

/// Clear the template cache (for hot reload).
pub fn clear_template_cache() {
    if let Some(tc) = TEMPLATE_CACHE.get() {
        tc.clear();
    }
}

/// Check if templates have changes (for hot reload).
pub fn templates_have_changes() -> bool {
    TEMPLATE_CACHE
        .get()
        .map(|tc| tc.has_changes())
        .unwrap_or(false)
}

/// Get the template cache, initializing if necessary.
pub fn get_template_cache() -> Result<Arc<TemplateCache>, String> {
    if let Some(tc) = TEMPLATE_CACHE.get() {
        return Ok(Arc::clone(tc));
    }

    // Try to initialize from global views dir
    if let Ok(dir_guard) = VIEWS_DIR.lock() {
        if let Some(views_dir) = dir_guard.as_ref() {
            let views_dir_clone = views_dir.clone();
            drop(dir_guard);
            let _ = TEMPLATE_CACHE.set(Arc::new(TemplateCache::new(views_dir_clone)));
            if let Some(tc) = TEMPLATE_CACHE.get() {
                return Ok(Arc::clone(tc));
            }
        }
    }

    Err("Template system not initialized. Call init_templates() first.".to_string())
}

/// Render an error template with the given status code and context.
/// Returns None if the custom template doesn't exist.
pub fn render_error_template(status_code: u16, message: &str, request_id: &str) -> Option<String> {
    let template_cache = match get_template_cache() {
        Ok(tc) => tc,
        Err(_) => return None,
    };

    let template_name = format!("errors/{}", status_code);

    // Create error context for the template
    let mut error_map: HashPairs = HashPairs::default();
    error_map.insert(
        HashKey::String("status".into()),
        Value::Int(status_code as i64),
    );
    error_map.insert(
        HashKey::String("message".into()),
        Value::String(message.to_string().into()),
    );
    error_map.insert(
        HashKey::String("request_id".into()),
        Value::String(request_id.to_string().into()),
    );
    let error_data = Value::Hash(Rc::new(RefCell::new(error_map)));

    // Try to render the template without layout (error pages should be standalone)
    template_cache
        .render(&template_name, &error_data, Some(None))
        .ok()
}

/// Check if a value could potentially contain futures (quick discriminant check).
#[inline]
fn could_contain_futures(v: &Value) -> bool {
    matches!(v, Value::Future(_) | Value::Hash(_) | Value::Array(_))
}

/// Recursively resolve all Future values in a Value.
/// This ensures that async operations (like HTTP requests) are completed
/// before the data is used in templates.
/// Fast path: returns the value as-is when no futures are present (zero allocations).
fn resolve_futures_in_value(value: Value) -> Value {
    // Fast path: primitive values never contain futures
    match &value {
        Value::Future(_) | Value::Hash(_) | Value::Array(_) => {}
        _ => return value,
    }
    // Slow path: check for and resolve futures
    match value {
        Value::Future(_) => match value.resolve() {
            Ok(resolved) => resolve_futures_in_value(resolved),
            Err(e) => Value::String(format!("<future error: {}>", e).into()),
        },
        Value::Hash(hash) => {
            // Quick scan: if no values could contain futures, return as-is
            let needs = hash.borrow().values().any(could_contain_futures);
            if !needs {
                return Value::Hash(hash);
            }
            let resolved_pairs: HashPairs = hash
                .borrow()
                .iter()
                .map(|(k, v)| (k.clone(), resolve_futures_in_value(v.clone())))
                .collect();
            Value::Hash(Rc::new(RefCell::new(resolved_pairs)))
        }
        Value::Array(arr) => {
            // Quick scan: if no items could contain futures, return as-is
            let needs = arr.borrow().iter().any(could_contain_futures);
            if !needs {
                return Value::Array(arr);
            }
            let resolved_items: Vec<Value> = arr
                .borrow()
                .iter()
                .map(|v| resolve_futures_in_value(v.clone()))
                .collect();
            Value::Array(Rc::new(RefCell::new(resolved_items)))
        }
        _ => unreachable!(),
    }
}

/// Get file modification time with caching.
/// In production mode: O(1) lookup from cache after first stat.
/// In dev mode: always refreshes mtime from filesystem.
fn get_file_mtime_cached(path: &PathBuf) -> Result<String, String> {
    FILE_MTIME_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let dev_mode = DEV_MODE.load(Ordering::Relaxed);

        // Get current mtime
        let metadata =
            std::fs::metadata(path).map_err(|e| format!("Failed to stat file: {}", e))?;
        let modified = metadata
            .modified()
            .map_err(|e| format!("Failed to get mtime: {}", e))?;
        let mtime_secs = modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // In production, return cached value if available
        if !dev_mode {
            if let Some(cached) = cache.get(path) {
                return Ok(cached.mtime_secs.to_string());
            }
        }

        // Store and return
        if cache.len() >= FILE_MTIME_CACHE_MAX_SIZE {
            cache.clear();
        }
        cache.insert(path.clone(), CachedFileMtime { mtime_secs });
        Ok(mtime_secs.to_string())
    })
}

/// Inject template helper functions into the data context.
/// Static helpers (range, html_escape, etc.) are in the thread-local builtins.
/// User-defined view helpers from app/helpers/*.sl are injected via VIEW_HELPERS.
pub fn inject_template_helpers(data: &Value) {
    if let Value::Hash(hash) = data {
        VIEW_HELPERS.with(|helpers| {
            let helpers_map = helpers.borrow();
            if !helpers_map.is_empty() {
                let mut h = hash.borrow_mut();
                for (name, value) in helpers_map.iter() {
                    let key = HashKey::String(name.clone().into());
                    if !h.contains_key(&key) {
                        h.insert(key, value.clone());
                    }
                }
            }
        });
    }
}

/// Inject req, params, and cookies from current request context into data hash.
pub fn inject_request_context(data: &Value) {
    if let Value::Hash(hash) = data {
        if let Some(req) = get_current_request() {
            let mut h = hash.borrow_mut();
            let req_key = HashKey::String("req".into());
            if !h.contains_key(&req_key) {
                h.insert(req_key, req.clone());
            }

            if let Value::Hash(req_hash) = &req {
                let borrowed = req_hash.borrow();

                // Extract params from req and inject as top-level "params"
                if let Some(params) = borrowed.get(&HashKey::String("params".into())) {
                    let params_key = HashKey::String("params".into());
                    if !h.contains_key(&params_key) {
                        h.insert(params_key, params.clone());
                    }
                }

                // Extract cookies from req and inject as top-level "cookies"
                if let Some(cookies) = borrowed.get(&HashKey::String("cookies".into())) {
                    let cookies_key = HashKey::String("cookies".into());
                    if !h.contains_key(&cookies_key) {
                        h.insert(cookies_key, cookies.clone());
                    }
                }
            }
        }
    }
}

/// Read a string field off the thread-local current request. Returns `Null` when
/// there's no active request or the field isn't a string. Used by the
/// `current_path()` / `current_method()` view helpers.
fn current_request_string_field(field: &str) -> Value {
    let Some(req) = get_current_request() else {
        return Value::Null;
    };
    let Value::Hash(h) = req else {
        return Value::Null;
    };
    let borrowed = h.borrow();
    match borrowed.get(&HashKey::String(field.to_string().into())) {
        Some(Value::String(s)) => Value::String(s.clone()),
        _ => Value::Null,
    }
}

/// Read a query-string param off the thread-local current request's `query`
/// sub-hash. Returns `None` when there is no active request, no `query` hash, or
/// the key is absent / not a string. Used by `render_jsonp` to resolve the
/// `?callback` name.
fn current_request_query_param(name: &str) -> Option<String> {
    let Value::Hash(h) = get_current_request()? else {
        return None;
    };
    let borrowed = h.borrow();
    let Some(Value::Hash(query)) = borrowed.get(&HashKey::String("query".into())) else {
        return None;
    };
    let query = query.borrow();
    match query.get(&HashKey::String(name.to_string().into())) {
        Some(Value::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// Resolve the registered layout for a controller class + the in-flight
/// action: the first matching per-action `this.layout(...)` rule, else the
/// controller-wide `this.layout = "..."` default. `None` when the class isn't
/// registered or declared no layout.
fn registered_layout_for_class(class_name: &str) -> Option<String> {
    let registry = crate::interpreter::builtins::controller::registry::CONTROLLER_REGISTRY
        .read()
        .ok()?;
    let info = registry.get_by_name(class_name)?;
    // Resolve against the in-flight action so per-action `this.layout(...)`
    // rules win over the controller-wide default.
    let action = crate::interpreter::builtins::current_action_name();
    info.layout_for(&action)
}

/// Resolve the registered layout for an explicit controller instance.
/// Used by the server's auto-render path (an action that sets `@vars` and
/// lets the matching view render with no explicit `render(...)` call), so
/// `static { this.layout = ... }` is honored there too — not just on the
/// explicit-`render` path.
pub fn registered_layout_for_instance(controller_instance: &Value) -> Option<String> {
    let Value::Instance(inst) = controller_instance else {
        return None;
    };
    let class_name = inst.borrow().class.name.clone();
    registered_layout_for_class(&class_name)
}

/// Read the current controller's registered layout (from its
/// `static { this.layout = "..." }` block) as a `Value::String`, or `None`
/// if no controller is active, the instance has no class metadata, or the
/// controller didn't register a layout. Used by `render(...)` as the
/// third-tier fallback (after the explicit `layout` key in options/data)
/// before the "application" default.
fn get_current_controller_registered_layout() -> Option<Value> {
    let ctrl = crate::interpreter::builtins::controller::registry::get_current_controller()?;
    let Value::Instance(inst) = ctrl else {
        return None;
    };
    let class_name = inst.borrow().class.name.clone();
    registered_layout_for_class(&class_name).map(|s| Value::String(s.into()))
}

/// Expose the current controller's instance fields as view locals.
/// Mirrors Rails' `@ivar → view local` behavior, scoped to the action currently running.
/// Skips framework-injected fields already supplied by `inject_request_context` and
/// never clobbers keys the action passed explicitly to `render(...)`.
pub fn inject_controller_instance_vars(data: &Value) {
    let Value::Hash(hash) = data else { return };
    let Some(ctrl) = crate::interpreter::builtins::controller::registry::get_current_controller()
    else {
        return;
    };
    let Value::Instance(inst) = ctrl else { return };

    let inst_ref = inst.borrow();
    let mut h = hash.borrow_mut();
    for (name, value) in &inst_ref.fields {
        if matches!(name.as_str(), "req" | "params" | "session" | "headers") {
            continue;
        }
        let key = HashKey::String(name.clone().into());
        if !h.contains_key(&key) {
            h.insert(key, value.clone());
        }
    }
}

/// Pure-Soli form-builder layer (`form_with` / `FormBuilder` / `csrf_field`
/// / `csrf_meta_tag` / `button_to`), evaluated into the shared template
/// builtins environment at seed time (see `core_eval::get_builtins_rc`).
const FORM_BUILDER_SOURCE: &str = include_str!("form_builder.sl");

/// Evaluate the embedded form-builder Soli source into the template builtins
/// environment so `form_with(...)` and friends resolve in every view.
pub fn register_form_builder(env: &Rc<RefCell<Environment>>) -> Result<(), String> {
    let tokens = crate::lexer::Scanner::new(FORM_BUILDER_SOURCE)
        .scan_tokens()
        .map_err(|e| format!("form builder lexer error: {}", e))?;
    let program = crate::parser::Parser::new(tokens)
        .parse()
        .map_err(|e| format!("form builder parser error: {}", e))?;
    let mut interpreter = crate::interpreter::Interpreter::with_environment(env.clone());
    for stmt in &program.statements {
        interpreter
            .execute(stmt)
            .map_err(|e| format!("form builder eval error: {}", e))?;
    }
    Ok(())
}

/// Register static template helpers into an Environment (called once per thread).
/// These helpers (range, public_path, html_escape, etc.) are created once and
/// shared via the thread-local builtins Rc, avoiding ~20 NativeFunction allocations per render.
pub fn register_static_template_helpers(env: &mut Environment) {
    // __soli_form_names(record) — internal support for `form_with`: a class
    // instance yields {"collection": <derived collection name>, "key": <_key
    // or null>} so the builder can derive RESTful URLs; anything else yields
    // null (the caller must then pass an explicit "url" option).
    env.define(
        "__soli_form_names".to_string(),
        Value::NativeFunction(NativeFunction::new("__soli_form_names", Some(1), |args| {
            let Value::Instance(inst) = &args[0] else {
                return Ok(Value::Null);
            };
            let inst_ref = inst.borrow();
            let collection = crate::interpreter::builtins::model::core::class_name_to_collection(
                &inst_ref.class.name,
            );
            let key = inst_ref.get("_key").unwrap_or(Value::Null);
            let mut names = HashPairs::default();
            names.insert(
                HashKey::String("collection".into()),
                Value::String(collection.into()),
            );
            names.insert(HashKey::String("key".into()), key);
            Ok(Value::Hash(Rc::new(RefCell::new(names))))
        })),
    );

    env.define(
        "range".to_string(),
        Value::NativeFunction(NativeFunction::new("range", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "range() expects 2 or 3 arguments, got {}",
                    args.len()
                ));
            }
            let start = match &args[0] {
                Value::Int(n) => *n,
                other => {
                    return Err(format!(
                        "range() expects integer start, got {}",
                        other.type_name()
                    ))
                }
            };
            let end = match &args[1] {
                Value::Int(n) => *n,
                other => {
                    return Err(format!(
                        "range() expects integer end, got {}",
                        other.type_name()
                    ))
                }
            };
            let step = if args.len() == 3 {
                match &args[2] {
                    Value::Int(n) if *n == 0 => {
                        return Err("range() step cannot be zero".to_string())
                    }
                    Value::Int(n) => *n,
                    other => {
                        return Err(format!(
                            "range() expects integer step, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                1
            };
            let mut values = Vec::new();
            if step > 0 {
                let mut i = start;
                while i < end {
                    values.push(Value::Int(i));
                    i += step;
                }
            } else {
                let mut i = start;
                while i > end {
                    values.push(Value::Int(i));
                    i += step;
                }
            }
            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );

    env.define(
        "public_path".to_string(),
        Value::NativeFunction(NativeFunction::new("public_path", Some(1), |args| {
            let path = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "public_path() expects string path, got {}",
                        other.type_name()
                    ))
                }
            };
            let public_dir = match PUBLIC_DIR.lock() {
                Ok(dir_guard) => dir_guard.clone(),
                _ => None,
            };
            let public_dir = public_dir.unwrap_or_else(|| PathBuf::from("public"));
            let full_path = public_dir.join(&*path);
            match get_file_mtime_cached(&full_path) {
                Ok(mtime) => {
                    if path.contains('?') {
                        Ok(Value::String(format!("/{}&v={}", path, mtime).into()))
                    } else {
                        Ok(Value::String(format!("/{}?v={}", path, mtime).into()))
                    }
                }
                Err(_) => Ok(Value::String(format!("/{}", path).into())),
            }
        })),
    );

    // camera_preview(options?) — a <video> wired to the camera.
    //
    // Showing a camera is six lines of getUserMedia, so this exists for what
    // those six lines leave out: the script it turns on stops the tracks when
    // the element goes away. Instant navigation swaps the body without a page
    // unload, so a hand-rolled preview keeps its stream and the camera
    // indicator stays lit after the user has moved on.
    //
    // Options: facing ("user" | "environment"), scan (formats, e.g. "qr_code"),
    // width, height, audio, class, id, fallback (a selector revealed on error).
    env.define(
        "camera_preview".to_string(),
        Value::NativeFunction(NativeFunction::new("camera_preview", None, |args| {
            let options = match args.first() {
                None | Some(Value::Null) => None,
                Some(Value::Hash(h)) => Some(h.borrow().clone()),
                Some(other) => {
                    return Err(format!(
                        "camera_preview() expects an options hash, got {}",
                        other.type_name()
                    ))
                }
            };

            let get = |key: &str| -> Option<String> {
                options.as_ref().and_then(|o| {
                    o.get(&HashKey::String(key.into())).and_then(|v| match v {
                        Value::String(s) => Some(s.to_string()),
                        Value::Int(i) => Some(i.to_string()),
                        Value::Bool(b) => Some(b.to_string()),
                        _ => None,
                    })
                })
            };

            let mut attributes = String::new();
            // `playsinline` and `muted` are not decoration: without them iOS
            // takes the video fullscreen, and autoplay is refused.
            attributes.push_str(" data-soli-camera autoplay playsinline muted");

            if let Some(id) = get("id") {
                attributes.push_str(&format!(" id=\"{}\"", super::html::html_escape(&id)));
            }
            if let Some(class) = get("class") {
                attributes.push_str(&format!(" class=\"{}\"", super::html::html_escape(&class)));
            }
            for (option, attribute) in [
                ("facing", "data-facing"),
                ("scan", "data-soli-scan"),
                ("width", "data-width"),
                ("height", "data-height"),
                ("interval", "data-scan-interval"),
                ("fallback", "data-fallback"),
            ] {
                if let Some(value) = get(option) {
                    attributes.push_str(&format!(
                        " {}=\"{}\"",
                        attribute,
                        super::html::html_escape(&value)
                    ));
                }
            }
            for (option, attribute) in [
                ("audio", "data-audio"),
                ("continuous", "data-scan-continuous"),
                ("manual", "data-manual"),
            ] {
                if matches!(
                    options
                        .as_ref()
                        .and_then(|o| o.get(&HashKey::String(option.into())).cloned()),
                    Some(Value::Bool(true))
                ) {
                    attributes.push_str(&format!(" {}", attribute));
                }
            }

            Ok(Value::String(
                format!("<video{}></video>", attributes).into(),
            ))
        })),
    );

    // native_channel(channel) — emit the meta tag that turns on the native
    // bridge for this page and names the channel it listens to.
    //
    // The channel travels as a signed token, not as plain text: subscribing is
    // a GET the browser makes, so an unsigned `?channel=user:42` would let
    // anyone listen to anyone. The tag's presence is also what gates script
    // injection, so a page that never calls this downloads nothing.
    env.define(
        "native_channel".to_string(),
        Value::NativeFunction(NativeFunction::new("native_channel", Some(1), |args| {
            let channel = match &args[0] {
                Value::String(s) => s.to_string(),
                other => {
                    return Err(format!(
                        "native_channel() expects string channel, got {}",
                        other.type_name()
                    ))
                }
            };
            let token = crate::interpreter::builtins::native::sign_channel(&channel)?;
            Ok(Value::String(
                format!("<meta name=\"soli-native\" content=\"{}\">", token).into(),
            ))
        })),
    );

    env.define(
        "strip_html".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "strip_html",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(html::strip_html(s).into())),
                other => Err(format!(
                    "strip_html() expects string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    env.define(
        "substring".to_string(),
        Value::NativeFunction(NativeFunction::new("substring", Some(3), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "substring() expects string as first argument, got {}",
                        other.type_name()
                    ))
                }
            };
            let start = match &args[1] {
                Value::Int(n) => *n as usize,
                other => {
                    return Err(format!(
                        "substring() expects int as second argument, got {}",
                        other.type_name()
                    ))
                }
            };
            let end = match &args[2] {
                Value::Int(n) => *n as usize,
                other => {
                    return Err(format!(
                        "substring() expects int as third argument, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(html::substring(&s, start, end).into()))
        })),
    );

    env.define(
        "html_escape".to_string(),
        Value::NativeFunction(NativeFunction::new("html_escape", Some(1), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => format!("{}", other).into(),
            };
            Ok(Value::String(html::html_escape(&s).into()))
        })),
    );

    env.define(
        "html_unescape".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "html_unescape",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(html::html_unescape(s).into())),
                other => Err(format!(
                    "html_unescape() expects string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    env.define(
        "sanitize_html".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "sanitize_html",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(html::sanitize_html(s).into())),
                other => Err(format!(
                    "sanitize_html() expects string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    env.define(
        "datetime_now".to_string(),
        Value::NativeFunction(NativeFunction::new("datetime_now", Some(0), |_args| {
            Ok(Value::Int(datetime_helpers::datetime_now()))
        })),
    );

    env.define(
        "freeze_time".to_string(),
        Value::NativeFunction(NativeFunction::new("freeze_time", Some(1), |args| {
            let timestamp = match &args[0] {
                Value::Int(n) => *n,
                Value::String(s) => datetime_helpers::datetime_parse(s)
                    .ok_or_else(|| format!("freeze_time(): invalid date string {:?}", s))?,
                other => {
                    return Err(format!(
                        "freeze_time() expects timestamp (int) or date string, got {}",
                        other.type_name()
                    ));
                }
            };
            datetime_helpers::freeze_datetime(timestamp);
            Ok(Value::Int(timestamp))
        })),
    );

    env.define(
        "travel_to".to_string(),
        Value::NativeFunction(NativeFunction::new("travel_to", Some(1), |args| {
            let timestamp = match &args[0] {
                Value::Int(n) => *n,
                Value::String(s) => datetime_helpers::datetime_parse(s)
                    .ok_or_else(|| format!("travel_to(): invalid date string {:?}", s))?,
                other => {
                    return Err(format!(
                        "travel_to() expects timestamp (int) or date string, got {}",
                        other.type_name()
                    ));
                }
            };
            datetime_helpers::freeze_datetime(timestamp);
            Ok(Value::Int(timestamp))
        })),
    );

    env.define(
        "unfreeze_time".to_string(),
        Value::NativeFunction(NativeFunction::new("unfreeze_time", Some(0), |_args| {
            datetime_helpers::unfreeze_datetime();
            Ok(Value::Null)
        })),
    );

    env.define("datetime_format".to_string(), Value::NativeFunction(NativeFunction::new("datetime_format", Some(2), |args| {
        let timestamp = match &args[0] {
            Value::Int(n) => *n,
            Value::String(s) => datetime_helpers::datetime_parse(s).unwrap_or(0),
            other => return Err(format!("datetime_format() expects timestamp (int) or date string as first argument, got {}", other.type_name())),
        };
        let format = match &args[1] {
            Value::String(s) => s.clone(),
            other => return Err(format!("datetime_format() expects string format as second argument, got {}", other.type_name())),
        };
        Ok(Value::String(datetime_helpers::datetime_format(timestamp, &format).into()))
    })));

    env.define(
        "datetime_parse".to_string(),
        Value::NativeFunction(NativeFunction::new("datetime_parse", Some(1), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "datetime_parse() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            match datetime_helpers::datetime_parse(&s) {
                Some(ts) => Ok(Value::Int(ts)),
                None => Ok(Value::Null),
            }
        })),
    );

    env.define(
        "datetime_add_days".to_string(),
        Value::NativeFunction(NativeFunction::new("datetime_add_days", Some(2), |args| {
            let timestamp = match &args[0] {
                Value::Int(n) => *n,
                other => {
                    return Err(format!(
                        "datetime_add_days() expects timestamp (int) as first argument, got {}",
                        other.type_name()
                    ))
                }
            };
            let days = match &args[1] {
                Value::Int(n) => *n,
                other => {
                    return Err(format!(
                        "datetime_add_days() expects int as second argument, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::Int(datetime_helpers::datetime_add_days(
                timestamp, days,
            )))
        })),
    );

    env.define(
        "datetime_add_hours".to_string(),
        Value::NativeFunction(NativeFunction::new("datetime_add_hours", Some(2), |args| {
            let timestamp = match &args[0] {
                Value::Int(n) => *n,
                other => {
                    return Err(format!(
                        "datetime_add_hours() expects timestamp (int) as first argument, got {}",
                        other.type_name()
                    ))
                }
            };
            let hours = match &args[1] {
                Value::Int(n) => *n,
                other => {
                    return Err(format!(
                        "datetime_add_hours() expects int as second argument, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::Int(datetime_helpers::datetime_add_hours(
                timestamp, hours,
            )))
        })),
    );

    env.define(
        "datetime_diff".to_string(),
        Value::NativeFunction(NativeFunction::new("datetime_diff", Some(2), |args| {
            let t1 = match &args[0] {
                Value::Int(n) => *n,
                other => {
                    return Err(format!(
                        "datetime_diff() expects timestamp (int) as first argument, got {}",
                        other.type_name()
                    ))
                }
            };
            let t2 = match &args[1] {
                Value::Int(n) => *n,
                other => {
                    return Err(format!(
                        "datetime_diff() expects timestamp (int) as second argument, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::Int(datetime_helpers::datetime_diff(t1, t2)))
        })),
    );

    env.define(
        "time_ago".to_string(),
        Value::NativeFunction(NativeFunction::new("time_ago", Some(1), |args| {
            let timestamp = match &args[0] {
                Value::Int(n) => *n,
                Value::String(s) => datetime_helpers::datetime_parse(s).unwrap_or(0),
                other => {
                    return Err(format!(
                        "time_ago() expects timestamp (int) or date string, got {}",
                        other.type_name()
                    ))
                }
            };
            let locale = i18n_helpers::get_locale();
            Ok(Value::String(
                datetime_helpers::time_ago_localized(timestamp, &locale).into(),
            ))
        })),
    );

    env.define(
        "locale".to_string(),
        Value::NativeFunction(NativeFunction::new("locale", Some(0), |_args| {
            Ok(Value::String(i18n_helpers::get_locale().into()))
        })),
    );

    env.define(
        "set_locale".to_string(),
        Value::NativeFunction(NativeFunction::new("set_locale", Some(1), |args| {
            let locale = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "set_locale() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            i18n_helpers::set_locale(&locale);
            Ok(Value::String(locale))
        })),
    );

    env.define(
        "t".to_string(),
        Value::NativeFunction(NativeFunction::new("t", Some(1), |args| {
            let key = match &args[0] {
                Value::String(s) => s.clone(),
                other => return Err(format!("t() expects string key, got {}", other.type_name())),
            };
            Ok(Value::String(key))
        })),
    );

    env.define(
        "l".to_string(),
        Value::NativeFunction(NativeFunction::new("l", None, |args| {
            if args.is_empty() {
                return Err("l() requires at least 1 argument (timestamp)".to_string());
            }
            let timestamp = match &args[0] {
                Value::Int(n) => *n,
                Value::String(s) => datetime_helpers::datetime_parse(s).unwrap_or(0),
                other => {
                    return Err(format!(
                        "l() expects timestamp (int) or date string as first argument, got {}",
                        other.type_name()
                    ))
                }
            };
            let format = if args.len() > 1 {
                match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => "short".into(),
                }
            } else {
                "short".into()
            };
            let locale = i18n_helpers::get_locale();
            Ok(Value::String(
                datetime_helpers::localize_date(timestamp, &locale, &format).into(),
            ))
        })),
    );

    // Request-context helpers: read fields off the current request inside views.
    // Return Null when called outside an active request (e.g. during tests).
    env.define(
        "current_path".to_string(),
        Value::NativeFunction(NativeFunction::new("current_path", Some(0), |_args| {
            Ok(current_request_string_field("path"))
        })),
    );
    env.define(
        "current_method".to_string(),
        Value::NativeFunction(NativeFunction::new("current_method", Some(0), |_args| {
            Ok(current_request_string_field("method"))
        })),
    );
    env.define(
        "current_path?".to_string(),
        Value::NativeFunction(NativeFunction::new("current_path?", Some(1), |args| {
            let expected = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "current_path?() expects string, got {}",
                        other.type_name()
                    ))
                }
            };
            let path = match current_request_string_field("path") {
                Value::String(s) => s,
                _ => return Ok(Value::Bool(false)),
            };
            Ok(Value::Bool(path == expected))
        })),
    );
    env.define(
        "content_for?".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "content_for?",
            Some(1),
            |args| match &args[0] {
                Value::String(name) => Ok(Value::Bool(crate::template::content_store::has(name))),
                other => Err(format!(
                    "content_for?() expects string name, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    let render_partial_fn = NativeFunction::new("render_partial", None, |args| {
        if args.is_empty() {
            return Err("render_partial() requires at least 1 argument (partial name)".to_string());
        }
        let partial_name = match &args[0] {
            Value::String(s) => s.clone(),
            other => {
                return Err(format!(
                    "render_partial() expects string partial name, got {}",
                    other.type_name()
                ))
            }
        };
        let data = if args.len() > 1 {
            args[1].clone()
        } else {
            Value::Hash(Rc::new(RefCell::new(HashPairs::default())))
        };
        let data = resolve_futures_in_value(data);
        let cache = get_template_cache()?;
        inject_template_helpers(&data);
        cache
            .render_partial(&partial_name, &data)
            .map(|s| Value::String(s.into()))
    });
    env.define(
        "render_partial".to_string(),
        Value::NativeFunction(render_partial_fn.clone()),
    );
    // Alias: `partial(name, data?)` — shorter form used in views.
    env.define(
        "partial".to_string(),
        Value::NativeFunction(render_partial_fn),
    );

    // Pagination view helper.
    // Accepts the full result from Model.paginate(...) or just the "pagination" sub-hash.
    // Options (hash):
    //   "path": base URL or path to append ?page= (string)
    //   "param": query param name (default "page")
    //   "window": number of page links around current (default 2)
    //   "class": extra CSS class on the nav
    // Returns raw HTML (use with <%- %>). Links preserve other query params when possible.
    env.define(
        "paginate".to_string(),
        Value::NativeFunction(NativeFunction::new("paginate", None, |args| {
            if args.is_empty() {
                return Err(
                    "paginate() requires the pagination data (from Model.paginate or similar)"
                        .to_string(),
                );
            }
            let pag_data = &args[0];
            let opts = if args.len() > 1 {
                &args[1]
            } else {
                &Value::Hash(Rc::new(RefCell::new(HashPairs::default())))
            };

            match paginate_html(pag_data, opts) {
                Ok(html) => Ok(Value::String(html.into())),
                Err(e) => Err(e),
            }
        })),
    );

    // number_with_delimiter(value, delimiter?) -> String
    // Formats a number with thousands separators (default ",") for display,
    // e.g. number_with_delimiter(1234567) -> "1,234,567". Pairs with paginate()
    // totals. Any fractional part is preserved; the integer part is grouped.
    env.define(
        "number_with_delimiter".to_string(),
        Value::NativeFunction(NativeFunction::new("number_with_delimiter", None, |args| {
            if args.is_empty() {
                return Err("number_with_delimiter() requires a number".to_string());
            }
            let delimiter = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                _ => ",".to_string(),
            };
            let grouped = |s: &str| -> String {
                match s.split_once('.') {
                    Some((int_part, frac)) => {
                        format!("{}.{}", group_integer_str(int_part, &delimiter), frac)
                    }
                    None => group_integer_str(s, &delimiter),
                }
            };
            let formatted = match &args[0] {
                Value::Int(n) => group_integer_str(&n.to_string(), &delimiter),
                Value::Float(f) => grouped(&format!("{}", f)),
                Value::String(s) => grouped(s.as_ref()),
                other => {
                    return Err(format!(
                        "number_with_delimiter() expects a number, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(Value::String(formatted.into()))
        })),
    );

    // props("name", "name", ...) declares the props a component expects. In
    // --dev the renderer warns (dev bar + console) about any declared prop the
    // caller didn't provide. No-op outside a component render; also lint-checked
    // (`component/props`). Returns nothing — use as a statement: `<% props("title") %>`.
    env.define(
        "props".to_string(),
        Value::NativeFunction(NativeFunction::new("props", None, |args| {
            let names: Vec<String> = args
                .iter()
                .filter_map(|a| match a {
                    Value::String(s) => Some(s.to_string()),
                    _ => None,
                })
                .collect();
            crate::template::declared_props::declare(names);
            Ok(Value::Null)
        })),
    );

    // Component helper for reusable view pieces (better composition than deep partial nesting).
    // Looks for app/views/components/<name>.html.slv (or a path containing /).
    // data: hash of locals passed to the component.
    // Usage: <%- component("card", {"title": "Stats"}) %>
    // For body content, pre-capture or pass "content": "..." ; future versions may support block syntax.
    env.define(
        "component".to_string(),
        Value::NativeFunction(NativeFunction::new("component", None, |args| {
            if args.is_empty() {
                return Err("component() requires at least a name".to_string());
            }
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "component() expects string name, got {}",
                        other.type_name()
                    ))
                }
            };
            let data = if args.len() > 1 {
                args[1].clone()
            } else {
                Value::Hash(Rc::new(RefCell::new(HashPairs::default())))
            };
            let data = resolve_futures_in_value(data);
            let cache = get_template_cache()?;

            // Collection form: component("card", { "collection": items, "as": "post", ...rest })
            // renders the component once per element. Item locals: <as> (default =
            // the component's base name), <as>_index (0-based), <as>_counter
            // (1-based). Other keys pass through to every item.
            if let Some(Value::Array(items)) = opt_value(&data, "collection") {
                let name_str: &str = &name;
                let base = name_str.rsplit('/').next().unwrap_or(name_str);
                let base = base.split('.').next().unwrap_or(base);
                let as_name = opt_str(&data, "as").unwrap_or_else(|| base.to_string());
                let mut out = String::new();
                for (i, item) in items.borrow().iter().enumerate() {
                    let mut item_map: HashPairs = HashPairs::default();
                    if let Value::Hash(h) = &data {
                        for (k, v) in h.borrow().iter() {
                            let skip = matches!(k, HashKey::String(s)
                                if s.as_ref() == "collection" || s.as_ref() == "as");
                            if !skip {
                                item_map.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    item_map.insert(HashKey::String(as_name.clone().into()), item.clone());
                    item_map.insert(
                        HashKey::String(format!("{}_index", as_name).into()),
                        Value::Int(i as i64),
                    );
                    item_map.insert(
                        HashKey::String(format!("{}_counter", as_name).into()),
                        Value::Int(i as i64 + 1),
                    );
                    let item_data = Value::Hash(Rc::new(RefCell::new(item_map)));
                    inject_template_helpers(&item_data);
                    match cache.render_component(&name, &item_data) {
                        Ok(s) => out.push_str(&s),
                        Err(e) => return Err(format!("component('{}') failed: {}", name, e)),
                    }
                }
                return Ok(Value::String(out.into()));
            }

            // Fragment caching (scalar form): a "cache" option memoizes the
            // rendered HTML in the KV cache. `"cache": "key"` uses an explicit
            // key; `"cache": true` derives one from the data. `"cache_ttl"` sets
            // the TTL. Best-effort — any cache error falls through to a normal
            // render, and only side-effect-free renders are stored.
            let cache_opt = opt_value(&data, "cache");
            let cache_ttl = opt_int(&data, "cache_ttl").map(|n| n as u64);
            // Strip the control keys so they neither leak into the component as
            // locals nor perturb the auto cache key.
            let data = if cache_opt.is_some() {
                match &data {
                    Value::Hash(h) => {
                        let mut m: HashPairs = HashPairs::default();
                        for (k, v) in h.borrow().iter() {
                            let ctrl = matches!(k, HashKey::String(s)
                                if s.as_ref() == "cache" || s.as_ref() == "cache_ttl");
                            if !ctrl {
                                m.insert(k.clone(), v.clone());
                            }
                        }
                        Value::Hash(Rc::new(RefCell::new(m)))
                    }
                    other => other.clone(),
                }
            } else {
                data
            };
            let cache_key = match &cache_opt {
                None => None,
                Some(Value::String(k)) => Some(format!("component:{}:{}", name, k)),
                Some(_) => Some(format!(
                    "component:{}:{}",
                    name,
                    crate::template::response_cache::data_signature(&data)
                )),
            };
            if let Some(key) = &cache_key {
                if let Ok(Value::String(html)) =
                    crate::interpreter::builtins::cache::cache_get_impl(key)
                {
                    return Ok(Value::String(html));
                }
            }

            inject_template_helpers(&data);
            match cache.render_component(&name, &data) {
                Ok(s) => {
                    // Store only clean renders — a component that pulled clock/
                    // random or set a cookie/session must not be memoized.
                    if let Some(key) = &cache_key {
                        if !crate::template::response_cache::is_data_dirty()
                            && !crate::template::response_cache::is_response_dirty()
                        {
                            let _ = crate::interpreter::builtins::cache::cache_set_impl(
                                key,
                                &Value::String(s.clone().into()),
                                cache_ttl,
                            );
                        }
                    }
                    Ok(Value::String(s.into()))
                }
                Err(e) => Err(format!("component('{}') failed: {}", name, e)),
            }
        })),
    );
}

fn redirect_response(location: String) -> Value {
    let mut headers_map: HashPairs = HashPairs::default();
    headers_map.insert(
        HashKey::String("Location".into()),
        Value::String(location.into()),
    );
    let headers = Value::Hash(Rc::new(RefCell::new(headers_map)));

    let mut response_map: HashPairs = HashPairs::default();
    response_map.insert(HashKey::String("status".into()), Value::Int(302));
    response_map.insert(HashKey::String("headers".into()), headers);
    response_map.insert(
        HashKey::String("body".into()),
        Value::String(String::new().into()),
    );

    Value::Hash(Rc::new(RefCell::new(response_map)))
}

fn has_redirect_control_chars(url: &str) -> bool {
    url.chars().any(char::is_control)
}

fn validate_local_redirect_url(url: &str) -> Result<(), String> {
    if url.is_empty() || has_redirect_control_chars(url) {
        return Err("redirect() expects a non-empty local path".to_string());
    }
    if !url.starts_with('/') || url.starts_with("//") || url.contains('\\') {
        return Err(
            "redirect() only accepts local absolute paths like '/dashboard'; use redirect_external() for trusted external URLs"
                .to_string(),
        );
    }
    Ok(())
}

/// Resolve `redirect(:back)` to a safe local path.
///
/// Reads the `Referer` header from the per-request thread-local. If it points
/// at our own host (matching scheme+host from `named_routes`) we keep its
/// path+query; otherwise we fall back to `/`. Missing/invalid Referer also
/// falls back to `/`. The chosen path is run through `validate_local_redirect_url`
/// so a malformed Referer can never produce an unsafe Location header.
fn resolve_back_redirect() -> String {
    let Some(req) = get_current_request() else {
        return "/".to_string();
    };
    let Value::Hash(req_hash) = &req else {
        return "/".to_string();
    };
    let referer = req_hash
        .borrow()
        .get(&HashKey::String("headers".into()))
        .and_then(|h| match h {
            Value::Hash(hh) => hh
                .borrow()
                .get(&HashKey::String("referer".into()))
                .and_then(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    _ => None,
                }),
            _ => None,
        });

    let Some(referer) = referer else {
        return "/".to_string();
    };
    if referer.is_empty() || has_redirect_control_chars(&referer) {
        return "/".to_string();
    }

    let lower = referer.to_ascii_lowercase();
    let (referer_scheme, rest) = if let Some(r) = lower.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = lower.strip_prefix("http://") {
        ("http", r)
    } else {
        return "/".to_string();
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() || authority.contains('@') {
        return "/".to_string();
    }

    let Some((cur_scheme, cur_host)) =
        crate::interpreter::builtins::named_routes::current_request_host()
    else {
        return "/".to_string();
    };
    if !cur_scheme.eq_ignore_ascii_case(referer_scheme) || !cur_host.eq_ignore_ascii_case(authority)
    {
        return "/".to_string();
    }

    // Re-slice from the original (case-preserving) Referer so the returned
    // path keeps its real casing rather than the lowercased copy used for
    // scheme/host matching.
    let prefix_len = referer_scheme.len() + 3 + authority.len();
    let path_and_query = if referer.len() > prefix_len {
        &referer[prefix_len..]
    } else {
        "/"
    };

    if validate_local_redirect_url(path_and_query).is_err() {
        return "/".to_string();
    }
    path_and_query.to_string()
}

fn validate_external_redirect_url(url: &str) -> Result<(), String> {
    if url.is_empty() || has_redirect_control_chars(url) {
        return Err("redirect_external() expects a non-empty URL".to_string());
    }

    let lower = url.to_ascii_lowercase();
    let Some(rest) = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
    else {
        return Err("redirect_external() only accepts http:// or https:// URLs".to_string());
    };

    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err("redirect_external() expects an absolute URL with a host".to_string());
    }

    Ok(())
}

/// Insert `delimiter` every three digits from the right of the integer string
/// `int_str`, preserving a leading `-` sign. Content that isn't a plain run of
/// digits is returned unchanged (so non-numeric strings pass through as-is).
fn group_integer_str(int_str: &str, delimiter: &str) -> String {
    let (sign, digits) = match int_str.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", int_str),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return int_str.to_string();
    }
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    let len = digits.len();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            grouped.push_str(delimiter);
        }
        grouped.push(ch);
    }
    format!("{}{}", sign, grouped)
}

// ===== Pagination view helper implementation =====

/// Build a page URL by taking a base path and setting/replacing the page param.
/// Preserves other query params when possible.
fn build_pagination_url(base: &str, page: i64, param_name: &str) -> String {
    if page <= 0 {
        return base.to_string();
    }

    // Split base into path and query
    let (path_part, query_part) = if let Some(qi) = base.find('?') {
        (&base[..qi], Some(&base[qi + 1..]))
    } else {
        (base, None)
    };

    // Collect kept query params (drop the target page param)
    let mut kept: Vec<String> = Vec::new();
    if let Some(qs) = query_part {
        for part in qs.split('&') {
            if !part.starts_with(&format!("{}=", param_name)) && !part.is_empty() {
                kept.push(part.to_string());
            }
        }
    }

    // Add the page param
    kept.push(format!("{}={}", param_name, page));

    if kept.is_empty() {
        path_part.to_string()
    } else {
        format!("{}?{}", path_part, kept.join("&"))
    }
}

/// Render pagination HTML from the pagination metadata.
/// Accepts either the full paginate result hash or the inner "pagination" hash.
/// Read an option value by key from a trailing options hash (`Value::Hash`).
/// Returns None when `opts` isn't a hash or the key is absent. Shared by the
/// view helpers (paginate / component) so option reads have one typed home.
fn opt_value(opts: &Value, key: &str) -> Option<Value> {
    match opts {
        Value::Hash(h) => h.borrow().get(&HashKey::String(key.into())).cloned(),
        _ => None,
    }
}

/// Read a string option (`opt_value` + string-type check).
fn opt_str(opts: &Value, key: &str) -> Option<String> {
    match opt_value(opts, key) {
        Some(Value::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

/// Read an integer option.
fn opt_int(opts: &Value, key: &str) -> Option<i64> {
    match opt_value(opts, key) {
        Some(Value::Int(n)) => Some(n),
        _ => None,
    }
}

fn paginate_html(pag_data: &Value, opts: &Value) -> Result<String, String> {
    // Extract pagination sub object
    let pagination = if let Value::Hash(h) = pag_data {
        let borrowed = h.borrow();
        if let Some(p) = borrowed.get(&HashKey::String("pagination".into())) {
            p.clone()
        } else {
            pag_data.clone()
        }
    } else {
        pag_data.clone()
    };

    let (page, total_pages) = if let Value::Hash(h) = &pagination {
        let b = h.borrow();
        let pg = match b.get(&HashKey::String("page".into())) {
            Some(Value::Int(n)) => *n,
            _ => 1,
        };
        let tp = match b.get(&HashKey::String("total_pages".into())) {
            Some(Value::Int(n)) => *n,
            _ => 1,
        };
        (pg, tp.max(1))
    } else {
        (1, 1)
    };

    if total_pages <= 1 {
        return Ok(String::new());
    }

    // Options
    let param = opt_str(opts, "param").unwrap_or_else(|| "page".to_string());
    let window = opt_int(opts, "window")
        .map(|n| (n as usize).clamp(0, 10))
        .unwrap_or(2);
    let extra_class = opt_str(opts, "class")
        .map(|s| format!(" {}", s))
        .unwrap_or_default();
    let base_path =
        opt_str(opts, "path").unwrap_or_else(|| match current_request_string_field("path") {
            Value::String(p) => p.to_string(),
            _ => "/".to_string(),
        });

    let mut links = Vec::new();

    // Prev
    if page > 1 {
        let url = build_pagination_url(&base_path, page - 1, &param);
        links.push(format!(
            r#"<a href="{}" class="page prev" rel="prev">Previous</a>"#,
            html::html_escape(&url)
        ));
    } else {
        links.push(r#"<span class="page prev disabled">Previous</span>"#.to_string());
    }

    // Page numbers with window
    let start = (page as isize - window as isize).max(1) as i64;
    let end = (page + window as i64).min(total_pages);

    if start > 1 {
        let url = build_pagination_url(&base_path, 1, &param);
        links.push(format!(
            r#"<a href="{}" class="page">1</a>"#,
            html::html_escape(&url)
        ));
        if start > 2 {
            links.push(r#"<span class="gap">…</span>"#.to_string());
        }
    }

    for p in start..=end {
        if p == page {
            links.push(format!(
                r#"<span class="page current" aria-current="page">{}</span>"#,
                p
            ));
        } else {
            let url = build_pagination_url(&base_path, p, &param);
            links.push(format!(
                r#"<a href="{}" class="page">{}</a>"#,
                html::html_escape(&url),
                p
            ));
        }
    }

    if end < total_pages {
        if end < total_pages - 1 {
            links.push(r#"<span class="gap">…</span>"#.to_string());
        }
        let url = build_pagination_url(&base_path, total_pages, &param);
        links.push(format!(
            r#"<a href="{}" class="page">{}</a>"#,
            html::html_escape(&url),
            total_pages
        ));
    }

    // Next
    if page < total_pages {
        let url = build_pagination_url(&base_path, page + 1, &param);
        links.push(format!(
            r#"<a href="{}" class="page next" rel="next">Next</a>"#,
            html::html_escape(&url)
        ));
    } else {
        links.push(r#"<span class="page next disabled">Next</span>"#.to_string());
    }

    let inner = links.join("\n  ");
    Ok(format!(
        r#"<nav class="pagination{}" aria-label="Pagination">{}</nav>"#,
        extra_class,
        if inner.is_empty() {
            String::new()
        } else {
            format!("\n  {}\n", inner)
        }
    ))
}

// ===== end pagination helper =====

/// Maximum size of the serialized locals shipped to the e2e test client via
/// the `x-soli-test-assigns` response header. Beyond this we fall back to a
/// keys-only payload so a single huge collection can't blow the header limit.
const MAX_CAPTURED_ASSIGNS_BYTES: usize = 48 * 1024;

/// Serialize render() locals to JSON for the e2e test client's `assigns()`
/// helper. Returns `(json, partial)`. Each top-level value is serialized
/// independently (a non-serializable value degrades to `null` rather than
/// nuking the whole payload). When the full payload exceeds
/// `MAX_CAPTURED_ASSIGNS_BYTES` it degrades to a keys-only object
/// (`{"posts": null, ...}`) with `partial = true`, so `has_key`-style
/// assertions still work on large collections.
pub(crate) fn capture_assigns_json(data: &Value) -> (String, bool) {
    let Value::Hash(hash) = data else {
        return ("{}".to_string(), false);
    };
    let mut obj = serde_json::Map::new();
    for (key, value) in hash.borrow().iter() {
        if let HashKey::String(name) = key {
            let json = value_to_json(value).unwrap_or(serde_json::Value::Null);
            obj.insert(name.to_string(), json);
        }
    }
    let full = serde_json::Value::Object(obj).to_string();
    if full.len() <= MAX_CAPTURED_ASSIGNS_BYTES {
        return (full, false);
    }
    let mut keys_only = serde_json::Map::new();
    for (key, _) in hash.borrow().iter() {
        if let HashKey::String(name) = key {
            keys_only.insert(name.to_string(), serde_json::Value::Null);
        }
    }
    (serde_json::Value::Object(keys_only).to_string(), true)
}

/// The view path reported to the e2e `view_path()` helper: the rendered
/// template name with the conventional `.html` extension (the `.slv` source
/// suffix and any layout are not part of it). Render names are extension-less
/// by convention (`render("posts/index", ...)`), so we append `.html` to match
/// the documented `view_path() == "posts/index.html"`. A name that already
/// carries an extension is passed through unchanged.
pub(crate) fn captured_view_path(template_name: &str) -> String {
    let last_segment = template_name.rsplit('/').next().unwrap_or(template_name);
    if last_segment.contains('.') {
        template_name.to_string()
    } else {
        format!("{}.html", template_name)
    }
}

#[cfg(test)]
mod assigns_capture_tests {
    use super::*;

    fn hash(pairs: Vec<(&str, Value)>) -> Value {
        let mut hp = HashPairs::default();
        for (key, value) in pairs {
            hp.insert(HashKey::String(key.into()), value);
        }
        Value::Hash(Rc::new(RefCell::new(hp)))
    }

    #[test]
    fn small_locals_serialize_in_full() {
        let data = hash(vec![
            ("title", Value::String("Hi".into())),
            ("count", Value::Int(3)),
        ]);
        let (json, partial) = capture_assigns_json(&data);
        assert!(!partial);
        assert!(json.contains("\"title\":\"Hi\""), "json was {json}");
        assert!(json.contains("\"count\":3"), "json was {json}");
    }

    #[test]
    fn oversized_locals_degrade_to_keys_only() {
        let big = "x".repeat(MAX_CAPTURED_ASSIGNS_BYTES + 10);
        let data = hash(vec![
            ("blob", Value::String(big.into())),
            ("title", Value::String("Hi".into())),
        ]);
        let (json, partial) = capture_assigns_json(&data);
        assert!(partial);
        // Keys preserved, values nulled — so has_key assertions still work.
        assert!(json.contains("\"blob\":null"), "json was {json}");
        assert!(json.contains("\"title\":null"), "json was {json}");
        assert!(json.len() < 1024);
    }

    #[test]
    fn non_hash_locals_serialize_to_empty_object() {
        let (json, partial) = capture_assigns_json(&Value::Int(5));
        assert_eq!(json, "{}");
        assert!(!partial);
    }

    #[test]
    fn view_path_gets_html_extension() {
        assert_eq!(captured_view_path("posts/index"), "posts/index.html");
        assert_eq!(captured_view_path("dashboard"), "dashboard.html");
        // Already-extensioned names pass through unchanged.
        assert_eq!(captured_view_path("posts/feed.json"), "posts/feed.json");
    }
}

/// Register template-related builtin functions.
pub fn register_template_builtins(env: &mut Environment) {
    // render(template, data, options?) - Render a template with data
    // Returns a response hash with status, headers, and body
    env.define(
        "render".to_string(),
        Value::NativeFunction(NativeFunction::new("render", None, |args| {
            if args.is_empty() {
                return Err("render() requires at least 1 argument (template name)".to_string());
            }

            // Get template name
            let template_name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "render() template name must be a string, got {}",
                        other.type_name()
                    ))
                }
            };

            // Get data (default to empty hash)
            let data = if args.len() > 1 {
                args[1].clone()
            } else {
                Value::Hash(Rc::new(RefCell::new(HashPairs::default())))
            };

            // Validate data is a hash
            if !matches!(data, Value::Hash(_)) {
                return Err(format!(
                    "render() data must be a hash, got {}",
                    data.type_name()
                ));
            }

            // Resolve any futures in the data before rendering
            // This ensures async operations (HTTP requests, etc.) complete before template use
            let data = resolve_futures_in_value(data);

            // Capture the pristine locals for the e2e test client *before* the
            // req/helpers/instance-var injections below — those would pollute
            // assigns() with framework internals (and unserializable native
            // functions). Only when this process is a test-runner server;
            // otherwise it's a single atomic load with zero further cost.
            let captured_assigns: Option<(String, bool)> =
                if crate::interpreter::builtins::test_server::is_test_runner_process() {
                    Some(capture_assigns_json(&data))
                } else {
                    None
                };

            // Get options (layout, status, etc.)
            let options = if args.len() > 2 {
                match &args[2] {
                    Value::Hash(h) => Some(h.clone()),
                    other => {
                        return Err(format!(
                            "render() options must be a hash, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                None
            };

            // Extract layout option - check options hash first, then data hash
            let layout = if let Some(opts) = &options {
                opts.borrow().get(&StrKey("layout")).cloned()
            } else {
                None
            };

            // If not found in options, check data hash for layout key
            let layout = if layout.is_none() {
                if let Value::Hash(data_hash) = &data {
                    data_hash.borrow().get(&StrKey("layout")).cloned()
                } else {
                    None
                }
            } else {
                layout
            };

            // Still nothing? Fall back to the controller's registered
            // layout — the `static { this.layout = "..." }` declaration on
            // the controller class. This is the last stop before the
            // "application" default, so an explicit `layout` in the
            // `render()` call or data hash always wins.
            let layout = if layout.is_none() {
                get_current_controller_registered_layout()
            } else {
                layout
            };

            // Process layout value
            let layout = match layout {
                Some(Value::String(s)) => Some(Some(s)),
                Some(Value::Bool(false)) => Some(None), // layout: false
                Some(Value::Null) => Some(None),
                None => None, // No layout specified
                _ => None,
            };

            // Extract status option (default 200)
            let status = if let Some(opts) = &options {
                if let Some(Value::Int(n)) = opts.borrow().get(&StrKey("status")) {
                    *n
                } else {
                    200
                }
            } else {
                200
            };

            // Get template cache and render
            let cache = get_template_cache()?;

            // Inject req from current thread context into data (before helpers)
            // This allows views to access req.params, req.query, etc.
            inject_request_context(&data);

            // Expose controller instance fields (e.g. `@title = "..."` in the action
            // becomes a bare `title` local in the view). Explicit render() data wins.
            inject_controller_instance_vars(&data);

            // Inject template helper functions into data context (in-place, no clone)
            inject_template_helpers(&data);

            // Convert layout option for render call
            let layout_arg = match &layout {
                Some(Some(name)) => Some(Some(name.as_ref())),
                Some(None) => Some(None),
                None => None,
            };

            // Set view context for debugging (in case of error)
            set_view_debug_context(Some(data.clone()));

            let result = {
                let _phase = crate::serve::phase_log::PhaseTimer::start("view");
                cache.render(&template_name, &data, layout_arg)
            };

            match result {
                Ok(rendered) => {
                    // Clear context on success
                    clear_current_request();
                    set_view_debug_context(None);
                    // Ship the rendered view path + locals to the e2e test
                    // client (test-runner only). Response finalization reads
                    // this and emits the `x-soli-test-*` headers.
                    if let Some((assigns_json, partial)) = captured_assigns {
                        crate::interpreter::builtins::test_server::set_captured_render(
                            captured_view_path(&template_name),
                            assigns_json,
                            partial,
                        );
                    }
                    Ok(html_response(rendered, status))
                }
                Err(e) => {
                    // Keep context set for debugging
                    clear_current_request();
                    Err(e)
                }
            }
        })),
    );

    // render_partial(name, data?) - Render a partial template (no layout).
    // Also exposed as the shorter alias `partial(...)`.
    let render_partial_fn = NativeFunction::new("render_partial", None, |args| {
        if args.is_empty() {
            return Err("render_partial() requires at least 1 argument (partial name)".to_string());
        }

        // Get partial name
        let partial_name = match &args[0] {
            Value::String(s) => s.clone(),
            other => {
                return Err(format!(
                    "render_partial() name must be a string, got {}",
                    other.type_name()
                ))
            }
        };

        // Get data (default to empty hash)
        let data = if args.len() > 1 {
            args[1].clone()
        } else {
            Value::Hash(Rc::new(RefCell::new(HashPairs::default())))
        };

        // Resolve any futures in the data before rendering
        let data = resolve_futures_in_value(data);

        // Get template cache and render
        let cache = get_template_cache()?;

        // Inject req and params from current request context into data
        inject_request_context(&data);

        // Inject template helper functions into data context (in-place)
        inject_template_helpers(&data);

        let rendered = {
            let _phase = crate::serve::phase_log::PhaseTimer::start("view");
            cache.render_partial(&partial_name, &data)?
        };

        // Return just the string for partials (they're typically embedded)
        Ok(Value::String(rendered.into()))
    });
    env.define(
        "render_partial".to_string(),
        Value::NativeFunction(render_partial_fn.clone()),
    );
    env.define(
        "partial".to_string(),
        Value::NativeFunction(render_partial_fn),
    );

    // html_escape(string) - Escape HTML special characters
    env.define(
        "html_escape".to_string(),
        Value::NativeFunction(NativeFunction::new("html_escape", Some(1), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => format!("{}", other).into(),
            };
            Ok(Value::String(
                crate::template::renderer::html_escape(&s)
                    .into_owned()
                    .into(),
            ))
        })),
    );

    // h(string) - Alias for html_escape
    env.define(
        "h".to_string(),
        Value::NativeFunction(NativeFunction::new("h", Some(1), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => format!("{}", other).into(),
            };
            Ok(Value::String(
                crate::template::renderer::html_escape(&s)
                    .into_owned()
                    .into(),
            ))
        })),
    );

    // j(string) - JavaScript escape for embedding in script blocks
    env.define(
        "j".to_string(),
        Value::NativeFunction(NativeFunction::new("j", Some(1), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => format!("{}", other).into(),
            };
            Ok(Value::String(
                crate::template::renderer::js_escape(&s).into_owned().into(),
            ))
        })),
    );

    // attr(string) - Attribute escape for embedding in HTML attribute values
    env.define(
        "attr".to_string(),
        Value::NativeFunction(NativeFunction::new("attr", Some(1), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => format!("{}", other).into(),
            };
            Ok(Value::String(
                crate::template::renderer::attr_escape(&s)
                    .into_owned()
                    .into(),
            ))
        })),
    );

    // url(string) - URL escape for embedding in query parameters
    env.define(
        "url".to_string(),
        Value::NativeFunction(NativeFunction::new("url", Some(1), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => format!("{}", other).into(),
            };
            Ok(Value::String(
                crate::template::renderer::url_escape(&s)
                    .into_owned()
                    .into(),
            ))
        })),
    );

    // range(start, end, step?) - Create a range of integers
    env.define(
        "range".to_string(),
        Value::NativeFunction(NativeFunction::new("range", None, |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(format!(
                    "range() expects 2 or 3 arguments, got {}",
                    args.len()
                ));
            }

            let start = match &args[0] {
                Value::Int(n) => *n,
                other => {
                    return Err(format!(
                        "range() expects integer start, got {}",
                        other.type_name()
                    ))
                }
            };
            let end = match &args[1] {
                Value::Int(n) => *n,
                other => {
                    return Err(format!(
                        "range() expects integer end, got {}",
                        other.type_name()
                    ))
                }
            };

            let step = if args.len() == 3 {
                match &args[2] {
                    Value::Int(n) => {
                        if *n == 0 {
                            return Err("range() step cannot be zero".to_string());
                        }
                        *n
                    }
                    other => {
                        return Err(format!(
                            "range() expects integer step, got {}",
                            other.type_name()
                        ))
                    }
                }
            } else {
                1
            };

            let mut values = Vec::new();
            if step > 0 {
                let mut i = start;
                while i < end {
                    values.push(Value::Int(i));
                    i += step;
                }
            } else {
                let mut i = start;
                while i > end {
                    values.push(Value::Int(i));
                    i += step;
                }
            }

            Ok(Value::Array(Rc::new(RefCell::new(values))))
        })),
    );

    // redirect(path) - Create a local redirect response (302 Found).
    // Accepts either a local absolute path string ("/dashboard") or the symbol
    // `:back`, which resolves to the request's Referer header when it points at
    // our own host (and falls back to "/" otherwise).
    env.define(
        "redirect".to_string(),
        Value::NativeFunction(NativeFunction::new("redirect", Some(1), |args| {
            let url = match &args[0] {
                Value::String(s) => {
                    validate_local_redirect_url(s)?;
                    s.clone()
                }
                Value::Symbol(s) if **s == *"back" => resolve_back_redirect().into(),
                Value::Symbol(s) => {
                    return Err(format!(
                        "redirect() does not understand :{s}; only :back is supported"
                    ))
                }
                other => {
                    return Err(format!(
                        "redirect() expects string URL or :back, got {}",
                        other.type_name()
                    ))
                }
            };

            Ok(redirect_response(url.to_string()))
        })),
    );

    // redirect_external(url) - Explicit escape hatch for trusted external redirects.
    env.define(
        "redirect_external".to_string(),
        Value::NativeFunction(NativeFunction::new("redirect_external", Some(1), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "redirect_external() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            validate_external_redirect_url(&url)?;
            Ok(redirect_response(url.to_string()))
        })),
    );

    // halt(status, message) - Build a plain error response hash. Used to
    // short-circuit before_action hooks (`return halt(403, "Forbidden")`) and
    // from actions that want a terse error page. The return value is a
    // response hash with `status`/`headers`/`body`, so it's recognized by
    // `check_for_response` and terminates the request immediately.
    //
    // Named `halt` (Sinatra convention) specifically because `error` is the
    // most common local name in form/validation partials, and having a global
    // builtin with that name caused silent collisions in defensive lookup
    // patterns like `defined("error") && !error.nil?`.
    env.define(
        "halt".to_string(),
        Value::NativeFunction(NativeFunction::new("halt", Some(2), |args| {
            let status = match &args[0] {
                Value::Int(n) => *n,
                other => {
                    return Err(format!(
                        "halt() expects Int status as first argument, got {}",
                        other.type_name()
                    ))
                }
            };
            let message = match &args[1] {
                Value::String(s) => s.clone(),
                other => format!("{}", other).into(),
            };

            let mut headers_map: HashPairs = HashPairs::default();
            headers_map.insert(
                HashKey::String("Content-Type".into()),
                Value::String("text/plain; charset=utf-8".into()),
            );
            let headers = Value::Hash(Rc::new(RefCell::new(headers_map)));

            let mut response_map: HashPairs = HashPairs::default();
            response_map.insert(HashKey::String("status".into()), Value::Int(status));
            response_map.insert(HashKey::String("headers".into()), headers);
            response_map.insert(HashKey::String("body".into()), Value::String(message));

            Ok(Value::Hash(Rc::new(RefCell::new(response_map))))
        })),
    );

    // forbidden(message?) - Raise an authorization error that the request
    // handler maps to a 403 response (mirrors how Model.find raises a 404).
    // Unlike `halt`, this RAISES rather than returning a hash, so it can be
    // called as a bare statement deep inside a call chain (e.g. the auth
    // Policy layer's `authorize(record)`) and still halt the request.
    env.define(
        "forbidden".to_string(),
        Value::NativeFunction(NativeFunction::new("forbidden", None, |args| {
            let message = match args.first() {
                Some(Value::String(s)) => s.to_string(),
                Some(other) => format!("{}", other),
                None => "Forbidden".to_string(),
            };
            Err(format!(
                "{}{}",
                crate::error::RuntimeError::FORBIDDEN_MARKER,
                message
            ))
        })),
    );

    // render_json(data, status?) - Render JSON response with automatic content type
    env.define(
        "render_json".to_string(),
        Value::NativeFunction(NativeFunction::new("render_json", None, |args| {
            if args.is_empty() {
                return Err("render_json() requires at least one argument".to_string());
            }

            // Auto-resolve any Futures in the data
            let data = resolve_futures_in_value(args[0].clone());
            let status = if args.len() > 1 {
                match &args[1] {
                    Value::Int(n) => *n as u16,
                    _ => 200,
                }
            } else {
                200
            };

            let json_body = match &data {
                Value::String(s) => s.clone(),
                _ => value_to_json(&data)?.to_string().into(),
            };

            // Set fast-path response to bypass Value::Hash round-trip in extract_response
            crate::interpreter::builtins::server::set_fast_path_response(
                crate::interpreter::builtins::server::FastPathResponse {
                    status,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "application/json; charset=utf-8".to_string(),
                    )],
                    body: json_body.to_string(),
                },
            );

            // Return Null since extract_response will use the fast-path
            Ok(Value::Null)
        })),
    );

    // render_text(text, status?) - Render plain text response with automatic content type
    env.define(
        "render_text".to_string(),
        Value::NativeFunction(NativeFunction::new("render_text", None, |args| {
            if args.is_empty() {
                return Err("render_text() requires at least one argument".to_string());
            }

            let text = match &args[0] {
                Value::String(s) => s.clone(),
                other => format!("{}", other).into(),
            };

            let status = if args.len() > 1 {
                match &args[1] {
                    Value::Int(n) => *n as u16,
                    _ => 200,
                }
            } else {
                200
            };

            // Set fast-path response to bypass Value::Hash round-trip in extract_response
            crate::interpreter::builtins::server::set_fast_path_response(
                crate::interpreter::builtins::server::FastPathResponse {
                    status,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "text/plain; charset=utf-8".to_string(),
                    )],
                    body: text.to_string(),
                },
            );

            // Return Null since extract_response will use the fast-path
            Ok(Value::Null)
        })),
    );

    // render_jsonp(data, status?) - Render a JSONP response wrapping the JSON in
    // the request's `?callback` function. Opt-in: JSONP is cross-origin readable,
    // so this is a dedicated helper rather than auto-enabled on render_json.
    // Falls back to a plain JSON response when no callback param is present, and
    // returns 400 (without reflecting the value) when the callback is invalid.
    env.define(
        "render_jsonp".to_string(),
        Value::NativeFunction(NativeFunction::new("render_jsonp", None, |args| {
            if args.is_empty() {
                return Err("render_jsonp() requires at least one argument".to_string());
            }

            // Auto-resolve any Futures in the data
            let data = resolve_futures_in_value(args[0].clone());
            let status = if args.len() > 1 {
                match &args[1] {
                    Value::Int(n) => *n as u16,
                    _ => 200,
                }
            } else {
                200
            };

            let json_body = match &data {
                Value::String(s) => s.to_string(),
                _ => value_to_json(&data)?.to_string(),
            };

            // Resolve the callback from `?callback`; wrap when valid, fall back
            // to plain JSON when absent, reject when present-but-invalid.
            let (status, content_type, body) = match current_request_query_param("callback") {
                None => (
                    status,
                    "application/json; charset=utf-8".to_string(),
                    json_body,
                ),
                Some(callback) if crate::interpreter::jsonp::is_valid_jsonp_callback(&callback) => {
                    (
                        status,
                        "application/javascript; charset=utf-8".to_string(),
                        // `/**/` is the standard content-sniffing hardening prefix.
                        format!("/**/{}({});", callback, json_body),
                    )
                }
                Some(_) => (
                    400,
                    "text/plain; charset=utf-8".to_string(),
                    "Invalid JSONP callback".to_string(),
                ),
            };

            crate::interpreter::builtins::server::set_fast_path_response(
                crate::interpreter::builtins::server::FastPathResponse {
                    status,
                    headers: vec![("Content-Type".to_string(), content_type)],
                    body,
                },
            );

            Ok(Value::Null)
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::builtins::controller::registry::{
        clear_current_controller, set_current_controller,
    };
    use crate::interpreter::value::{Class, Instance};

    fn make_instance(fields: &[(&str, Value)]) -> Value {
        let class = Rc::new(Class {
            name: "TestController".to_string(),
            ..Default::default()
        });
        let mut inst = Instance::new(class);
        for (k, v) in fields {
            inst.fields.insert(k.to_string(), v.clone());
        }
        Value::Instance(Rc::new(RefCell::new(inst)))
    }

    fn empty_data() -> Value {
        Value::Hash(Rc::new(RefCell::new(HashPairs::default())))
    }

    fn call_builtin(name: &str, args: Vec<Value>) -> Result<Value, String> {
        let mut env = Environment::new();
        register_template_builtins(&mut env);
        let Value::NativeFunction(function) = env.get(name).unwrap() else {
            panic!("expected native function");
        };
        (function.func)(args)
    }

    fn response_location(response: &Value) -> Option<String> {
        let Value::Hash(response_hash) = response else {
            return None;
        };
        let headers = response_hash
            .borrow()
            .get(&HashKey::String("headers".into()))?
            .clone();
        let Value::Hash(headers_hash) = headers else {
            return None;
        };
        let location = headers_hash
            .borrow()
            .get(&HashKey::String("Location".into()))
            .and_then(|value| match value {
                Value::String(location) => Some(location.clone()),
                _ => None,
            });
        location.map(|s| s.to_string())
    }

    fn get_str(data: &Value, key: &str) -> Option<String> {
        let Value::Hash(h) = data else { return None };
        h.borrow()
            .get(&HashKey::String(key.to_string().into()))
            .and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
            .map(|s| s.to_string())
    }

    #[test]
    fn redirect_accepts_local_absolute_path() {
        let response = call_builtin("redirect", vec![Value::String("/dashboard".into())])
            .expect("local redirect should succeed");

        assert_eq!(response_location(&response).as_deref(), Some("/dashboard"));
    }

    #[test]
    fn redirect_rejects_external_or_malformed_locations() {
        for url in [
            "https://example.com",
            "http://example.com",
            "//example.com",
            "dashboard",
            "/\\evil.com",
            "/ok\r\nX-Injected: yes",
        ] {
            let err = call_builtin("redirect", vec![Value::String(url.to_string().into())])
                .expect_err("redirect should reject unsafe location");
            assert!(
                err.contains("local") || err.contains("non-empty"),
                "unexpected error for {url:?}: {err}"
            );
        }
    }

    #[test]
    fn redirect_external_requires_explicit_http_url() {
        let response = call_builtin(
            "redirect_external",
            vec![Value::String("https://example.com/login".into())],
        )
        .expect("external redirect should succeed");

        assert_eq!(
            response_location(&response).as_deref(),
            Some("https://example.com/login")
        );

        for url in [
            "javascript:alert(1)",
            "//example.com",
            "https://",
            "https://user@example.com",
        ] {
            assert!(
                call_builtin(
                    "redirect_external",
                    vec![Value::String(url.to_string().into())]
                )
                .is_err(),
                "expected redirect_external to reject {url:?}"
            );
        }
    }

    fn make_request_with_referer(referer: Option<&str>) -> Value {
        let mut headers = HashPairs::default();
        if let Some(r) = referer {
            headers.insert(
                HashKey::String("referer".into()),
                Value::String(r.to_string().into()),
            );
        }
        make_request(&[("headers", Value::Hash(Rc::new(RefCell::new(headers))))])
    }

    fn with_request_host<F: FnOnce()>(scheme: &str, host: &str, f: F) {
        crate::interpreter::builtins::named_routes::set_current_request_host(
            scheme.to_string(),
            host.to_string(),
        );
        f();
        crate::interpreter::builtins::named_routes::clear_current_request_host();
    }

    #[test]
    fn redirect_back_uses_same_host_referer_path() {
        set_current_request(make_request_with_referer(Some(
            "https://app.test/posts/42?tab=comments",
        )));
        with_request_host("https", "app.test", || {
            let resp = call_builtin("redirect", vec![Value::Symbol("back".into())])
                .expect("redirect(:back) should succeed");
            assert_eq!(
                response_location(&resp).as_deref(),
                Some("/posts/42?tab=comments")
            );
        });
        clear_current_request();
    }

    #[test]
    fn redirect_back_falls_back_to_root_for_external_or_missing_referer() {
        // External Referer → "/"
        set_current_request(make_request_with_referer(Some("https://evil.test/x")));
        with_request_host("https", "app.test", || {
            let resp = call_builtin("redirect", vec![Value::Symbol("back".into())]).unwrap();
            assert_eq!(response_location(&resp).as_deref(), Some("/"));
        });

        // Scheme mismatch → "/"
        set_current_request(make_request_with_referer(Some("http://app.test/x")));
        with_request_host("https", "app.test", || {
            let resp = call_builtin("redirect", vec![Value::Symbol("back".into())]).unwrap();
            assert_eq!(response_location(&resp).as_deref(), Some("/"));
        });

        // Missing Referer → "/"
        set_current_request(make_request_with_referer(None));
        with_request_host("https", "app.test", || {
            let resp = call_builtin("redirect", vec![Value::Symbol("back".into())]).unwrap();
            assert_eq!(response_location(&resp).as_deref(), Some("/"));
        });

        // Userinfo trick → "/"
        set_current_request(make_request_with_referer(Some(
            "https://app.test@evil.test/x",
        )));
        with_request_host("https", "app.test", || {
            let resp = call_builtin("redirect", vec![Value::Symbol("back".into())]).unwrap();
            assert_eq!(response_location(&resp).as_deref(), Some("/"));
        });

        // Non-http scheme → "/"
        set_current_request(make_request_with_referer(Some("javascript:alert(1)")));
        with_request_host("https", "app.test", || {
            let resp = call_builtin("redirect", vec![Value::Symbol("back".into())]).unwrap();
            assert_eq!(response_location(&resp).as_deref(), Some("/"));
        });

        clear_current_request();
    }

    #[test]
    fn redirect_rejects_unknown_symbols() {
        let err = call_builtin("redirect", vec![Value::Symbol("forward".into())])
            .expect_err("unknown symbol must error");
        assert!(err.contains(":back"), "unexpected error: {err}");
    }

    #[test]
    fn injects_instance_fields_as_view_locals() {
        set_current_controller(make_instance(&[
            ("title", Value::String("Hi".into())),
            ("count", Value::Int(3)),
        ]));
        let data = empty_data();
        inject_controller_instance_vars(&data);
        assert_eq!(get_str(&data, "title").as_deref(), Some("Hi"));
        let Value::Hash(h) = &data else {
            unreachable!()
        };
        assert!(h.borrow().contains_key(&HashKey::String("count".into())));
        clear_current_controller();
    }

    #[test]
    fn inject_request_context_exposes_params_as_top_level_local() {
        let mut params_map = HashPairs::default();
        params_map.insert(HashKey::String("q".into()), Value::String("search".into()));
        set_current_request(make_request(&[(
            "params",
            Value::Hash(Rc::new(RefCell::new(params_map))),
        )]));
        let data = empty_data();
        inject_request_context(&data);
        let Value::Hash(h) = &data else {
            unreachable!()
        };
        let h = h.borrow();
        assert!(
            h.contains_key(&HashKey::String("params".into())),
            "params should be injected as top-level view local"
        );
        let params = h.get(&HashKey::String("params".into())).unwrap();
        let Value::Hash(params_hash) = params else {
            unreachable!("params should be a Hash")
        };
        let params = params_hash.borrow();
        assert_eq!(
            params.get(&HashKey::String("q".into())),
            Some(&Value::String("search".into()))
        );
        clear_current_request();
    }

    #[test]
    fn explicit_render_data_wins_over_instance_field() {
        set_current_controller(make_instance(&[(
            "title",
            Value::String("from_instance".into()),
        )]));
        let data = empty_data();
        if let Value::Hash(h) = &data {
            h.borrow_mut().insert(
                HashKey::String("title".into()),
                Value::String("from_render".into()),
            );
        }
        inject_controller_instance_vars(&data);
        assert_eq!(get_str(&data, "title").as_deref(), Some("from_render"));
        clear_current_controller();
    }

    #[test]
    fn skips_framework_injected_fields() {
        // req/params/session/headers are populated by setup_controller_context on
        // the instance. The template layer already injects request context separately,
        // so these must not be re-exposed as top-level view locals from the instance.
        set_current_controller(make_instance(&[
            ("req", Value::String("should_not_leak".into())),
            ("params", Value::String("should_not_leak".into())),
            ("session", Value::String("should_not_leak".into())),
            ("headers", Value::String("should_not_leak".into())),
            ("title", Value::String("ok".into())),
        ]));
        let data = empty_data();
        inject_controller_instance_vars(&data);
        assert_eq!(get_str(&data, "title").as_deref(), Some("ok"));
        let Value::Hash(h) = &data else {
            unreachable!()
        };
        let h = h.borrow();
        for k in ["req", "params", "session", "headers"] {
            assert!(
                !h.contains_key(&HashKey::String(k.to_string().into())),
                "framework field {} leaked into view locals",
                k
            );
        }
        clear_current_controller();
    }

    #[test]
    fn noop_when_no_current_controller() {
        clear_current_controller();
        let data = empty_data();
        inject_controller_instance_vars(&data);
        let Value::Hash(h) = &data else {
            unreachable!()
        };
        assert!(h.borrow().is_empty());
    }

    fn make_request(fields: &[(&str, Value)]) -> Value {
        let mut map = HashPairs::default();
        for (k, v) in fields {
            map.insert(HashKey::String(k.to_string().into()), v.clone());
        }
        Value::Hash(Rc::new(RefCell::new(map)))
    }

    #[test]
    fn current_request_string_field_reads_live_field() {
        set_current_request(make_request(&[
            ("path", Value::String("/users".into())),
            ("method", Value::String("GET".into())),
        ]));
        assert_eq!(
            current_request_string_field("path"),
            Value::String("/users".into())
        );
        assert_eq!(
            current_request_string_field("method"),
            Value::String("GET".into())
        );
        clear_current_request();
    }

    #[test]
    fn current_request_string_field_returns_null_when_absent() {
        clear_current_request();
        assert_eq!(current_request_string_field("path"), Value::Null);

        // Present request, missing field.
        set_current_request(make_request(&[("path", Value::String("/x".into()))]));
        assert_eq!(current_request_string_field("missing"), Value::Null);

        // Present field, wrong type — treated as absent rather than coerced.
        set_current_request(make_request(&[("path", Value::Int(42))]));
        assert_eq!(current_request_string_field("path"), Value::Null);

        clear_current_request();
    }

    /// Build a request whose `query` sub-hash carries the given `?callback` value
    /// (or no callback at all), for the render_jsonp tests.
    fn make_request_with_callback(callback: Option<&str>) -> Value {
        let mut query = HashPairs::default();
        if let Some(cb) = callback {
            query.insert(HashKey::String("callback".into()), Value::String(cb.into()));
        }
        let query_val = Value::Hash(Rc::new(RefCell::new(query)));
        make_request(&[("query", query_val)])
    }

    fn one_key_data() -> Value {
        let mut map = HashPairs::default();
        map.insert(HashKey::String("n".into()), Value::Int(1));
        Value::Hash(Rc::new(RefCell::new(map)))
    }

    fn take_fast_path() -> crate::interpreter::builtins::server::FastPathResponse {
        crate::interpreter::builtins::server::take_fast_path_response()
            .expect("render_jsonp should set a fast-path response")
    }

    fn content_type(resp: &crate::interpreter::builtins::server::FastPathResponse) -> Option<&str> {
        resp.headers
            .iter()
            .find(|(k, _)| k == "Content-Type")
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn render_jsonp_wraps_when_callback_present() {
        set_current_request(make_request_with_callback(Some("handleData")));
        let result = call_builtin("render_jsonp", vec![one_key_data()]).expect("render_jsonp ok");
        assert_eq!(result, Value::Null);

        let resp = take_fast_path();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, "/**/handleData({\"n\":1});");
        assert_eq!(
            content_type(&resp),
            Some("application/javascript; charset=utf-8")
        );
        clear_current_request();
    }

    #[test]
    fn render_jsonp_falls_back_to_json_without_callback() {
        set_current_request(make_request_with_callback(None));
        call_builtin("render_jsonp", vec![one_key_data()]).expect("render_jsonp ok");

        let resp = take_fast_path();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.body, "{\"n\":1}");
        assert_eq!(content_type(&resp), Some("application/json; charset=utf-8"));
        clear_current_request();
    }

    #[test]
    fn render_jsonp_rejects_invalid_callback_without_reflecting() {
        set_current_request(make_request_with_callback(Some("alert(document.cookie)")));
        call_builtin("render_jsonp", vec![one_key_data()]).expect("render_jsonp ok");

        let resp = take_fast_path();
        assert_eq!(resp.status, 400);
        assert_eq!(resp.body, "Invalid JSONP callback");
        // The attacker-supplied name must never appear in the response body.
        assert!(!resp.body.contains("alert"));
        assert_eq!(content_type(&resp), Some("text/plain; charset=utf-8"));
        clear_current_request();
    }

    #[test]
    fn number_with_delimiter_groups_thousands() {
        // Core grouping used by the number_with_delimiter() view helper.
        assert_eq!(group_integer_str("0", ","), "0");
        assert_eq!(group_integer_str("100", ","), "100");
        assert_eq!(group_integer_str("1000", ","), "1,000");
        assert_eq!(group_integer_str("1234567", ","), "1,234,567");
        // Sign is preserved, grouping applies to the digits.
        assert_eq!(group_integer_str("-1234567", ","), "-1,234,567");
        // Custom delimiters (locale variants).
        assert_eq!(group_integer_str("1234567", " "), "1 234 567");
        assert_eq!(group_integer_str("1234", "."), "1.234");
        // Non-numeric content passes through untouched.
        assert_eq!(group_integer_str("abc", ","), "abc");
    }

    #[test]
    fn opt_helpers_read_typed_options() {
        let mut h: HashPairs = HashPairs::default();
        h.insert(HashKey::String("param".into()), Value::String("q".into()));
        h.insert(HashKey::String("window".into()), Value::Int(5));
        let opts = Value::Hash(Rc::new(RefCell::new(h)));
        assert_eq!(opt_str(&opts, "param").as_deref(), Some("q"));
        assert_eq!(opt_int(&opts, "window"), Some(5));
        assert_eq!(opt_str(&opts, "missing"), None);
        assert_eq!(opt_int(&opts, "param"), None); // wrong type
        assert_eq!(opt_str(&Value::Null, "param"), None); // not a hash
    }

    #[test]
    fn paginate_helper_renders_links() {
        let mut p: HashPairs = HashPairs::default();
        p.insert(HashKey::String("page".into()), Value::Int(2));
        p.insert(HashKey::String("per".into()), Value::Int(10));
        p.insert(HashKey::String("total".into()), Value::Int(45));
        p.insert(HashKey::String("total_pages".into()), Value::Int(5));
        let pagination = Value::Hash(Rc::new(RefCell::new(p)));

        let html = paginate_html(
            &pagination,
            &Value::Hash(Rc::new(RefCell::new(HashPairs::default()))),
        )
        .unwrap();
        assert!(html.contains("pagination"));
        assert!(html.contains("Previous"));
        assert!(html.contains("Next"));
        assert!(html.contains("page=1"));
        assert!(html.contains("page=3"));
        assert!(html.contains(r#"class="page current""#));
    }
}
