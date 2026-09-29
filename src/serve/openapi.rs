//! OpenAPI 3 generator.
//!
//! `GET /openapi.json` returns a spec built from the app's registered routes;
//! `GET /openapi` serves a Scalar API-reference UI over it. Both are on under
//! `--dev` and off otherwise, so a production route table isn't exposed unless
//! the app opts in with `SOLI_OPENAPI=1` — then they're available in
//! production too, like `/_metrics`. An explicit `SOLI_OPENAPI=0` turns them
//! off in dev as well.
//!
//! Structure comes from the routes: every route becomes a path + method with
//! its handler as the operationId, path params (`:id`) as required string
//! parameters, and a generic `200`. Doc comments on the action (see
//! `controller::api_docs`) add a summary, description, typed path/query/header
//! parameters, a request body, responses, a tag, `deprecated` or `hidden`; a
//! `permit(params, {…})` whitelist describes the body when no `@body` does.

use std::sync::OnceLock;

use crate::interpreter::builtins::controller::api_docs::{
    openapi_type, schema_from_example, ActionDoc, DocParam, DocPayload,
};
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
        let doc = action_doc(&route.handler_name);
        if doc.as_ref().is_some_and(|d| d.hidden) {
            continue;
        }
        let op = build_operation(&method, &route.handler_name, &params, doc.as_ref());

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

/// The doc comments of the action behind `handler` (`posts#show`,
/// `admin/categories#index`), from the controller registry.
fn action_doc(handler: &str) -> Option<ActionDoc> {
    let (controller, action) = handler.split_once('#')?;
    crate::interpreter::builtins::controller::registry::CONTROLLER_REGISTRY
        .read(|registry| registry.get(controller)?.docs.get(action).cloned())
}

/// One operation: the route's structure, plus what its doc comments add.
fn build_operation(
    method: &str,
    handler: &str,
    path_params: &[String],
    doc: Option<&ActionDoc>,
) -> serde_json::Map<String, serde_json::Value> {
    use serde_json::{json, Map, Value};

    let mut op = Map::new();
    op.insert("operationId".into(), json!(operation_id(method, handler)));
    let summary = doc.and_then(|d| d.summary.clone());
    op.insert(
        "summary".into(),
        json!(summary.unwrap_or_else(|| handler.to_string())),
    );
    if let Some(description) = doc.and_then(|d| d.description.clone()) {
        op.insert("description".into(), json!(description));
    }
    let tag = doc.and_then(|d| d.tag.clone()).or_else(|| {
        handler
            .split('#')
            .next()
            .filter(|t| !t.is_empty())
            .map(String::from)
    });
    if let Some(tag) = tag {
        op.insert("tags".into(), json!([tag]));
    }
    if doc.is_some_and(|d| d.deprecated) {
        op.insert("deprecated".into(), json!(true));
    }

    let param = |p: &DocParam, location: &str, required: bool| {
        let mut v = json!({
            "name": p.name,
            "in": location,
            "required": required,
            "schema": { "type": openapi_type(&p.kind) }
        });
        if let Some(description) = &p.description {
            v["description"] = json!(description);
        }
        v
    };
    let mut parameters: Vec<Value> = path_params
        .iter()
        .map(
            |name| match doc.and_then(|d| d.params.iter().find(|p| &p.name == name)) {
                Some(p) => param(p, "path", true),
                None => json!({
                    "name": name,
                    "in": "path",
                    "required": true,
                    "schema": { "type": "string" }
                }),
            },
        )
        .collect();
    if let Some(d) = doc {
        parameters.extend(d.query.iter().map(|p| param(p, "query", p.required)));
        parameters.extend(d.headers.iter().map(|p| param(p, "header", p.required)));
    }
    if !parameters.is_empty() {
        op.insert("parameters".into(), json!(parameters));
    }

    if matches!(method, "post" | "put" | "patch") {
        if let Some(body) = doc.and_then(request_body) {
            op.insert("requestBody".into(), body);
        }
    }

    let mut responses = Map::new();
    for r in doc.map(|d| d.responses.as_slice()).unwrap_or_default() {
        responses.insert(r.code.clone(), response(&r.code, &r.payload));
    }
    if responses.is_empty() {
        responses.insert("200".into(), json!({ "description": "OK" }));
    }
    op.insert("responses".into(), Value::Object(responses));
    op
}

