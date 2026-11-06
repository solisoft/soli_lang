//! The overview at `/__soli/`: health at a glance, what needs attention most
//! urgently across every operator page, and how the app is being used.
//!
//! Behind [`admin_auth`] like the pages it summarises, and never wider than
//! them: it opens for anyone who may open at least one of them (or who holds
//! `SOLI_OVERVIEW_*` / the shared `SOLI_ADMIN_*`), and shows each source only to
//! a caller who may open that source's own page. Usage and the engine events
//! are the overview's own, under `SOLI_OVERVIEW_*` / `SOLI_ADMIN_*`. `--dev`
//! opens it to a local request; production with nothing configured is a
//! plain 404.
//!
//! What it reads is what the pages already read — the open error groups, the
//! slow-query groups, the refused posts, the queue window of the jobs page —
//! each a bounded list, plus the in-memory usage recorder ([`usage`]) and
//! engine events ([`engine_events`]). It has no forms of its own: a row's
//! action is a link, or the POST form of an action its page already offers.
//!
//! [`usage`]: super::usage
//! [`engine_events`]: super::engine_events

use hyper::{header::HeaderMap, Response};

use crate::interpreter::builtins::server::parse_query_string;

use super::admin_auth::{self, DashAuth};
use super::attention::{
    self, fmt_rate, fmt_us, span, thousands, Action, ErrorSummary, Item, JobSummary, SlowSummary,
    Tier,
};
use super::engine_events::EngineEvent;
use super::operator_shell::{self, Extras, Section};
use super::tenant_writer::StatsSnapshot;
use super::usage::{self, Range, UsageView, Window};
use super::{dev_bar, html_ok, ResponseBody};

/// What each source reads: the same limits as the source's own page.
const ERROR_LIMIT: usize = 200;
const SLOW_LIMIT: usize = 200;
const SPAM_LIMIT: usize = 200;
const JOB_WINDOW: usize = 500;
/// Rows shown in "Needs attention"; the rest are counted.
const MAX_ITEMS: usize = 30;
/// Routes shown in the table.
const MAX_ROUTES_SHOWN: usize = 10;
const REFRESH_SECS: u32 = 30;

fn esc(value: &str) -> String {
    dev_bar::html_escape(value)
}

/// `GET`/`HEAD` on `/__soli/` or `/__soli`.
pub(crate) fn is_overview_path(method: &str, path: &str) -> bool {
    matches!(path, "/__soli" | "/__soli/") && matches!(method, "GET" | "HEAD")
}

/// Which sources the caller may see.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Allowed {
    /// Usage and engine events: `SOLI_OVERVIEW_*`, `SOLI_ADMIN_*`, or `--dev`.
    pub overview: bool,
    pub errors: bool,
    pub slow_queries: bool,
    pub spam: bool,
    pub jobs: bool,
}

impl Allowed {
    fn any(&self) -> bool {
        self.overview || self.errors || self.slow_queries || self.spam || self.jobs
    }
}

/// Each source's gate, by its `SOLI_<prefix>_*` variables. A source is shown
/// only when its own gate lets the caller through; the page opens when any
/// does. Refused everywhere: 401 when some gate asked for credentials, the
/// hidden 404 when none is configured at all.
pub(crate) fn gate(decide: impl Fn(&str) -> DashAuth) -> Result<Allowed, DashAuth> {
    let mut need_auth = false;
    let mut check = |prefix: &str| match decide(prefix) {
        DashAuth::Allow => true,
        DashAuth::NeedAuth => {
            need_auth = true;
            false
        }
        DashAuth::Hidden => false,
    };
    let allowed = Allowed {
        overview: check("OVERVIEW"),
        errors: check("ERRORS"),
        slow_queries: check("SLOW_QUERIES"),
        spam: check("SPAM"),
        jobs: check("JOBS"),
    };
    if allowed.any() {
        Ok(allowed)
    } else if need_auth {
        Err(DashAuth::NeedAuth)
    } else {
        Err(DashAuth::Hidden)
    }
}

/// Serve the overview when the path matches; `None` otherwise.
pub(crate) fn dispatch(
    method: &str,
    path: &str,
    query: Option<&str>,
    headers: &HeaderMap,
    dev_mode: bool,
    peer_ip: std::net::IpAddr,
) -> Option<Response<ResponseBody>> {
    if !is_overview_path(method, path) {
        return None;
    }
    Some(serve(
        query,
        |prefix| admin_auth::authorize(headers, dev_mode, peer_ip, prefix),
        &fetch_live,
        chrono::Utc::now().timestamp(),
        dev_mode,
    ))
}

type Fetch<'a> = &'a dyn Fn(&Allowed, Range) -> Sources;

fn serve(
    query: Option<&str>,
    decide: impl Fn(&str) -> DashAuth,
    fetch: Fetch<'_>,
    now: i64,
    dev_mode: bool,
) -> Response<ResponseBody> {
    let allowed = match gate(decide) {
        Ok(allowed) => allowed,
        Err(refused) => {
            return admin_auth::refusal(refused, "Soli")
                .unwrap_or_else(admin_auth::hidden_not_found)
        }
    };
    let params = Params::parse(query);
    let sources = fetch(&allowed, params.range);
    html_ok(render_page(&params, &allowed, &sources, now, dev_mode))
}

/// `?range=1h|24h|7d` and `?refresh=30`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Params {
    pub range: Range,
    pub refresh: bool,
}

impl Params {
    fn parse(query: Option<&str>) -> Params {
        let params = query.map(parse_query_string).unwrap_or_default();
        Params {
            range: params
                .get("range")
                .and_then(|r| Range::parse(r))
                .unwrap_or(Range::Day),
            refresh: params.get("refresh").is_some_and(|r| r == "30"),
        }
    }

