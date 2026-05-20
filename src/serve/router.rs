//! Convention-based routing for MVC controllers.
//!
//! Routes are derived from controller path + function name:
//! - `home_controller.sl` maps to root `/`
//! - `users_controller.sl` maps to `/users`
//! - `admin/merchants_controller.sl` maps to `/admin/merchants`
//!   (handler key `admin/merchants`, class `AdminMerchantsController`)
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
/// Accepts either a bare controller filename stem (`home_controller`,
/// `users_controller`) or a nested key with `/` separators
/// (`admin/merchants_controller`).
///
/// - `home_controller` → `/`
/// - `users_controller` → `/users`
/// - `admin/merchants_controller` → `/admin/merchants`
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

    // Simple regex-like parsing to find function declarations.
    // Accept both `fn name(...)` and `def name(...)` — the lexer treats them
    // as the same keyword, so route derivation must too. Without this, a
    // function-style controller written with `def` silently has no routes.
    for line in source.lines() {
        let trimmed = line.trim();

        let rest = trimmed
            .strip_prefix("fn ")
            .or_else(|| trimmed.strip_prefix("def "));

        if let Some(rest) = rest {
            // Find the function name (until '(')
            if let Some(paren_pos) = rest.find('(') {
                let func_name = rest[..paren_pos].trim().to_string();

                // Check if there are parameters between ( and )
                if let Some(close_paren) = rest.find(')') {
                    let params_str = &rest[paren_pos + 1..close_paren].trim();
                    // Has params if the params string is not empty
                    // Ignore the 'req' parameter as it's always present for handlers
                    let has_id_param =
                        !params_str.is_empty() && *params_str != "req" && *params_str != "req: Any";

                    functions.push((func_name, has_id_param));
                }
            } else {
                // Soli allows omitting `()` for no-arg functions: `fn index`.
                // Stop at the first whitespace / `{` / `->` so we don't pull
                // in a return-type annotation or a brace from a one-liner.
                let func_name: String = rest
                    .chars()
                    .take_while(|c| !c.is_whitespace() && *c != '{')
                    .collect();
                if !func_name.is_empty() {
                    functions.push((func_name, false));
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

/// Convert a controller key (e.g., "posts", "user_profiles", "admin/merchants")
/// to PascalCase class name (e.g., "PostsController", "UserProfilesController",
/// "AdminMerchantsController"). Both `_` and `/` act as word separators.
pub fn to_pascal_case_controller(controller_key: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;

    for c in controller_key.chars() {
        if c == '_' || c == '/' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }

    result.push_str("Controller");
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_controller_base_path() {
        assert_eq!(controller_base_path("home_controller"), "/");
        assert_eq!(controller_base_path("users_controller"), "/users");
        assert_eq!(controller_base_path("blog_posts_controller"), "/blog_posts");
        assert_eq!(
            controller_base_path("admin/merchants_controller"),
            "/admin/merchants"
        );
    }

    #[test]
    fn test_to_pascal_case_controller() {
        assert_eq!(to_pascal_case_controller("posts"), "PostsController");
        assert_eq!(
            to_pascal_case_controller("user_profiles"),
            "UserProfilesController"
        );
        assert_eq!(
            to_pascal_case_controller("admin/merchants"),
            "AdminMerchantsController"
        );
        assert_eq!(
            to_pascal_case_controller("admin/user_profiles"),
            "AdminUserProfilesController"
        );
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
    fn test_extract_function_names_no_parens() {
        // Soli allows omitting `()` for zero-arg functions. The route parser
        // must recognise this form, otherwise function-based controllers
        // written in the idiomatic style (e.g. `fn builtins_websocket` in
        // www/app/controllers/docs_controller.sl) silently have no actions
        // registered and every request 500s with "Action not found".
        let source = r#"
fn index
    render("home")
end

fn show
    render("show")
end

fn create(req)
    return {"status": 201};
end
"#;

        let functions = extract_function_names(source);
        assert_eq!(functions.len(), 3);
        assert_eq!(functions[0].0, "index");
        assert!(!functions[0].1, "no-paren form must report has_id_param=false");
        assert_eq!(functions[1].0, "show");
        assert!(!functions[1].1);
        assert_eq!(functions[2].0, "create");
    }

    #[test]
    fn test_derive_routes_no_parens() {
        // Same scenario as test_extract_function_names_no_parens but via the
        // higher-level entry point, to lock in the end-to-end behaviour.
        let routes = derive_routes_from_controller(
            "docs_controller",
            r#"
fn index
    render("docs/index")
end

fn show
    render("docs/show")
end
"#,
        )
        .unwrap();

        assert!(routes.iter().any(|r| r.function_name == "index"));
        assert!(routes.iter().any(|r| r.function_name == "show"));
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
