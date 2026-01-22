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

/// Compute MD5 hash of file contents.
fn compute_file_md5(path: &PathBuf) -> Result<String, String> {
    let data = std::fs::read(path).map_err(|e| format!("Failed to read file for MD5: {}", e))?;

    let hash = md5::compute(&data);
    Ok(format!("{:x}", hash))
}

/// Inject template helper functions into the data context
fn inject_template_helpers(data: &Value) -> Value {
    match data {
        Value::Hash(hash) => {
            let mut new_hash: Vec<(Value, Value)> = hash.borrow().clone();

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

                        // Compute MD5 hash if file exists, otherwise return path without hash
                        match compute_file_md5(&full_path) {
                            Ok(hash) => {
                                // Return path with query parameter
                                if path.contains('?') {
                                    Ok(Value::String(format!("/{}&v={}", path, hash)))
                                } else {
                                    Ok(Value::String(format!("/{}?v={}", path, hash)))
                                }
                            }
                            Err(_) => {
                                // File doesn't exist, return path without hash
                                Ok(Value::String(format!("/{}", path)))
                            }
                        }
                    }));

                new_hash.push((public_path_key, public_path_func));
            }

            // Add strip_html() function if not present
            let strip_html_key = Value::String("strip_html".to_string());
            let has_strip_html = hash.borrow().iter().any(|(k, _)| k.hash_eq(&strip_html_key));

            if !has_strip_html {
                let strip_html_func =
                    Value::NativeFunction(NativeFunction::new("strip_html", Some(1), |args| {
                        let s = match &args[0] {
                            Value::String(s) => s.clone(),
                            other => {
                                return Err(format!(
                                    "strip_html() expects string, got {}",
                                    other.type_name()
                                ))
                            }
                        };

                        let mut result = String::new();
                        let mut in_tag = false;

                        for c in s.chars() {
                            if c == '<' {
                                in_tag = true;
                            } else if c == '>' {
                                in_tag = false;
                            } else if !in_tag {
                                result.push(c);
                            }
                        }

                        Ok(Value::String(result))
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

                        let chars: Vec<char> = s.chars().collect();
                        let end = end.min(chars.len());
                        let start = start.min(end);
                        let result: String = chars[start..end].iter().collect();
                        Ok(Value::String(result))
                    }));

                new_hash.push((substring_key, substring_func));
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

            let rendered = cache.render(&template_name, &data_with_helpers, layout_arg)?;

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

            let data = args[0].clone();
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