    fn href(range: Range, refresh: bool) -> String {
        let mut href = format!("/__soli/?range={}", range.param());
        if refresh {
            href.push_str(&format!("&refresh={REFRESH_SECS}"));
        }
        href
    }
}

/// What the page is built from; a source the caller may not see is `None`.
#[derive(Default)]
pub(crate) struct Sources {
    pub errors: Option<Result<Vec<serde_json::Value>, String>>,
    pub slow: Option<Result<Vec<serde_json::Value>, String>>,
    pub slow_threshold_ms: u64,
    pub spam: Option<Result<Vec<serde_json::Value>, String>>,
    pub jobs: Option<Result<Vec<serde_json::Value>, String>>,
    /// The chosen range.
    pub usage: Option<UsageView>,
    /// The last hour, for the traffic rows.
    pub usage_hour: Option<UsageView>,
    pub engine: Vec<EngineEvent>,
    /// (what, its page, the tracker's counters) for the trackers the caller
    /// may see.
    pub recording: Vec<(&'static str, &'static str, StatsSnapshot)>,
}

/// A collection that was never created is an empty one.
fn or_empty(
    result: Result<Vec<serde_json::Value>, String>,
) -> Result<Vec<serde_json::Value>, String> {
    match result {
        Err(e) if super::internal_store::is_missing_collection(&e) => Ok(Vec::new()),
        other => other,
    }
}

fn fetch_live(allowed: &Allowed, range: Range) -> Sources {
    let mut sources = Sources {
        slow_threshold_ms: super::slow_queries::threshold_ms(),
        ..Sources::default()
    };
    if allowed.errors {
        sources.errors = Some(or_empty(super::error_tracker::list("open", ERROR_LIMIT)));
        sources
            .recording
            .push(("error", "/__soli/errors", super::error_tracker::stats()));
        sources
            .recording
            .push(("notification", "/__soli/errors", super::notify::stats()));
    }
    if allowed.slow_queries {
        sources.slow = Some(or_empty(super::slow_queries::list("recent", SLOW_LIMIT)));
        sources.recording.push((
            "slow query",
            "/__soli/slow_queries",
            super::slow_queries::stats(),
        ));
    }
    if allowed.spam {
        sources.spam = Some(or_empty(super::spam::list("spam", SPAM_LIMIT)));
        sources
            .recording
            .push(("spam post", "/__soli/spam", super::spam::stats()));
    }
    if allowed.jobs {
        sources.jobs = Some(or_empty(crate::jobs::store::list(None)));
    }
    if allowed.overview {
        if usage::enabled() {
            let view = usage::snapshot(range);
            sources.usage_hour = Some(if range == Range::Hour {
                view.clone()
            } else {
                usage::snapshot(Range::Hour)
            });
            sources.usage = Some(view);
            sources.recording.push(("usage hour", "", usage::stats()));
        }
        sources.engine = super::engine_events::snapshot();
    }
    sources
}

// --- what the page shows ------------------------------------------------------------

/// The rows and the per-source summaries the tiles read.
struct Collected {
    items: Vec<Item>,
    errors: Option<Result<ErrorSummary, String>>,
    slow: Option<Result<SlowSummary, String>>,
    jobs: Option<Result<JobSummary, String>>,
}

/// A source that could not be read: its row, and the error for its tile.
fn unreadable(items: &mut Vec<Item>, what: &str, page: &str, error: &str, now: i64) -> String {
    items.push(attention::unreadable_item(what, page, error, now));
    error.to_string()
}

fn collect(allowed: &Allowed, sources: &Sources, now: i64) -> Collected {
    let mut items = Vec::new();
    let errors = sources.errors.as_ref().map(|rows| match rows {
        Ok(rows) => {
            let (found, summary) = attention::error_items(rows, ERROR_LIMIT, now);
            items.extend(found);
            Ok(summary)
        }
        Err(e) => Err(unreadable(
            &mut items,
            "error store",
            "/__soli/errors",
            e,
            now,
        )),
    });
    let slow = sources.slow.as_ref().map(|rows| match rows {
        Ok(rows) => {
            let (found, summary) =
                attention::slow_query_items(rows, sources.slow_threshold_ms, now);
            items.extend(found);
            Ok(summary)
        }
        Err(e) => Err(unreadable(
            &mut items,
            "slow-query store",
            "/__soli/slow_queries",
            e,
            now,
        )),
    });
    let jobs = sources.jobs.as_ref().map(|rows| match rows {
        Ok(rows) => {
            let (found, summary) = attention::job_items(rows, JOB_WINDOW, now);
            items.extend(found);
            Ok(summary)
        }
        Err(e) => Err(unreadable(&mut items, "job queue", "/__soli/jobs", e, now)),
    });
    if let Some(rows) = &sources.spam {
        match rows {
            Ok(rows) => items.extend(attention::spam_items(rows, now).0),
            Err(e) => {
                let _ = unreadable(&mut items, "spam quarantine", "/__soli/spam", e, now);
            }
        }
    }
    if allowed.overview {
        items.extend(attention::engine_items(&sources.engine, now));
        if let Some(hour) = &sources.usage_hour {
            items.extend(attention::traffic_items(hour, allowed.errors, now));
        }
    }
    for (what, page, stats) in &sources.recording {
        items.extend(attention::recording_item(what, page, stats, now));
    }
    attention::rank(&mut items);
    Collected {
        items,
        errors,
        slow,
        jobs,
    }
}

fn intro(allowed: &Allowed, range: Range) -> String {
    if allowed.overview {
        format!(
            "What needs your attention, most urgent first. Usage over {}.",
            range.phrase()
        )
    } else {
        "What needs your attention, most urgent first.".to_string()
    }
}

