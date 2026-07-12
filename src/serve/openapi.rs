//! Opt-in OpenAPI 3 generator.
//!
//! `GET /openapi.json` returns a spec built from the app's registered routes;
//! `GET /openapi` serves a Scalar API-reference UI over it. Both are gated by
//! `SOLI_OPENAPI` (off by default) so the route table isn't exposed unless the
//! app opts in — then they're available in production too, like `/_metrics`.
//!
//! There is no annotation infrastructure to read types/bodies from, so the spec
//! is structural: every route becomes a path + method with its handler as the
//! operationId/summary, path params (`:id`) as required string parameters, and a
//! generic `200`. It's a discoverability aid, not a hand-authored contract.

use std::sync::OnceLock;

use crate::interpreter::builtins::server::get_routes;

/// Whether the OpenAPI endpoints are enabled (`SOLI_OPENAPI=1`/`true`). Read
/// once, process-wide — mirrors `metrics_enabled()`.
pub fn openapi_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SOLI_OPENAPI")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    })
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
