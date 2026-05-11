use crate::interpreter::builtins::server::{
    register_route_with_middleware, register_route_with_name,
};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};
use std::cell::RefCell;
use std::collections::HashMap;

/// Singularize a plural resource name for member-route naming.
///
/// Handles three cases, in order:
/// 1. A small irregulars table for the most common English plurals that
///    don't follow the `s` rule (`people → person`, `mice → mouse`, ...).
/// 2. `ies → y` when the letter before `ies` is a consonant
///    (`categories → category`, `parties → party`). The consonant guard
///    prevents `pies → py` / `lies → ly` regressions.
/// 3. A trailing `s` (`posts → post`, `users → user`).
///
/// Everything else falls through unchanged. See `www/docs/routing.md`
/// (Plural-to-singular limitations) for the documented edge cases that
/// require choosing a different resource name (`news`, `species`, ...).
fn singularize(name: &str) -> String {
    // Irregulars table: lowercase plural → lowercase singular. Matched on a
    // lowercased copy of `name` so `People` and `people` both work, while
    // preserving any user-chosen casing on the result is not attempted
    // because resource names in routes are conventionally lowercase.
    const IRREGULARS: &[(&str, &str)] = &[
        ("people", "person"),
        ("men", "man"),
        ("women", "woman"),
        ("children", "child"),
        ("mice", "mouse"),
        ("geese", "goose"),
        ("feet", "foot"),
        ("teeth", "tooth"),
    ];
    let lower = name.to_ascii_lowercase();
    for (pl, sg) in IRREGULARS {
        if lower == *pl {
            return (*sg).to_string();
        }
    }

    // `ies → y` when preceded by a consonant. Requires stem length ≥ 2 so
    // single-letter stems (`pies`, `lies`, `ties`, `dies`) fall through to
    // the bare `s` rule — those are singular nouns ending in `ie`, not
    // `y`-plurals. Two-letter+ stems (`cities`, `flies`, `cries`, `tries`,
    // `parties`, `companies`, `agencies`) are virtually always `y`-plurals.
    if let Some(stem) = name.strip_suffix("ies") {
        let last = stem.chars().last().map(|c| c.to_ascii_lowercase());
        let consonant_stem =
            matches!(last, Some(c) if !matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y'));
        if consonant_stem && stem.len() >= 2 {
            return format!("{}y", stem);
        }
    }

    // Generic `s` rule. Falls back to the original name when no trailing `s`.
    if let Some(stripped) = name.strip_suffix('s') {
        stripped.to_string()
    } else {
        name.to_string()
    }
}

/// Compose a Rails-style route name from the ancestor chain plus the
/// current segment. `prefix` is the underscore-joined chain of singular
/// ancestor names (empty at the top level).
fn compose_name(prefix: &str, segment: &str) -> String {
    if prefix.is_empty() {
        segment.to_string()
    } else {
        format!("{}_{}", prefix, segment)
    }
}

/// Extract middleware values from a value (array of middleware names).
/// Looks up each middleware name in the global middleware registry.
fn extract_middleware_from_value(value: &Value) -> Vec<Value> {
    let mut middleware = Vec::new();

    if let Value::String(name) = value {
        // Single middleware name as string
        match crate::serve::get_middleware_by_name(name) {
            Some(mw) => {
                if mw.global_only {
                    eprintln!(
                        "Warning: Middleware '{}' is global_only and cannot be scoped",
                        name
                    );
                } else {
                    middleware.push(mw.handler.clone());
                }
            }
            _ => {
                eprintln!("Warning: Middleware '{}' not found", name);
            }
        }
    } else if let Value::Array(arr) = value {
        for item in arr.borrow().iter() {
            if let Value::String(name) = item {
                // Look up the middleware function by name
                match crate::serve::get_middleware_by_name(name) {
                    Some(mw) => {
                        if mw.global_only {
                            eprintln!(
                                "Warning: Middleware '{}' is global_only and cannot be scoped",
                                name
                            );
                        } else {
                            middleware.push(mw.handler.clone());
                        }
                    }
                    _ => {
                        eprintln!("Warning: Middleware '{}' not found", name);
                    }
                }
            } else {
                // If it's already a function value, use it directly
                middleware.push(item.clone());
            }
        }
    }

    middleware
}

/// Extract middleware names from a value (string or array of strings).
fn extract_middleware_names(value: &Value) -> Vec<String> {
    let mut names = Vec::new();
    if let Value::String(name) = value {
        names.push(name.clone());
    } else if let Value::Array(arr) = value {
        for item in arr.borrow().iter() {
            if let Value::String(name) = item {
                names.push(name.clone());
            }
        }
    }
    names
}

// Global registries
thread_local! {
    // Map "controller_name" -> "action_name" -> FunctionValue
    #[allow(clippy::missing_const_for_thread_local)]
    pub static CONTROLLERS: RefCell<HashMap<String, HashMap<String, Value>>> = RefCell::new(HashMap::new());

    // Routing context stack
    #[allow(clippy::missing_const_for_thread_local)]
    static ROUTER_CONTEXT: RefCell<Vec<RouterScope>> = RefCell::new(vec![RouterScope::default()]);
}

/// Reset router context for hot reload.
/// Clears the context stack and reinitializes with a default scope.
pub fn reset_router_context() {
    ROUTER_CONTEXT.with(|ctx| {
        let mut stack = ctx.borrow_mut();
        stack.clear();
        stack.push(RouterScope::default());
    });
}

#[derive(Clone)]
struct RouterScope {
    path_prefix: String,           // Current URL prefix (e.g. "/users/:user_id")
    controller: Option<String>,    // Current controller context
    is_member: bool,               // Are we inside a member block?
    is_collection: bool,           // Are we inside a collection block?
    middleware: Vec<Value>,        // Active middleware
    middleware_names: Vec<String>, // Middleware names for worker thread transfer
    /// Underscore-joined chain of singular ancestor resource names — used to
    /// build Rails-style route names for nested resources (e.g. `post_comment`
    /// from `resources("posts") do resources("comments") end`).
    name_prefix: String,
}

impl Default for RouterScope {
    fn default() -> Self {
        Self {
            path_prefix: "/".to_string(),
            controller: None,
            is_member: false,
            is_collection: false,
            middleware: Vec::new(),
            middleware_names: Vec::new(),
            name_prefix: String::new(),
        }
    }
}

/// Register a controller action for lookup.
pub fn register_controller_action(controller: &str, action: &str, handler: Value) {
    CONTROLLERS.with(|c| {
        c.borrow_mut()
            .entry(controller.to_string())
            .or_default()
            .insert(action.to_string(), handler);
    });
}

/// Get all controller actions (for propagating to workers).
pub fn get_controllers() -> HashMap<String, HashMap<String, Value>> {
    CONTROLLERS.with(|c| c.borrow().clone())
}

/// Set controller actions in current thread (for worker initialization).
pub fn set_controllers(controllers: HashMap<String, HashMap<String, Value>>) {
    CONTROLLERS.with(|c| *c.borrow_mut() = controllers);
}

/// Resolve "controller#action" or "action" to a function value.
/// This is used by WebSocket handlers to look up the handler function.
pub fn resolve_handler(
    action_str: &str,
    current_controller: Option<&str>,
) -> Result<Value, String> {
    let (controller, action) = if let Some((c, a)) = action_str.split_once('#') {
        (c, a)
    } else if let Some(c) = current_controller {
        (c, action_str)
    } else {
        return Err(format!(
            "No controller specified for action '{}'",
            action_str
        ));
    };

    CONTROLLERS.with(|c| {
        let controllers = c.borrow();
        if let Some(actions) = controllers.get(controller) {
            if let Some(handler) = actions.get(action) {
                return Ok(handler.clone());
            }
        }
        Err(format!(
            "Action '{}' not found in controller '{}'",
            action, controller
        ))
    })
}

pub fn register_router_builtins(env: &mut Environment) {
    // router_resource_enter(name, options)
    env.define(
        "router_resource_enter".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "router_resource_enter",
            Some(2),
            |args| {
                let name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err("Expected string for resource name".to_string()),
                };

                // Options (optional hash) - ignored for now

                ROUTER_CONTEXT.with(|ctx| {
                    let mut stack = ctx.borrow_mut();
                    let current = stack.last().unwrap().clone();

                    let base_path = if current.path_prefix == "/" {
                        format!("/{}", name)
                    } else {
                        format!("{}/{}", current.path_prefix, name)
                    };

                    let controller = name.clone(); // Default controller is resource name
                    let middleware = current.middleware.clone();
                    let mw_names = current.middleware_names.clone();

                    // Rails-style names. `name` is the plural collection segment
                    // (e.g. "posts"); `singular` is the member segment ("post").
                    // Names compose with any ancestor name_prefix so nested
                    // resources get `post_comment_path` etc. Only the routes
                    // that have a Rails-style helper get a name — the POST/PUT/
                    // PATCH/DELETE variants share the GET name's path so we
                    // intentionally leave them unnamed (calling `post_path(p)`
                    // returns the same string regardless of HTTP verb intent).
                    let singular = singularize(&name);
                    let collection_name = compose_name(&current.name_prefix, &name);
                    let member_name = compose_name(&current.name_prefix, &singular);
                    let new_name = compose_name(&current.name_prefix, &format!("new_{}", singular));
                    let edit_name =
                        compose_name(&current.name_prefix, &format!("edit_{}", singular));

                    // Register standard routes immediately
                    // Index: GET base_path → `<plural>_path`
                    let handler_name = format!("{}#index", controller);
                    register_route_with_name(
                        "GET",
                        &base_path,
                        handler_name,
                        Some(collection_name),
                        middleware.clone(),
                        mw_names.clone(),
                    );

                    // Create: POST base_path (unnamed — same path as index)
                    let handler_name = format!("{}#create", controller);
                    register_route_with_middleware(
                        "POST",
                        &base_path,
                        handler_name,
                        middleware.clone(),
                        mw_names.clone(),
                    );

                    // New: GET base_path/new → `new_<singular>_path`
                    let handler_name = format!("{}#new", controller);
                    register_route_with_name(
                        "GET",
                        &format!("{}/new", base_path),
                        handler_name,
                        Some(new_name),
                        middleware.clone(),
                        mw_names.clone(),
                    );

                    // Member routes base path
                    let member_path = format!("{}/:id", base_path);

                    // Show: GET member_path → `<singular>_path`
                    let handler_name = format!("{}#show", controller);
                    register_route_with_name(
                        "GET",
                        &member_path,
                        handler_name,
                        Some(member_name),
                        middleware.clone(),
                        mw_names.clone(),
                    );

                    // Update: PUT/PATCH member_path, plus POST member_path/update
                    // alias so HTML forms (which can't natively PUT/PATCH) can reach it.
                    let handler_name = format!("{}#update", controller);
                    register_route_with_middleware(
                        "PUT",
                        &member_path,
                        handler_name.clone(),
                        middleware.clone(),
                        mw_names.clone(),
                    );
                    register_route_with_middleware(
                        "PATCH",
                        &member_path,
                        handler_name.clone(),
                        middleware.clone(),
                        mw_names.clone(),
                    );
                    register_route_with_middleware(
                        "POST",
                        &format!("{}/update", member_path),
                        handler_name,
                        middleware.clone(),
                        mw_names.clone(),
                    );

                    // Destroy: DELETE member_path, plus POST member_path/delete
                    // alias for HTML form compatibility.
                    let handler_name = format!("{}#destroy", controller);
                    register_route_with_middleware(
                        "DELETE",
                        &member_path,
                        handler_name.clone(),
                        middleware.clone(),
                        mw_names.clone(),
                    );
                    register_route_with_middleware(
                        "POST",
                        &format!("{}/delete", member_path),
                        handler_name,
                        middleware.clone(),
                        mw_names.clone(),
                    );

                    // Edit: GET member_path/edit → `edit_<singular>_path`
                    let handler_name = format!("{}#edit", controller);
                    register_route_with_name(
                        "GET",
                        &format!("{}/edit", member_path),
                        handler_name,
                        Some(edit_name),
                        middleware.clone(),
                        mw_names.clone(),
                    );

                    // Push new scope for nested resources
                    // Child base: /users/:user_id
                    // Simple singularization: remove trailing 's' or append '_id' to full name
                    let param_name = format!("{}_id", singular);

                    let child_path = format!("{}/:{}", base_path, param_name);
                    let child_name_prefix = compose_name(&current.name_prefix, &singular);

                    stack.push(RouterScope {
                        path_prefix: child_path,
                        controller: Some(controller),
                        is_member: false,
                        is_collection: false,
                        middleware: current.middleware.clone(),
                        middleware_names: current.middleware_names.clone(),
                        name_prefix: child_name_prefix,
                    });
                });

                Ok(Value::Null)
            },
        )),
    );

    // router_resource_exit()
    env.define(
        "router_resource_exit".to_string(),
        Value::NativeFunction(NativeFunction::new("router_resource_exit", Some(0), |_| {
            ROUTER_CONTEXT.with(|ctx| {
                ctx.borrow_mut().pop();
            });
            Ok(Value::Null)
        })),
    );

    // router_match(method, path, action, name?)
    // Variadic so the DSL can pass a 4th `name` arg for `*_path` / `*_url`
    // helper generation while older callers continue to pass 3.
    env.define(
        "router_match".to_string(),
        Value::NativeFunction(NativeFunction::new("router_match", None, |args| {
            if args.len() < 3 || args.len() > 4 {
                return Err(format!(
                    "router_match expects 3 or 4 arguments, got {}",
                    args.len()
                ));
            }
            let method = args[0].to_string().to_uppercase();
            let path = args[1].to_string();
            let action = args[2].to_string();
            let name = match args.get(3) {
                Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
                _ => None,
            };

            ROUTER_CONTEXT.with(|ctx| {
                let stack = ctx.borrow();
                let current = stack.last().unwrap();

                // Calculate full path
                let full_path = if current.path_prefix == "/" {
                    if path.starts_with('/') {
                        path
                    } else {
                        format!("/{}", path)
                    }
                } else if path.is_empty() {
                    current.path_prefix.clone()
                } else if path.starts_with('/') {
                    // Absolute path overrides context? Or appends?
                    // Rails: match 'foo' -> prefix/foo
                    format!("{}/{}", current.path_prefix, path.trim_start_matches('/'))
                } else {
                    format!("{}/{}", current.path_prefix, path)
                };

                let handler = action.clone();
                // Pass scoped middleware from current context
                let middleware = current.middleware.clone();
                let mw_names = current.middleware_names.clone();
                register_route_with_name(&method, &full_path, handler, name, middleware, mw_names);
                Ok(Value::Null)
            })
        })),
    );

    // router_member_enter()
    env.define(
        "router_member_enter".to_string(),
        Value::NativeFunction(NativeFunction::new("router_member_enter", Some(0), |_| {
            ROUTER_CONTEXT.with(|ctx| {
                let mut stack = ctx.borrow_mut();
                let current = stack.last().unwrap().clone();

                // The current prefix is likely the *nested* prefix (e.g. /users/:user_id).
                // We want the member prefix (e.g. /users/:id).
                let new_prefix = if let Some(idx) = current.path_prefix.rfind("/:") {
                    format!("{}/:id", &current.path_prefix[..idx])
                } else {
                    current.path_prefix.clone()
                };

                stack.push(RouterScope {
                    path_prefix: new_prefix,
                    controller: current.controller,
                    is_member: true,
                    is_collection: false,
                    middleware: current.middleware,
                    middleware_names: current.middleware_names,
                    name_prefix: current.name_prefix,
                });
            });
            Ok(Value::Null)
        })),
    );

    // router_member_exit()
    env.define(
        "router_member_exit".to_string(),
        Value::NativeFunction(NativeFunction::new("router_member_exit", Some(0), |_| {
            ROUTER_CONTEXT.with(|ctx| {
                ctx.borrow_mut().pop();
            });
            Ok(Value::Null)
        })),
    );

    // router_collection_enter()
    env.define(
        "router_collection_enter".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "router_collection_enter",
            Some(0),
            |_| {
                ROUTER_CONTEXT.with(|ctx| {
                    let mut stack = ctx.borrow_mut();
                    let current = stack.last().unwrap().clone();

                    // Collection prefix: Remove the param.
                    // /users/:user_id -> /users
                    let new_prefix = if let Some(idx) = current.path_prefix.rfind("/:") {
                        current.path_prefix[..idx].to_string()
                    } else {
                        current.path_prefix.clone()
                    };

                    stack.push(RouterScope {
                        path_prefix: new_prefix,
                        controller: current.controller,
                        is_member: false,
                        is_collection: true,
                        middleware: current.middleware,
                        middleware_names: current.middleware_names,
                        name_prefix: current.name_prefix,
                    });
                });
                Ok(Value::Null)
            },
        )),
    );

    // router_collection_exit()
    env.define(
        "router_collection_exit".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "router_collection_exit",
            Some(0),
            |_| {
                ROUTER_CONTEXT.with(|ctx| {
                    ctx.borrow_mut().pop();
                });
                Ok(Value::Null)
            },
        )),
    );

    // router_namespace_enter(name)
    env.define(
        "router_namespace_enter".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "router_namespace_enter",
            Some(1),
            |args| {
                let name = args[0].to_string();
                ROUTER_CONTEXT.with(|ctx| {
                    let mut stack = ctx.borrow_mut();
                    let current = stack.last().unwrap().clone();

                    let new_prefix = if current.path_prefix == "/" {
                        format!("/{}", name)
                    } else {
                        format!("{}/{}", current.path_prefix, name)
                    };

                    stack.push(RouterScope {
                        path_prefix: new_prefix,
                        controller: None, // Namespaces usually don't imply a controller unless explicit?
                        is_member: false,
                        is_collection: false,
                        middleware: current.middleware,
                        middleware_names: current.middleware_names,
                        name_prefix: current.name_prefix,
                    });
                });
                Ok(Value::Null)
            },
        )),
    );

    // router_middleware_scope(middleware_array)
    // Scopes subsequent routes to run the specified middleware
    env.define(
        "router_middleware_scope".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "router_middleware_scope",
            Some(1),
            |args| {
                // Extract middleware values from the argument (array of names or single name)
                let middleware_values = extract_middleware_from_value(&args[0]);
                let middleware_names_new = extract_middleware_names(&args[0]);

                ROUTER_CONTEXT.with(|ctx| {
                    let mut stack = ctx.borrow_mut();
                    let current = stack.last().unwrap().clone();

                    // Push new scope with middleware added
                    let mut new_middleware = current.middleware.clone();
                    new_middleware.extend(middleware_values);
                    let mut new_names = current.middleware_names.clone();
                    new_names.extend(middleware_names_new);

                    stack.push(RouterScope {
                        path_prefix: current.path_prefix,
                        controller: current.controller,
                        is_member: current.is_member,
                        is_collection: current.is_collection,
                        middleware: new_middleware,
                        middleware_names: new_names,
                        name_prefix: current.name_prefix,
                    });
                });

                Ok(Value::Null)
            },
        )),
    );

    // router_namespace_exit()
    env.define(
        "router_namespace_exit".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "router_namespace_exit",
            Some(0),
            |_| {
                ROUTER_CONTEXT.with(|ctx| {
                    ctx.borrow_mut().pop();
                });
                Ok(Value::Null)
            },
        )),
    );

    // router_middleware_scope_exit()
    env.define(
        "router_middleware_scope_exit".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "router_middleware_scope_exit",
            Some(0),
            |_| {
                ROUTER_CONTEXT.with(|ctx| {
                    ctx.borrow_mut().pop();
                });
                Ok(Value::Null)
            },
        )),
    );

    // router_websocket(path, action) - Register WebSocket route
    env.define(
        "router_websocket".to_string(),
        Value::NativeFunction(NativeFunction::new("router_websocket", Some(2), |args| {
            let path = args[0].to_string();
            let action = args[1].to_string();

            ROUTER_CONTEXT.with(|ctx| {
                let stack = ctx.borrow();
                let current = stack.last().unwrap();

                // Calculate full path
                let full_path = if current.path_prefix == "/" {
                    if path.starts_with('/') {
                        path
                    } else {
                        format!("/{}", path)
                    }
                } else if path.is_empty() {
                    current.path_prefix.clone()
                } else if path.starts_with('/') {
                    format!("{}/{}", current.path_prefix, path.trim_start_matches('/'))
                } else {
                    format!("{}/{}", current.path_prefix, path)
                };

                // Register the WebSocket route with just the path and action string
                // The handler will be looked up from CONTROLLERS when events are processed
                crate::serve::register_websocket_route(&full_path, &action);
                Ok(Value::Null)
            })
        })),
    );

    // router_live(component, action) - Register LiveView route
    // component: name of the component (e.g., "counter")
    // action: controller#action string (e.g., "live#counter")
    env.define(
        "router_live".to_string(),
        Value::NativeFunction(NativeFunction::new("router_live", Some(2), |args| {
            let component = args[0].to_string();
            let action = args[1].to_string();

            // Register the LiveView route
            crate::live::socket::register_liveview_route(&component, &action);
            Ok(Value::Null)
        })),
    );

    // SEC-014: skip_csrf(path_pattern) — declare a path or path-prefix
    // exempt from the same-origin CSRF check. Call from `config/routes.sl`
    // (or a controller's `static` block) for webhook endpoints, public
    // APIs, or any path that legitimately needs cross-origin POST.
    //
    // Examples:
    //   skip_csrf("/webhooks/stripe")    # exact path
    //   skip_csrf("/api/*")              # everything under /api/
    //
    // Without this, state-changing requests (POST/PUT/PATCH/DELETE) whose
    // Origin or Referer doesn't match the request Host get a 403.
    env.define(
        "skip_csrf".to_string(),
        Value::NativeFunction(NativeFunction::new("skip_csrf", Some(1), |args| {
            let pattern = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "skip_csrf() expects string path pattern, got {}",
                        other.type_name()
                    ))
                }
            };
            crate::serve::register_csrf_skip_pattern(pattern);
            Ok(Value::Null)
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singularize_strips_trailing_s() {
        assert_eq!(singularize("posts"), "post");
        assert_eq!(singularize("users"), "user");
        assert_eq!(singularize("comments"), "comment");
    }

    #[test]
    fn singularize_handles_ies_to_y() {
        assert_eq!(singularize("categories"), "category");
        assert_eq!(singularize("parties"), "party");
        assert_eq!(singularize("cities"), "city");
        assert_eq!(singularize("companies"), "company");
        assert_eq!(singularize("agencies"), "agency");
        assert_eq!(singularize("summaries"), "summary");
    }

    #[test]
    fn singularize_skips_ies_for_single_letter_stems() {
        // Single-letter stems are singular nouns ending in "ie" (pies, lies,
        // ties, dies), not "y"-plurals. The bare-s rule gives the right
        // answer: `pies → pie`, not `py`.
        assert_eq!(singularize("pies"), "pie");
        assert_eq!(singularize("lies"), "lie");
        assert_eq!(singularize("ties"), "tie");
        assert_eq!(singularize("dies"), "die");
    }

    #[test]
    fn singularize_two_letter_stems_take_ies_to_y() {
        // Two-letter stems are typically y-plurals (flies, cries, tries).
        assert_eq!(singularize("flies"), "fly");
        assert_eq!(singularize("cries"), "cry");
        assert_eq!(singularize("tries"), "try");
    }

    #[test]
    fn singularize_handles_irregulars() {
        assert_eq!(singularize("people"), "person");
        assert_eq!(singularize("men"), "man");
        assert_eq!(singularize("women"), "woman");
        assert_eq!(singularize("children"), "child");
        assert_eq!(singularize("mice"), "mouse");
        assert_eq!(singularize("geese"), "goose");
        assert_eq!(singularize("feet"), "foot");
        assert_eq!(singularize("teeth"), "tooth");
    }

    #[test]
    fn singularize_leaves_unmatched_words_alone() {
        // No trailing `s`, not in the irregulars table — return as-is. The
        // helper will end up named the same as the collection helper, which
        // routing.md documents as a known limitation.
        assert_eq!(singularize("data"), "data");
        assert_eq!(singularize("info"), "info");
    }

    #[test]
    fn compose_name_joins_with_underscore() {
        assert_eq!(compose_name("", "posts"), "posts");
        assert_eq!(compose_name("post", "comments"), "post_comments");
        assert_eq!(compose_name("post_comment", "edit"), "post_comment_edit");
    }
}
