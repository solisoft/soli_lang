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
    if !helpers_dir.exists() {
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

    for entry in std::fs::read_dir(helpers_dir)
        .map_err(|e| format!("Failed to read helpers directory: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "sl") {
            let source = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read helper file '{}': {}", path.display(), e))?;

            // Lex
            let tokens = crate::lexer::Scanner::new(&source)
                .scan_tokens()
                .map_err(|e| format!("Lexer error in {}: {}", path.display(), e))?;

            // Parse
            let program = crate::parser::Parser::new(tokens)
                .parse()
                .map_err(|e| format!("Parser error in {}: {}", path.display(), e))?;

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
        HashKey::String("status".to_string()),
        Value::Int(status_code as i64),
    );
    error_map.insert(
        HashKey::String("message".to_string()),
        Value::String(message.to_string()),
    );
    error_map.insert(
        HashKey::String("request_id".to_string()),
        Value::String(request_id.to_string()),
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
            Err(e) => Value::String(format!("<future error: {}>", e)),
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
                    let key = HashKey::String(name.clone());
                    if !h.contains_key(&key) {
                        h.insert(key, value.clone());
                    }
                }
            }
        });
    }
}

/// Inject req and params from current request context into data hash.
fn inject_request_context(data: &Value) {
    if let Value::Hash(hash) = data {
        if let Some(req) = get_current_request() {
            let mut h = hash.borrow_mut();
            let req_key = HashKey::String("req".to_string());
            if !h.contains_key(&req_key) {
                h.insert(req_key, req.clone());
            }

            // Extract params from req and inject as top-level "params"
            if let Value::Hash(req_hash) = &req {
                if let Some(params) = req_hash
                    .borrow()
                    .get(&HashKey::String("params".to_string()))
                {
                    let params_key = HashKey::String("params".to_string());
                    if !h.contains_key(&params_key) {
                        h.insert(params_key, params.clone());
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
    match borrowed.get(&HashKey::String(field.to_string())) {
        Some(Value::String(s)) => Value::String(s.clone()),
        _ => Value::Null,
    }
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
    let registry =
        crate::interpreter::builtins::controller::registry::CONTROLLER_REGISTRY
            .read()
            .ok()?;
    let info = registry.get_by_name(&class_name)?;
    info.layout.clone().map(Value::String)
}

/// Expose the current controller's instance fields as view locals.
/// Mirrors Rails' `@ivar → view local` behavior, scoped to the action currently running.
/// Skips framework-injected fields already supplied by `inject_request_context` and
/// never clobbers keys the action passed explicitly to `render(...)`.
fn inject_controller_instance_vars(data: &Value) {
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
        let key = HashKey::String(name.clone());
        if !h.contains_key(&key) {
            h.insert(key, value.clone());
        }
    }
}

/// Register static template helpers into an Environment (called once per thread).
/// These helpers (range, public_path, html_escape, etc.) are created once and
/// shared via the thread-local builtins Rc, avoiding ~20 NativeFunction allocations per render.
pub fn register_static_template_helpers(env: &mut Environment) {
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
            let full_path = public_dir.join(&path);
            match get_file_mtime_cached(&full_path) {
                Ok(mtime) => {
                    if path.contains('?') {
                        Ok(Value::String(format!("/{}&v={}", path, mtime)))
                    } else {
                        Ok(Value::String(format!("/{}?v={}", path, mtime)))
                    }
                }
                Err(_) => Ok(Value::String(format!("/{}", path))),
            }
        })),
    );

    env.define(
        "strip_html".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "strip_html",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(html::strip_html(s))),
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
            Ok(Value::String(html::substring(&s, start, end)))
        })),
    );

    env.define(
        "html_escape".to_string(),
        Value::NativeFunction(NativeFunction::new("html_escape", Some(1), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => format!("{}", other),
            };
            Ok(Value::String(html::html_escape(&s)))
        })),
    );

    env.define(
        "html_unescape".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "html_unescape",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(html::html_unescape(s))),
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
                Value::String(s) => Ok(Value::String(html::sanitize_html(s))),
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
        Ok(Value::String(datetime_helpers::datetime_format(timestamp, &format)))
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
            Ok(Value::String(datetime_helpers::time_ago_localized(
                timestamp, &locale,
            )))
        })),
    );

    env.define(
        "locale".to_string(),
        Value::NativeFunction(NativeFunction::new("locale", Some(0), |_args| {
            Ok(Value::String(i18n_helpers::get_locale()))
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
                    _ => "short".to_string(),
                }
            } else {
                "short".to_string()
            };
            let locale = i18n_helpers::get_locale();
            Ok(Value::String(datetime_helpers::localize_date(
                timestamp, &locale, &format,
            )))
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
            .map(Value::String)
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
                Some(Some(name)) => Some(Some(name.as_str())),
                Some(None) => Some(None),
                None => None,
            };

            // Set view context for debugging (in case of error)
            set_view_debug_context(Some(data.clone()));

            let result = cache.render(&template_name, &data, layout_arg);

            match result {
                Ok(rendered) => {
                    // Clear context on success
                    clear_current_request();
                    set_view_debug_context(None);
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

        let rendered = cache.render_partial(&partial_name, &data)?;

        // Return just the string for partials (they're typically embedded)
        Ok(Value::String(rendered))
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
                other => format!("{}", other),
            };
            Ok(Value::String(
                crate::template::renderer::html_escape(&s).into_owned(),
            ))
        })),
    );

    // h(string) - Alias for html_escape
    env.define(
        "h".to_string(),
        Value::NativeFunction(NativeFunction::new("h", Some(1), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => format!("{}", other),
            };
            Ok(Value::String(
                crate::template::renderer::html_escape(&s).into_owned(),
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

    // redirect(url) - Create a redirect response (302 Found)
    env.define(
        "redirect".to_string(),
        Value::NativeFunction(NativeFunction::new("redirect", Some(1), |args| {
            let url = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "redirect() expects string URL, got {}",
                        other.type_name()
                    ))
                }
            };

            let mut headers_map: HashPairs = HashPairs::default();
            headers_map.insert(HashKey::String("Location".to_string()), Value::String(url));
            let headers = Value::Hash(Rc::new(RefCell::new(headers_map)));

            let mut response_map: HashPairs = HashPairs::default();
            response_map.insert(HashKey::String("status".to_string()), Value::Int(302));
            response_map.insert(HashKey::String("headers".to_string()), headers);
            response_map.insert(
                HashKey::String("body".to_string()),
                Value::String(String::new()),
            );

            Ok(Value::Hash(Rc::new(RefCell::new(response_map))))
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
                other => format!("{}", other),
            };

            let mut headers_map: HashPairs = HashPairs::default();
            headers_map.insert(
                HashKey::String("Content-Type".to_string()),
                Value::String("text/plain; charset=utf-8".to_string()),
            );
            let headers = Value::Hash(Rc::new(RefCell::new(headers_map)));

            let mut response_map: HashPairs = HashPairs::default();
            response_map.insert(HashKey::String("status".to_string()), Value::Int(status));
            response_map.insert(HashKey::String("headers".to_string()), headers);
            response_map.insert(HashKey::String("body".to_string()), Value::String(message));

            Ok(Value::Hash(Rc::new(RefCell::new(response_map))))
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
                _ => value_to_json(&data)?.to_string(),
            };

            // Set fast-path response to bypass Value::Hash round-trip in extract_response
            crate::interpreter::builtins::server::set_fast_path_response(
                crate::interpreter::builtins::server::FastPathResponse {
                    status,
                    headers: vec![(
                        "Content-Type".to_string(),
                        "application/json; charset=utf-8".to_string(),
                    )],
                    body: json_body,
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
                other => format!("{}", other),
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
                    body: text,
                },
            );

            // Return Null since extract_response will use the fast-path
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

    fn get_str(data: &Value, key: &str) -> Option<String> {
        let Value::Hash(h) = data else { return None };
        h.borrow()
            .get(&HashKey::String(key.to_string()))
            .and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                _ => None,
            })
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
        assert!(h
            .borrow()
            .contains_key(&HashKey::String("count".to_string())));
        clear_current_controller();
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
                HashKey::String("title".to_string()),
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
                !h.contains_key(&HashKey::String(k.to_string())),
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
            map.insert(HashKey::String(k.to_string()), v.clone());
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
}
