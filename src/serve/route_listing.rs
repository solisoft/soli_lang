//! `soli routes` support: load an app's route table without starting the
//! server, and format it for display.
//!
//! Mirrors the serve boot sequence (middleware → engine mounts → routes DSL →
//! `config/routes.sl` → engine routes) so the listing is exactly what the
//! server would register. Rows keep registration order — that is also the
//! match-precedence order, so the table reads top-to-bottom the way requests
//! are matched. Everything runs on the calling thread (the route registry is
//! thread-local); no workers, no DB.

use std::path::Path;

use crate::interpreter::builtins::router;
use crate::interpreter::builtins::server::{self, Route};
use crate::interpreter::Interpreter;

use super::app_loader::{define_routes_dsl, execute_file, load_middleware};
use super::engine_loader;
use super::hot_reload::FileTracker;
use super::websocket::WebSocketRoute;

/// The collected route table: HTTP routes in registration order, plus
/// WebSocket routes (which live in a separate registry).
pub struct RouteListing {
    pub routes: Vec<Route>,
    pub websockets: Vec<WebSocketRoute>,
}

/// Load `config/routes.sl` (and engine routes) exactly like server boot and
/// return the resulting table. Requires an explicit routes file — apps that
/// rely purely on controller-convention routing aren't supported here, since
/// the boot path only keeps convention routes when no routes.sl exists.
pub fn collect_routes(app_path: &Path) -> Result<RouteListing, String> {
    if !app_path.is_dir() {
        return Err(format!("Folder '{}' does not exist", app_path.display()));
    }
    let routes_file = app_path.join("config").join("routes.sl");
    if !routes_file.is_file() {
        return Err(format!(
            "No config/routes.sl found in '{}' — is this a Soli app folder?",
            app_path.display()
        ));
    }

    let mut interpreter = Interpreter::new();

    // `.env` for parity with serve — middleware/config code may read env
    // vars. No `init_db_config()`: the routing DSL never touches SoliDB.
    super::env_loader::load_env_files(app_path);

    router::reset_router_context();

    // Load app middleware first so `middleware("auth", ...)` scopes resolve
    // without "Middleware 'x' not found" stderr noise. The printable
    // `middleware_names` are recorded either way.
    let middleware_dir = app_path.join("app").join("middleware");
    if middleware_dir.is_dir() {
        let mut file_tracker = FileTracker::new();
        if let Err(e) = load_middleware(&mut interpreter, &middleware_dir, &mut file_tracker) {
            eprintln!("Warning: Failed to load middleware: {}", e);
        }
    }

    // Engine mounts (config/engines.sl) — needed so engine routes.sl files
    // are found below. Engine controllers/models are skipped: any
    // convention routes they'd register are wiped by `clear_routes()` when
    // an explicit routes.sl exists, exactly as at server boot.
    let mut has_engines = false;
    match engine_loader::load_engines_config(app_path) {
        Ok(config) => {
            if !config.engines.is_empty() {
                has_engines = true;
                if let Err(e) = engine_loader::mount_engines(app_path, &config) {
                    eprintln!("Warning: Failed to mount engines: {}", e);
                }
            }
        }
        Err(e) => eprintln!("Warning: Failed to load engine config: {}", e),
    }

    // `config/application.sl` runs before routes at boot; routes.sl may use
    // helpers it defines. Non-fatal — a failure there shouldn't block a
    // route listing.
    let application_file = app_path.join("config").join("application.sl");
    if application_file.is_file() {
        if let Err(e) = execute_file(&mut interpreter, &application_file) {
            eprintln!("Warning: config/application.sl failed: {}", e);
        }
    }

    // The exact serve sequence: DSL → clear convention routes → routes.sl →
    // engine routes.
    define_routes_dsl(&mut interpreter).map_err(|e| format!("Routes DSL error: {}", e))?;
    server::clear_routes();
    execute_file(&mut interpreter, &routes_file)
        .map_err(|e| format!("Error in {}: {}", routes_file.display(), e))?;
    if has_engines {
        if let Err(e) = engine_loader::load_engine_routes(&mut interpreter) {
            eprintln!("Warning: Failed to load engine routes: {}", e);
        }
    }

    Ok(RouteListing {
        routes: server::get_routes(),
        websockets: super::websocket::get_websocket_routes(),
    })
}

/// One display row, unified across HTTP and WebSocket routes.
struct Row {
    method: String,
    path: String,
    handler: String,
    helper: String,
    middleware: String,
}

