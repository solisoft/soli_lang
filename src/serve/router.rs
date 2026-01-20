//! Convention-based routing for MVC controllers.
//!
//! Routes are derived from controller filename + function name:
//! - `home_controller.soli` maps to root `/`
//! - `users_controller.soli` maps to `/users`
//! - Function names map to HTTP methods and actions:
//!   - `index()` → GET on collection (e.g., `/users`)
//!   - `show(id)` → GET with param (e.g., `/users/:id`)
//!   - `create()` → POST on collection
//!   - `update(id)` → PUT with param
//!   - `destroy(id)` → DELETE with param
//!   - Other functions → GET `/controller/function_name`

use crate::error::RuntimeError;

/// A route derived from a controller.
#[derive(Debug, Clone)]
pub struct ControllerRoute {
    pub method: String,
    pub path: String,
    pub function_name: String,
    pub has_id_param: bool,
}

/// Derive routes from a controller file.
///
/// # Arguments
/// * `controller_name` - The controller filename without extension (e.g., "users_controller")
/// * `source` - The source code of the controller
pub fn derive_routes_from_controller(
    controller_name: &str,
    source: &str,
) -> Result<Vec<ControllerRoute>, RuntimeError> {
    let base_path = controller_base_path(controller_name);
    let functions = extract_function_names(source);

    let mut routes = Vec::new();

    for (func_name, has_params) in functions {
        if let Some(route) = derive_route(&base_path, &func_name, has_params) {
            routes.push(route);
        }
    }

    Ok(routes)
}

/// Get the base path for a controller.
///
/// - `home_controller` → `/`
/// - `users_controller` → `/users`
pub fn controller_base_path(controller_name: &str) -> String {
    let name = controller_name.trim_end_matches("_controller");

    if name == "home" {
        "/".to_string()
    } else {
        format!("/{}", name)
    }
}

/// Extract function names and whether they have parameters from source code.
///
/// Returns a vector of (function_name, has_params) tuples.
fn extract_function_names(source: &str) -> Vec<(String, bool)> {
    let mut functions = Vec::new();

    // Simple regex-like parsing to find function declarations
    // Looking for: fn function_name(params) or fn function_name()
    for line in source.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("fn ") {
            // Extract function name and check for params
            if let Some(rest) = trimmed.strip_prefix("fn ") {
                // Find the function name (until '(')
                if let Some(paren_pos) = rest.find('(') {
                    let func_name = rest[..paren_pos].trim().to_string();

                    // Check if there are parameters between ( and )
                    if let Some(close_paren) = rest.find(')') {
                        let params_str = &rest[paren_pos + 1..close_paren].trim();
                        // Has params if the params string is not empty
                        // Ignore the 'req' parameter as it's always present for handlers
                        let has_id_param = !params_str.is_empty()
                            && *params_str != "req"
                            && *params_str != "req: Any";

                        functions.push((func_name, has_id_param));
                    }
                }
            }
        }
    }

    functions
}

/// Derive a route from a function name using conventions.
fn derive_route(base_path: &str, func_name: &str, _has_params: bool) -> Option<ControllerRoute> {
    let is_home = base_path == "/";

    match func_name {
        // RESTful actions
        "index" => Some(ControllerRoute {
            method: "GET".to_string(),
            path: base_path.to_string(),
            function_name: func_name.to_string(),
            has_id_param: false,
        }),

        "show" => Some(ControllerRoute {
            method: "GET".to_string(),
            path: if is_home {
                "/:id".to_string()
            } else {
                format!("{}/:id", base_path)
            },
            function_name: func_name.to_string(),
            has_id_param: true,
        }),

        "create" => Some(ControllerRoute {
            method: "POST".to_string(),
            path: base_path.to_string(),
            function_name: func_name.to_string(),
            has_id_param: false,
        }),

        "update" => Some(ControllerRoute {
            method: "PUT".to_string(),
            path: if is_home {
                "/:id".to_string()
            } else {
                format!("{}/:id", base_path)
            },
            function_name: func_name.to_string(),
            has_id_param: true,
        }),

        "destroy" => Some(ControllerRoute {
            method: "DELETE".to_string(),
            path: if is_home {
                "/:id".to_string()
            } else {
                format!("{}/:id", base_path)
            },
            function_name: func_name.to_string(),
            has_id_param: true,
        }),

        "new" => Some(ControllerRoute {
            method: "GET".to_string(),
            path: if is_home {
                "/new".to_string()
            } else {
                format!("{}/new", base_path)
            },
            function_name: func_name.to_string(),
            has_id_param: false,
        }),

        "edit" => Some(ControllerRoute {
            method: "GET".to_string(),
            path: if is_home {
                "/:id/edit".to_string()
            } else {
                format!("{}/:id/edit", base_path)
            },
            function_name: func_name.to_string(),
            has_id_param: true,
        }),

        // Custom actions - map to GET /{base}/{action}
        _ => {
            // Skip private/helper functions (starting with _)
            if func_name.starts_with('_') {
                return None;
            }

            Some(ControllerRoute {
                method: "GET".to_string(),
                path: if is_home {
                    format!("/{}", func_name)
                } else {
                    format!("{}/{}", base_path, func_name)
                },
                function_name: func_name.to_string(),
                has_id_param: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_controller_base_path() {
        assert_eq!(controller_base_path("home_controller"), "/");
        assert_eq!(controller_base_path("users_controller"), "/users");
        assert_eq!(controller_base_path("blog_posts_controller"), "/blog_posts");
    }

    #[test]
    fn test_extract_function_names() {
        let source = r#"
fn index(req: Any) -> Any {
    return {"status": 200};
}

fn show(req: Any) -> Any {
    return {"status": 200};
}

fn _helper() {
    // private
}
"#;

        let functions = extract_function_names(source);
        assert_eq!(functions.len(), 3);
        assert_eq!(functions[0].0, "index");
        assert_eq!(functions[1].0, "show");
        assert_eq!(functions[2].0, "_helper");
    }

    #[test]
    fn test_derive_routes() {
        let routes = derive_routes_from_controller(
            "users_controller",
            r#"
fn index(req: Any) -> Any { return {}; }
fn show(req: Any) -> Any { return {}; }
fn create(req: Any) -> Any { return {}; }
fn update(req: Any) -> Any { return {}; }
fn destroy(req: Any) -> Any { return {}; }
fn custom_action(req: Any) -> Any { return {}; }
fn _private() { }
"#,
        )
        .unwrap();

        // Should have 6 routes (private function excluded)
        assert_eq!(routes.len(), 6);

        // Check specific routes
        let index = routes.iter().find(|r| r.function_name == "index").unwrap();
        assert_eq!(index.method, "GET");
        assert_eq!(index.path, "/users");

        let show = routes.iter().find(|r| r.function_name == "show").unwrap();
        assert_eq!(show.method, "GET");
        assert_eq!(show.path, "/users/:id");

        let create = routes.iter().find(|r| r.function_name == "create").unwrap();
        assert_eq!(create.method, "POST");
        assert_eq!(create.path, "/users");
    }
}
