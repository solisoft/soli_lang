//! The error dashboard at `/__soli/errors`: the failures [`error_tracker`]
//! grouped, with their latest samples, and the buttons that triage them.
//!
//! Behind [`admin_auth`], like the jobs page: open in `--dev` to a local
//! request; otherwise served only when `SOLI_ERRORS_USER` +
//! `SOLI_ERRORS_PASSWORD`, `SOLI_ERRORS_TOKEN` or the shared `SOLI_ADMIN_*` are
//! set, and 404 when none is. The action forms are same-origin POSTs, which the
//! Origin/Referer gate checks (see `csrf::is_operator_dashboard_path`).
//!
//! [`error_tracker`]: super::error_tracker

use hyper::{header::HeaderMap, Response, StatusCode};

use crate::interpreter::builtins::server::parse_query_string;

use super::error_tracker::{self, STATUSES};
use super::operator_shell::{self, Section};
use super::{admin_auth, dev_bar, full, html_ok, Bytes, ResponseBody};

const BASE: &str = "/__soli/errors";

/// Groups listed per status. The table is ordered by `last_seen`, so what is
/// cut is what has been quiet the longest.
const LIST_LIMIT: usize = 200;

const INTRO: &str = "Requests that failed with a 500, grouped by cause. \
Samples are redacted before they are stored.";

fn errors_page(body: &str) -> String {
    operator_shell::page(Section::Errors, "Errors", INTRO, body)
}

fn esc(value: &str) -> String {
    dev_bar::html_escape(value)
}

fn str_field<'a>(row: &'a serde_json::Value, field: &str) -> &'a str {
    row.get(field).and_then(|v| v.as_str()).unwrap_or("")
}

/// Decide whether this request is the errors dashboard (any method).
pub(crate) fn is_errors_dashboard_path(method: &str, path: &str) -> bool {
    if path == BASE {
        return method == "GET" || method == "HEAD";
    }
    let Some(rest) = path.strip_prefix("/__soli/errors/") else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    matches!(method, "GET" | "HEAD" | "POST")
}

/// Serve `/__soli/errors` when the path matches. `None` if this is not a
/// dashboard request. Production without credentials is a plain 404.
pub(crate) fn dispatch(
    method: &str,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    dev_mode: bool,
    peer_ip: std::net::IpAddr,
) -> Option<Response<ResponseBody>> {
    if !is_errors_dashboard_path(method, path) {
        return None;
    }
    let decision = admin_auth::authorize(headers, dev_mode, peer_ip, "ERRORS");
    if let Some(refused) = admin_auth::refusal(decision, "Soli errors") {
        return Some(refused);
    }
    if path == BASE {
        return Some(handle_index(query));
    }
    let rest = path.strip_prefix("/__soli/errors/").unwrap_or("");
    if method == "POST" {
        return Some(handle_action(rest, query));
    }
    Some(handle_show(rest))
}

fn status_from_query(query: Option<&str>) -> &'static str {
    let wanted = query
        .map(parse_query_string)
        .and_then(|params| params.get("status").cloned())
        .unwrap_or_default();
    STATUSES
        .iter()
        .copied()
        .find(|s| *s == wanted)
        .unwrap_or("open")
}