fn build_rows(listing: &RouteListing, grep: Option<&str>) -> Vec<Row> {
    let needle = grep.map(str::to_lowercase);
    let matches = |row: &Row| {
        let Some(n) = &needle else { return true };
        row.method.to_lowercase().contains(n)
            || row.path.to_lowercase().contains(n)
            || row.handler.to_lowercase().contains(n)
            || row.helper.to_lowercase().contains(n)
    };

    let mut rows: Vec<Row> = Vec::with_capacity(listing.routes.len() + listing.websockets.len());
    for r in &listing.routes {
        rows.push(Row {
            method: r.method.clone(),
            path: r.path_pattern.clone(),
            handler: r.handler_name.clone(),
            helper: r
                .name
                .as_ref()
                .map(|n| format!("{}_path", n))
                .unwrap_or_default(),
            middleware: r.middleware_names.join(", "),
        });
    }
    for ws in &listing.websockets {
        rows.push(Row {
            method: "WS".to_string(),
            path: ws.path_pattern.clone(),
            handler: ws.handler_name.clone(),
            helper: String::new(),
            middleware: String::new(),
        });
    }
    rows.retain(|r| matches(r));
    rows
}

/// Render the table (two-space indent, bold header, dynamic column widths).
pub fn format_table(listing: &RouteListing, grep: Option<&str>) -> String {
    let rows = build_rows(listing, grep);
    let mut out = String::new();

    if rows.is_empty() {
        match grep {
            Some(pattern) => out.push_str(&format!("  No routes match '{}'.\n", pattern)),
            None => out.push_str("  No routes defined.\n"),
        }
        return out;
    }

    let headers = ["METHOD", "PATH", "HANDLER", "HELPER", "MIDDLEWARE"];
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in &rows {
        let cells = [
            &row.method,
            &row.path,
            &row.handler,
            &row.helper,
            &row.middleware,
        ];
        for (w, cell) in widths.iter_mut().zip(cells) {
            *w = (*w).max(cell.len());
        }
    }

    out.push('\n');
    out.push_str("  \x1b[1m");
    for (i, (header, width)) in headers.iter().zip(&widths).enumerate() {
        if i > 0 {
            out.push_str("  ");
        }
        out.push_str(&format!("{:<width$}", header, width = width));
    }
    out.push_str("\x1b[0m\n");

    for row in &rows {
        let cells = [
            &row.method,
            &row.path,
            &row.handler,
            &row.helper,
            &row.middleware,
        ];
        out.push_str("  ");
        for (i, (cell, width)) in cells.iter().zip(&widths).enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            out.push_str(&format!("{:<width$}", cell, width = width));
        }
        // No trailing padding noise on the last column.
        while out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
    }

    let ws_count = rows.iter().filter(|r| r.method == "WS").count();
    let noun = if rows.len() == 1 { "route" } else { "routes" };
    out.push('\n');
    if ws_count > 0 {
        out.push_str(&format!(
            "  {} {} ({} websocket)\n",
            rows.len(),
            noun,
            ws_count
        ));
    } else {
        out.push_str(&format!("  {} {}\n", rows.len(), noun));
    }
    out
}