/// The whole page.
pub(crate) fn render_page(
    params: &Params,
    allowed: &Allowed,
    sources: &Sources,
    now: i64,
    dev_mode: bool,
) -> String {
    let collected = collect(allowed, sources, now);
    let mut body = String::from("<div class=\"ov\">");
    body.push_str(&tiles(allowed, sources, &collected, params));
    body.push_str(&attention_list(&collected.items));
    if allowed.overview {
        body.push_str(&usage_section(params, sources.usage.as_ref()));
    }
    body.push_str("</div>");
    body.push_str(AGE_SCRIPT);

    let head = if params.refresh {
        format!("<meta http-equiv=\"refresh\" content=\"{REFRESH_SECS}\">")
    } else {
        String::new()
    };
    let refresh_toggle = if params.refresh {
        format!(
            "<a class=\"btn ghost\" href=\"{}\" aria-pressed=\"true\" title=\"Stop reloading the page\">Auto-refresh: {REFRESH_SECS} s</a>",
            esc(&Params::href(params.range, false))
        )
    } else {
        format!(
            "<a class=\"btn ghost\" href=\"{}\" aria-pressed=\"false\" title=\"Reload the page every {REFRESH_SECS} seconds\">Auto-refresh: off</a>",
            esc(&Params::href(params.range, true))
        )
    };
    let aside = format!(
        "<span>Updated <span id=\"ov-age\">0 s</span> ago</span>\
<a class=\"btn ghost\" href=\"{}\">Refresh</a>{refresh_toggle}",
        esc(&Params::href(params.range, params.refresh)),
    );
    operator_shell::render(
        Section::Overview,
        "Overview",
        &esc(&intro(allowed, params.range)),
        &body,
        &Extras {
            head: &head,
            aside: &aside,
        },
        dev_mode,
    )
}

/// Ticks "Updated N s ago" from the moment the page loaded. Optional: without
/// it the page reads "0 s ago", which it was when it was sent.
const AGE_SCRIPT: &str = "<script>(function(){var e=document.getElementById('ov-age');\
if(!e)return;var t=Date.now();setInterval(function(){var s=Math.round((Date.now()-t)/1000);\
e.textContent=s<60?s+' s':Math.floor(s/60)+' min'},5000)})()</script>";

// --- the tiles ------------------------------------------------------------------------

struct Tile {
    label: String,
    value: String,
    note: String,
    /// `red`, `amber`, `green` or "" (grey).
    dot: &'static str,
    href: &'static str,
}

fn tiles(allowed: &Allowed, sources: &Sources, collected: &Collected, params: &Params) -> String {
    let mut tiles: Vec<Tile> = Vec::new();
    if let Some(view) = sources.usage.as_ref().filter(|_| allowed.overview) {
        tiles.extend(usage_tiles(view, params.range));
    }
    if let Some(errors) = &collected.errors {
        tiles.push(match errors {
            Ok(s) => Tile {
                label: "Open errors".into(),
                value: format!(
                    "{}{}",
                    thousands(s.open as u64),
                    if s.capped { "+" } else { "" }
                ),
                note: format!("{} new in the last hour", s.new_last_hour),
                dot: if s.new_last_hour > 0 {
                    "red"
                } else if s.open > 0 {
                    "amber"
                } else {
                    "green"
                },
                href: "/__soli/errors",
            },
            Err(_) => unread_tile("Open errors", "/__soli/errors"),
        });
    }
    if let Some(jobs) = &collected.jobs {
        tiles.push(match jobs {
            Ok(s) => Tile {
                label: "Jobs".into(),
                value: format!("{} dead", thousands(s.dead_24h)),
                note: if s.queued == 0 {
                    "nothing waiting".to_string()
                } else {
                    format!(
                        "{} queued \u{b7} oldest {}",
                        thousands(s.queued),
                        span(s.oldest_wait.unwrap_or(0))
                    )
                },
                dot: if s.dead_24h > 0 || s.oldest_wait.unwrap_or(0) >= 1800 {
                    "red"
                } else if s.oldest_wait.unwrap_or(0) >= 300 || s.queued >= 100 {
                    "amber"
                } else {
                    "green"
                },
                href: "/__soli/jobs",
            },
            Err(_) => unread_tile("Jobs", "/__soli/jobs"),
        });
    }
    if let Some(slow) = &collected.slow {
        tiles.push(match slow {
            Ok(s) => Tile {
                label: "Slow queries".into(),
                value: thousands(s.seen_24h as u64),
                note: format!(
                    "over {} ms in 24 h \u{b7} {} new",
                    sources.slow_threshold_ms, s.new_24h
                ),
                dot: if s.new_24h > 0 {
                    "amber"
                } else if s.seen_24h > 0 {
                    ""
                } else {
                    "green"
                },
                href: "/__soli/slow_queries",
            },
            Err(_) => unread_tile("Slow queries", "/__soli/slow_queries"),
        });
    }
    if tiles.is_empty() {
        return String::new();
    }
    let mut out = String::from("<section class=\"tiles\" aria-label=\"Health at a glance\">");
    for tile in tiles {
        out.push_str(&format!(
            "<a class=\"tile\" href=\"{href}\"><span class=\"tile-label\">{label}</span>\
<span class=\"tile-value\">{value}</span>\
<span class=\"tile-note\"><span class=\"dot {dot}\" aria-hidden=\"true\"></span>{note}</span></a>",
            href = esc(tile.href),
            label = esc(&tile.label),
            value = esc(&tile.value),
            dot = tile.dot,
            note = esc(&tile.note),
        ));
    }
    out.push_str("</section>");
    out
}

fn unread_tile(label: &str, href: &'static str) -> Tile {
    Tile {
        label: label.into(),
        value: "\u{2013}".into(),
        note: "could not be read".into(),
        dot: "red",
        href,
    }
}