/// `GET /__soli/errors` — groups in one status, most recently seen first.
fn handle_index(query: Option<&str>) -> Response<ResponseBody> {
    let status = status_from_query(query);
    let now = chrono::Utc::now();
    let mut body = String::new();

    // Every tab carries its size, so a quiet `open` tab reads as good news
    // rather than as an empty page. Three small reads on an operator page.
    let mut listed: Vec<(&str, Result<Vec<serde_json::Value>, String>)> = STATUSES
        .iter()
        .map(|tab| (*tab, error_tracker::list(tab, LIST_LIMIT)))
        .collect();

    body.push_str("<nav class=\"tabs\">");
    for (tab, rows) in &listed {
        let on = if *tab == status { " on" } else { "" };
        let size = match rows {
            Ok(rows) if rows.len() >= LIST_LIMIT => format!("{LIST_LIMIT}+"),
            Ok(rows) => rows.len().to_string(),
            Err(_) => "\u{2013}".to_string(),
        };
        body.push_str(&format!(
            "<a class=\"tab{on}\" href=\"{BASE}?status={tab}\">{tab}<span class=\"n\">{size}</span></a>"
        ));
    }
    body.push_str("</nav>");

    if !error_tracker::enabled() {
        body.push_str(
            "<p class=\"notice\">Recording is off (<code>SOLI_ERRORS=off</code>, or \
<code>APP_ENV=test</code>). Existing groups are still listed.</p>",
        );
    }
    let dropped = error_tracker::dropped();
    if dropped > 0 {
        body.push_str(&format!(
            "<p class=\"notice\">{dropped} occurrence(s) dropped since this process started: \
errors arrived faster than they could be written.</p>"
        ));
    }

    let index = STATUSES.iter().position(|s| *s == status).unwrap_or(0);
    let rows = match std::mem::replace(&mut listed[index].1, Ok(Vec::new())) {
        Ok(rows) => rows,
        Err(e) => {
            // Nothing has failed yet, so the collection does not exist.
            if e.contains("404")
                || e.to_lowercase().contains("not found")
                || e.contains("no such table")
            {
                body.push_str(
                    "<div class=\"empty\"><b>No errors recorded yet.</b>\
<span>A request that fails with a 500 will show up here within a second.</span></div>",
                );
            } else {
                body.push_str(&format!(
                    "<p class=\"notice bad\">Could not read {}: {}</p>",
                    error_tracker::ERRORS_COLLECTION,
                    esc(&e)
                ));
            }
            return html_ok(errors_page(&body));
        }
    };

    if rows.is_empty() {
        let hint = match status {
            "open" => "Nothing is failing right now.",
            "resolved" => "Groups you resolve move here, and come back if they fail again.",
            _ => "Groups you ignore keep counting here, out of the open list.",
        };
        body.push_str(&format!(
            "<div class=\"empty\"><b>No {status} errors.</b><span>{hint}</span></div>"
        ));
        return html_ok(errors_page(&body));
    }

    let hours = last_hours(now);
    let events_today: u64 = rows
        .iter()
        .map(|row| {
            let hourly = error_tracker::parse_hourly(row.get("hourly"));
            hours.iter().filter_map(|h| hourly.get(h)).sum::<u64>()
        })
        .sum();
    body.push_str(&format!(
        "<p class=\"summary\">{} {status} group{} \u{b7} {events_today} occurrence{} in the last 24 h</p>",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" },
        if events_today == 1 { "" } else { "s" },
    ));

    body.push_str("<div class=\"list\">");
    for row in &rows {
        let key = str_field(row, "_key");
        if !error_tracker::valid_fingerprint(key) {
            continue;
        }
        let count = row.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        let regressed = if status == "open" && !str_field(row, "regressed_at").is_empty() {
            "<span class=\"tag regressed\">regressed</span>"
        } else {
            ""
        };
        let last_request = str_field(row, "last_request");
        let request = if last_request.is_empty() {
            String::new()
        } else {
            format!("<span class=\"req\">{}</span>", esc(last_request))
        };
        let last_seen = str_field(row, "last_seen");
        let first_seen = str_field(row, "first_seen");
        body.push_str(&format!(
            "<article class=\"row {status}\">\
<div class=\"count\" title=\"{count} occurrence(s) in total\">{count_short}</div>\
<div class=\"main\"><a class=\"msg\" href=\"{BASE}/{key}\">{message}</a>\
<div class=\"meta\">{regressed}<span class=\"loc\">{location}</span>{request}</div></div>\
<div class=\"trend\" title=\"last 24 hours\">{spark}</div>\
<div class=\"when\"><span title=\"{last_iso}\">{last_rel}</span>\
<span class=\"first\" title=\"{first_iso}\">first {first_rel}</span></div>\
<div class=\"actions\">{actions}</div></article>",
            count_short = short_count(count),
            message = esc(str_field(row, "message")),
            location = esc(str_field(row, "location")),
            spark = sparkline(&error_tracker::parse_hourly(row.get("hourly")), &hours),
            last_iso = esc(last_seen),
            last_rel = esc(&relative(last_seen, now)),
            first_iso = esc(first_seen),
            first_rel = esc(&relative(first_seen, now)),
            actions = action_forms(key, status, false),
        ));
    }
    body.push_str("</div>");
    if rows.len() >= LIST_LIMIT {
        body.push_str(&format!(
            "<p class=\"summary\">Showing the {LIST_LIMIT} most recently seen.</p>"
        ));
    }
    html_ok(errors_page(&body))
}