/// `@body`, else the `permit(...)` shape.
fn request_body(doc: &ActionDoc) -> Option<serde_json::Value> {
    use serde_json::json;
    if let Some(body) = &doc.body {
        let mut out = json!({ "required": true, "content": { "application/json": {} } });
        if let Some(example) = parse_json(body.example.as_deref()) {
            out["content"]["application/json"] =
                json!({ "schema": schema_from_example(&example), "example": example });
        }
        if let Some(description) = &body.description {
            out["description"] = json!(description);
        }
        return Some(out);
    }
    let schema = parse_json(doc.permit_schema.as_deref())?;
    Some(json!({
        "description": "Fields accepted by the action's permit() whitelist",
        "content": { "application/json": { "schema": schema } }
    }))
}

fn response(code: &str, payload: &DocPayload) -> serde_json::Value {
    use serde_json::json;
    let reason = code
        .parse::<u16>()
        .ok()
        .and_then(|c| hyper::StatusCode::from_u16(c).ok())
        .and_then(|c| c.canonical_reason())
        .unwrap_or("Response");
    let mut out =
        json!({ "description": payload.description.clone().unwrap_or_else(|| reason.to_string()) });
    if let Some(example) = parse_json(payload.example.as_deref()) {
        out["content"] = json!({
            "application/json": { "schema": schema_from_example(&example), "example": example }
        });
    }
    out
}

fn parse_json(text: Option<&str>) -> Option<serde_json::Value> {
    serde_json::from_str(text?).ok()
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
    fn an_undocumented_route_keeps_the_structural_operation() {
        let op = build_operation("get", "posts#show", &["id".to_string()], None);
        assert_eq!(
            serde_json::Value::Object(op),
            serde_json::json!({
                "operationId": "get_posts_show",
                "summary": "posts#show",
                "tags": ["posts"],
                "parameters": [
                    { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }
                ],
                "responses": { "200": { "description": "OK" } }
            })
        );
    }

    #[test]
    fn doc_comments_fill_the_operation() {
        let src = r#"
  # Create a post.
  # Notifies followers.
  # @param id Int  Blog key
  # @query draft Bool Save as draft
  # @header X-Request-Id String! Idempotency key
  # @body {"title": "Hello", "views": 3}
  # @response 201 {"_key": "42"}
  # @response 422 Validation failed
  # @tag Blog
  # @deprecated
  def create
  end
"#;
        let docs = crate::interpreter::builtins::controller::api_docs::extract_action_docs(src);
        let op = serde_json::Value::Object(build_operation(
            "post",
            "posts#create",
            &["id".to_string()],
            docs.get("create"),
        ));
        assert_eq!(op["summary"], "Create a post.");
        assert_eq!(op["description"], "Notifies followers.");
        assert_eq!(op["tags"], serde_json::json!(["Blog"]));
        assert_eq!(op["deprecated"], true);
        let params = op["parameters"].as_array().unwrap();
        assert_eq!(params[0]["schema"]["type"], "integer");
        assert_eq!(params[0]["description"], "Blog key");
        assert_eq!(params[1]["in"], "query");
        assert_eq!(params[1]["required"], false);
        assert_eq!(params[2]["in"], "header");
        assert_eq!(params[2]["required"], true);
        let body = &op["requestBody"]["content"]["application/json"];
        assert_eq!(body["schema"]["properties"]["views"]["type"], "integer");
        assert_eq!(body["example"]["title"], "Hello");
        assert_eq!(op["responses"]["201"]["description"], "Created");
        assert_eq!(
            op["responses"]["201"]["content"]["application/json"]["example"]["_key"],
            "42"
        );
        assert_eq!(op["responses"]["422"]["description"], "Validation failed");
        assert!(
            op["responses"].get("200").is_none(),
            "declared responses replace the default"
        );
    }

    #[test]
    fn permit_describes_the_body_of_a_write_only() {
        let src = "  def update
    permit(params, {\"title\": true, \"tags\": []})
  end
";
        let docs = crate::interpreter::builtins::controller::api_docs::extract_action_docs(src);
        let put = serde_json::Value::Object(build_operation(
            "put",
            "posts#update",
            &[],
            docs.get("update"),
        ));
        let schema = &put["requestBody"]["content"]["application/json"]["schema"];
        assert_eq!(schema["properties"]["tags"]["type"], "array");
        assert_eq!(
            put["summary"], "posts#update",
            "no doc block: summary stays the handler"
        );
        let get = serde_json::Value::Object(build_operation(
            "get",
            "posts#update",
            &[],
            docs.get("update"),
        ));
        assert!(get.get("requestBody").is_none());
    }

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
