//! JSON twins of the dev bar's request inspector and of the error groups, for
//! `soli agent` (see [`crate::dev_agent`]).
//!
//! `GET /__solidev/requests`, `GET /__solidev/requests/:id` and
//! `GET /__solidev/errors` live under `/__solidev/`, so `dev_routes::dispatch`
//! holds them to the same trusted-peer and local-`Host` gates as the HTML
//! inspector, and they do not exist outside `--dev`.
//!
//! They carry what the HTML views already show and nothing more: no request
//! headers, cookies or bodies. The HTML `GET /__solidev/request/:id` the dev
//! bar parses is left as it is.

use bytes::Bytes;
use hyper::{Response, StatusCode};
use serde_json::{json, Value};

use super::dev_bar::{
    detect_n_plus_one, detect_ungrouped_reads, DevBarContext, UNGROUPED_READ_THRESHOLD,
};
use super::{dev_errors, dev_store, error_tracker, full, ResponseBody};

/// Open groups returned by `GET /__solidev/errors` when no `limit` is given.
const DEFAULT_ERROR_LIMIT: usize = 20;
const MAX_ERROR_LIMIT: usize = 200;

/// One row of `GET /__solidev/requests`.
pub(crate) fn request_summary(ctx: &DevBarContext) -> Value {
    json!({
        "id": ctx.request_id,
        "method": ctx.method,
        "path": ctx.path,
        "status": ctx.status,
        "elapsed_us": ctx.elapsed_us,
        "route": ctx.route,
        "query_count": ctx.queries.len(),
    })
}

/// `GET /__solidev/requests/:id`: the panels of one stored request.
///
/// `n_plus_one` and `ungrouped_reads` come from the detectors the dev bar
/// renders its alerts with, and are left out when they find nothing.
pub(crate) fn request_detail(ctx: &DevBarContext) -> Value {
    let queries: Vec<Value> = ctx
        .queries
        .iter()
        .map(|q| {
            json!({
                "statement": q.query,
                "binds": q.bind_vars,
                "duration_ms": q.duration_ms,
                "grouped": q.grouped,
                "replica": q.replica,
            })
        })
        .collect();
    let phases: Vec<Value> = ctx
        .phases
        .iter()
        .map(|(name, us)| json!({ "name": name, "us": us }))
        .collect();
    let middlewares: Vec<Value> = ctx
        .middlewares
        .iter()
        .map(|(name, us)| json!({ "name": name, "us": us }))
        .collect();
    let views: Vec<Value> = ctx
        .views
        .iter()
        .map(|(_, _, name, us)| json!({ "name": name, "us": us }))
        .collect();
    let http: Vec<Value> = ctx
        .http_requests
        .iter()
        .map(|h| {
            json!({
                "method": h.method,
                "url": h.url,
                "status": h.status,
                "duration_ms": h.duration_ms,
                "error": h.error,
            })
        })
        .collect();
    let kv: Vec<Value> = ctx
        .kv_calls
        .iter()
        .map(|k| {
            json!({
                "command": k.command,
                "key": k.key,
                "duration_ms": k.duration_ms,
                "error": k.error,
            })
        })
        .collect();

    let mut out = request_summary(ctx);
    let obj = out.as_object_mut().expect("summary is an object");
    obj.insert("phases".into(), Value::Array(phases));
    obj.insert("middlewares".into(), Value::Array(middlewares));
    obj.insert("views".into(), Value::Array(views));
    obj.insert("queries".into(), Value::Array(queries));
    obj.insert("http".into(), Value::Array(http));
    obj.insert("kv".into(), Value::Array(kv));
    if !ctx.warnings.is_empty() {
        obj.insert("warnings".into(), json!(ctx.warnings));
    }
    let n1 = detect_n_plus_one(&ctx.queries, 2);
    if !n1.is_empty() {
        let groups: Vec<Value> = n1
            .into_iter()
            .map(|(template, count, total_us)| {
                json!({ "statement": template, "count": count, "total_us": total_us })
            })
            .collect();
        obj.insert("n_plus_one".into(), Value::Array(groups));
    }
    let ungrouped = detect_ungrouped_reads(&ctx.queries, UNGROUPED_READ_THRESHOLD);
    if !ungrouped.is_empty() {
        obj.insert("ungrouped_reads".into(), json!(ungrouped));
    }
    out
}

fn json_response(status: StatusCode, body: &Value) -> Response<ResponseBody> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json; charset=utf-8")
        .header("Cache-Control", "no-store")
        .body(full(Bytes::from(body.to_string())))
        .unwrap()
}

/// `GET /__solidev/requests` — the ring buffer, newest first.
pub(super) fn handle_requests() -> Response<ResponseBody> {
    let mut rows = dev_store::map_all(|_, ctx| request_summary(ctx));
    rows.reverse();
    json_response(StatusCode::OK, &json!({ "requests": rows }))
}