/// The keys of the last 24 hours, oldest first, in the tracker's `hour_of`
/// shape (`2026-09-24T18`).
fn last_hours(now: chrono::DateTime<chrono::Utc>) -> Vec<String> {
    (0..error_tracker::HOURS_KEPT as i64)
        .rev()
        .map(|back| {
            (now - chrono::Duration::hours(back))
                .format("%Y-%m-%dT%H")
                .to_string()
        })
        .collect()
}

/// One bar per hour, scaled to the busiest hour. An hour with nothing is a
/// dim tick, so the line reads as "quiet", not "missing".
fn sparkline(hourly: &std::collections::BTreeMap<String, u64>, hours: &[String]) -> String {
    const HEIGHT: u64 = 22;
    let counts: Vec<u64> = hours
        .iter()
        .map(|h| hourly.get(h).copied().unwrap_or(0))
        .collect();
    let max = counts.iter().copied().max().unwrap_or(0).max(1);
    let mut bars = String::new();
    for (i, n) in counts.iter().enumerate() {
        let x = i * 4;
        if *n == 0 {
            bars.push_str(&format!(
                "<rect x=\"{x}\" y=\"{}\" width=\"3\" height=\"1\" class=\"z\"/>",
                HEIGHT - 1
            ));
        } else {
            let h = (n * HEIGHT).div_ceil(max).max(2);
            bars.push_str(&format!(
                "<rect x=\"{x}\" y=\"{}\" width=\"3\" height=\"{h}\" rx=\"0.5\"><title>{n}</title></rect>",
                HEIGHT - h
            ));
        }
    }
    format!(
        "<svg viewBox=\"0 0 {} {HEIGHT}\" width=\"{}\" height=\"{HEIGHT}\" aria-hidden=\"true\">{bars}</svg>",
        hours.len() * 4 - 1,
        hours.len() * 4 - 1
    )
}

/// `1234` → `1.2k`, so the count column keeps its width.
fn short_count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=9_999 => format!("{:.1}k", n as f64 / 1_000.0),
        10_000..=999_999 => format!("{}k", n / 1_000),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0),
    }
}

