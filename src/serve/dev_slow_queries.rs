//! The slow-query dashboard at `/__soli/slow_queries`: the queries
//! [`slow_queries`] grouped by shape, their timings, and the slowest runs of
//! each with the request or job that ran them.
//!
//! Behind [`admin_auth`] like the errors page: open in `--dev` to a local
//! request; otherwise served only when `SOLI_SLOW_QUERIES_USER` +
//! `SOLI_SLOW_QUERIES_PASSWORD`, `SOLI_SLOW_QUERIES_TOKEN` or the shared
//! `SOLI_ADMIN_*` are set, and 404 when none is. The delete form is a
//! same-origin POST, which the Origin/Referer gate checks.
//!
//! [`slow_queries`]: super::slow_queries

use hyper::{header::HeaderMap, Response, StatusCode};

use crate::interpreter::builtins::server::parse_query_string;

use super::dev_errors::{html_status, last_hours, relative, short_count, sparkline};
use super::operator_shell::{self, Section};
use super::slow_queries::{self, ORDERS};
use super::{admin_auth, dev_bar, full, html_ok, Bytes, ResponseBody};

const BASE: &str = "/__soli/slow_queries";

/// Groups listed. What is cut is what matters least in the chosen order.
const LIST_LIMIT: usize = 200;

fn intro() -> String {
    format!(
        "Database queries that took {} ms or more, grouped by shape. \
Literals are taken out of the shape; each group keeps its slowest runs.",
        slow_queries::threshold_ms()
    )
}

fn slow_page(body: &str) -> String {
    operator_shell::page(Section::SlowQueries, "Slow queries", &intro(), body)
}

fn esc(value: &str) -> String {
    dev_bar::html_escape(value)
}

fn str_field<'a>(row: &'a serde_json::Value, field: &str) -> &'a str {
    row.get(field).and_then(|v| v.as_str()).unwrap_or("")
}

fn num_field(row: &serde_json::Value, field: &str) -> f64 {
    row.get(field).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

/// `1234.5` → `1.2 s`, `87.3` → `87 ms`.
fn duration(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{:.1} s", ms / 1000.0)
    } else {
        format!("{ms:.0} ms")
    }
}

/// Decide whether this request is the slow-query dashboard (any method).
pub(crate) fn is_slow_queries_dashboard_path(method: &str, path: &str) -> bool {
    if path == BASE {
        return method == "GET" || method == "HEAD";
    }
    let Some(rest) = path.strip_prefix("/__soli/slow_queries/") else {
        return false;
    };
    if rest.is_empty() {
        return false;
    }
    matches!(method, "GET" | "HEAD" | "POST")
}

/// Serve `/__soli/slow_queries` when the path matches. `None` if this is not
/// a dashboard request. Production without credentials is a plain 404.
pub(crate) fn dispatch(
    method: &str,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    dev_mode: bool,
    peer_ip: std::net::IpAddr,
) -> Option<Response<ResponseBody>> {
    if !is_slow_queries_dashboard_path(method, path) {
        return None;
    }
    let decision = admin_auth::authorize(headers, dev_mode, peer_ip, "SLOW_QUERIES");
    if let Some(refused) = admin_auth::refusal(decision, "Soli slow queries") {
        return Some(refused);
    }
    if path == BASE {
        return Some(handle_index(query));
    }
    let rest = path.strip_prefix("/__soli/slow_queries/").unwrap_or("");
    if method == "POST" {
        return Some(handle_action(rest));
    }
    Some(handle_show(rest))
}

fn order_from_query(query: Option<&str>) -> &'static str {
    let wanted = query
        .map(parse_query_string)
        .and_then(|params| params.get("order").cloned())
        .unwrap_or_default();
    ORDERS
        .iter()
        .map(|(name, _)| *name)
        .find(|name| *name == wanted)
        .unwrap_or("impact")
}

