//! Template rendering builtins for Soli MVC.
//!
//! Provides the `render()` function for use in controllers to render templates.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use std::path::Path;

use crate::ast::stmt::StmtKind;
use crate::interpreter::builtins::datetime::helpers as datetime_helpers;
use crate::interpreter::builtins::html;
use crate::interpreter::builtins::i18n::helpers as i18n_helpers;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Function, NativeFunction, Value};
use crate::template::{html_response, TemplateCache};

// Thread-local template cache
thread_local! {
    static TEMPLATE_CACHE: RefCell<Option<Rc<TemplateCache>>> = const { RefCell::new(None) };
}

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

/// Load view helpers from a directory (app/helpers/*.sl).
/// Parses each file and extracts function definitions without executing in interpreter.
pub fn load_view_helpers(helpers_dir: &Path) -> Result<usize, String> {
    if !helpers_dir.exists() {
        return Ok(0);
    }

    let mut count = 0;

    // Create a minimal environment for helper functions (they can call each other)
    let helper_env = Rc::new(RefCell::new(Environment::new()));

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

    // Create thread-local cache
    TEMPLATE_CACHE.with(|cache| {
        *cache.borrow_mut() = Some(Rc::new(TemplateCache::new(views_dir)));
    });
}

/// Initialize the public directory for public_path() helper.
pub fn init_public_dir(public_dir: PathBuf) {
    if let Ok(mut dir) = PUBLIC_DIR.lock() {
        *dir = Some(public_dir);
    }
}

/// Clear the template cache (for hot reload).
pub fn clear_template_cache() {
    TEMPLATE_CACHE.with(|cache| {
        if let Some(tc) = cache.borrow().as_ref() {
            tc.clear();
        }
    });
}

/// Check if templates have changes (for hot reload).
pub fn templates_have_changes() -> bool {
    TEMPLATE_CACHE.with(|cache| {
        cache
            .borrow()
            .as_ref()
            .map(|tc| tc.has_changes())
            .unwrap_or(false)
    })
}

/// Get the template cache, initializing if necessary.
pub fn get_template_cache() -> Result<Rc<TemplateCache>, String> {
    TEMPLATE_CACHE.with(|cache| {
        let cache_ref = cache.borrow();
        if let Some(tc) = cache_ref.as_ref() {
            return Ok(tc.clone());
        }
        drop(cache_ref);

        // Try to initialize from global views dir
        if let Ok(dir_guard) = VIEWS_DIR.lock() {
            if let Some(views_dir) = dir_guard.as_ref() {
                let views_dir_clone = views_dir.clone();
                drop(dir_guard);
                let tc = Rc::new(TemplateCache::new(views_dir_clone));
                *cache.borrow_mut() = Some(tc.clone());
                return Ok(tc);
            }
        }

        Err("Template system not initialized. Call init_templates() first.".to_string())
    })
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
    let error_data = Value::Hash(Rc::new(RefCell::new(vec![
        (
            Value::String("status".to_string()),
            Value::Int(status_code as i64),
        ),
        (
            Value::String("message".to_string()),
            Value::String(message.to_string()),
        ),
        (
            Value::String("request_id".to_string()),
            Value::String(request_id.to_string()),
        ),
    ])));

    // Try to render the template without layout (error pages should be standalone)
    match template_cache.render(&template_name, &error_data, Some(None)) {
        Ok(content) => Some(content),
        Err(_) => None,
    }
}

