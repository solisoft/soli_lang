//! The "all queries" view of `/__soli/slow_queries`: every query shape
//! [`query_stats`] counted, ranked by the database time it took in the last
//! 24 hours, with the most times one request ran it.
//!
//! Served through [`dev_slow_queries::dispatch`], so it sits behind the same
//! gate and in the same page.
//!
//! [`query_stats`]: super::query_stats
//! [`dev_slow_queries::dispatch`]: super::dev_slow_queries::dispatch

use std::collections::{BTreeMap, HashSet};

use hyper::{Response, StatusCode};

use super::dev_errors::{html_status, last_hours, relative, short_count, sparkline};
use super::dev_slow_queries::{duration, view_tabs, View, BASE};
use super::operator_shell::{self, Section};
use super::query_stats::{self, Bucket};
use super::{dev_bar, html_ok, internal_store, slow_queries, ResponseBody};

/// Shapes listed; what is cut took the least time.
const LIST_LIMIT: usize = 200;

/// Runs of one shape within one request from which the row says "N+1".
pub(crate) const N_PLUS_ONE_AT: u64 = 10;

const INTRO: &str = "Every database query, the fast ones too, grouped by shape and ranked by the \
time it took over the last 24 hours. Totals are written once a minute; no bind value is kept.";

fn page(body: &str) -> String {
    operator_shell::page(Section::SlowQueries, "Queries", INTRO, body)
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

/// A shape's last 24 hours, summed from its hourly totals.
#[derive(Default, Debug, PartialEq)]
struct Day {
    count: u64,
    ms: f64,
    peak: u64,
}

fn day_of(hourly: &BTreeMap<String, Bucket>, hours: &[String]) -> Day {
    let mut day = Day::default();
    for bucket in hours.iter().filter_map(|h| hourly.get(h)) {
        day.count += bucket.count;
        day.ms += bucket.ms;
        day.peak = day.peak.max(bucket.peak);
    }
    day
}

/// The trend line shows time, not calls: time is what the list ranks by. An
/// hour with any call gets at least one unit, so it never reads as quiet.
fn time_per_hour(hourly: &BTreeMap<String, Bucket>) -> BTreeMap<String, u64> {
    hourly
        .iter()
        .map(|(hour, b)| {
            let ms = if b.count > 0 {
                b.ms.ceil().max(1.0)
            } else {
                0.0
            };
            (hour.clone(), ms as u64)
        })
        .collect()
}

/// `N+1 · 48×` from [`N_PLUS_ONE_AT`] runs in one request, `up to 3× per
/// request` below it, nothing for a shape run once.
fn repeat_label(peak: u64, context: &str) -> String {
    let context = if context.is_empty() {
        String::new()
    } else {
        format!(" <span class=\"req\">{}</span>", esc(context))
    };
    match peak {
        0 | 1 => String::new(),
        n if n >= N_PLUS_ONE_AT => format!(
            "<span class=\"tag regressed\" title=\"run {n} times within one request or job\">N+1 \u{b7} {n}\u{d7}</span>{context}"
        ),
        n => format!("<span>up to {n}\u{d7} per request</span>{context}"),
    }
}

/// Fingerprints that also have slow runs, so a row can link to them.
fn slow_keys() -> HashSet<String> {
    internal_store::list(
        slow_queries::SLOW_COLLECTION,
        None,
        "last_seen",
        true,
        slow_queries::MAX_GROUPS as usize,
        &[],
    )
    .unwrap_or_default()
    .iter()
    .filter_map(|row| row.get("_key").and_then(|k| k.as_str()).map(str::to_string))
    .collect()
}

/// `GET /__soli/slow_queries` — every shape, most time first.
pub(super) fn handle_index() -> Response<ResponseBody> {
    let now = chrono::Utc::now();
    let mut body = view_tabs(View::All);

    if !query_stats::enabled() {
        body.push_str(
            "<p class=\"notice\">Counting is off (<code>SOLI_QUERY_STATS=off</code>, or \
<code>APP_ENV=test</code>). Existing totals are still listed.</p>",
        );
    }
    body.push_str(&recording_notices(&query_stats::stats()));
    // The landing view of `/__soli/slow_queries`: where an operator turns on
    // push for new slow queries, as on the slow view.
    body.push_str(&super::notify::status_html());
    body.push_str(&super::operator_push::page_block(
        super::operator_push::Topic::SlowQueries,
    ));

    let rows = match query_stats::list_all() {
        Ok(rows) => rows,
        Err(e) if internal_store::is_missing_collection(&e) => {
            body.push_str(
                "<div class=\"empty\"><b>No queries counted yet.</b>\
<span>Totals are written once a minute, from the first request that queries the database.</span></div>",
            );
            return html_ok(page(&body));
        }
        Err(e) => {
            body.push_str(&format!(
                "<p class=\"notice bad\">Could not read {}: {}</p>",
                query_stats::STATS_COLLECTION,
                esc(&e)
            ));
            return html_status(StatusCode::INTERNAL_SERVER_ERROR, page(&body));
        }
    };

    let hours = last_hours(now);
    let mut ranked: Vec<(Day, &serde_json::Value)> = rows
        .iter()
        .filter(|row| query_stats::valid_fingerprint(str_field(row, "_key")))
        .map(|row| {
            let hourly = query_stats::parse_hourly(row.get("hourly"));
            (day_of(&hourly, &hours), row)
        })
        .collect();
    let stored = ranked.len();
    ranked.retain(|(day, _)| day.count > 0);
    ranked.sort_by(|a, b| b.0.ms.total_cmp(&a.0.ms));
    let quiet = stored - ranked.len();

    if ranked.is_empty() {
        body.push_str(
            "<div class=\"empty\"><b>No queries in the last 24 hours.</b>\
<span>Shapes nothing has run for a day are kept, and listed again once they run.</span></div>",
        );
        return html_ok(page(&body));
    }

    let day_ms: f64 = ranked.iter().map(|(day, _)| day.ms).sum();
    let day_count: u64 = ranked.iter().map(|(day, _)| day.count).sum();
    body.push_str(&format!(
        "<p class=\"summary\">{} shape{} \u{b7} {} quer{} \u{b7} {} of database time in the last 24 h{}</p>",
        ranked.len(),
        if ranked.len() == 1 { "" } else { "s" },
        short_count(day_count),
        if day_count == 1 { "y" } else { "ies" },
        duration(day_ms),
        if quiet > 0 {
            format!(" \u{b7} {quiet} quiet shape{} not shown", if quiet == 1 { "" } else { "s" })
        } else {
            String::new()
        },
    ));

    let slow = slow_keys();
    body.push_str("<div class=\"list\">");
    for (day, row) in ranked.iter().take(LIST_LIMIT) {
        let key = str_field(row, "_key");
        let share = if day_ms > 0.0 {
            day.ms / day_ms * 100.0
        } else {
            0.0
        };
        let last_seen = str_field(row, "last_seen");
        let slow_link = if slow.contains(key) {
            format!("<a href=\"{BASE}/{key}\" title=\"This shape also has runs over the slow threshold\">slow runs</a>")
        } else {
            String::new()
        };
        body.push_str(&format!(
            "<article class=\"row slow\">\
<div class=\"count\" title=\"{count} call(s) in the last 24 h\">{count_short}</div>\
<div class=\"main\"><a class=\"msg query\" href=\"{BASE}/q/{key}\">{shape}</a>\
<div class=\"meta\"><span><span class=\"ms\">{time}</span> \u{b7} {share:.0}%</span>\
<span>avg <span class=\"ms\">{mean}</span></span>{repeat}</div></div>\
<div class=\"trend\" title=\"time per hour, last 24 hours\">{spark}</div>\
<div class=\"when\"><span title=\"{last_iso}\">{last_rel}</span></div>\
<div class=\"actions\">{slow_link}</div></article>",
            count = day.count,
            count_short = short_count(day.count),
            shape = esc(str_field(row, "query")),
            time = duration(day.ms),
            mean = duration_fine(day.ms / day.count.max(1) as f64),
            repeat = repeat_label(day.peak, str_field(row, "peak_context")),
            spark = sparkline(
                &time_per_hour(&query_stats::parse_hourly(row.get("hourly"))),
                &hours
            ),
            last_iso = esc(last_seen),
            last_rel = esc(&relative(last_seen, now)),
        ));
    }
    body.push_str("</div>");
    if ranked.len() > LIST_LIMIT {
        body.push_str(&format!(
            "<p class=\"summary\">Showing the {LIST_LIMIT} that took the most time.</p>"
        ));
    }
    html_ok(page(&body))
}

/// Like [`duration`], with a decimal under 10 ms: most queries live there.
fn duration_fine(ms: f64) -> String {
    if ms < 10.0 {
        format!("{ms:.1} ms")
    } else {
        duration(ms)
    }
}

/// `GET /__soli/slow_queries/q/:fingerprint` — one shape, hour by hour.
pub(super) fn handle_show(key: &str) -> Response<ResponseBody> {
    if !query_stats::valid_fingerprint(key) {
        return not_found();
    }
    let group = match query_stats::get(key) {
        Ok(Some(group)) => group,
        Ok(None) => return not_found(),
        Err(e) => {
            return html_status(
                StatusCode::INTERNAL_SERVER_ERROR,
                page(&format!("<p class=\"err\">{}</p>", esc(&e))),
            )
        }
    };
    let now = chrono::Utc::now();
    let hours = last_hours(now);
    let hourly = query_stats::parse_hourly(group.get("hourly"));
    let day = day_of(&hourly, &hours);
    let count = group.get("count").and_then(|v| v.as_u64()).unwrap_or(0);
    let total = num_field(&group, "total_ms");
    let when = |field: &str| {
        let iso = str_field(&group, field);
        format!(
            "{} <span class=\"muted mono\">{}</span>",
            esc(&relative(iso, now)),
            esc(iso)
        )
    };
    let repeat = match repeat_label(day.peak, str_field(&group, "peak_context")) {
        label if label.is_empty() => "once".to_string(),
        label => label,
    };
    let slow_link = if slow_queries::get(key).ok().flatten().is_some() {
        format!("<dt>Slow runs</dt><dd><a href=\"{BASE}/{key}\">the slowest runs, with their binds</a></dd>")
    } else {
        String::new()
    };
    let mut body = format!(
        "<pre>{shape}</pre>\
<div class=\"panel\"><dl>\
<dt>Last 24 h</dt><dd>{day_count} call(s) \u{b7} {day_time} \u{b7} avg {day_mean}</dd>\
<dt>Most per request</dt><dd class=\"meta\">{repeat}</dd>\
<dt>All time</dt><dd>{count} call(s) \u{b7} {total_time} \u{b7} avg {mean} \u{b7} slowest {max}</dd>\
{slow_link}\
<dt>Last seen</dt><dd>{last}</dd><dt>First seen</dt><dd>{first}</dd></dl></div>",
        shape = esc(str_field(&group, "query")),
        day_count = day.count,
        day_time = duration(day.ms),
        day_mean = duration_fine(day.ms / day.count.max(1) as f64),
        total_time = duration(total),
        mean = duration_fine(total / count.max(1) as f64),
        max = duration_fine(num_field(&group, "max_ms")),
        last = when("last_seen"),
        first = when("first_seen"),
    );

    body.push_str(
        "<h3>Hour by hour</h3><div class=\"table-wrap\"><table><thead><tr>\
<th>Hour (UTC)</th><th>Calls</th><th>Time</th><th>Avg</th><th>Most per request</th>\
</tr></thead><tbody>",
    );
    for (hour, bucket) in hourly.iter().rev() {
        body.push_str(&format!(
            "<tr><td class=\"mono\">{hour}:00</td><td class=\"num\">{count}</td><td>{time}</td>\
<td>{mean}</td><td class=\"num\">{peak}</td></tr>",
            hour = esc(&hour.replace('T', " ")),
            count = bucket.count,
            time = duration(bucket.ms),
            mean = duration_fine(bucket.ms / bucket.count.max(1) as f64),
            peak = bucket.peak,
        ));
    }
    body.push_str("</tbody></table></div>");

    html_ok(operator_shell::page(
        Section::SlowQueries,
        "Query",
        &format!("<a href=\"{BASE}\">\u{2190} All queries</a>"),
        &body,
    ))
}

fn not_found() -> Response<ResponseBody> {
    let mut response = html_ok(page(&format!(
        "<p class=\"err\">No such query.</p><p><a href=\"{BASE}\">back to queries</a></p>"
    )));
    *response.status_mut() = StatusCode::NOT_FOUND;
    response
}

/// What went wrong with writing the totals, in the slow-query page's words.
fn recording_notices(stats: &query_stats::StatsSnapshot) -> String {
    let mut out = String::new();
    if stats.dropped > 0 {
        out.push_str(&format!(
            "<p class=\"notice\">{} minute(s) of totals dropped since this process started: \
the writer was still busy with the previous ones.</p>",
            stats.dropped
        ));
    }
    if stats.failed > 0 {
        out.push_str(&format!(
            "<p class=\"notice bad\">{} call(s) could not be written since this process started \
(see the <code>[query-stats]</code> lines on stderr).</p>",
            stats.failed
        ));
    }
    if stats.overflowed > 0 {
        out.push_str(&format!(
            "<p class=\"notice bad\">The shape limit is reached: at most {} shapes are kept per app, \
and {} call(s) of new shapes were counted but not stored. Shapes nothing has run for a week are \
removed to make room.</p>",
            query_stats::MAX_GROUPS,
            stats.overflowed
        ));
    }
    if stats.restarts > 0 || stats.lost > 0 {
        out.push_str(&format!(
            "<p class=\"notice bad\">The query-stats writer stopped unexpectedly and was restarted {} time(s); \
{} minute(s) of totals were lost.</p>",
            stats.restarts, stats.lost
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bucket(count: u64, ms: f64, peak: u64) -> Bucket {
        Bucket { count, ms, peak }
    }

    #[test]
    fn the_day_sums_only_the_last_24_hours() {
        let hours = vec!["2026-09-25T17".to_string(), "2026-09-25T18".to_string()];
        let hourly = BTreeMap::from([
            ("2026-09-24T09".to_string(), bucket(100, 900.0, 60)),
            ("2026-09-25T17".to_string(), bucket(3, 6.0, 2)),
            ("2026-09-25T18".to_string(), bucket(5, 4.0, 12)),
        ]);
        assert_eq!(
            day_of(&hourly, &hours),
            Day {
                count: 8,
                ms: 10.0,
                peak: 12
            }
        );
    }

    #[test]
    fn a_busy_hour_under_a_millisecond_still_shows() {
        let hourly = BTreeMap::from([
            ("a".to_string(), bucket(4, 0.2, 1)),
            ("b".to_string(), bucket(0, 0.0, 0)),
        ]);
        let per_hour = time_per_hour(&hourly);
        assert_eq!(per_hour["a"], 1);
        assert_eq!(per_hour["b"], 0);
    }

    #[test]
    fn repeats_read_as_n_plus_one_from_the_threshold() {
        assert_eq!(repeat_label(1, "GET /"), "");
        assert!(repeat_label(3, "").contains("up to 3\u{d7} per request"));
        let label = repeat_label(N_PLUS_ONE_AT, "GET /<x> \u{2192} posts#index");
        assert!(label.contains("N+1"), "{label}");
        assert!(
            label.contains("GET /&lt;x&gt;"),
            "the context is escaped: {label}"
        );
    }

    #[test]
    fn small_averages_keep_a_decimal() {
        assert_eq!(duration_fine(0.34), "0.3 ms");
        assert_eq!(duration_fine(42.0), "42 ms");
        assert_eq!(duration_fine(2500.0), "2.5 s");
    }

    #[test]
    fn a_malformed_key_is_a_404() {
        assert_eq!(handle_show("../x").status(), StatusCode::NOT_FOUND);
    }
}