/// `GET /__soli/slow_queries` — groups in the chosen order.
fn handle_index(query: Option<&str>) -> Response<ResponseBody> {
    let order = order_from_query(query);
    let now = chrono::Utc::now();
    let mut body = String::new();

    body.push_str("<nav class=\"tabs\">");
    for (name, _) in ORDERS {
        let on = if name == order { " on" } else { "" };
        body.push_str(&format!(
            "<a class=\"tab{on}\" href=\"{BASE}?order={name}\">{name}</a>"
        ));
    }
    body.push_str("</nav>");

    if !slow_queries::enabled() {
        body.push_str(
            "<p class=\"notice\">Recording is off (<code>SOLI_SLOW_QUERIES=off</code>, or \
<code>APP_ENV=test</code>). Existing groups are still listed.</p>",
        );
    }
    body.push_str(&recording_notices(&slow_queries::stats()));
    body.push_str(&super::notify::status_html());

    let rows = match slow_queries::list(order, LIST_LIMIT) {
        Ok(rows) => rows,
        Err(e) if super::internal_store::is_missing_collection(&e) => {
            body.push_str(&format!(
                "<div class=\"empty\"><b>No slow queries recorded yet.</b>\
<span>A query taking {} ms or more will show up here within a second.</span></div>",
                slow_queries::threshold_ms()
            ));
            return html_ok(slow_page(&body));
        }
        Err(e) => {
            body.push_str(&format!(
                "<p class=\"notice bad\">Could not read {}: {}</p>",
                slow_queries::SLOW_COLLECTION,
                esc(&e)
            ));
            return html_status(StatusCode::INTERNAL_SERVER_ERROR, slow_page(&body));
        }
    };

    if rows.is_empty() {
        body.push_str(
            "<div class=\"empty\"><b>No slow queries.</b>\
<span>Every query has been under the threshold.</span></div>",
        );
        return html_ok(slow_page(&body));
    }

    let hours = last_hours(now);
    let total_ms: f64 = rows.iter().map(|row| num_field(row, "total_ms")).sum();
    body.push_str(&format!(
        "<p class=\"summary\">{} shape{} \u{b7} {} spent in slow runs</p>",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" },
        duration(total_ms),
    ));

    body.push_str("<div class=\"list\">");
    for row in &rows {
        let key = str_field(row, "_key");
        if !slow_queries::valid_fingerprint(key) {
            continue;
        }
        let count = row.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
        let total = num_field(row, "total_ms");
        let mean = if count > 0 { total / count as f64 } else { 0.0 };
        let context = str_field(row, "last_context");
        let context = if context.is_empty() {
            String::new()
        } else {
            format!("<span class=\"req\">{}</span>", esc(context))
        };
        let last_seen = str_field(row, "last_seen");
        let first_seen = str_field(row, "first_seen");
        body.push_str(&format!(
            "<article class=\"row slow\">\
<div class=\"count\" title=\"{count} slow run(s)\">{count_short}</div>\
<div class=\"main\"><a class=\"msg query\" href=\"{BASE}/{key}\">{shape}</a>\
<div class=\"meta\"><span>avg <span class=\"ms\">{mean}</span></span>\
<span>max <span class=\"ms\">{max}</span></span>\
<span>total <span class=\"ms\">{total}</span></span>{context}</div></div>\
<div class=\"trend\" title=\"last 24 hours\">{spark}</div>\
<div class=\"when\"><span title=\"{last_iso}\">{last_rel}</span>\
<span class=\"first\" title=\"{first_iso}\">first {first_rel}</span></div>\
<div class=\"actions\">{delete}</div></article>",
            count_short = short_count(count),
            shape = esc(str_field(row, "query")),
            mean = duration(mean),
            max = duration(num_field(row, "max_ms")),
            total = duration(total),
            spark = sparkline(&slow_queries::parse_hourly(row.get("hourly")), &hours),
            last_iso = esc(last_seen),
            last_rel = esc(&relative(last_seen, now)),
            first_iso = esc(first_seen),
            first_rel = esc(&relative(first_seen, now)),
            delete = delete_form(key, "ghost"),
        ));
    }
    body.push_str("</div>");
    if rows.len() >= LIST_LIMIT {
        body.push_str(&format!(
            "<p class=\"summary\">Showing the first {LIST_LIMIT}.</p>"
        ));
    }
    html_ok(slow_page(&body))
}