fn usage_tiles(view: &UsageView, range: Range) -> Vec<Tile> {
    let now = &view.current;
    let before = view.previous.as_ref();
    let change = match before {
        Some(prev) if prev.requests() > 0 => {
            let pct =
                (now.requests() as f64 - prev.requests() as f64) / prev.requests() as f64 * 100.0;
            let sign = if pct >= 0.0 { "+" } else { "\u{2212}" };
            format!("{sign}{}% vs {}", pct.abs().round() as u64, range.before())
        }
        _ => format!("no traffic in {} to compare", range.before()),
    };
    let rate = now.error_rate();
    let rate_note = match before {
        _ if now.requests() == 0 => "5xx \u{b7} no requests yet".to_string(),
        Some(prev) => {
            let was = prev.error_rate();
            if (rate - was).abs() < 0.0005 {
                "5xx \u{b7} steady".to_string()
            } else if rate > was {
                format!("5xx \u{b7} up from {}", fmt_rate(was))
            } else {
                format!("5xx \u{b7} down from {}", fmt_rate(was))
            }
        }
        None => "5xx".to_string(),
    };
    let p95 = now.p95_us();
    let p50 = now.p50_us();
    let latency_note = {
        let p50 = p50.map(fmt_us).unwrap_or_else(|| "\u{2013}".into());
        match (p95, before.and_then(|w| w.p95_us())) {
            (Some(cur), Some(was)) if cur as f64 > was as f64 * 1.25 => {
                format!("p50 {p50} \u{b7} up from {}", fmt_us(was))
            }
            (Some(cur), Some(was)) if (cur as f64) < was as f64 * 0.8 => {
                format!("p50 {p50} \u{b7} down from {}", fmt_us(was))
            }
            (Some(_), Some(_)) => format!("p50 {p50} \u{b7} steady"),
            _ => format!("p50 {p50}"),
        }
    };
    vec![
        Tile {
            label: format!("Requests \u{b7} {}", range.label()),
            value: thousands(now.requests()),
            note: change,
            dot: "",
            href: "#usage-h",
        },
        Tile {
            label: "Server errors".into(),
            value: fmt_rate(rate),
            note: rate_note,
            dot: if rate >= 0.01 {
                "red"
            } else if rate >= 0.001 {
                "amber"
            } else {
                "green"
            },
            href: "#usage-h",
        },
        Tile {
            label: "Latency p95".into(),
            value: p95.map(fmt_us).unwrap_or_else(|| "\u{2013}".into()),
            note: latency_note,
            dot: match p95 {
                Some(us) if us >= 3_000_000 => "red",
                Some(us) if us >= 1_000_000 => "amber",
                Some(_) => "green",
                None => "",
            },
            href: "#usage-h",
        },
    ]
}

// --- needs attention ------------------------------------------------------------------

fn attention_list(items: &[Item]) -> String {
    let count = |tier: Tier| items.iter().filter(|i| i.tier == tier).count();
    let mut pills = String::new();
    for (tier, one, many) in [
        (Tier::Critical, "critical", "critical"),
        (Tier::Warning, "warning", "warnings"),
        (Tier::Info, "info", "info"),
    ] {
        let n = count(tier);
        if n > 0 {
            pills.push_str(&format!(
                "<span class=\"pill {}\">{n} {}</span>",
                tier.class(),
                if n == 1 { one } else { many }
            ));
        }
    }
    let mut out = format!(
        "<section class=\"attn\" aria-labelledby=\"attn-h\"><div class=\"attn-head\">\
<h2 id=\"attn-h\">Needs attention</h2><div class=\"counts\">{pills}</div></div>"
    );
    if items.is_empty() {
        out.push_str(
            "<div class=\"attn-empty\"><b>Nothing needs attention.</b>\
<span>No open errors, failing jobs, slow queries or spikes on the pages you can open.</span></div>",
        );
    } else {
        out.push_str("<ol>");
        for item in items.iter().take(MAX_ITEMS) {
            out.push_str(&item_row(item));
        }
        out.push_str("</ol>");
    }
    let more = items.len().saturating_sub(MAX_ITEMS);
    let more = if more > 0 {
        format!(" {more} more not shown.")
    } else {
        String::new()
    };
    out.push_str(&format!(
        "<div class=\"attn-foot\">Ranked by tier, then by how fast it is growing, how recently it \
was seen and how often. Resolved and ignored items are left out.{more}</div></section>"
    ));
    out
}

fn item_row(item: &Item) -> String {
    let action = match &item.action {
        Action::None => String::new(),
        Action::Link { href, label } => format!(
            "<a class=\"btn ghost\" href=\"{}\">{}</a>",
            esc(href),
            esc(label)
        ),
        Action::Post { href, label } => format!(
            "<form method=\"post\" action=\"{}\"><button class=\"ghost\" type=\"submit\">{}</button></form>",
            esc(href),
            esc(label)
        ),
    };
    format!(
        "<li class=\"item\"><span class=\"badge {class}\">{tier}</span>\
<div class=\"item-main\"><div class=\"item-title\"><span class=\"chip\">{source}</span><b>{title}</b></div>\
<span class=\"item-detail\">{detail}</span><span class=\"item-why\">{why}</span></div>\
<div class=\"item-metric\"><b>{metric}</b><span>{note}</span></div>{action}</li>",
        class = item.tier.class(),
        tier = item.tier.label(),
        source = item.source.label(),
        title = esc(&item.title),
        detail = esc(&item.detail),
        why = esc(&item.why),
        metric = esc(&item.metric),
        note = esc(&item.metric_note),
    )
}

// --- usage ------------------------------------------------------------------------------

