//! OpenAPI 3 generator.
//!
//! `GET /openapi.json` returns a spec built from the app's registered routes;
//! `GET /openapi` serves a Scalar API-reference UI over it. Both are on under
//! `--dev` and off otherwise, so a production route table isn't exposed unless
//! the app opts in with `SOLI_OPENAPI=1` — then they're available in
//! production too, like `/_metrics`. An explicit `SOLI_OPENAPI=0` turns them
//! off in dev as well.
//!
//! There is no annotation infrastructure to read types/bodies from, so the spec
//! is structural: every route becomes a path + method with its handler as the
//! operationId/summary, path params (`:id`) as required string parameters, and a
//! generic `200`. It's a discoverability aid, not a hand-authored contract.

use std::sync::OnceLock;

use crate::interpreter::builtins::server::get_routes;

/// Whether the OpenAPI endpoints are enabled: `SOLI_OPENAPI` when it is set
/// (`1`/`true` on, anything else off), otherwise whether the server runs with
/// `--dev`. The variable is read once, process-wide — mirrors
/// `metrics_enabled()`; dev mode is a process-global set at startup.
pub fn openapi_enabled() -> bool {
    static EXPLICIT: OnceLock<Option<bool>> = OnceLock::new();
    let explicit =
        *EXPLICIT.get_or_init(|| std::env::var("SOLI_OPENAPI").ok().map(|v| parse_flag(&v)));
    resolve_enabled(
        explicit,
        crate::interpreter::builtins::template::is_dev_mode(),
    )
}

fn parse_flag(value: &str) -> bool {
    value == "1" || value.eq_ignore_ascii_case("true")
}

/// An explicit `SOLI_OPENAPI` wins; without one, dev mode decides.
fn resolve_enabled(explicit: Option<bool>, dev_mode: bool) -> bool {
    explicit.unwrap_or(dev_mode)
}

/// The spec title (`SOLI_OPENAPI_TITLE`, default `"Soli API"`).
fn spec_title() -> String {
    std::env::var("SOLI_OPENAPI_TITLE").unwrap_or_else(|_| "Soli API".to_string())
}

/// Convert a Soli path pattern (`/posts/:id`, `/files/*path`) to an OpenAPI
/// path (`/posts/{id}`, `/files/{path}`) plus the extracted parameter names.
fn openapi_path(pattern: &str) -> (String, Vec<String>) {
    let mut params = Vec::new();
    let mut out = String::new();
    for seg in pattern.split('/') {
        if seg.is_empty() {
            continue;
        }
        out.push('/');
        if let Some(name) = seg.strip_prefix(':').or_else(|| seg.strip_prefix('*')) {
            out.push('{');
            out.push_str(name);
            out.push('}');
            params.push(name.to_string());
        } else {
            out.push_str(seg);
        }
    }
    if out.is_empty() {
        out.push('/');
    }
    (out, params)
}

/// A stable, unique operationId for a route.
fn operation_id(method: &str, handler: &str) -> String {
    let sanitized: String = handler
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{}_{}", method.to_lowercase(), sanitized)
}

/// Build the OpenAPI 3 document from the registered app routes. Internal
/// (`/_*`, `/__*`) paths and non-HTTP verbs (WebSocket) are skipped; routes
/// sharing a path collapse into one path item with multiple methods.
pub fn generate_spec() -> serde_json::Value {
    use serde_json::{json, Map, Value};

    const HTTP_METHODS: [&str; 7] = ["get", "post", "put", "patch", "delete", "head", "options"];

    let mut paths: Map<String, Value> = Map::new();
    for route in get_routes() {
        if route.path_pattern.starts_with("/_") {
            continue; // framework/internal (also covers /__*)
        }
        let method = route.method.to_lowercase();
        if !HTTP_METHODS.contains(&method.as_str()) {
            continue; // e.g. WS
        }
        let (opath, params) = openapi_path(&route.path_pattern);

        let mut op = Map::new();
        op.insert(
            "operationId".into(),
            json!(operation_id(&method, &route.handler_name)),
        );
        op.insert("summary".into(), json!(route.handler_name.clone()));
        if let Some(tag) = route.handler_name.split('#').next() {
            if !tag.is_empty() {
                op.insert("tags".into(), json!([tag]));
            }
        }
        if !params.is_empty() {
            let ps: Vec<Value> = params
                .iter()
                .map(|p| {
                    json!({
                        "name": p,
                        "in": "path",
                        "required": true,
                        "schema": { "type": "string" }
                    })
                })
                .collect();
            op.insert("parameters".into(), json!(ps));
        }
        op.insert(
            "responses".into(),
            json!({ "200": { "description": "OK" } }),
        );

        let entry = paths.entry(opath).or_insert_with(|| json!({}));
        if let Value::Object(m) = entry {
            m.insert(method, Value::Object(op));
        }
    }

    json!({
        "openapi": "3.0.3",
        "info": { "title": spec_title(), "version": "1.0.0" },
        "paths": Value::Object(paths),
    })
}

/// The spec serialized as pretty JSON.
pub fn generate_spec_json() -> String {
    serde_json::to_string_pretty(&generate_spec()).unwrap_or_else(|_| "{}".to_string())
}

/// A self-hosting Scalar API-reference page pointed at `/openapi.json`. Scalar
/// loads from a CDN, so this needs network access in the browser (documented).
pub fn ui_page() -> String {
    "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>API Reference</title></head><body>\
<script id=\"api-reference\" data-url=\"/openapi.json\"></script>\
<script src=\"https://cdn.jsdelivr.net/npm/@scalar/api-reference\"></script>\
</body></html>"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_mode_enables_it_unless_the_variable_says_otherwise() {
        assert!(resolve_enabled(None, true), "on by default under --dev");
        assert!(
            !resolve_enabled(None, false),
            "off by default in production"
        );
        assert!(
            !resolve_enabled(Some(parse_flag("0")), true),
            "SOLI_OPENAPI=0 wins in dev"
        );
        assert!(
            resolve_enabled(Some(parse_flag("true")), false),
            "SOLI_OPENAPI=true in production"
        );
        assert!(!resolve_enabled(Some(parse_flag("off")), false));
    }

    #[test]
    fn openapi_path_templates_params() {
        assert_eq!(
            openapi_path("/posts/:id"),
            ("/posts/{id}".into(), vec!["id".into()])
        );
        assert_eq!(
            openapi_path("/users/:uid/posts/:pid"),
            (
                "/users/{uid}/posts/{pid}".into(),
                vec!["uid".into(), "pid".into()]
            )
        );
        assert_eq!(
            openapi_path("/files/*path"),
            ("/files/{path}".into(), vec!["path".into()])
        );
        assert_eq!(openapi_path("/"), ("/".into(), Vec::<String>::new()));
        assert_eq!(
            openapi_path("/health"),
            ("/health".into(), Vec::<String>::new())
        );
    }

    #[test]
    fn operation_id_is_sanitized() {
        assert_eq!(operation_id("GET", "posts#show"), "get_posts_show");
        assert_eq!(operation_id("post", "users#create"), "post_users_create");
    }
}
