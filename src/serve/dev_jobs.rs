//! The job dashboard at `/__soli/jobs`: inspect queues, cancel pending
//! work, and retry failed/dead rows.
//!
//! Behind [`admin_auth`]: open in `--dev` to a local request; otherwise served
//! only when `SOLI_JOBS_USER` + `SOLI_JOBS_PASSWORD`, `SOLI_JOBS_TOKEN` or the
//! shared `SOLI_ADMIN_*` are set, and 404 when none is.

use hyper::{header::HeaderMap, Response, StatusCode};

use crate::interpreter::builtins::server::parse_query_string;
use crate::jobs::store;

use super::operator_shell::{self, Section};
use super::{admin_auth, dev_bar, full, html_ok, Bytes, ResponseBody};

const DEFAULT_PER_PAGE: usize = 25;

const INTRO: &str = "Queue rows on the default connection. Cancel pending work, retry failed \
or dead jobs \u{b7} also <code>soli jobs list</code>.";

fn jobs_page(body: &str) -> String {
    operator_shell::page(Section::Jobs, "Jobs", INTRO, body)
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

/// Serve `/__soli/jobs` when the path matches. `None` if this is not a
/// jobs-dashboard request. Production without credentials is a plain 404.
pub(crate) fn dispatch(
    method: &str,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    dev_mode: bool,
    peer_ip: std::net::IpAddr,
) -> Option<Response<ResponseBody>> {
    if !is_jobs_dashboard_path(method, path) {
        return None;
    }
    let decision = admin_auth::authorize(headers, dev_mode, peer_ip, "JOBS");
    if let Some(refused) = admin_auth::refusal(decision, "Soli jobs") {
        return Some(refused);
    }
    Some(route(method, path, query))
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

    let mut body = String::new();

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
        body.push_str(
            "<div class=\"empty\"><b>No jobs match.</b>\
<span>Enqueued work shows up here; clear the filters to see every queue.</span></div>",
        );
        return html_ok(jobs_page(&body));
    }

    body.push_str(
        "<div class=\"table-wrap\"><table><thead><tr>\
<th>id</th><th>state</th><th>queue</th><th>handler</th>\
<th>tries</th><th>run at</th><th></th>\
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
<td class=\"mono\"><a href=\"/__soli/jobs/{id_esc}\">{id_esc}</a></td>\
<td><span class=\"tag {}\">{}</span></td>\
<td>{}</td><td class=\"mono\">{}</td>\
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
    body.push_str("</tbody></table></div>");

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
            "<p class=\"pager\">page {}/{} \u{b7} {}</p>",
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
        "<div class=\"bar\"><span class=\"mono\">{}</span><span class=\"tag {}\">{}</span>\
<span class=\"muted mono\">{}</span><span class=\"grow\"></span><span class=\"actions\">{}</span></div>\
<pre>{}</pre>",
        cell(&doc.key),
        state_class(doc.state.as_str()),
        cell(doc.state.as_str()),
        cell(&doc.handler),
        actions,
        dev_bar::html_escape(&pretty),
    );
    html_ok(operator_shell::page(
        Section::Jobs,
        "Job",
        "<a href=\"/__soli/jobs\">\u{2190} All jobs</a>",
        &body,
    ))
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
}