fn usage_section(params: &Params, view: Option<&UsageView>) -> String {
    let mut range_links = String::new();
    for range in Range::ALL {
        let current = if range == params.range {
            " aria-current=\"true\""
        } else {
            ""
        };
        range_links.push_str(&format!(
            "<a href=\"{}\"{current}>{}</a>",
            esc(&Params::href(range, params.refresh)),
            range.label()
        ));
    }
    let mut out = format!(
        "<section class=\"usage\" aria-labelledby=\"usage-h\"><div class=\"usage-head\">\
<h2 id=\"usage-h\">Usage</h2><nav class=\"range\" aria-label=\"Time range\">{range_links}</nav></div>"
    );
    let Some(view) = view else {
        out.push_str(
            "<p class=\"notice\">Usage is not recorded (<code>SOLI_USAGE=off</code>, or \
<code>APP_ENV=test</code>).</p></section>",
        );
        return out;
    };
    out.push_str(&format!(
        "<p class=\"muted\">Requests this process answered{}, all workers together; \
the framework's own pages and probes are left out. Times are UTC.</p>",
        if view.range == Range::Week {
            ", plus the hours stored before it started"
        } else {
            ""
        }
    ));
    if view.current.requests() == 0 {
        out.push_str(&format!(
            "<div class=\"ucard\"><p class=\"usage-empty\">No requests recorded over {} yet.</p></div></section>",
            view.range.phrase()
        ));
        return out;
    }
    out.push_str("<div class=\"usage-grid\"><div class=\"ucard\">");
    out.push_str(&chart(view));
    out.push_str(&status_mix(&view.current));
    out.push_str("</div>");
    out.push_str(&routes_table(view));
    out.push_str("</div></section>");
    out
}

fn bar_unit(range: Range) -> &'static str {
    match range {
        Range::Hour => "5 minutes",
        Range::Day => "hour",
        Range::Week => "6 hours",
    }
}

fn clock(unix: u64, range: Range) -> String {
    let format = match range {
        Range::Hour => "%H:%M",
        Range::Day => "%H:00",
        Range::Week => "%a %H:00",
    };
    chrono::DateTime::from_timestamp(unix as i64, 0)
        .map(|t| t.format(format).to_string())
        .unwrap_or_default()
}

/// The smallest of 1, 2, 2.5, 5 × 10^k at or above `n`.
fn nice_step(n: u64) -> u64 {
    let mut magnitude = 1u64;
    loop {
        for factor in [10u64, 20, 25, 50] {
            let step = magnitude.saturating_mul(factor) / 10;
            if step >= n.max(1) {
                return step;
            }
        }
        if magnitude > u64::MAX / 100 {
            return n.max(1);
        }
        magnitude *= 10;
    }
}

/// `12k`, `2.5k`, `1.2M`, `40`: an axis label.
fn compact(n: u64) -> String {
    let trim = |v: f64, suffix: &str| {
        let text = format!("{v:.1}");
        let text = text.strip_suffix(".0").unwrap_or(&text).to_string();
        format!("{text}{suffix}")
    };
    if n >= 1_000_000 {
        trim(n as f64 / 1_000_000.0, "M")
    } else if n >= 1_000 {
        trim(n as f64 / 1_000.0, "k")
    } else {
        n.to_string()
    }
}

fn chart(view: &UsageView) -> String {
    let unit = bar_unit(view.range);
    let total = view.current.requests();
    let peak = view.bars.iter().copied().max_by(|a, b| {
        a.requests
            .cmp(&b.requests)
            .then_with(|| b.start.cmp(&a.start))
    });
    let peak_requests = peak.map(|b| b.requests).unwrap_or(0);
    let step = nice_step(peak_requests.div_ceil(3));
    let top = step * 3;
    let peak_at = peak.map(|b| clock(b.start, view.range)).unwrap_or_default();
    let mut cols = String::new();
    let last = view.bars.len().saturating_sub(1);
    for (i, bar) in view.bars.iter().enumerate() {
        let pct = if top == 0 {
            0.0
        } else {
            bar.requests as f64 * 100.0 / top as f64
        };
        let tip = format!(
            "{} \u{b7} {} request{}",
            clock(bar.start, view.range),
            thousands(bar.requests),
            if bar.requests == 1 { "" } else { "s" }
        );
        cols.push_str(&format!(
            "<div class=\"c{now}\" title=\"{tip}\" style=\"height:{pct:.1}%\"></div>",
            now = if i == last { " now" } else { "" },
            tip = esc(&tip),
        ));
    }
    let mut xaxis = String::new();
    if !view.bars.is_empty() {
        let n = view.bars.len();
        for k in 0..4 {
            let i = k * (n - 1) / 4;
            xaxis.push_str(&format!(
                "<span>{}</span>",
                esc(&clock(view.bars[i].start, view.range))
            ));
        }
        xaxis.push_str("<span>now</span>");
    }
    let label = format!(
        "Requests per {unit} over {}, peaking at {} at {peak_at} UTC, {} in total",
        view.range.phrase(),
        thousands(peak_requests),
        thousands(total)
    );
    format!(
        "<div class=\"ucard-head\"><h3>Requests per {unit}</h3>\
<span>peak {peak} at {peak_at} \u{b7} {total} in total</span></div>\
<div class=\"chart\"><div class=\"yaxis\" aria-hidden=\"true\"><span>{y3}</span><span>{y2}</span><span>{y1}</span><span>0</span></div>\
<div class=\"cols\" role=\"img\" aria-label=\"{label}\">{cols}</div></div>\
<div class=\"xaxis\" aria-hidden=\"true\">{xaxis}</div>",
        peak = thousands(peak_requests),
        peak_at = esc(&peak_at),
        total = thousands(total),
        y3 = compact(step * 3),
        y2 = compact(step * 2),
        y1 = compact(step),
        label = esc(&label),
    )
}