/// `GET /__solidev/requests/:id`.
pub(super) fn handle_request(id: &str) -> Response<ResponseBody> {
    match dev_store::get(id) {
        Some(ctx) => json_response(StatusCode::OK, &request_detail(&ctx)),
        None => json_response(
            StatusCode::NOT_FOUND,
            &json!({ "error": format!(
                "request {id} is no longer in the dev buffer (it keeps the last {} requests)",
                dev_store::CAP
            ) }),
        ),
    }
}

/// `GET /__solidev/errors[?limit=N]` — open error groups, each with its latest
/// sample. Reads `error_tracker` the way `/__soli/errors` does; the sample was
/// redacted when it was stored.
pub(super) fn handle_errors(query: Option<&str>) -> Response<ResponseBody> {
    let limit = query
        .map(crate::interpreter::builtins::server::parse_query_string)
        .and_then(|params| params.get("limit").and_then(|v| v.parse::<usize>().ok()))
        .unwrap_or(DEFAULT_ERROR_LIMIT)
        .clamp(1, MAX_ERROR_LIMIT);
    let groups = match error_tracker::list("open", limit) {
        Ok(rows) => rows,
        Err(e) => {
            return json_response(
                StatusCode::SERVICE_UNAVAILABLE,
                &json!({ "error": format!("error groups unavailable: {e}") }),
            )
        }
    };
    let out: Vec<Value> = groups
        .iter()
        .map(|row| {
            let key = row.get("_key").and_then(Value::as_str).unwrap_or("");
            let latest = if error_tracker::valid_fingerprint(key) {
                error_tracker::get(key)
                    .ok()
                    .flatten()
                    .and_then(|group| {
                        group
                            .get("samples")
                            .and_then(Value::as_array)
                            .and_then(|s| s.first().cloned())
                    })
                    .map(|sample| dev_errors::sample_for_agent(&sample))
            } else {
                None
            };
            json!({
                "fingerprint": key,
                "message": row.get("message"),
                "location": row.get("location"),
                "count": row.get("count"),
                "first_seen": row.get("first_seen"),
                "last_seen": row.get("last_seen"),
                "regressed_at": row.get("regressed_at"),
                "last_request": row.get("last_request"),
                "latest_sample": latest,
            })
        })
        .collect();
    json_response(StatusCode::OK, &json!({ "errors": out }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::builtins::model::query_log::LoggedQuery;

    fn query(statement: &str, key: &str) -> LoggedQuery {
        let mut binds = std::collections::HashMap::new();
        binds.insert("k".to_string(), json!(key));
        LoggedQuery {
            query: statement.to_string(),
            bind_vars: Some(binds),
            duration_ms: 1.5,
            grouped: false,
            replica: false,
        }
    }

    fn ctx(queries: Vec<LoggedQuery>) -> DevBarContext {
        DevBarContext {
            method: "GET".into(),
            path: "/orders".into(),
            status: 200,
            elapsed_us: 412_000,
            request_id: "r_1".into(),
            route: Some("orders#index".into()),
            queries,
            db_role: None,
            http_requests: vec![],
            kv_calls: vec![],
            phases: vec![("middleware".into(), 10), ("view".into(), 300)],
            middlewares: vec![],
            views: vec![(1, None, "orders/index".into(), 250)],
            spans: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn summary_has_the_list_columns() {
        let row = request_summary(&ctx(vec![query("FOR o IN orders RETURN o", "x")]));
        assert_eq!(row["id"], "r_1");
        assert_eq!(row["method"], "GET");
        assert_eq!(row["path"], "/orders");
        assert_eq!(row["status"], 200);
        assert_eq!(row["elapsed_us"], 412_000);
        assert_eq!(row["route"], "orders#index");
        assert_eq!(row["query_count"], 1);
    }

    #[test]
    fn detail_carries_queries_phases_and_n_plus_one() {
        let n1 = "FOR c IN customers FILTER c._key == @k RETURN c";
        let detail = request_detail(&ctx(vec![
            query("FOR o IN orders RETURN o", "x"),
            query(n1, "a"),
            query(n1, "b"),
        ]));
        assert_eq!(detail["queries"].as_array().unwrap().len(), 3);
        assert_eq!(detail["queries"][1]["statement"], n1);
        assert_eq!(detail["queries"][1]["binds"]["k"], "a");
        assert_eq!(detail["queries"][1]["duration_ms"], 1.5);
        assert_eq!(detail["phases"][1]["name"], "view");
        assert_eq!(detail["views"][0]["name"], "orders/index");
        assert_eq!(detail["n_plus_one"][0]["statement"], n1);
        assert_eq!(detail["n_plus_one"][0]["count"], 2);
        // Nothing the HTML inspector leaves out.
        assert!(detail.get("headers").is_none());
        assert!(detail.get("body").is_none());
        assert!(detail.get("cookies").is_none());
    }

    #[test]
    fn detail_omits_n_plus_one_when_none_is_found() {
        let detail = request_detail(&ctx(vec![query("FOR o IN orders RETURN o", "x")]));
        assert!(detail.get("n_plus_one").is_none());
        assert!(detail.get("warnings").is_none());
    }
}
