use crate::interpreter::builtins::server::register_route_with_middleware;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};
use std::cell::RefCell;
use std::collections::HashMap;

/// Extract middleware values from a value (array of middleware names).
/// Looks up each middleware name in the global middleware registry.
fn extract_middleware_from_value(value: &Value) -> Vec<Value> {
    let mut middleware = Vec::new();

    if let Value::String(name) = value {
        // Single middleware name as string
        if let Some(mw) = crate::serve::get_middleware_by_name(name) {
            if mw.global_only {
                eprintln!(
                    "Warning: Middleware '{}' is global_only and cannot be scoped",
                    name
                );
            } else {
                middleware.push(mw.handler.clone());
            }
        } else {
            eprintln!("Warning: Middleware '{}' not found", name);
        }
    } else if let Value::Array(arr) = value {
        for item in arr.borrow().iter() {
            if let Value::String(name) = item {
                // Look up the middleware function by name
                if let Some(mw) = crate::serve::get_middleware_by_name(name) {
                    if mw.global_only {
                        eprintln!(
                            "Warning: Middleware '{}' is global_only and cannot be scoped",
                            name
                        );
                    } else {
                        middleware.push(mw.handler.clone());
                    }
                } else {
                    eprintln!("Warning: Middleware '{}' not found", name);
                }
            } else {
                // If it's already a function value, use it directly
                middleware.push(item.clone());
            }
        }
    }

    middleware
}

// Global registries
thread_local! {
    // Map "controller_name" -> "action_name" -> FunctionValue
    pub static CONTROLLERS: RefCell<HashMap<String, HashMap<String, Value>>> = RefCell::new(HashMap::new());

    // Routing context stack
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
    path_prefix: String,        // Current URL prefix (e.g. "/users/:user_id")
    controller: Option<String>, // Current controller context
    is_member: bool,            // Are we inside a member block?
    is_collection: bool,        // Are we inside a collection block?
    middleware: Vec<Value>,     // Active middleware
}

impl Default for RouterScope {
    fn default() -> Self {
        Self {
            path_prefix: "/".to_string(),
            controller: None,
            is_member: false,
            is_collection: false,
            middleware: Vec::new(),
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

                    // Register standard routes immediately
                    // Index: GET base_path
                    let handler_name = format!("{}#index", controller);
                    register_route_with_middleware(
                        "GET",
                        &base_path,
                        handler_name,
                        middleware.clone(),
                    );

                    // Create: POST base_path
                    let handler_name = format!("{}#create", controller);
                    register_route_with_middleware(
                        "POST",
                        &base_path,
                        handler_name,
                        middleware.clone(),
                    );

                    // New: GET base_path/new
                    let handler_name = format!("{}#new", controller);
                    register_route_with_middleware(
                        "GET",
                        &format!("{}/new", base_path),
                        handler_name,
                        middleware.clone(),
                    );

                    // Member routes base path
                    let member_path = format!("{}/:id", base_path);

                    // Show: GET member_path
                    let handler_name = format!("{}#show", controller);
                    register_route_with_middleware(
                        "GET",
                        &member_path,
                        handler_name,
                        middleware.clone(),
                    );

                    // Update: PUT/PATCH member_path
                    let handler_name = format!("{}#update", controller);
                    register_route_with_middleware(
                        "PUT",
                        &member_path,
                        handler_name.clone(),
                        middleware.clone(),
                    );
                    register_route_with_middleware(
                        "PATCH",
                        &member_path,
                        handler_name,
                        middleware.clone(),
                    );

                    // Destroy: DELETE member_path
                    let handler_name = format!("{}#destroy", controller);
                    register_route_with_middleware(
                        "DELETE",
                        &member_path,
                        handler_name,
                        middleware.clone(),
                    );

                    // Edit: GET member_path/edit
                    let handler_name = format!("{}#edit", controller);
                    register_route_with_middleware(
                        "GET",
                        &format!("{}/edit", member_path),
                        handler_name,
                        middleware.clone(),
                    );

                    // Push new scope for nested resources
                    // Child base: /users/:user_id
                    // Simple singularization: remove trailing 's' or append '_id' to full name
                    let param_name = if name.ends_with('s') {
                        format!("{}_id", &name[..name.len() - 1])
                    } else {
                        format!("{}_id", name)
                    };

                    let child_path = format!("{}/:{}", base_path, param_name);

                    stack.push(RouterScope {
                        path_prefix: child_path,
                        controller: Some(controller),
                        is_member: false,
                        is_collection: false,
                        middleware: current.middleware.clone(),
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

    // router_match(method, path, action)
    env.define(
        "router_match".to_string(),
        Value::NativeFunction(NativeFunction::new("router_match", Some(3), |args| {
            let method = args[0].to_string().to_uppercase();
            let path = args[1].to_string();
            let action = args[2].to_string();

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
                } else {
                    if path.is_empty() {
                        current.path_prefix.clone()
                    } else if path.starts_with('/') {
                        // Absolute path overrides context? Or appends?
                        // Rails: match 'foo' -> prefix/foo
                        format!("{}/{}", current.path_prefix, path.trim_start_matches('/'))
                    } else {
                        format!("{}/{}", current.path_prefix, path)
                    }
                };

                let handler = action.clone();
                // Pass scoped middleware from current context
                let middleware = current.middleware.clone();
                register_route_with_middleware(&method, &full_path, handler, middleware);
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

                ROUTER_CONTEXT.with(|ctx| {
                    let mut stack = ctx.borrow_mut();
                    let current = stack.last().unwrap().clone();

                    // Push new scope with middleware added
                    let mut new_middleware = current.middleware.clone();
                    new_middleware.extend(middleware_values);

                    stack.push(RouterScope {
                        path_prefix: current.path_prefix,
                        controller: current.controller,
                        is_member: current.is_member,
                        is_collection: current.is_collection,
                        middleware: new_middleware,
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
                } else {
                    if path.is_empty() {
                        current.path_prefix.clone()
                    } else if path.starts_with('/') {
                        format!("{}/{}", current.path_prefix, path.trim_start_matches('/'))
                    } else {
                        format!("{}/{}", current.path_prefix, path)
                    }
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
}