fn pct(part: u64, whole: u64) -> String {
    if whole == 0 || part == 0 {
        return "0%".to_string();
    }
    let value = part as f64 * 100.0 / whole as f64;
    if value < 0.1 {
        "<0.1%".to_string()
    } else {
        format!("{value:.1}%")
    }
}

fn status_mix(window: &Window) -> String {
    let total = window.requests();
    let classes = [("2xx", "s2"), ("3xx", "s3"), ("4xx", "s4"), ("5xx", "s5")];
    let mut segments = String::new();
    let mut legend = String::new();
    let mut label = Vec::new();
    for (i, (name, class)) in classes.iter().enumerate() {
        let n = window.status[i];
        let share = pct(n, total);
        if n > 0 {
            segments.push_str(&format!(
                "<span class=\"{class}\" style=\"flex:{} 1 0\"></span>",
                n
            ));
        }
        legend.push_str(&format!(
            "<span><i class=\"{class}\" aria-hidden=\"true\"></i>{name} {}</span>",
            esc(&share)
        ));
        label.push(format!("{name} {share}"));
    }
    format!(
        "<div class=\"status-mix\"><h3>Responses by status</h3>\
<div class=\"seg\" role=\"img\" aria-label=\"{}\">{segments}</div>\
<div class=\"legend\">{legend}</div></div>",
        esc(&label.join(", "))
    )
}

fn routes_table(view: &UsageView) -> String {
    let top = view
        .routes
        .first()
        .map(|r| r.window.requests())
        .unwrap_or(0);
    let mut rows = String::new();
    for row in view.routes.iter().take(MAX_ROUTES_SHOWN) {
        let requests = row.window.requests();
        let share = if top == 0 {
            0.0
        } else {
            requests as f64 * 100.0 / top as f64
        };
        let p95 = row.window.p95_us();
        let p95_class = match p95 {
            Some(us) if us >= 1_000_000 => "slow",
            _ => "",
        };
        let rate = row.window.error_rate();
        let rate_class = if rate >= 0.01 { "bad" } else { "ok" };
        rows.push_str(&format!(
            "<tr><td><div class=\"route\"><code>{label}</code>\
<span class=\"share\" aria-hidden=\"true\"><span style=\"width:{share:.0}%\"></span></span></div></td>\
<td title=\"{exact}\">{short}</td><td class=\"{p95_class}\">{p95}</td><td class=\"{rate_class}\">{rate}</td></tr>",
            label = esc(&row.label),
            exact = thousands(requests),
            short = super::dev_errors::short_count(requests),
            p95 = p95.map(fmt_us).unwrap_or_else(|| "\u{2013}".into()),
            rate = fmt_rate(rate),
        ));
    }
    let more = view.routes.len().saturating_sub(MAX_ROUTES_SHOWN);
    let more = if more > 0 {
        format!(
            "<p class=\"muted\" style=\"padding:0 20px\">{more} more route{} not shown.</p>",
            if more == 1 { "" } else { "s" }
        )
    } else {
        String::new()
    };
    format!(
        "<div class=\"ucard routes\"><div class=\"ucard-head\"><h3>Top routes</h3><span>by requests</span></div>\
<div class=\"scroll\"><table class=\"rt\"><thead><tr><th scope=\"col\">Route</th><th scope=\"col\">Req</th>\
<th scope=\"col\">p95</th><th scope=\"col\">5xx</th></tr></thead><tbody>{rows}</tbody></table></div>{more}</div>"
    )
}

#[cfg(test)]
mod tests {
    use super::super::engine_events::EngineKind;
    use super::super::usage::{Bar, RouteRow};
    use super::*;
    use serde_json::json;

    /// 2025-11-06 14:30:00 UTC.
    const NOW: i64 = 1_762_439_400;

    fn iso(unix: i64) -> String {
        chrono::DateTime::from_timestamp(unix, 0)
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_default()
    }

    fn all() -> Allowed {
        Allowed {
            overview: true,
            errors: true,
            slow_queries: true,
            spam: true,
            jobs: true,
        }
    }

    fn params() -> Params {
        Params {
            range: Range::Day,
            refresh: false,
        }
    }

    fn usage_view() -> UsageView {
        let window = |ok, bad, us| Window::default().with(200, us, ok).with(500, us, bad);
        let bars = (0..24)
            .map(|i| Bar {
                start: (NOW as u64 / 3600 - 23 + i) * 3600,
                requests: 100 + i * 10,
            })
            .collect();
        UsageView {
            range: Range::Day,
            now: NOW as u64,
            bars,
            current: window(5_000, 20, 30_000),
            previous: Some(window(4_600, 2, 25_000)),
            routes: vec![
                RouteRow {
                    label: "GET /posts/:id".into(),
                    window: window(3_000, 0, 20_000),
                },
                RouteRow {
                    label: "GET /orders/:id".into(),
                    window: window(980, 20, 40_000),
                },
                RouteRow {
                    label: "GET /report".into(),
                    window: window(60, 0, 3_000_000),
                },
            ],
        }
    }

