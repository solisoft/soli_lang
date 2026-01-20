//! Template rendering builtins for Soli MVC.
//!
//! Provides the `render()` function for use in controllers to render templates.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Mutex;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};
use crate::template::{html_response, TemplateCache};

// Thread-local template cache
thread_local! {
    static TEMPLATE_CACHE: RefCell<Option<Rc<TemplateCache>>> = const { RefCell::new(None) };
}

// Global views directory for initialization
static VIEWS_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

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
fn get_template_cache() -> Result<Rc<TemplateCache>, String> {
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

            // Extract layout option
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
                match layout_value {
                    Some(Value::String(s)) => Some(Some(s)),
                    Some(Value::Bool(false)) => Some(None), // layout: false
                    Some(Value::Null) => Some(None),
                    None => None, // Use default
                    _ => None,
                }
            } else {
                None
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

            // Convert layout option for render call
            let layout_arg = match &layout {
                Some(Some(name)) => Some(Some(name.as_str())),
                Some(None) => Some(None),
                None => None,
            };

            let rendered = cache.render(&template_name, &data, layout_arg)?;

            Ok(html_response(rendered, status))
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

            // Get template cache and render
            let cache = get_template_cache()?;
            let rendered = cache.render_partial(&partial_name, &data)?;

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
}
