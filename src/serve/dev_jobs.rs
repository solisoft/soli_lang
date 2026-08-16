//! The job dashboard at `/__soli/jobs`: inspect queues, cancel pending
//! work, and retry failed/dead rows.
//!
//! Open in `--dev`. In production it is served only when
//! `SOLI_JOBS_USER` + `SOLI_JOBS_PASSWORD` and/or `SOLI_JOBS_TOKEN` are
//! set; otherwise the path 404s. Auth is HTTP Basic and/or `Bearer`.

use base64::Engine;
use hyper::{header::HeaderMap, Response, StatusCode};

use crate::interpreter::builtins::crypto::do_secure_compare;

use crate::interpreter::builtins::server::parse_query_string;
use crate::jobs::store;

use super::{dev_bar, full, html_ok, Bytes, ResponseBody};

const DEFAULT_PER_PAGE: usize = 25;

fn jobs_page(body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Soli \u{b7} Jobs</title>\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<style>body{{margin:0;font-family:'JetBrains Mono',ui-monospace,monospace;background:#08090b;color:#c9d1d9;padding:1.5rem;}}\
h1{{font-size:14px;letter-spacing:0.08em;color:#8b949e;font-weight:600;margin:0 0 0.75rem;}}\
a{{color:#8be9fd;text-decoration:none;}}a:hover{{text-decoration:underline;}}\
table{{border-collapse:collapse;width:100%;font-size:11px;}}\
th,td{{border:1px solid #30363d;padding:0.35rem 0.5rem;text-align:left;vertical-align:top;}}\
th{{background:#0b0d0f;color:#8b949e;}}tr:hover td{{background:#0e1013;}}\
pre{{background:#0b0d0f;border:1px solid #30363d;border-radius:6px;padding:0.75rem;overflow:auto;font-size:12px;white-space:pre-wrap;word-break:break-word;max-height:60vh;}}\
input,select{{background:#0b0d0f;color:#c9d1d9;border:1px solid #30363d;border-radius:6px;padding:0.35rem 0.5rem;font:inherit;}}\
button{{background:#1f6feb;color:#fff;border:0;border-radius:6px;padding:0.35rem 0.7rem;font:inherit;cursor:pointer;}}\
button.ghost{{background:transparent;color:#8b949e;border:1px solid #30363d;}}\
button.danger{{background:#da3633;}}\
.muted{{color:#8b949e;font-size:11px;}}.err{{color:#ff6b6b;}}\
.bar{{display:flex;flex-wrap:wrap;align-items:center;gap:0.5rem;margin:0 0 0.75rem;}}\
.grow{{flex:1 1 auto;}}\
.pending,.scheduled{{color:#f0c674;}}.running{{color:#8be9fd;}}.failed{{color:#ff6b6b;}}\
.dead{{color:#ff6b6b;}}.done{{color:#b8e986;}}\
.tag{{border:1px solid #30363d;border-radius:999px;padding:0.05rem 0.5rem;font-size:10px;}}\
</style></head><body>{back}<h1><a href=\"/__soli/jobs\">SOLI \u{b7} JOBS</a></h1>{body}</body></html>",
        back = super::dev_catalog::BACK_TO_APP,
        body = body,
    )
}

fn cell(value: &str) -> String {
    if value.trim().is_empty() {
        "<span class=\"muted\">\u{2014}</span>".to_string()
    } else {
        dev_bar::html_escape(value)
    }
}

fn state_class(state: &str) -> &'static str {
    match state {
        "pending" => "pending",
        "scheduled" => "scheduled",
        "running" => "running",
        "failed" => "failed",
        "dead" => "dead",
        "done" => "done",
        _ => "muted",
    }
}

fn paginate(total: usize, per: usize, page: usize) -> (usize, usize, usize) {
    let pages = total.div_ceil(per).max(1);
    let page = page.min(pages - 1);
    let start = page * per;
    (page, start, (start + per).min(total))
}

/// Decide whether this request is the jobs dashboard (any method).
pub(crate) fn is_jobs_dashboard_path(method: &str, path: &str) -> bool {
    if path == "/__soli/jobs" {
        return method == "GET" || method == "HEAD";
    }
    let Some(rest) = path.strip_prefix("/__soli/jobs/") else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    matches!(method, "GET" | "HEAD" | "POST")
}

#[derive(Debug, PartialEq, Eq)]
enum DashAuth {
    Allow,
    NeedAuth,
    Hidden,
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

fn configured_basic() -> Option<(String, String)> {
    let user = env_nonempty("SOLI_JOBS_USER")?;
    let password = env_nonempty("SOLI_JOBS_PASSWORD")?;
    Some((user, password))
}

fn configured_token() -> Option<String> {
    env_nonempty("SOLI_JOBS_TOKEN")
}

fn parse_basic(headers: &HeaderMap) -> Option<(String, String)> {
    let raw = headers.get(hyper::header::AUTHORIZATION)?.to_str().ok()?;
    let b64 = raw.strip_prefix("Basic ")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()?;
    let decoded = String::from_utf8(bytes).ok()?;
    let (user, password) = decoded.split_once(':')?;
    Some((user.to_string(), password.to_string()))
}

fn parse_bearer(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(hyper::header::AUTHORIZATION)?.to_str().ok()?;
    raw.strip_prefix("Bearer ").map(|s| s.trim().to_string())
}

fn authorize(headers: &HeaderMap, dev_mode: bool) -> DashAuth {
    authorize_with(headers, dev_mode, configured_basic(), configured_token())
}

fn authorize_with(
    headers: &HeaderMap,
    dev_mode: bool,
    basic: Option<(String, String)>,
    token: Option<String>,
) -> DashAuth {
    if dev_mode {
        return DashAuth::Allow;
    }
    if basic.is_none() && token.is_none() {
        return DashAuth::Hidden;
    }
    if let (Some((want_user, want_pass)), Some((got_user, got_pass))) =
        (basic.as_ref(), parse_basic(headers))
    {
        if do_secure_compare(want_user, &got_user) && do_secure_compare(want_pass, &got_pass) {
            return DashAuth::Allow;
        }
    }
    if let (Some(want), Some(got)) = (token.as_ref(), parse_bearer(headers)) {
        if do_secure_compare(want, &got) {
            return DashAuth::Allow;
        }
    }
    DashAuth::NeedAuth
}

fn unauthorized() -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("WWW-Authenticate", "Basic realm=\"Soli jobs\"")
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(full(Bytes::from("Unauthorized")))
        .unwrap()
}

fn hidden_not_found() -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(full(Bytes::from("Not Found")))
        .unwrap()
}

/// Serve `/__soli/jobs` when the path matches. `None` if this is not a
/// jobs-dashboard request. Production without credentials is a plain 404.
pub(crate) fn dispatch(
    method: &str,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    dev_mode: bool,
) -> Option<Response<ResponseBody>> {
    if !is_jobs_dashboard_path(method, path) {
        return None;
    }
    match authorize(headers, dev_mode) {
        DashAuth::Allow => Some(route(method, path, query)),
        DashAuth::NeedAuth => Some(unauthorized()),
        DashAuth::Hidden => Some(hidden_not_found()),
    }
}

fn route(method: &str, path: &str, query: Option<&str>) -> Response<ResponseBody> {
    if path == "/__soli/jobs" {
        return handle_index(query);
    }
    let rest = path.strip_prefix("/__soli/jobs/").unwrap_or("");
    if method == "POST" {
        return handle_action(rest);
    }
    handle_show(rest)
}

fn page_link(queue: &str, state: &str, per: usize, page: usize) -> String {
    format!(
        "/__soli/jobs?queue={}&state={}&per={}&page={}",
        urlencoding::encode(queue),
        urlencoding::encode(state),
        per,
        page
    )
}

/// `GET /__soli/jobs` — filterable, paginated list.
pub(crate) fn handle_index(query: Option<&str>) -> Response<ResponseBody> {
    let params = query.map(parse_query_string).unwrap_or_default();
    let queue = params.get("queue").cloned().unwrap_or_default();
    let state = params.get("state").cloned().unwrap_or_default();
    let per = params
        .get("per")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_PER_PAGE)
        .clamp(5, 200);
    let requested_page = params
        .get("page")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);

    let listed = match store::list(if queue.is_empty() {
        None
    } else {
        Some(queue.as_str())
    }) {
        Ok(rows) => rows,
        Err(e) => {
            let hint = if e.contains("401") {
                "<p class=\"muted\">SolidB rejected the query. The <code>_jobs</code> collection \
is privileged — set <code>SOLIDB_API_KEY</code> (or <code>api_key</code> on the default \
connection in <code>config/database.toml</code>) to an admin key, then restart.</p>"
            } else {
                ""
            };
            return html_ok(jobs_page(&format!(
                "<p class=\"err\">Could not read _jobs: {}</p>{hint}",
                dev_bar::html_escape(&e)
            )));
        }
    };

    let matches: Vec<_> = listed
        .into_iter()
        .filter(|row| {
            if state.is_empty() {
                return true;
            }
            row.get("state")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.eq_ignore_ascii_case(&state))
        })
        .collect();

    let total = matches.len();
    let (page, start, end) = paginate(total, per, requested_page);
    let pages = total.div_ceil(per).max(1);
    let slice = &matches[start..end];

    let mut body = String::from(
        "<p class=\"muted\">Queue rows on the default connection \u{b7} cancel pending work \
or retry failed/dead jobs \u{b7} also available as <code>soli jobs list</code>.</p>",
    );

    let queue_esc = dev_bar::html_escape(&queue);
    let state_esc = dev_bar::html_escape(&state);
    body.push_str(&format!(
        "<div class=\"bar\">\
<form method=\"get\" action=\"/__soli/jobs\" class=\"bar grow\" style=\"margin:0;\">\
<input type=\"search\" name=\"queue\" value=\"{queue_esc}\" placeholder=\"queue name\" style=\"width:10rem;\">\
<input type=\"search\" name=\"state\" value=\"{state_esc}\" placeholder=\"state (pending, failed, \u{2026})\" style=\"width:14rem;\">\
<input type=\"hidden\" name=\"per\" value=\"{per}\">\
<button type=\"submit\">Filter</button>\
</form>\
<span class=\"muted\">{total} job(s)</span>\
</div>"
    ));

    if slice.is_empty() {
        body.push_str("<p class=\"muted\">No jobs match.</p>");
        return html_ok(jobs_page(&body));
    }

    body.push_str(
        "<table><thead><tr>\
<th>id</th><th>state</th><th>queue</th><th>handler</th>\
<th>tries</th><th>run_at</th><th></th>\
</tr></thead><tbody>",
    );
    for row in slice {
        let id = row.get("_key").and_then(|v| v.as_str()).unwrap_or("");
        let st = row.get("state").and_then(|v| v.as_str()).unwrap_or("?");
        let q = row.get("queue").and_then(|v| v.as_str()).unwrap_or("");
        let handler = row.get("handler").and_then(|v| v.as_str()).unwrap_or("");
        let attempts = row.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
        let run_at = row.get("run_at").and_then(|v| v.as_str()).unwrap_or("");
        let id_esc = dev_bar::html_escape(id);
        let actions = action_forms(id, st);
        body.push_str(&format!(
            "<tr>\
<td><a href=\"/__soli/jobs/{id_esc}\">{id_esc}</a></td>\
<td><span class=\"{}\">{}</span></td>\
<td>{}</td><td>{}</td>\
<td>{}</td><td>{}</td>\
<td style=\"white-space:nowrap;\">{actions}</td>\
</tr>",
            state_class(st),
            cell(st),
            cell(q),
            cell(handler),
            attempts,
            cell(run_at),
        ));
    }
    body.push_str("</tbody></table>");

    let mut nav = String::new();
    if page > 0 {
        nav.push_str(&format!(
            "<a href=\"{}\">&larr; prev</a>",
            page_link(&queue, &state, per, page - 1)
        ));
    }
    if page + 1 < pages {
        if !nav.is_empty() {
            nav.push_str(" \u{b7} ");
        }
        nav.push_str(&format!(
            "<a href=\"{}\">next &rarr;</a>",
            page_link(&queue, &state, per, page + 1)
        ));
    }
    if !nav.is_empty() {
        body.push_str(&format!(
            "<p class=\"muted\" style=\"margin-top:0.75rem;\">page {}/{} \u{b7} {}</p>",
            page + 1,
            pages,
            nav
        ));
    }

    html_ok(jobs_page(&body))
}

/// `GET /__soli/jobs/:id` — one row, pretty-printed.
pub(crate) fn handle_show(id: &str) -> Response<ResponseBody> {
    if !valid_id(id) {
        return not_found("No such job.");
    }
    let doc = match store::get(id) {
        Ok(Some(doc)) => doc,
        Ok(None) => return not_found("No such job."),
        Err(e) => {
            return html_ok(jobs_page(&format!(
                "<p class=\"err\">{}</p>",
                dev_bar::html_escape(&e)
            )));
        }
    };
    let json = match doc.to_json() {
        Ok(v) => v,
        Err(e) => serde_json::json!({ "error": e }),
    };
    let pretty = serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string());
    let actions = action_forms(&doc.key, doc.state.as_str());
    let body = format!(
        "<p class=\"muted\">{} \u{b7} <span class=\"{}\">{}</span> \u{b7} {}</p>\
<p>{}</p>\
<pre>{}</pre>",
        cell(&doc.key),
        state_class(doc.state.as_str()),
        cell(doc.state.as_str()),
        cell(&doc.handler),
        actions,
        dev_bar::html_escape(&pretty),
    );
    html_ok(jobs_page(&body))
}

/// `POST /__soli/jobs/:id/cancel` or `/retry`.
pub(crate) fn handle_action(rest: &str) -> Response<ResponseBody> {
    let Some((id, action)) = rest.split_once('/') else {
        return not_found("Unknown job action.");
    };
    if !valid_id(id) {
        return not_found("No such job.");
    }
    let result = match action {
        "cancel" => store::cancel(id).map(|_| ()),
        "retry" => store::retry(id).map(|_| ()),
        _ => return not_found("Unknown job action."),
    };
    match result {
        Ok(()) => Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header("Location", format!("/__soli/jobs/{id}"))
            .body(full(Bytes::new()))
            .unwrap(),
        Err(e) => html_ok(jobs_page(&format!(
            "<p class=\"err\">{}</p><p><a href=\"/__soli/jobs/{}\">back</a></p>",
            dev_bar::html_escape(&e),
            dev_bar::html_escape(id)
        ))),
    }
}

fn action_forms(id: &str, state: &str) -> String {
    let id_esc = dev_bar::html_escape(id);
    let mut out = String::new();
    if matches!(state, "scheduled" | "pending" | "failed") {
        out.push_str(&format!(
            "<form method=\"post\" action=\"/__soli/jobs/{id_esc}/cancel\" style=\"display:inline;\">\
<button class=\"ghost\" type=\"submit\">cancel</button></form> "
        ));
    }
    if matches!(state, "failed" | "dead") {
        out.push_str(&format!(
            "<form method=\"post\" action=\"/__soli/jobs/{id_esc}/retry\" style=\"display:inline;\">\
<button type=\"submit\">retry</button></form>"
        ));
    }
    out
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn not_found(message: &str) -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(full(Bytes::from(jobs_page(&format!(
            "<p class=\"err\">{}</p><p><a href=\"/__soli/jobs\">back to jobs</a></p>",
            dev_bar::html_escape(message)
        )))))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_ids_are_uuid_safe() {
        assert!(valid_id("550e8400-e29b-41d4-a716-446655440000"));
        assert!(valid_id("job_1"));
        assert!(!valid_id(""));
        assert!(!valid_id("../etc/passwd"));
        assert!(!valid_id("a/b"));
    }

    #[test]
    fn action_forms_match_state_machine() {
        assert!(action_forms("x", "pending").contains("cancel"));
        assert!(!action_forms("x", "pending").contains("retry"));
        assert!(action_forms("x", "failed").contains("cancel"));
        assert!(action_forms("x", "failed").contains("retry"));
        assert!(action_forms("x", "dead").contains("retry"));
        assert!(!action_forms("x", "dead").contains("cancel"));
        assert!(action_forms("x", "running").is_empty());
        assert!(action_forms("x", "done").is_empty());
    }

    #[test]
    fn jobs_paths() {
        assert!(is_jobs_dashboard_path("GET", "/__soli/jobs"));
        assert!(is_jobs_dashboard_path("GET", "/__soli/jobs/abc"));
        assert!(is_jobs_dashboard_path("POST", "/__soli/jobs/abc/cancel"));
        assert!(!is_jobs_dashboard_path("GET", "/__soli/inbox"));
        assert!(!is_jobs_dashboard_path("POST", "/__soli/jobs"));
        assert!(!is_jobs_dashboard_path("GET", "/jobs"));
    }

    fn headers_with(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(hyper::header::AUTHORIZATION, value.parse().expect("header"));
        h
    }

    #[test]
    fn prod_hidden_without_credentials() {
        assert_eq!(
            authorize_with(&HeaderMap::new(), false, None, None),
            DashAuth::Hidden
        );
        assert_eq!(
            authorize_with(&HeaderMap::new(), true, None, None),
            DashAuth::Allow
        );
    }

    #[test]
    fn prod_basic_auth() {
        let creds = Some(("ops".into(), "s3cret".into()));
        let ok = base64::engine::general_purpose::STANDARD.encode("ops:s3cret");
        let bad = base64::engine::general_purpose::STANDARD.encode("ops:wrong");
        assert_eq!(
            authorize_with(
                &headers_with(&format!("Basic {ok}")),
                false,
                creds.clone(),
                None
            ),
            DashAuth::Allow
        );
        assert_eq!(
            authorize_with(
                &headers_with(&format!("Basic {bad}")),
                false,
                creds.clone(),
                None
            ),
            DashAuth::NeedAuth
        );
        assert_eq!(
            authorize_with(&HeaderMap::new(), false, creds, None),
            DashAuth::NeedAuth
        );
    }

    #[test]
    fn prod_bearer_token() {
        let token = Some("tok-xyz".into());
        assert_eq!(
            authorize_with(&headers_with("Bearer tok-xyz"), false, None, token.clone()),
            DashAuth::Allow
        );
        assert_eq!(
            authorize_with(&headers_with("Bearer nope"), false, None, token),
            DashAuth::NeedAuth
        );
    }
}