    fn populated() -> Sources {
        let hourly = |n: u64| {
            json!([[
                chrono::DateTime::from_timestamp(NOW, 0)
                    .map(|t| t.format("%Y-%m-%dT%H").to_string())
                    .unwrap_or_default(),
                n
            ]])
        };
        Sources {
            errors: Some(Ok(vec![json!({
                "_key": "0123456789abcdef", "message": "NoMethodError: undefined method 'total' for nil",
                "location": "app/controllers/orders_controller.sl:42", "status": "open", "count": 214,
                "first_seen": iso(NOW - 2_280), "last_seen": iso(NOW - 60), "hourly": hourly(214),
            })])),
            slow: Some(Ok(vec![json!({
                "_key": "fedcba9876543210", "query": "FOR doc IN posts FILTER doc.author_id == @id RETURN doc",
                "count": 12, "total_ms": 21_600.0, "max_ms": 2_400.0, "first_seen": iso(NOW - 600),
                "last_seen": iso(NOW - 60), "last_context": "GET /posts \u{2192} posts#index", "hourly": hourly(12),
            })])),
            slow_threshold_ms: 200,
            spam: Some(Ok(vec![
                json!({"created_at": iso(NOW - 120), "path": "/users/sign_up", "ip": "10.0.0.1", "reason": "honeypot"}),
            ])),
            jobs: Some(Ok(vec![json!({
                "_key": "job-1", "state": "dead", "handler": "InvoiceMailerJob", "finished_at": iso(NOW - 720),
                "attempts": 6, "max_retries": 5, "last_error": "SMTP connection refused",
            })])),
            usage: Some(usage_view()),
            usage_hour: Some(usage_view()),
            engine: vec![EngineEvent {
                tenant: crate::serve::tenant::TenantId::PRIMARY,
                kind: EngineKind::Refused,
                subject: "reports#export".into(),
                detail: "unsupported opcode".into(),
                count: 1,
                first_at: NOW - 3 * 3600,
                last_at: NOW - 3 * 3600,
            }],
            recording: Vec::new(),
        }
    }

    fn empty() -> Sources {
        Sources {
            errors: Some(Ok(Vec::new())),
            slow: Some(Ok(Vec::new())),
            slow_threshold_ms: 200,
            spam: Some(Ok(Vec::new())),
            jobs: Some(Ok(Vec::new())),
            usage: Some(UsageView {
                range: Range::Day,
                now: NOW as u64,
                bars: Vec::new(),
                current: Window::default(),
                previous: None,
                routes: Vec::new(),
            }),
            usage_hour: None,
            engine: Vec::new(),
            recording: Vec::new(),
        }
    }