/// Recursively resolve all Future values in a Value.
/// This ensures that async operations (like HTTP requests) are completed
/// before the data is used in templates.
fn resolve_futures_in_value(value: Value) -> Value {
    match value {
        Value::Future(_) => {
            // Resolve the future, blocking until complete
            match value.resolve() {
                Ok(resolved) => resolve_futures_in_value(resolved),
                Err(e) => Value::String(format!("<future error: {}>", e)),
            }
        }
        Value::Hash(hash) => {
            let resolved_pairs: Vec<(Value, Value)> = hash
                .borrow()
                .iter()
                .map(|(k, v)| {
                    let resolved_v = resolve_futures_in_value(v.clone());
                    (k.clone(), resolved_v)
                })
                .collect();
            Value::Hash(Rc::new(RefCell::new(resolved_pairs)))
        }
        Value::Array(arr) => {
            let resolved_items: Vec<Value> = arr
                .borrow()
                .iter()
                .map(|v| resolve_futures_in_value(v.clone()))
                .collect();
            Value::Array(Rc::new(RefCell::new(resolved_items)))
        }
        // Other value types don't contain futures
        other => other,
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

/// Inject template helper functions into the data context
fn inject_template_helpers(data: &Value) -> Value {
    match data {
        Value::Hash(hash) => {
            let mut new_hash: Vec<(Value, Value)> = hash.borrow().clone();

            // Inject user-defined view helpers from app/helpers/*.sl
            VIEW_HELPERS.with(|helpers| {
                let helpers_map = helpers.borrow();
                for (name, value) in helpers_map.iter() {
                    let key = Value::String(name.clone());
                    // Only add if not already present in data (allow override)
                    let exists = hash.borrow().iter().any(|(k, _)| k.hash_eq(&key));
                    if !exists {
                        new_hash.push((key, value.clone()));
                    }
                }
            });

            // Check if public_path already exists
            let public_path_key = Value::String("public_path".to_string());
            let has_public_path = hash
                .borrow()
                .iter()
                .any(|(k, _)| k.hash_eq(&public_path_key));

            // Add range() function if not present
            let range_key = Value::String("range".to_string());
            let has_range = hash.borrow().iter().any(|(k, _)| k.hash_eq(&range_key));

            if !has_range {
                let range_func =
                    Value::NativeFunction(NativeFunction::new("range", Some(2), |args| {
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

                        let values: Vec<Value> = (start..end).map(Value::Int).collect();
                        Ok(Value::Array(Rc::new(RefCell::new(values))))
                    }));

                new_hash.push((range_key, range_func));
            }

            if !has_public_path {
                // Create the public_path native function
                let public_path_func =
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

                        // Get public directory
                        let public_dir = if let Ok(dir_guard) = PUBLIC_DIR.lock() {
                            dir_guard.clone()
                        } else {
                            None
                        };

                        let public_dir = match public_dir {
                            Some(dir) => dir,
                            None => {
                                // Default to "public" in current directory
                                PathBuf::from("public")
                            }
                        };

                        // Build full file path
                        let full_path = public_dir.join(&path);

                        // Get mtime if file exists, otherwise return path without version
                        match get_file_mtime_cached(&full_path) {
                            Ok(mtime) => {
                                // Return path with query parameter
                                if path.contains('?') {
                                    Ok(Value::String(format!("/{}&v={}", path, mtime)))
                                } else {
                                    Ok(Value::String(format!("/{}?v={}", path, mtime)))
                                }
                            }
                            Err(_) => {
                                // File doesn't exist, return path without version
                                Ok(Value::String(format!("/{}", path)))
                            }
                        }
                    }));

                new_hash.push((public_path_key, public_path_func));
            }

            // Add strip_html() function if not present
            let strip_html_key = Value::String("strip_html".to_string());
            let has_strip_html = hash
                .borrow()
                .iter()
                .any(|(k, _)| k.hash_eq(&strip_html_key));

            if !has_strip_html {
                let strip_html_func =
                    Value::NativeFunction(NativeFunction::new("strip_html", Some(1), |args| {
                        match &args[0] {
                            Value::String(s) => Ok(Value::String(html::strip_html(s))),
                            other => Err(format!(
                                "strip_html() expects string, got {}",
                                other.type_name()
                            )),
                        }
                    }));
                new_hash.push((strip_html_key, strip_html_func));
            }

            // Add substring() function if not present
            let substring_key = Value::String("substring".to_string());
            let has_substring = hash.borrow().iter().any(|(k, _)| k.hash_eq(&substring_key));

            if !has_substring {
                let substring_func =
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
                    }));
                new_hash.push((substring_key, substring_func));
            }

            // Add html_escape() function if not present
            let html_escape_key = Value::String("html_escape".to_string());
            let has_html_escape = hash
                .borrow()
                .iter()
                .any(|(k, _)| k.hash_eq(&html_escape_key));

            if !has_html_escape {
                let html_escape_func =
                    Value::NativeFunction(NativeFunction::new("html_escape", Some(1), |args| {
                        let s = match &args[0] {
                            Value::String(s) => s.clone(),
                            other => format!("{}", other),
                        };
                        Ok(Value::String(html::html_escape(&s)))
                    }));
                new_hash.push((html_escape_key, html_escape_func));
            }

            // Add html_unescape() function if not present
            let html_unescape_key = Value::String("html_unescape".to_string());
            let has_html_unescape = hash
                .borrow()
                .iter()
                .any(|(k, _)| k.hash_eq(&html_unescape_key));

            if !has_html_unescape {
                let html_unescape_func =
                    Value::NativeFunction(NativeFunction::new("html_unescape", Some(1), |args| {
                        match &args[0] {
                            Value::String(s) => Ok(Value::String(html::html_unescape(s))),
                            other => Err(format!(
                                "html_unescape() expects string, got {}",
                                other.type_name()
                            )),
                        }
                    }));
                new_hash.push((html_unescape_key, html_unescape_func));
            }

            // Add sanitize_html() function if not present
            let sanitize_html_key = Value::String("sanitize_html".to_string());
            let has_sanitize_html = hash
                .borrow()
                .iter()
                .any(|(k, _)| k.hash_eq(&sanitize_html_key));

            if !has_sanitize_html {
                let sanitize_html_func =
                    Value::NativeFunction(NativeFunction::new("sanitize_html", Some(1), |args| {
                        match &args[0] {
                            Value::String(s) => Ok(Value::String(html::sanitize_html(s))),
                            other => Err(format!(
                                "sanitize_html() expects string, got {}",
                                other.type_name()
                            )),
                        }
                    }));
                new_hash.push((sanitize_html_key, sanitize_html_func));
            }

            // Add datetime_now() function if not present
            let datetime_now_key = Value::String("datetime_now".to_string());
            let has_datetime_now = hash
                .borrow()
                .iter()
                .any(|(k, _)| k.hash_eq(&datetime_now_key));

            if !has_datetime_now {
                let datetime_now_func =
                    Value::NativeFunction(NativeFunction::new("datetime_now", Some(0), |_args| {
                        Ok(Value::Int(datetime_helpers::datetime_now()))
                    }));
                new_hash.push((datetime_now_key, datetime_now_func));
            }

            // Add datetime_format() function if not present
            let datetime_format_key = Value::String("datetime_format".to_string());
            let has_datetime_format = hash
                .borrow()
                .iter()
                .any(|(k, _)| k.hash_eq(&datetime_format_key));

            if !has_datetime_format {
                let datetime_format_func = Value::NativeFunction(NativeFunction::new(
                    "datetime_format",
                    Some(2),
                    |args| {
                        let timestamp = match &args[0] {
                            Value::Int(n) => *n,
                            Value::String(s) => {
                                // Try to parse string as timestamp
                                datetime_helpers::datetime_parse(s).unwrap_or(0)
                            }
                            other => {
                                return Err(format!(
                                    "datetime_format() expects timestamp (int) or date string as first argument, got {}",
                                    other.type_name()
                                ))
                            }
                        };
                        let format = match &args[1] {
                            Value::String(s) => s.clone(),
                            other => {
                                return Err(format!(
                                    "datetime_format() expects string format as second argument, got {}",
                                    other.type_name()
                                ))
                            }
                        };
                        Ok(Value::String(datetime_helpers::datetime_format(
                            timestamp, &format,
                        )))
                    },
                ));
                new_hash.push((datetime_format_key, datetime_format_func));
            }

            // Add datetime_parse() function if not present
            let datetime_parse_key = Value::String("datetime_parse".to_string());
            let has_datetime_parse = hash
                .borrow()
                .iter()
                .any(|(k, _)| k.hash_eq(&datetime_parse_key));

            if !has_datetime_parse {
                let datetime_parse_func =
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
                    }));
                new_hash.push((datetime_parse_key, datetime_parse_func));
            }

            // Add datetime_add_days() function if not present
            let datetime_add_days_key = Value::String("datetime_add_days".to_string());
            let has_datetime_add_days = hash
                .borrow()
                .iter()
                .any(|(k, _)| k.hash_eq(&datetime_add_days_key));

            if !has_datetime_add_days {
                let datetime_add_days_func = Value::NativeFunction(NativeFunction::new(
                    "datetime_add_days",
                    Some(2),
                    |args| {
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
                    },
                ));
                new_hash.push((datetime_add_days_key, datetime_add_days_func));
            }

            // Add datetime_add_hours() function if not present
            let datetime_add_hours_key = Value::String("datetime_add_hours".to_string());
            let has_datetime_add_hours = hash
                .borrow()
                .iter()
                .any(|(k, _)| k.hash_eq(&datetime_add_hours_key));

            if !has_datetime_add_hours {
                let datetime_add_hours_func = Value::NativeFunction(NativeFunction::new(
                    "datetime_add_hours",
                    Some(2),
                    |args| {
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
                    },
                ));
                new_hash.push((datetime_add_hours_key, datetime_add_hours_func));
            }

            // Add datetime_diff() function if not present
            let datetime_diff_key = Value::String("datetime_diff".to_string());
            let has_datetime_diff = hash
                .borrow()
                .iter()
                .any(|(k, _)| k.hash_eq(&datetime_diff_key));

            if !has_datetime_diff {
                let datetime_diff_func = Value::NativeFunction(NativeFunction::new(
                    "datetime_diff",
                    Some(2),
                    |args| {
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
                    },
                ));
                new_hash.push((datetime_diff_key, datetime_diff_func));
            }

            // Add time_ago() function if not present
            let time_ago_key = Value::String("time_ago".to_string());
            let has_time_ago = hash.borrow().iter().any(|(k, _)| k.hash_eq(&time_ago_key));

            if !has_time_ago {
                let time_ago_func =
                    Value::NativeFunction(NativeFunction::new("time_ago", Some(1), |args| {
                        let timestamp = match &args[0] {
                            Value::Int(n) => *n,
                            Value::String(s) => {
                                // Try to parse string as timestamp
                                datetime_helpers::datetime_parse(s).unwrap_or(0)
                            }
                            other => {
                                return Err(format!(
                                    "time_ago() expects timestamp (int) or date string, got {}",
                                    other.type_name()
                                ))
                            }
                        };
                        // Use current locale for localized output
                        let locale = i18n_helpers::get_locale();
                        Ok(Value::String(datetime_helpers::time_ago_localized(
                            timestamp, &locale,
                        )))
                    }));
                new_hash.push((time_ago_key, time_ago_func));
            }

            // Add locale() function if not present
            let locale_key = Value::String("locale".to_string());
            let has_locale = hash.borrow().iter().any(|(k, _)| k.hash_eq(&locale_key));

            if !has_locale {
                let locale_func =
                    Value::NativeFunction(NativeFunction::new("locale", Some(0), |_args| {
                        Ok(Value::String(i18n_helpers::get_locale()))
                    }));
                new_hash.push((locale_key, locale_func));
            }

            // Add set_locale() function if not present
            let set_locale_key = Value::String("set_locale".to_string());
            let has_set_locale = hash
                .borrow()
                .iter()
                .any(|(k, _)| k.hash_eq(&set_locale_key));

            if !has_set_locale {
                let set_locale_func =
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
                    }));
                new_hash.push((set_locale_key, set_locale_func));
            }

            // Add t() function (translate alias) if not present
            let t_key = Value::String("t".to_string());
            let has_t = hash.borrow().iter().any(|(k, _)| k.hash_eq(&t_key));

            if !has_t {
                let t_func = Value::NativeFunction(NativeFunction::new("t", Some(1), |args| {
                    let key = match &args[0] {
                        Value::String(s) => s.clone(),
                        other => {
                            return Err(format!(
                                "t() expects string key, got {}",
                                other.type_name()
                            ))
                        }
                    };
                    // Return the key itself as fallback (translations loaded elsewhere)
                    Ok(Value::String(key))
                }));
                new_hash.push((t_key, t_func));
            }

            // Add l() function (localize date) if not present
            let l_key = Value::String("l".to_string());
            let has_l = hash.borrow().iter().any(|(k, _)| k.hash_eq(&l_key));

            if !has_l {
                let l_func = Value::NativeFunction(NativeFunction::new("l", None, |args| {
                    if args.is_empty() {
                        return Err("l() requires at least 1 argument (timestamp)".to_string());
                    }

                    // Get timestamp (first arg)
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

                    // Get format (second arg, default "short")
                    let format = if args.len() > 1 {
                        match &args[1] {
                            Value::String(s) => s.clone(),
                            _ => "short".to_string(),
                        }
                    } else {
                        "short".to_string()
                    };

                    // Get locale from i18n helpers
                    let locale = i18n_helpers::get_locale();

                    Ok(Value::String(datetime_helpers::localize_date(
                        timestamp, &locale, &format,
                    )))
                }));
                new_hash.push((l_key, l_func));
            }

            // Add render_partial() function if not present
            let render_partial_key = Value::String("render_partial".to_string());
            let has_render_partial = hash
                .borrow()
                .iter()
                .any(|(k, _)| k.hash_eq(&render_partial_key));

            if !has_render_partial {
                let render_partial_func =
                    Value::NativeFunction(NativeFunction::new("render_partial", None, |args| {
                        if args.is_empty() {
                            return Err(
                                "render_partial() requires at least 1 argument (partial name)"
                                    .to_string(),
                            );
                        }

                        // Get partial name
                        let partial_name = match &args[0] {
                            Value::String(s) => s.clone(),
                            other => {
                                return Err(format!(
                                    "render_partial() expects string partial name, got {}",
                                    other.type_name()
                                ))
                            }
                        };

                        // Get optional data context (default to empty hash)
                        let data = if args.len() > 1 {
                            args[1].clone()
                        } else {
                            Value::Hash(Rc::new(RefCell::new(vec![])))
                        };

                        // Resolve any futures in the data before rendering
                        let data = resolve_futures_in_value(data);

                        // Get template cache and render partial
                        let cache = get_template_cache()?;

                        // Inject helpers into the data context for nested partials
                        let data_with_helpers = inject_template_helpers(&data);

                        cache
                            .render_partial(&partial_name, &data_with_helpers)
                            .map(Value::String)
                    }));
                new_hash.push((render_partial_key, render_partial_func));
            }

            Value::Hash(Rc::new(RefCell::new(new_hash)))
        }
        _ => data.clone(),
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
                Value::Hash(Rc::new(RefCell::new(vec![])))
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
                let opts = opts.borrow();
                let layout_key = Value::String("layout".to_string());
                let mut layout_value = None;
                for (k, v) in opts.iter() {
                    if k.hash_eq(&layout_key) {
                        layout_value = Some(v.clone());
                        break;
                    }
                }
                layout_value
            } else {
                None
            };

            // If not found in options, check data hash for layout key
            let layout = if layout.is_none() {
                if let Value::Hash(data_hash) = &data {
                    let data_hash = data_hash.borrow();
                    let layout_key = Value::String("layout".to_string());
                    let mut layout_value = None;
                    for (k, v) in data_hash.iter() {
                        if k.hash_eq(&layout_key) {
                            layout_value = Some(v.clone());
                            break;
                        }
                    }
                    layout_value
                } else {
                    None
                }
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
                let opts = opts.borrow();
                let status_key = Value::String("status".to_string());
                let mut status_value = 200i64;
                for (k, v) in opts.iter() {
                    if k.hash_eq(&status_key) {
                        if let Value::Int(n) = v {
                            status_value = *n;
                        }
                        break;
                    }
                }
                status_value
            } else {
                200
            };

            // Get template cache and render
            let cache = get_template_cache()?;

            // Inject template helper functions into data context
            let data_with_helpers = inject_template_helpers(&data);

            // Convert layout option for render call
            let layout_arg = match &layout {
                Some(Some(name)) => Some(Some(name.as_str())),
                Some(None) => Some(None),
                None => None,
            };

            // Set view context for debugging (in case of error)
            set_view_debug_context(Some(data.clone()));

            let result = cache.render(&template_name, &data_with_helpers, layout_arg);

            match result {
                Ok(rendered) => {
                    // Clear context on success
                    set_view_debug_context(None);
                    Ok(html_response(rendered, status))
                }
                Err(e) => {
                    // Keep context set for debugging
                    Err(e)
                }
            }
        })),
    );

    // render_partial(name, data?) - Render a partial template (no layout)
    env.define(
        "render_partial".to_string(),
        Value::NativeFunction(NativeFunction::new("render_partial", None, |args| {
            if args.is_empty() {
                return Err(
                    "render_partial() requires at least 1 argument (partial name)".to_string(),
                );
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
                Value::Hash(Rc::new(RefCell::new(vec![])))
            };

            // Resolve any futures in the data before rendering
            let data = resolve_futures_in_value(data);

            // Get template cache and render
            let cache = get_template_cache()?;

            // Inject template helper functions into data context
            let data_with_helpers = inject_template_helpers(&data);

            let rendered = cache.render_partial(&partial_name, &data_with_helpers)?;

            // Return just the string for partials (they're typically embedded)
            Ok(Value::String(rendered))
        })),
    );

    // html_escape(string) - Escape HTML special characters
    env.define(
        "html_escape".to_string(),
        Value::NativeFunction(NativeFunction::new("html_escape", Some(1), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => format!("{}", other),
            };
            Ok(Value::String(crate::template::renderer::html_escape(&s)))
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
            Ok(Value::String(crate::template::renderer::html_escape(&s)))
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

            let headers = Value::Hash(Rc::new(RefCell::new(vec![(
                Value::String("Location".to_string()),
                Value::String(url),
            )])));

            Ok(Value::Hash(Rc::new(RefCell::new(vec![
                (Value::String("status".to_string()), Value::Int(302)),
                (Value::String("headers".to_string()), headers),
                (
                    Value::String("body".to_string()),
                    Value::String(String::new()),
                ),
            ]))))
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
                    Value::Int(n) => *n as i64,
                    _ => 200,
                }
            } else {
                200
            };

            let json_body = match &data {
                Value::String(s) => s.clone(),
                Value::Null => "null".to_string(),
                _ => format!("{}", data),
            };

            let headers = Value::Hash(Rc::new(RefCell::new(vec![(
                Value::String("Content-Type".to_string()),
                Value::String("application/json; charset=utf-8".to_string()),
            )])));

            Ok(Value::Hash(Rc::new(RefCell::new(vec![
                (Value::String("status".to_string()), Value::Int(status)),
                (Value::String("headers".to_string()), headers),
                (Value::String("body".to_string()), Value::String(json_body)),
            ]))))
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
                    Value::Int(n) => *n as i64,
                    _ => 200,
                }
            } else {
                200
            };

            let headers = Value::Hash(Rc::new(RefCell::new(vec![(
                Value::String("Content-Type".to_string()),
                Value::String("text/plain; charset=utf-8".to_string()),
            )])));

            Ok(Value::Hash(Rc::new(RefCell::new(vec![
                (Value::String("status".to_string()), Value::Int(status)),
                (Value::String("headers".to_string()), headers),
                (Value::String("body".to_string()), Value::String(text)),
            ]))))
        })),
    );
}