/// `GET /__soli/slow_queries/:fingerprint` — one shape and its slowest runs.
fn handle_show(key: &str) -> Response<ResponseBody> {
    if !slow_queries::valid_fingerprint(key) {
        return not_found("No such query.");
    }
    let group = match slow_queries::get(key) {
        Ok(Some(group)) => group,
        Ok(None) => return not_found("No such query."),
        Err(e) => {
            return html_status(
                StatusCode::INTERNAL_SERVER_ERROR,
                slow_page(&format!("<p class=\"err\">{}</p>", esc(&e))),
            )
        }
    };
    let now = chrono::Utc::now();
    let when = |field: &str| {
        let iso = str_field(&group, field);
        format!(
            "{} <span class=\"muted mono\">{}</span>",
            esc(&relative(iso, now)),
            esc(iso)
        )
    };
    let count = group.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
    let total = num_field(&group, "total_ms");
    let mean = if count > 0 { total / count as f64 } else { 0.0 };
    let mut body = format!(
        "<div class=\"bar\"><span class=\"grow\"></span><span class=\"actions\">{delete}</span></div>\
<pre>{shape}</pre>\
<div class=\"panel\"><dl><dt>Slow runs</dt><dd class=\"num\">{count}</dd>\
<dt>Average</dt><dd>{mean}</dd><dt>Slowest</dt><dd>{max}</dd>\
<dt>Total</dt><dd>{total}</dd>\
<dt>Last from</dt><dd class=\"mono\">{context}</dd>\
<dt>Last seen</dt><dd>{last}</dd><dt>First seen</dt><dd>{first}</dd></dl></div>",
        delete = delete_form(key, "danger"),
        shape = esc(str_field(&group, "query")),
        mean = duration(mean),
        max = duration(num_field(&group, "max_ms")),
        total = duration(total),
        context = esc(str_field(&group, "last_context")),
        last = when("last_seen"),
        first = when("first_seen"),
    );

    let samples = group
        .get("samples")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    body.push_str(&format!(
        "<h3>Slowest {} run{}</h3>",
        samples.len(),
        if samples.len() == 1 { "" } else { "s" }
    ));
    for (i, sample) in samples.iter().enumerate() {
        body.push_str(&render_sample(sample, i == 0, now));
    }
    html_ok(operator_shell::page(
        Section::SlowQueries,
        "Slow query",
        &format!("<a href=\"{BASE}\">\u{2190} All slow queries</a>"),
        &body,
    ))
}

fn render_sample(
    sample: &serde_json::Value,
    open: bool,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let mut out = format!(
        "<details{open}><summary><span class=\"ms\">{ms}</span> \u{b7} \
<span class=\"mono\">{context}</span> \u{b7} <span title=\"{at}\">{at_rel}</span></summary>\
<div class=\"muted\">query</div><pre>{query}</pre>",
        open = if open { " open" } else { "" },
        ms = duration(num_field(sample, "ms")),
        context = esc(str_field(sample, "context")),
        at = esc(str_field(sample, "at")),
        at_rel = esc(&relative(str_field(sample, "at"), now)),
        query = esc(str_field(sample, "query")),
    );
    if let Some(binds) = sample
        .get("binds")
        .filter(|b| b.as_object().is_some_and(|o| !o.is_empty()))
    {
        out.push_str(&format!(
            "<div class=\"muted\">binds</div><pre>{}</pre>",
            esc(&serde_json::to_string_pretty(binds).unwrap_or_default())
        ));
    }
    out.push_str("</details>");
    out
}

fn delete_form(key: &str, class: &str) -> String {
    format!(
        "<form method=\"post\" action=\"{BASE}/{key}/delete\" style=\"display:inline;margin:0;\">\
<button class=\"{class}\" type=\"submit\" title=\"Forget this shape; it starts again from its next slow run\">delete</button></form>"
    )
}