    fn body_of(resp: Response<ResponseBody>) -> String {
        use http_body_util::BodyExt;
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() else {
            return String::new();
        };
        let bytes = runtime
            .block_on(resp.into_body().collect())
            .map(|c| c.to_bytes())
            .unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn the_paths() {
        assert!(is_overview_path("GET", "/__soli/"));
        assert!(is_overview_path("GET", "/__soli"));
        assert!(is_overview_path("HEAD", "/__soli/"));
        assert!(!is_overview_path("POST", "/__soli/"));
        assert!(!is_overview_path("GET", "/__soli/errors"));
        assert!(!is_overview_path("GET", "/__solix"));
    }

    #[test]
    fn production_without_credentials_is_hidden() {
        let resp = serve(
            None,
            |_| DashAuth::Hidden,
            &|_, _| panic!("no read"),
            NOW,
            false,
        );
        assert_eq!(resp.status(), 404);
        assert!(!body_of(resp).contains("Overview"));
    }

    #[test]
    fn configured_but_not_presented_asks_for_credentials() {
        let decide = |p: &str| {
            if p == "JOBS" {
                DashAuth::NeedAuth
            } else {
                DashAuth::Hidden
            }
        };
        let resp = serve(None, decide, &|_, _| panic!("no read"), NOW, false);
        assert_eq!(resp.status(), 401);
        assert!(resp.headers().contains_key("www-authenticate"));
    }

    /// Credentials for one page open the overview with that page's source
    /// only: no other tile, no usage, no row from elsewhere.
    #[test]
    fn page_credentials_show_only_that_page() {
        let decide = |p: &str| {
            if p == "JOBS" {
                DashAuth::Allow
            } else {
                DashAuth::NeedAuth
            }
        };
        let allowed = gate(decide).expect("opens");
        assert_eq!(
            allowed,
            Allowed {
                jobs: true,
                ..Allowed::default()
            }
        );
        let fetch = |allowed: &Allowed, _: Range| {
            assert!(!allowed.errors && !allowed.overview, "nothing else is read");
            let mut sources = populated();
            sources.errors = None;
            sources.slow = None;
            sources.spam = None;
            sources.usage = None;
            sources.usage_hour = None;
            sources
        };
        let resp = serve(None, decide, &fetch, NOW, false);
        assert_eq!(resp.status(), 200);
        let html = body_of(resp);
        assert!(html.contains("InvoiceMailerJob"));
        assert!(!html.contains("Open errors"));
        assert!(!html.contains("id=\"usage-h\""));
        assert!(
            !html.contains("slower engine"),
            "engine events are the overview's own"
        );
        assert!(!html.contains("NoMethodError"));
    }

    #[test]
    fn dev_local_or_admin_sees_everything() {
        let allowed = gate(|_| DashAuth::Allow).expect("opens");
        assert_eq!(allowed, all());
        let html = render_page(&params(), &allowed, &populated(), NOW, true);
        for needle in [
            "Requests \u{b7} 24 h",
            "Server errors",
            "Latency p95",
            "Open errors",
            "1 dead",
            "Slow queries",
            "Needs attention",
            "id=\"usage-h\"",
            "Top routes",
            "Responses by status",
        ] {
            assert!(html.contains(needle), "missing {needle}");
        }
        // The overview is the current page and the first item of the bar.
        assert!(html.contains("<a href=\"/__soli/\" aria-current=\"page\">Overview</a>"));
    }

    #[test]
    fn rows_are_ranked_with_their_actions() {
        let html = render_page(&params(), &all(), &populated(), NOW, false);
        let critical = html.find("badge crit").expect("a critical row");
        let warning = html.find("badge warn").expect("a warning row");
        assert!(critical < warning, "critical first");
        assert!(html.contains("href=\"/__soli/errors/0123456789abcdef\""));
        assert!(html.contains(
            "<form method=\"post\" action=\"/__soli/jobs/job-1/retry\"><button class=\"ghost\" type=\"submit\">Retry</button></form>"
        ));
        assert!(html.contains("href=\"/__soli/slow_queries/fedcba9876543210\""));
        assert!(html.contains("A handler runs on the slower engine"));
        assert!(
            html.contains("<span class=\"chip\">Traffic</span>"),
            "a slow route"
        );
        // The p95 of a route is highlighted, never its raw path.
        assert!(html.contains("<code>GET /report</code>"));
        assert!(html.contains("class=\"slow\""));
        assert!(html.contains("class=\"bad\""));
        assert!(!html.contains("/posts/7"));
    }

    #[test]
    fn empty_sources_say_so() {
        let html = render_page(&params(), &all(), &empty(), NOW, false);
        assert!(html.contains("Nothing needs attention."));
        assert!(html.contains("No requests recorded over the last 24 hours yet."));
        assert!(html.contains("nothing waiting"));
        assert!(!html.contains("<ol>"));
        // Not recording at all.
        let mut off = empty();
        off.usage = None;
        let html = render_page(&params(), &all(), &off, NOW, false);
        assert!(html.contains("SOLI_USAGE=off"));
    }

    #[test]
    fn an_unreadable_store_is_a_row_and_a_tile() {
        let mut sources = empty();
        sources.errors = Some(Err("HTTP 503 <busy>".into()));
        let html = render_page(&params(), &all(), &sources, NOW, false);
        assert!(html.contains("Could not read the error store"));
        assert!(html.contains("HTTP 503 &lt;busy&gt;"));
        assert!(html.contains("could not be read"));
    }

    #[test]
    fn hostile_strings_are_escaped_everywhere() {
        let evil = "<script>alert(1)</script>\"'&";
        let mut sources = populated();
        sources.errors = Some(Ok(vec![json!({
            "_key": "0123456789abcdef", "message": evil, "location": evil,
            "count": 3, "first_seen": iso(NOW - 60), "last_seen": iso(NOW - 60), "hourly": [],
        })]));
        sources.slow = Some(Ok(vec![json!({
            "_key": "fedcba9876543210", "query": evil, "count": 1, "total_ms": 900.0, "max_ms": 900.0,
            "first_seen": iso(NOW - 60), "last_seen": iso(NOW - 60), "last_context": format!("GET / \u{2192} {evil}"),
        })]));
        sources.spam = Some(Ok(vec![
            json!({"created_at": iso(NOW - 60), "path": evil, "ip": evil}),
        ]));
        sources.jobs = Some(Ok(vec![json!({
            "_key": "x\"><script>", "state": "dead", "handler": evil, "finished_at": iso(NOW - 60), "last_error": evil,
        })]));
        if let Some(view) = sources.usage.as_mut() {
            view.routes[0].label = format!("GET {evil}");
        }
        sources.engine[0].subject = evil.into();
        sources.engine[0].detail = evil.into();
        let html = render_page(
            &Params {
                range: Range::Day,
                refresh: true,
            },
            &all(),
            &sources,
            NOW,
            false,
        );
        assert!(!html.contains("<script>alert"), "raw markup leaked");
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;&quot;&#39;&amp;"));
        // A job id that is not an id gets no retry form.
        assert!(!html.contains("/retry"));
        // Only our own script and the refresh meta are markup.
        assert_eq!(html.matches("<script>").count(), 1);
        assert!(html.contains("<meta http-equiv=\"refresh\" content=\"30\">"));
    }

    #[test]
    fn ranges_and_refresh_are_plain_links() {
        let p = Params::parse(Some("range=7d&refresh=30"));
        assert_eq!(
            p,
            Params {
                range: Range::Week,
                refresh: true
            }
        );
        assert_eq!(Params::parse(Some("range=bogus")).range, Range::Day);
        assert!(!Params::parse(Some("refresh=1")).refresh);
        let html = render_page(&params(), &all(), &populated(), NOW, false);
        assert!(html.contains("<a href=\"/__soli/?range=24h\" aria-current=\"true\">24 h</a>"));
        assert!(html.contains("<a href=\"/__soli/?range=1h\">1 h</a>"));
        assert!(html.contains("<a href=\"/__soli/?range=7d\">7 d</a>"));
        assert!(html.contains("href=\"/__soli/?range=24h&amp;refresh=30\""));
        assert!(!html.contains("http-equiv=\"refresh\""), "off by default");
    }

    #[test]
    fn the_chart_scales_and_labels() {
        assert_eq!(nice_step(0), 1);
        assert_eq!(nice_step(3_947), 5_000);
        assert_eq!(nice_step(4_000), 5_000);
        assert_eq!(nice_step(2_100), 2_500);
        assert_eq!(compact(12_000), "12k");
        assert_eq!(compact(2_500), "2.5k");
        assert_eq!(compact(40), "40");
        let html = chart(&usage_view());
        assert_eq!(
            html.matches("<div class=\"c\"").count() + html.matches("<div class=\"c now\"").count(),
            24
        );
        assert_eq!(html.matches("class=\"c now\"").count(), 1);
        assert!(html.contains("peak 330 at 14:00"));
        assert!(html.contains("role=\"img\" aria-label=\"Requests per hour over the last 24 hours, peaking at 330 at 14:00 UTC"));
        assert!(html.contains("title=\"14:00 \u{b7} 330 requests\""));
    }

    /// The sample quoted in the change's report.
    #[test]
    fn print_a_sample() {
        let html = render_page(&params(), &all(), &populated(), NOW, false);
        let start = html.find("<section class=\"attn\"").unwrap_or(0);
        let end = html[start..]
            .find("</ol>")
            .map(|i| start + i + 5)
            .unwrap_or(html.len());
        eprintln!("[overview sample]\n{}", &html[start..end]);
    }
}