/// `2026-09-24T18:34:17Z` → `3 min ago`. Anything unparseable is shown as is.
fn relative(iso: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let Ok(at) = chrono::DateTime::parse_from_rfc3339(iso) else {
        return iso.to_string();
    };
    let secs = (now - at.with_timezone(&chrono::Utc)).num_seconds().max(0);
    match secs {
        0..=59 => "just now".to_string(),
        60..=3_599 => format!("{} min ago", secs / 60),
        3_600..=86_399 => format!("{} h ago", secs / 3_600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

/// `GET /__soli/errors/:fingerprint` — one group and its latest samples.
fn handle_show(key: &str) -> Response<ResponseBody> {
    if !error_tracker::valid_fingerprint(key) {
        return not_found("No such error.");
    }
    let group = match error_tracker::get(key) {
        Ok(Some(group)) => group,
        Ok(None) => return not_found("No such error."),
        Err(e) => return html_ok(errors_page(&format!("<p class=\"err\">{}</p>", esc(&e)))),
    };
    let status = str_field(&group, "status");
    let now = chrono::Utc::now();
    let when = |field: &str| {
        let iso = str_field(&group, field);
        format!(
            "{} <span class=\"muted mono\">{}</span>",
            esc(&relative(iso, now)),
            esc(iso)
        )
    };
    let mut body = format!(
        "<div class=\"bar\"><span class=\"tag {status_class}\">{status}</span>\
<span class=\"grow\"></span><span class=\"actions\">{actions}</span></div>\
<div class=\"panel\"><dl><dt>Raised in</dt><dd class=\"mono\">{location}</dd>\
<dt>Occurrences</dt><dd class=\"num\">{count}</dd>\
<dt>Last seen</dt><dd>{last}</dd>\
<dt>First seen</dt><dd>{first}</dd>",
        status_class = match status {
            "open" | "resolved" | "ignored" => status,
            _ => "",
        },
        status = esc(status),
        actions = action_forms(key, status, true),
        location = esc(str_field(&group, "location")),
        count = group.get("count").and_then(|v| v.as_u64()).unwrap_or(0),
        first = when("first_seen"),
        last = when("last_seen"),
    );
    for (field, label) in [("resolved_at", "Resolved"), ("regressed_at", "Regressed")] {
        if !str_field(&group, field).is_empty() {
            body.push_str(&format!("<dt>{label}</dt><dd>{}</dd>", when(field)));
        }
    }
    body.push_str("</dl></div>");

    let samples = group
        .get("samples")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    body.push_str(&format!(
        "<h3>Latest {} occurrence{}</h3>",
        samples.len(),
        if samples.len() == 1 { "" } else { "s" }
    ));
    for (i, sample) in samples.iter().enumerate() {
        body.push_str(&render_sample(sample, i == 0));
    }
    html_ok(operator_shell::page(
        Section::Errors,
        str_field(&group, "message"),
        &format!("<a href=\"{BASE}\">\u{2190} All errors</a>"),
        &body,
    ))
}

fn render_sample(sample: &serde_json::Value, open: bool) -> String {
    let stack = sample
        .get("stack")
        .and_then(|v| v.as_array())
        .map(|frames| {
            frames
                .iter()
                .filter_map(|f| f.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let pretty = |field: &str| {
        sample
            .get(field)
            .filter(|v| !v.is_null())
            .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
    };
    let mut out = format!(
        "<details{open}><summary><span class=\"mono\">{method} {path}</span> \u{b7} \
<span title=\"{at}\">{at_rel}</span> <span class=\"muted mono\">{request_id}</span></summary>\
<pre>{error}</pre>",
        open = if open { " open" } else { "" },
        at = esc(str_field(sample, "at")),
        at_rel = esc(&relative(str_field(sample, "at"), chrono::Utc::now())),
        method = esc(str_field(sample, "method")),
        path = esc(str_field(sample, "path")),
        request_id = esc(str_field(sample, "request_id")),
        error = esc(str_field(sample, "error")),
    );
    if !stack.is_empty() {
        out.push_str(&format!(
            "<div class=\"muted\">stack</div><pre>{}</pre>",
            esc(&stack)
        ));
    }
    if let Some(request) = reproducible_request(sample) {
        out.push_str(&format!(
            "<div class=\"muted\">reproduce</div><pre>{}</pre>",
            esc(&curl_command(request))
        ));
    }
    // The handler's `req` local (under locals) is the fuller copy; the snapshot
    // is shown only when there is no such local.
    let has_req_local = sample.get("env").and_then(|env| env.get("req")).is_some();
    if let Some(request) = pretty("request").filter(|_| !has_req_local) {
        out.push_str(&format!(
            "<div class=\"muted\">request</div><pre>{}</pre>",
            esc(&request)
        ));
    }
    if let Some(env) = pretty("env") {
        out.push_str(&format!(
            "<div class=\"muted\">locals</div><pre>{}</pre>",
            esc(&env)
        ));
    }
    out.push_str("</details>");
    out
}

/// The fullest redacted view of the request a sample holds. The handler's
/// `req` local carries the query and headers; the snapshot taken where the
/// error is logged can have neither, so it is only the fallback.
fn reproducible_request(sample: &serde_json::Value) -> Option<&serde_json::Value> {
    sample
        .get("env")
        .and_then(|env| env.get("req"))
        .filter(|req| req.get("method").is_some() && req.get("path").is_some())
        .or_else(|| sample.get("request"))
}

/// A `curl` line that re-sends the sample against a local server. Redacted
/// headers are left out — a `[REDACTED]` value would only fail differently —
/// and so are the ones curl sets itself.
fn curl_command(request: &serde_json::Value) -> String {
    let method = str_field(request, "method");
    let path = str_field(request, "path");
    let mut url = format!("http://localhost:5011{path}");
    if let Some(query) = request.get("query").and_then(|v| v.as_object()) {
        let pairs: Vec<String> = query
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|v| (k, v)))
            .filter(|(_, v)| *v != "[REDACTED]")
            .map(|(k, v)| format!("{}={}", urlencoding::encode(k), urlencoding::encode(v)))
            .collect();
        if !pairs.is_empty() {
            url.push('?');
            url.push_str(&pairs.join("&"));
        }
    }
    let mut out = format!("curl -X {} {}", shell_quote(method), shell_quote(&url));
    if let Some(headers) = request.get("headers").and_then(|v| v.as_object()) {
        for (name, value) in headers {
            let Some(value) = value.as_str() else {
                continue;
            };
            if value == "[REDACTED]"
                || matches!(
                    name.as_str(),
                    "host" | "content-length" | "connection" | "accept-encoding"
                )
            {
                continue;
            }
            out.push_str(&format!(
                " \\\n  -H {}",
                shell_quote(&format!("{name}: {value}"))
            ));
        }
    }
    out
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The triage buttons for a group in `status`. `detail` keeps the page on the
/// group after the action; from the list it goes back to the list.
fn action_forms(key: &str, status: &str, detail: bool) -> String {
    // On the list every row has these, so none of them shouts; on a group's
    // own page, resolving is the thing you came to do.
    let resolve_class = if detail { "" } else { "ghost" };
    let actions: &[(&str, &str)] = match status {
        "open" => &[("resolve", resolve_class), ("ignore", "ghost")],
        "resolved" | "ignored" => &[("reopen", "ghost")],
        _ => &[],
    };
    let mut out = String::new();
    for (action, class) in actions {
        out.push_str(&form(key, action, class, detail));
    }
    if detail {
        out.push_str(&form(key, "delete", "danger", detail));
    }
    out
}

fn form(key: &str, action: &str, class: &str, detail: bool) -> String {
    let back = if detail { "detail" } else { "list" };
    format!(
        "<form method=\"post\" action=\"{BASE}/{key}/{action}?back={back}\" style=\"display:inline;margin:0;\">\
<button class=\"{class}\" type=\"submit\">{action}</button></form> "
    )
}

/// `POST /__soli/errors/:fingerprint/(resolve|ignore|reopen|delete)`.
fn handle_action(rest: &str, query: Option<&str>) -> Response<ResponseBody> {
    let Some((key, action)) = rest.split_once('/') else {
        return not_found("Unknown action.");
    };
    if !error_tracker::valid_fingerprint(key) {
        return not_found("No such error.");
    }
    let result = match action {
        "resolve" => error_tracker::set_status(key, "resolved"),
        "ignore" => error_tracker::set_status(key, "ignored"),
        "reopen" => error_tracker::set_status(key, "open"),
        "delete" => error_tracker::delete(key),
        _ => return not_found("Unknown action."),
    };
    match result {
        Ok(true) => {
            // Back to the list after a delete (the group is gone) or when the
            // button was pressed there, so the next group to triage is under
            // the cursor; otherwise back to the group.
            let from_list = query
                .map(parse_query_string)
                .and_then(|params| params.get("back").cloned())
                .is_some_and(|back| back == "list");
            let location = if action == "delete" || from_list {
                BASE.to_string()
            } else {
                format!("{BASE}/{key}")
            };
            Response::builder()
                .status(StatusCode::SEE_OTHER)
                .header("Location", location)
                .body(full(Bytes::new()))
                .unwrap()
        }
        Ok(false) => not_found("No such error."),
        Err(e) => html_ok(errors_page(&format!(
            "<p class=\"err\">{}</p><p><a href=\"{BASE}/{key}\">back</a></p>",
            esc(&e)
        ))),
    }
}

fn not_found(message: &str) -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(full(Bytes::from(errors_page(&format!(
            "<p class=\"err\">{}</p><p><a href=\"{BASE}\">back to errors</a></p>",
            esc(message)
        )))))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_paths() {
        assert!(is_errors_dashboard_path("GET", "/__soli/errors"));
        assert!(is_errors_dashboard_path(
            "GET",
            "/__soli/errors/0123456789abcdef"
        ));
        assert!(is_errors_dashboard_path(
            "POST",
            "/__soli/errors/0123456789abcdef/resolve"
        ));
        assert!(!is_errors_dashboard_path("POST", "/__soli/errors"));
        assert!(!is_errors_dashboard_path("GET", "/__soli/errorsx"));
        assert!(!is_errors_dashboard_path("GET", "/errors"));
    }

    #[test]
    fn actions_follow_the_status() {
        assert!(action_forms("k", "open", false).contains("/resolve"));
        assert!(action_forms("k", "open", false).contains("/ignore"));
        assert!(!action_forms("k", "open", false).contains("/delete"));
        assert!(action_forms("k", "open", true).contains("/delete"));
        assert!(action_forms("k", "resolved", false).contains("/reopen"));
        assert!(!action_forms("k", "resolved", false).contains("/resolve"));
        assert!(action_forms("k", "ignored", false).contains("/reopen"));
    }

    #[test]
    fn an_unknown_status_filter_falls_back_to_open() {
        assert_eq!(status_from_query(None), "open");
        assert_eq!(status_from_query(Some("status=resolved")), "resolved");
        assert_eq!(status_from_query(Some("status=<script>")), "open");
    }

    #[test]
    fn curl_leaves_out_redacted_values_and_quotes_the_rest() {
        let request = serde_json::json!({
            "method": "POST",
            "path": "/orders",
            "query": {"page": "2", "token": "[REDACTED]"},
            "headers": {"authorization": "[REDACTED]", "x-note": "it's", "host": "example.com"},
        });
        let curl = curl_command(&request);
        assert!(
            curl.starts_with("curl -X 'POST' 'http://localhost:5011/orders?page=2'"),
            "{curl}"
        );
        assert!(!curl.contains("REDACTED"));
        assert!(!curl.contains("-H 'host:"));
        assert!(curl.contains("-H 'x-note: it'\\''s'"), "{curl}");
    }

    #[test]
    fn reproduce_prefers_the_handlers_req() {
        let sample = serde_json::json!({
            "request": {"method": "GET", "path": "/a", "query": {}},
            "env": {"req": {"method": "GET", "path": "/a", "query": {"page": "2"}}},
        });
        let request = reproducible_request(&sample).unwrap();
        assert!(curl_command(request).contains("/a?page=2"));

        let bare = serde_json::json!({"request": {"method": "GET", "path": "/b"}});
        assert_eq!(reproducible_request(&bare).unwrap()["path"], "/b");
    }

    #[test]
    fn relative_times_read_like_a_person_would_say_them() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-24T18:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(relative("2026-09-24T17:59:30Z", now), "just now");
        assert_eq!(relative("2026-09-24T17:57:00Z", now), "3 min ago");
        assert_eq!(relative("2026-09-24T15:00:00Z", now), "3 h ago");
        assert_eq!(relative("2026-09-21T18:00:00Z", now), "3 d ago");
        assert_eq!(relative("garbage", now), "garbage");
    }

    #[test]
    fn the_trend_covers_the_last_day_ending_now() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-24T18:34:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let hours = last_hours(now);
        assert_eq!(hours.len(), error_tracker::HOURS_KEPT);
        assert_eq!(hours.last().unwrap(), "2026-09-24T18");
        assert_eq!(hours.first().unwrap(), "2026-09-23T19");

        let hourly = std::collections::BTreeMap::from([("2026-09-24T18".to_string(), 4)]);
        let svg = sparkline(&hourly, &hours);
        assert_eq!(svg.matches("class=\"z\"").count(), hours.len() - 1);
        assert!(svg.contains("<title>4</title>"));
    }

    #[test]
    fn counts_stay_short() {
        assert_eq!(short_count(999), "999");
        assert_eq!(short_count(1_234), "1.2k");
        assert_eq!(short_count(45_000), "45k");
        assert_eq!(short_count(2_500_000), "2.5M");
    }

    #[test]
    fn samples_escape_what_the_request_sent() {
        let sample = serde_json::json!({
            "at": "t", "method": "GET", "path": "/<script>", "request_id": "r",
            "error": "<img src=x>", "stack": ["a at b.sl:1"],
        });
        let html = render_sample(&sample, true);
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img"));
    }
}