/// `POST /__soli/slow_queries/:fingerprint/delete`.
fn handle_action(rest: &str) -> Response<ResponseBody> {
    let Some((key, "delete")) = rest.split_once('/') else {
        return not_found("Unknown action.");
    };
    if !slow_queries::valid_fingerprint(key) {
        return not_found("No such query.");
    }
    match slow_queries::delete(key) {
        Ok(true) => Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header("Location", BASE)
            .body(full(Bytes::new()))
            .unwrap(),
        Ok(false) => not_found("No such query."),
        Err(e) => html_status(
            StatusCode::INTERNAL_SERVER_ERROR,
            slow_page(&format!(
                "<p class=\"err\">{}</p><p><a href=\"{BASE}/{key}\">back</a></p>",
                esc(&e)
            )),
        ),
    }
}

/// What went wrong with recording itself, in the same words as the errors
/// page uses for its own writer.
fn recording_notices(stats: &slow_queries::StatsSnapshot) -> String {
    let mut out = String::new();
    if stats.dropped > 0 {
        out.push_str(&format!(
            "<p class=\"notice\">{} slow run(s) dropped since this process started: \
they arrived faster than they could be written.</p>",
            stats.dropped
        ));
    }
    if stats.failed > 0 {
        out.push_str(&format!(
            "<p class=\"notice bad\">{} slow run(s) could not be written since this process started \
(see the <code>[slow-queries]</code> lines on stderr).</p>",
            stats.failed
        ));
    }
    if stats.overflowed > 0 {
        out.push_str(&format!(
            "<p class=\"notice bad\">The shape limit is reached: at most {} shapes are kept per app, \
and {} slow run(s) of new shapes were counted but not stored. Delete shapes you no longer \
need to make room.</p>",
            slow_queries::MAX_GROUPS,
            stats.overflowed
        ));
    }
    if stats.restarts > 0 || stats.lost > 0 {
        out.push_str(&format!(
            "<p class=\"notice bad\">The slow-query writer stopped unexpectedly and was restarted {} time(s); \
{} slow run(s) were lost.</p>",
            stats.restarts, stats.lost
        ));
    }
    out
}

fn not_found(message: &str) -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("Content-Type", "text/html; charset=utf-8")
        .body(full(Bytes::from(slow_page(&format!(
            "<p class=\"err\">{}</p><p><a href=\"{BASE}\">back to slow queries</a></p>",
            esc(message)
        )))))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_queries_paths() {
        assert!(is_slow_queries_dashboard_path(
            "GET",
            "/__soli/slow_queries"
        ));
        assert!(is_slow_queries_dashboard_path(
            "GET",
            "/__soli/slow_queries/0123456789abcdef"
        ));
        assert!(is_slow_queries_dashboard_path(
            "POST",
            "/__soli/slow_queries/0123456789abcdef/delete"
        ));
        assert!(!is_slow_queries_dashboard_path(
            "POST",
            "/__soli/slow_queries"
        ));
        assert!(!is_slow_queries_dashboard_path(
            "GET",
            "/__soli/slow_queries/"
        ));
        assert!(!is_slow_queries_dashboard_path(
            "GET",
            "/__soli/slow_queriesx"
        ));
    }

    #[test]
    fn durations_read_at_a_glance() {
        assert_eq!(duration(87.3), "87 ms");
        assert_eq!(duration(1234.5), "1.2 s");
    }

    #[test]
    fn an_unknown_order_is_impact() {
        assert_eq!(order_from_query(Some("order=slowest")), "slowest");
        assert_eq!(order_from_query(Some("order=drop table")), "impact");
        assert_eq!(order_from_query(None), "impact");
    }

    #[test]
    fn a_sample_escapes_its_query_and_binds() {
        let sample = serde_json::json!({
            "ms": 812.0,
            "context": "GET /x",
            "at": "2026-09-25T10:00:00Z",
            "query": "SELECT '<b>'",
            "binds": {"name": "<i>"},
        });
        let html = render_sample(&sample, true, chrono::Utc::now());
        assert!(html.contains("SELECT &#39;&lt;b&gt;&#39;"), "{html}");
        assert!(html.contains("&lt;i&gt;"));
        assert!(html.contains("812 ms"));
    }
}