/// Render the listing as pretty-printed JSON: a flat array of
/// `{method, path, handler, name, middleware}` objects (WebSocket rows use
/// `"method": "WS"`). Stable shape for tooling and agents.
pub fn format_json(listing: &RouteListing, grep: Option<&str>) -> String {
    let rows = build_rows(listing, grep);
    let entries: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "method": row.method,
                "path": row.path,
                "handler": row.handler,
                "name": if row.helper.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(
                        row.helper.trim_end_matches("_path").to_string(),
                    )
                },
                "middleware": row
                    .middleware
                    .split(", ")
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::Value::Array(entries))
        .unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_app(routes_sl: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("config")).unwrap();
        fs::write(dir.path().join("config").join("routes.sl"), routes_sl).unwrap();
        dir
    }

    #[test]
    fn collects_basic_routes_and_resources_in_order() {
        let app = write_app(
            r#"
get("/", "home#index", "root");
post("/login", "sessions#create");
resources("posts");
"#,
        );
        let listing = collect_routes(app.path()).unwrap();

        // Registration order preserved: explicit routes first.
        assert_eq!(listing.routes[0].method, "GET");
        assert_eq!(listing.routes[0].path_pattern, "/");
        assert_eq!(listing.routes[0].handler_name, "home#index");
        assert_eq!(listing.routes[0].name.as_deref(), Some("root"));
        assert_eq!(listing.routes[1].handler_name, "sessions#create");
        assert!(listing.routes[1].name.is_none());

        // resources("posts") expands to individual routes with Rails names.
        let find = |method: &str, path: &str| {
            listing
                .routes
                .iter()
                .find(|r| r.method == method && r.path_pattern == path)
                .unwrap_or_else(|| panic!("missing {} {}", method, path))
        };
        assert_eq!(find("GET", "/posts").handler_name, "posts#index");
        assert_eq!(find("GET", "/posts").name.as_deref(), Some("posts"));
        assert_eq!(find("GET", "/posts/:id").name.as_deref(), Some("post"));
        assert_eq!(find("GET", "/posts/new").name.as_deref(), Some("new_post"));
        assert_eq!(
            find("GET", "/posts/:id/edit").name.as_deref(),
            Some("edit_post")
        );
        assert_eq!(find("DELETE", "/posts/:id").handler_name, "posts#destroy");
    }

    #[test]
    fn collects_namespace_and_middleware_scopes() {
        let app = write_app(
            r#"
namespace("admin", fn() {
    get("/dashboard", "admin#dashboard");
});
middleware("auth", fn() {
    get("/secret", "secret#show");
});
"#,
        );
        let listing = collect_routes(app.path()).unwrap();

        assert!(listing
            .routes
            .iter()
            .any(|r| r.path_pattern == "/admin/dashboard"));
        let secret = listing
            .routes
            .iter()
            .find(|r| r.path_pattern == "/secret")
            .unwrap();
        assert_eq!(secret.middleware_names, vec!["auth".to_string()]);
    }

    #[test]
    fn collects_websocket_routes() {
        // WEBSOCKET_ROUTES is a process-global registry (unlike the
        // thread-local HTTP table), so assert containment of a unique path,
        // never exact equality.
        let app = write_app(r#"websocket("/ws/route_listing_test_x9z", "chat#handle");"#);
        let listing = collect_routes(app.path()).unwrap();
        assert!(listing
            .websockets
            .iter()
            .any(|w| w.path_pattern == "/ws/route_listing_test_x9z"
                && w.handler_name == "chat#handle"));
    }

    #[test]
    fn missing_folder_and_missing_routes_file_error() {
        let err = collect_routes(Path::new("/nonexistent-soli-app-x9z"))
            .err()
            .unwrap();
        assert!(err.contains("does not exist"), "got: {}", err);

        let dir = tempfile::tempdir().unwrap();
        let err = collect_routes(dir.path()).err().unwrap();
        assert!(err.contains("config/routes.sl"), "got: {}", err);
    }

    #[test]
    fn broken_routes_file_reports_error() {
        let app = write_app("get(/broken syntax");
        let err = collect_routes(app.path()).err().unwrap();
        assert!(err.contains("routes.sl"), "got: {}", err);
    }

    fn sample_listing() -> RouteListing {
        RouteListing {
            routes: vec![
                Route {
                    method: "GET".to_string(),
                    path_pattern: "/".to_string(),
                    handler_name: "home#index".to_string(),
                    name: Some("root".to_string()),
                    middleware: vec![],
                    middleware_names: vec![],
                },
                Route {
                    method: "POST".to_string(),
                    path_pattern: "/posts".to_string(),
                    handler_name: "posts#create".to_string(),
                    name: None,
                    middleware: vec![],
                    middleware_names: vec!["auth".to_string()],
                },
            ],
            websockets: vec![WebSocketRoute {
                path_pattern: "/ws/chat".to_string(),
                handler_name: "chat#handle".to_string(),
            }],
        }
    }

    #[test]
    fn format_table_renders_helper_middleware_and_count() {
        let out = format_table(&sample_listing(), None);
        assert!(out.contains("root_path"), "got: {}", out);
        assert!(out.contains("auth"), "got: {}", out);
        assert!(out.contains("WS"), "got: {}", out);
        assert!(out.contains("3 routes (1 websocket)"), "got: {}", out);
    }

    #[test]
    fn format_table_grep_filters_case_insensitively() {
        let out = format_table(&sample_listing(), Some("POSTS"));
        assert!(out.contains("posts#create"), "got: {}", out);
        assert!(!out.contains("home#index"), "got: {}", out);
        assert!(out.contains("1 route\n"), "got: {}", out);

        let out = format_table(&sample_listing(), Some("nomatch-x9z"));
        assert!(out.contains("No routes match"), "got: {}", out);
    }

    #[test]
    fn format_json_is_valid_and_carries_fields() {
        let out = format_json(&sample_listing(), None);
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["method"], "GET");
        assert_eq!(arr[0]["name"], "root");
        assert_eq!(arr[1]["name"], serde_json::Value::Null);
        assert_eq!(arr[1]["middleware"][0], "auth");
        assert_eq!(arr[2]["method"], "WS");
    }
}
