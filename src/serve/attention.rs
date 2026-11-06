//! "Needs attention": one ranked list across every operator source, for the
//! `/__soli/` overview.
//!
//! Each source turns what it stores into [`Item`]s — pure functions over the
//! rows the existing pages already read, so they are tested without a
//! database. [`rank`] then orders them:
//!
//! 1. by [`Tier`]: critical, then warning, then info;
//! 2. by [`score`], highest first — 40 points for recency, 35 for growth and
//!    25 for volume:
//!    - recency = `1 / (1 + hours since last seen)` (1 just now, ½ an hour ago);
//!    - growth = `(last hour − hourly average of the 23 before) / (sum of the
//!      two)`, floored at 0 (0 when steady or falling, 1 when out of nowhere);
//!    - volume = `log10(1 + count over 24 h) / 4`, capped at 1 (10 000 → 1);
//! 3. ties: most recently seen first, then by title and detail, so the order
//!    is stable from one refresh to the next.
//!
//! The tiers, source by source, are documented on each builder. Resolved and
//! ignored error groups never reach here: only the `open` list is read.

use super::dev_errors::short_count;
use super::engine_events::{EngineEvent, EngineKind};
use super::tenant_writer::StatsSnapshot;
use super::usage::UsageView;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Tier {
    Critical,
    Warning,
    Info,
}

impl Tier {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Tier::Critical => "Critical",
            Tier::Warning => "Warning",
            Tier::Info => "Info",
        }
    }

    /// The CSS class of its badge.
    pub(crate) fn class(self) -> &'static str {
        match self {
            Tier::Critical => "crit",
            Tier::Warning => "warn",
            Tier::Info => "info",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Error,
    Jobs,
    Query,
    Spam,
    Engine,
    Traffic,
    Recording,
}

impl Source {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Source::Error => "Error",
            Source::Jobs => "Jobs",
            Source::Query => "Query",
            Source::Spam => "Spam",
            Source::Engine => "Engine",
            Source::Traffic => "Traffic",
            Source::Recording => "Recording",
        }
    }
}

/// What a row offers: a link, or a POST form to an action an existing page
/// already has (it keeps that page's same-origin rule; nothing changes on GET).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    None,
    Link { href: String, label: &'static str },
    Post { href: String, label: &'static str },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    pub tier: Tier,
    pub source: Source,
    pub title: String,
    /// Shown in monospace: the message, the query, the handler.
    pub detail: String,
    /// Why it is here: counts, last seen, trend.
    pub why: String,
    pub metric: String,
    pub metric_note: String,
    pub action: Action,
    pub score: f64,
    /// Unix seconds; 0 when unknown.
    pub last_seen: i64,
}

/// What [`score`] reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Signals {
    /// Occurrences in the last hour.
    pub last_hour: u64,
    /// Occurrences in the 23 hours before it.
    pub previous_hours: u64,
    /// Occurrences over the last 24 hours (the two above).
    pub last_24h: u64,
    /// Unix seconds.
    pub last_seen: Option<i64>,
}

impl Signals {
    /// Average per hour over the 23 hours before the last one.
    pub(crate) fn previous_average(&self) -> f64 {
        self.previous_hours as f64 / 23.0
    }

    /// 0..=1: how much the last hour exceeds the hours before it.
    pub(crate) fn growth(&self) -> f64 {
        let now = self.last_hour as f64;
        let before = self.previous_average();
        if now + before <= 0.0 {
            return 0.0;
        }
        ((now - before) / (now + before)).clamp(0.0, 1.0)
    }

    /// `↑ rising`, `steady`, `↓ falling` or `quiet this hour`.
    pub(crate) fn trend(&self) -> &'static str {
        let now = self.last_hour as f64;
        let before = self.previous_average();
        if self.last_hour == 0 {
            "quiet this hour"
        } else if now >= before * 1.5 + 1.0 {
            "\u{2191} rising"
        } else if now * 1.5 + 1.0 <= before {
            "\u{2193} falling"
        } else {
            "steady"
        }
    }
}

/// The documented score: 40·recency + 35·growth + 25·volume, 0..=100.
pub(crate) fn score(signals: &Signals, now: i64) -> f64 {
    let recency = match signals.last_seen {
        Some(at) => 1.0 / (1.0 + (now - at).max(0) as f64 / 3600.0),
        None => 0.0,
    };
    let volume = ((1 + signals.last_24h) as f64).log10() / 4.0;
    40.0 * recency + 35.0 * signals.growth() + 25.0 * volume.min(1.0)
}

/// Tier, then score, then the most recent, then title and detail.
pub(crate) fn rank(items: &mut [Item]) {
    items.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then_with(|| b.score.total_cmp(&a.score))
            .then_with(|| b.last_seen.cmp(&a.last_seen))
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.detail.cmp(&b.detail))
    });
}

// --- shared helpers -------------------------------------------------------------

pub(crate) fn parse_iso(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|t| t.timestamp())
}

/// `3 min ago`.
pub(crate) fn ago(seconds: i64) -> String {
    match seconds.max(0) {
        0..=59 => "just now".to_string(),
        s @ 60..=3_599 => format!("{} min ago", s / 60),
        s @ 3_600..=86_399 => format!("{} h ago", s / 3_600),
        s => format!("{} d ago", s / 86_400),
    }
}

/// `9 min`, `2 h`, `3 d`: a duration.
pub(crate) fn span(seconds: i64) -> String {
    match seconds.max(0) {
        0..=59 => format!("{} s", seconds.max(0)),
        s @ 60..=3_599 => format!("{} min", s / 60),
        s @ 3_600..=86_399 => format!("{} h", s / 3_600),
        s => format!("{} d", s / 86_400),
    }
}

/// `182,430`.
pub(crate) fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `0.4 ms`, `184 ms`, `1.8 s`, from microseconds.
pub(crate) fn fmt_us(micros: u64) -> String {
    let ms = micros as f64 / 1000.0;
    fmt_ms(ms)
}

pub(crate) fn fmt_ms(ms: f64) -> String {
    if ms < 1.0 {
        format!("{ms:.1} ms")
    } else if ms < 1000.0 {
        format!("{} ms", ms.round() as u64)
    } else if ms < 10_000.0 {
        format!("{:.1} s", ms / 1000.0)
    } else {
        format!("{} s", (ms / 1000.0).round() as u64)
    }
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", thousands(n), if n == 1 { one } else { many })
}

fn cut(text: &str, max: usize) -> String {
    super::error_tracker::truncate_chars(text.trim(), max)
}

/// `%Y-%m-%dT%H` of a unix time: the trackers' hourly keys.
fn hour_label(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .map(|t| t.format("%Y-%m-%dT%H").to_string())
        .unwrap_or_default()
}

/// The trackers keep per-hour counts. The last hour is the current hour plus
/// the part of the hour before that the clock has not left yet.
pub(crate) fn hourly_signals(
    hourly: &std::collections::BTreeMap<String, u64>,
    now: i64,
    last_seen: Option<i64>,
) -> Signals {
    let at = |back: i64| {
        hourly
            .get(&hour_label(now - back * 3600))
            .copied()
            .unwrap_or(0)
    };
    let into_hour = (now.rem_euclid(3600)) as f64 / 3600.0;
    let last_hour = at(0) + (at(1) as f64 * (1.0 - into_hour)).round() as u64;
    let last_24h: u64 = (0..24).map(at).sum();
    Signals {
        last_hour: last_hour.min(last_24h),
        previous_hours: last_24h.saturating_sub(last_hour),
        last_24h,
        last_seen,
    }
}

fn str_field<'a>(row: &'a serde_json::Value, field: &str) -> &'a str {
    row.get(field).and_then(|v| v.as_str()).unwrap_or("")
}

fn u64_field(row: &serde_json::Value, field: &str) -> u64 {
    row.get(field).and_then(|v| v.as_u64()).unwrap_or(0)
}

fn f64_field(row: &serde_json::Value, field: &str) -> f64 {
    row.get(field).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

/// `app/models/order.sl:42` → `app/models/order.sl`.
fn file_of(location: &str) -> &str {
    match location.rsplit_once(':') {
        Some((file, line)) if !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit()) => file,
        _ => location,
    }
}

/// `/posts/42/comments` → `/posts/:id/comments`: what a raw path looks like
/// with its ids taken out (digits, long hex, UUIDs).
pub(crate) fn pattern_like(path: &str) -> String {
    let path = path.split(['?', '#']).next().unwrap_or("");
    path.split('/')
        .map(|seg| {
            let id_like = !seg.is_empty()
                && (seg.bytes().all(|b| b.is_ascii_digit())
                    || (seg.len() >= 8
                        && seg.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
                        && seg.bytes().any(|b| b.is_ascii_digit())));
            if id_like {
                ":id"
            } else {
                seg
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

// --- errors ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ErrorSummary {
    pub open: usize,
    /// The list hit its limit: there are at least `open`.
    pub capped: bool,
    pub new_last_hour: usize,
}

/// Open error groups (`error_tracker::list("open", limit)`).
///
/// - **Critical**: new (first seen within the hour); came back after being
///   resolved within the day and seen within the hour; or rising — 10 or more
///   in the last hour and at least three times the hourly average before.
/// - **Warning**: seen in the last 24 hours.
/// - **Info**: open, quiet for more than a day.
pub(crate) fn error_items(
    rows: &[serde_json::Value],
    limit: usize,
    now: i64,
) -> (Vec<Item>, ErrorSummary) {
    let mut summary = ErrorSummary {
        open: rows.len(),
        capped: rows.len() >= limit,
        new_last_hour: 0,
    };
    let mut items = Vec::new();
    for row in rows {
        let key = str_field(row, "_key");
        if !super::error_tracker::valid_fingerprint(key) {
            continue;
        }
        let last_seen = parse_iso(str_field(row, "last_seen"));
        let first_seen = parse_iso(str_field(row, "first_seen"));
        let regressed = parse_iso(str_field(row, "regressed_at"));
        let hourly = super::error_tracker::parse_hourly(row.get("hourly"));
        let signals = hourly_signals(&hourly, now, last_seen);
        let since = |t: Option<i64>| t.map(|t| now - t).unwrap_or(i64::MAX);

        let is_new = since(first_seen) < 3600;
        let came_back = since(regressed) < 86_400 && since(last_seen) < 3600;
        let rising = signals.last_hour >= 10
            && signals.last_hour as f64 >= 3.0 * signals.previous_average().max(1.0);
        if is_new {
            summary.new_last_hour += 1;
        }
        let (tier, headline) = if is_new {
            (Tier::Critical, "New error")
        } else if came_back {
            (Tier::Critical, "Error came back after it was resolved")
        } else if rising {
            (Tier::Critical, "Error is rising")
        } else if since(last_seen) < 86_400 {
            (Tier::Warning, "Recurring error")
        } else {
            (Tier::Info, "Open error, quiet for over a day")
        };
        let location = str_field(row, "location");
        let title = if location.is_empty() {
            headline.to_string()
        } else {
            format!("{headline} in {}", file_of(location))
        };
        let message = cut(str_field(row, "message"), 300);
        let detail = if location.is_empty() {
            message
        } else {
            format!("{message} \u{b7} {location}")
        };

        let total = u64_field(row, "count");
        let mut why = Vec::new();
        if is_new {
            why.push(format!("First seen {}", ago(since(first_seen))));
        } else if came_back {
            why.push(format!("Came back {}", ago(since(regressed))));
        }
        if let Some(seen) = last_seen {
            if !is_new {
                why.push(format!("last seen {}", ago(now - seen)));
            }
        }
        why.push(format!("{} in the last hour", thousands(signals.last_hour)));
        why.push(format!("{} in 24 h", thousands(signals.last_24h)));
        why.push(format!("{} in total", thousands(total)));
        let why = capitalize(&why.join(" \u{b7} "));

        let (metric, metric_note) = if signals.last_hour > 0 {
            (
                format!("{} / h", short_count(signals.last_hour)),
                signals.trend().to_string(),
            )
        } else {
            (short_count(total), "in total".to_string())
        };
        items.push(Item {
            tier,
            source: Source::Error,
            title,
            detail,
            why,
            metric,
            metric_note,
            action: Action::Link {
                href: format!("/__soli/errors/{key}"),
                label: "Open error",
            },
            score: score(&signals, now),
            last_seen: last_seen.unwrap_or(0),
        });
    }
    (items, summary)
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

// --- slow queries ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SlowSummary {
    /// Shapes seen slow in the last 24 hours.
    pub seen_24h: usize,
    /// Of those, first seen in the last 24 hours.
    pub new_24h: usize,
}

/// Slow-query groups seen in the last 24 hours (older ones are history, not
/// attention), at most [`MAX_QUERY_ITEMS`] of them, the best scored.
///
/// - **Warning**: new within the hour, or seen within the hour with a mean at
///   least twice the threshold.
/// - **Info**: the rest of the day's.
pub(crate) fn slow_query_items(
    rows: &[serde_json::Value],
    threshold_ms: u64,
    now: i64,
) -> (Vec<Item>, SlowSummary) {
    let mut summary = SlowSummary::default();
    let mut items = Vec::new();
    for row in rows {
        let key = str_field(row, "_key");
        if !super::slow_queries::valid_fingerprint(key) {
            continue;
        }
        let last_seen = parse_iso(str_field(row, "last_seen"));
        let first_seen = parse_iso(str_field(row, "first_seen"));
        let since = |t: Option<i64>| t.map(|t| now - t).unwrap_or(i64::MAX);
        if since(last_seen) >= 86_400 {
            continue;
        }
        summary.seen_24h += 1;
        let is_new = since(first_seen) < 3600;
        if since(first_seen) < 86_400 {
            summary.new_24h += 1;
        }
        let hourly = super::slow_queries::parse_hourly(row.get("hourly"));
        let signals = hourly_signals(&hourly, now, last_seen);
        let count = u64_field(row, "count");
        let mean = if count > 0 {
            f64_field(row, "total_ms") / count as f64
        } else {
            0.0
        };
        let max = f64_field(row, "max_ms");
        let heavy = since(last_seen) < 3600 && mean >= 2.0 * threshold_ms as f64;
        let tier = if is_new || heavy {
            Tier::Warning
        } else {
            Tier::Info
        };
        // `GET /orders/7 → orders#index`: only the handler, never the raw path.
        let context = str_field(row, "last_context");
        let handler = context
            .rsplit_once('\u{2192}')
            .map(|(_, h)| h.trim())
            .unwrap_or("");
        let headline = if is_new {
            "New slow query"
        } else {
            "Slow query"
        };
        let title = if handler.is_empty() {
            headline.to_string()
        } else {
            format!("{headline} in {handler}")
        };
        let mut why = Vec::new();
        if is_new {
            why.push(format!("New, first seen {}", ago(since(first_seen))));
        } else if let Some(seen) = last_seen {
            why.push(format!("Last seen {}", ago(now - seen)));
        }
        why.push(format!(
            "{} in the last hour",
            plural(signals.last_hour, "slow run", "slow runs")
        ));
        why.push(format!("max {}", fmt_ms(max)));
        why.push(format!("{} in total", thousands(count)));
        items.push(Item {
            tier,
            source: Source::Query,
            title,
            detail: cut(str_field(row, "query"), 240),
            why: why.join(" \u{b7} "),
            metric: fmt_ms(mean),
            metric_note: "avg".to_string(),
            action: Action::Link {
                href: format!("/__soli/slow_queries/{key}"),
                label: "Open query",
            },
            score: score(&signals, now),
            last_seen: last_seen.unwrap_or(0),
        });
    }
    rank(&mut items);
    items.truncate(MAX_QUERY_ITEMS);
    (items, summary)
}

/// Slow-query rows in the list, at most.
pub(crate) const MAX_QUERY_ITEMS: usize = 5;

// --- jobs -------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct JobSummary {
    /// Dead in the last 24 hours.
    pub dead_24h: u64,
    /// Due and waiting for a worker.
    pub queued: u64,
    /// How long the oldest due job has waited, in seconds.
    pub oldest_wait: Option<i64>,
    /// The rows read were the page's whole window (the jobs page reads 500).
    pub window_full: bool,
}

/// The queue rows the jobs page reads (`jobs::store::list(None)`, oldest
/// `run_at` first, at most 500).
///
/// - **Critical**: jobs dead in the last hour (retries exhausted); a backlog
///   whose oldest due job has waited 30 minutes or more.
/// - **Warning**: jobs dead in the last 24 hours; jobs failing and waiting for
///   a retry; jobs running past their lease (stuck); a backlog of 100 or more,
///   or whose oldest has waited 5 minutes.
///
/// Dead jobs are grouped per handler. A group of one offers the jobs page's own
/// Retry; more link to the dead jobs there.
pub(crate) fn job_items(
    rows: &[serde_json::Value],
    window: usize,
    now: i64,
) -> (Vec<Item>, JobSummary) {
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Group {
        count: u64,
        last_at: i64,
        last_error: String,
        first_id: String,
        attempts: i64,
        max_retries: i64,
    }

    let mut summary = JobSummary {
        window_full: rows.len() >= window,
        ..JobSummary::default()
    };
    let mut dead: BTreeMap<String, Group> = BTreeMap::new();
    let mut failed: BTreeMap<String, Group> = BTreeMap::new();
    let mut stuck: u64 = 0;
    let mut stuck_since: i64 = i64::MAX;
    let mut queues: BTreeMap<String, u64> = BTreeMap::new();

    for row in rows {
        let state = str_field(row, "state");
        let handler = str_field(row, "handler").to_string();
        let id = str_field(row, "_key");
        let run_at = parse_iso(str_field(row, "run_at"));
        let note = |group: &mut Group, at: i64| {
            group.count += 1;
            if at >= group.last_at {
                group.last_at = at;
                group.last_error = cut(str_field(row, "last_error"), 200);
                group.attempts = row.get("attempts").and_then(|v| v.as_i64()).unwrap_or(0);
                group.max_retries = row.get("max_retries").and_then(|v| v.as_i64()).unwrap_or(0);
            }
            if group.first_id.is_empty() {
                group.first_id = id.to_string();
            }
        };
        match state {
            "dead" => {
                let at = parse_iso(str_field(row, "finished_at"))
                    .or(run_at)
                    .unwrap_or(0);
                if now - at < 86_400 {
                    summary.dead_24h += 1;
                    note(dead.entry(handler).or_default(), at);
                }
            }
            "failed" => {
                let at = run_at.unwrap_or(now);
                note(failed.entry(handler).or_default(), at);
                if at <= now {
                    summary.queued += 1;
                    summary.oldest_wait = Some(summary.oldest_wait.unwrap_or(0).max(now - at));
                }
            }
            "pending" | "scheduled" => {
                if let Some(at) = run_at.filter(|at| *at <= now) {
                    summary.queued += 1;
                    summary.oldest_wait = Some(summary.oldest_wait.unwrap_or(0).max(now - at));
                    *queues
                        .entry(str_field(row, "queue").to_string())
                        .or_default() += 1;
                }
            }
            "running" => {
                if let Some(until) = parse_iso(str_field(row, "locked_until")) {
                    if until + 60 < now {
                        stuck += 1;
                        stuck_since = stuck_since.min(until);
                    }
                }
            }
            _ => {}
        }
    }

    let mut items = Vec::new();
    let valid = |id: &str| {
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    };
    for (handler, group) in &dead {
        let recent = now - group.last_at < 3600;
        let signals = Signals {
            last_hour: if recent { group.count } else { 0 },
            previous_hours: if recent { 0 } else { group.count },
            last_24h: group.count,
            last_seen: Some(group.last_at),
        };
        let action = if group.count == 1 && valid(&group.first_id) {
            Action::Post {
                href: format!("/__soli/jobs/{}/retry", group.first_id),
                label: "Retry",
            }
        } else {
            Action::Link {
                href: "/__soli/jobs?state=dead".to_string(),
                label: "See dead jobs",
            }
        };
        let name = if handler.is_empty() {
            "job"
        } else {
            handler.as_str()
        };
        items.push(Item {
            tier: if recent {
                Tier::Critical
            } else {
                Tier::Warning
            },
            source: Source::Jobs,
            title: if group.count == 1 {
                format!("1 {name} job is dead after its last retry")
            } else {
                format!(
                    "{} {name} jobs are dead after their last retry",
                    group.count
                )
            },
            detail: if group.last_error.is_empty() {
                format!("{name} \u{d7} {}", group.count)
            } else {
                format!("{name} \u{d7} {} \u{b7} {}", group.count, group.last_error)
            },
            why: format!(
                "Last one died {} \u{b7} tried {} time{}",
                ago(now - group.last_at),
                group.attempts,
                if group.attempts == 1 { "" } else { "s" }
            ),
            metric: group.count.to_string(),
            metric_note: "dead".to_string(),
            action,
            score: score(&signals, now),
            last_seen: group.last_at,
        });
    }
    for (handler, group) in &failed {
        let name = if handler.is_empty() {
            "job"
        } else {
            handler.as_str()
        };
        let next = group.last_at - now;
        let signals = Signals {
            last_hour: group.count,
            last_24h: group.count,
            last_seen: Some(now),
            ..Signals::default()
        };
        items.push(Item {
            tier: Tier::Warning,
            source: Source::Jobs,
            title: format!(
                "{} failing, waiting for a retry",
                plural(group.count, &format!("{name} job"), &format!("{name} jobs"))
            ),
            detail: if group.last_error.is_empty() {
                name.to_string()
            } else {
                format!("{name} \u{b7} {}", group.last_error)
            },
            why: format!(
                "Attempt {} of {} \u{b7} {}",
                group.attempts,
                group.max_retries + 1,
                if next > 0 {
                    format!("next retry in {}", span(next))
                } else {
                    "retry due now".to_string()
                }
            ),
            metric: group.count.to_string(),
            metric_note: "failing".to_string(),
            action: Action::Link {
                href: "/__soli/jobs?state=failed".to_string(),
                label: "See jobs",
            },
            score: score(&signals, now),
            last_seen: now,
        });
    }
    if stuck > 0 {
        items.push(Item {
            tier: Tier::Warning,
            source: Source::Jobs,
            title: format!(
                "{} running past {} lease",
                plural(stuck, "job is", "jobs are"),
                if stuck == 1 { "its" } else { "their" }
            ),
            detail: "state running \u{b7} lease expired".to_string(),
            why: format!(
                "The oldest lease ran out {} \u{b7} the poller reclaims it, or its worker died",
                ago(now - stuck_since)
            ),
            metric: stuck.to_string(),
            metric_note: "stuck".to_string(),
            action: Action::Link {
                href: "/__soli/jobs?state=running".to_string(),
                label: "See jobs",
            },
            score: score(
                &Signals {
                    last_hour: stuck,
                    last_24h: stuck,
                    last_seen: Some(stuck_since),
                    ..Signals::default()
                },
                now,
            ),
            last_seen: stuck_since,
        });
    }
    let oldest = summary.oldest_wait.unwrap_or(0);
    if summary.queued >= 100 || oldest >= 300 {
        let busiest = queues
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(q, n)| (q.clone(), *n));
        let detail = match busiest {
            Some((queue, n)) if !queue.is_empty() => {
                format!("{queue} queue \u{b7} {} waiting", thousands(n))
            }
            _ => format!("{} waiting", thousands(summary.queued)),
        };
        items.push(Item {
            tier: if oldest >= 1800 {
                Tier::Critical
            } else {
                Tier::Warning
            },
            source: Source::Jobs,
            title: "The queue is backing up".to_string(),
            detail,
            why: format!(
                "The oldest due job has waited {}{}",
                span(oldest),
                if summary.window_full {
                    " \u{b7} counted over the first 500 rows"
                } else {
                    ""
                }
            ),
            metric: thousands(summary.queued),
            metric_note: "waiting".to_string(),
            action: Action::Link {
                href: "/__soli/jobs?state=pending".to_string(),
                label: "See jobs",
            },
            score: score(
                &Signals {
                    last_hour: summary.queued,
                    last_24h: summary.queued,
                    last_seen: Some(now),
                    ..Signals::default()
                },
                now,
            ),
            last_seen: now,
        });
    }
    (items, summary)
}

// --- spam -------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SpamSummary {
    pub last_hour: u64,
    pub last_24h: u64,
}

/// Refused posts (`spam::list("spam", limit)`, newest first).
///
/// - **Warning**: a spike — 20 or more in the last hour and five times the
///   hourly average of the 23 before.
/// - **Info**: any refused in the last 24 hours. Refused is handled; it is
///   listed so a wave is seen, not because something failed.
pub(crate) fn spam_items(rows: &[serde_json::Value], now: i64) -> (Vec<Item>, SpamSummary) {
    use std::collections::{BTreeMap, BTreeSet};

    let mut summary = SpamSummary::default();
    let mut last_seen: Option<i64> = None;
    let mut paths: BTreeMap<String, u64> = BTreeMap::new();
    let mut reasons: BTreeMap<String, u64> = BTreeMap::new();
    let mut ips: BTreeSet<&str> = BTreeSet::new();
    for row in rows {
        let Some(at) = parse_iso(str_field(row, "created_at")) else {
            continue;
        };
        if now - at >= 86_400 {
            continue;
        }
        summary.last_24h += 1;
        last_seen = Some(last_seen.map_or(at, |s| s.max(at)));
        if now - at < 3600 {
            summary.last_hour += 1;
            *paths
                .entry(pattern_like(str_field(row, "path")))
                .or_default() += 1;
            let reason = str_field(row, "reason");
            *reasons
                .entry(if reason.is_empty() { "score" } else { reason }.to_string())
                .or_default() += 1;
            ips.insert(str_field(row, "ip"));
        }
    }
    if summary.last_24h == 0 {
        return (Vec::new(), summary);
    }
    let signals = Signals {
        last_hour: summary.last_hour,
        previous_hours: summary.last_24h - summary.last_hour,
        last_24h: summary.last_24h,
        last_seen,
    };
    let spike = summary.last_hour >= 20
        && summary.last_hour as f64 >= 5.0 * signals.previous_average().max(1.0);
    let top_path = paths
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(p, _)| p.clone());
    let detail = match &top_path {
        Some(path) => format!(
            "{path} \u{b7} {}",
            plural(ips.len() as u64, "source IP", "source IPs")
        ),
        None => "none in the last hour".to_string(),
    };
    let reasons = reasons
        .iter()
        .map(|(r, n)| format!("{r} {n}"))
        .collect::<Vec<_>>()
        .join(", ");
    let why = if summary.last_hour > 0 {
        format!(
            "In the last hour \u{b7} refused by {reasons} \u{b7} {} in 24 h",
            thousands(summary.last_24h)
        )
    } else {
        format!(
            "{} in 24 h \u{b7} last {}",
            thousands(summary.last_24h),
            ago(now - last_seen.unwrap_or(now))
        )
    };
    let item = Item {
        tier: if spike { Tier::Warning } else { Tier::Info },
        source: Source::Spam,
        title: if spike {
            "Spam spike on a form".to_string()
        } else {
            "Spam posts refused".to_string()
        },
        detail,
        why,
        metric: thousands(if summary.last_hour > 0 {
            summary.last_hour
        } else {
            summary.last_24h
        }),
        metric_note: if summary.last_hour > 0 {
            "refused / h".to_string()
        } else {
            "refused / 24 h".to_string()
        },
        action: Action::Link {
            href: "/__soli/spam".to_string(),
            label: "Open spam",
        },
        score: score(&signals, now),
        last_seen: last_seen.unwrap_or(0),
    };
    (vec![item], summary)
}

// --- engine -------------------------------------------------------------------------

/// What the engines recorded on this process ([`super::engine_events`]).
///
/// - **Critical**: a handler panicked within the hour.
/// - **Warning**: an older panic; a handler the VM refused (it now runs on the
///   slower tree-walker); a handler the two engines disagree on.
pub(crate) fn engine_items(events: &[EngineEvent], now: i64) -> Vec<Item> {
    events
        .iter()
        .map(|event| {
            let signals = Signals {
                last_hour: if now - event.last_at < 3600 {
                    event.count
                } else {
                    0
                },
                last_24h: event.count,
                last_seen: Some(event.last_at),
                ..Signals::default()
            };
            let (tier, title, what, note) = match event.kind {
                EngineKind::Panic => (
                    if now - event.last_at < 3600 {
                        Tier::Critical
                    } else {
                        Tier::Warning
                    },
                    "A handler panicked",
                    "the request got a 500 and the worker survived; the backtrace is on stderr",
                    if event.count == 1 { "panic" } else { "panics" },
                ),
                EngineKind::Refused => (
                    Tier::Warning,
                    "A handler runs on the slower engine",
                    "refused by the VM, it still answers on the interpreter",
                    if event.count == 1 {
                        "worker"
                    } else {
                        "workers"
                    },
                ),
                EngineKind::Divergence => (
                    Tier::Warning,
                    "The two engines disagree on a handler",
                    "failed on the VM, then succeeded on the interpreter: a VM bug worth reporting",
                    if event.count == 1 { "time" } else { "times" },
                ),
            };
            let detail = if event.detail.is_empty() {
                event.subject.clone()
            } else {
                format!("{} \u{b7} {}", event.subject, event.detail)
            };
            let since = chrono::DateTime::from_timestamp(event.first_at, 0)
                .map(|t| t.format("%H:%M UTC").to_string())
                .unwrap_or_default();
            Item {
                tier,
                source: Source::Engine,
                title: title.to_string(),
                detail,
                why: format!(
                    "Since {since} \u{b7} last {} \u{b7} {what}",
                    ago(now - event.last_at)
                ),
                metric: thousands(event.count),
                metric_note: note.to_string(),
                action: Action::None,
                score: score(&signals, now),
                last_seen: event.last_at,
            }
        })
        .collect()
}

// --- traffic (from the usage recorder) ------------------------------------------------

/// Over the last hour of requests:
///
/// - **Critical**: 5xx on 5 % or more of requests, at least 10 of them.
/// - **Warning**: 5xx on 1 % or more, at least 5 of them; a route with 50 or
///   more requests whose p95 is 2 s or more.
pub(crate) fn traffic_items(hour: &UsageView, errors_page: bool, now: i64) -> Vec<Item> {
    let mut items = Vec::new();
    let total = hour.current.requests();
    let errors = hour.current.server_errors();
    let rate = hour.current.error_rate();
    if (errors >= 10 && rate >= 0.05) || (errors >= 5 && rate >= 0.01) {
        let worst = hour
            .routes
            .iter()
            .filter(|r| r.window.server_errors() > 0)
            .max_by(|a, b| {
                a.window
                    .server_errors()
                    .cmp(&b.window.server_errors())
                    .then_with(|| b.label.cmp(&a.label))
            });
        let detail = match worst {
            Some(row) => format!(
                "{} \u{b7} {} of {}",
                row.label,
                plural(row.window.server_errors(), "5xx", "5xx"),
                short_count(row.window.requests())
            ),
            None => String::new(),
        };
        let critical = errors >= 10 && rate >= 0.05;
        let signals = Signals {
            last_hour: errors,
            last_24h: errors,
            last_seen: Some(now),
            ..Signals::default()
        };
        items.push(Item {
            tier: if critical {
                Tier::Critical
            } else {
                Tier::Warning
            },
            source: Source::Traffic,
            title: format!("Server errors on {} of requests", fmt_rate(rate)),
            detail,
            why: format!(
                "{} of {} requests in the last hour answered 5xx",
                thousands(errors),
                thousands(total)
            ),
            metric: fmt_rate(rate),
            metric_note: "5xx".to_string(),
            action: if errors_page {
                Action::Link {
                    href: "/__soli/errors".to_string(),
                    label: "Open errors",
                }
            } else {
                Action::None
            },
            score: score(&signals, now),
            last_seen: now,
        });
    }
    for row in &hour.routes {
        let Some(p95) = row.window.p95_us() else {
            continue;
        };
        if row.window.requests() < 50 || p95 < 2_000_000 {
            continue;
        }
        items.push(Item {
            tier: Tier::Warning,
            source: Source::Traffic,
            title: "A route is slow".to_string(),
            detail: row.label.clone(),
            why: format!(
                "p95 {} over {} requests in the last hour \u{b7} p50 {}",
                fmt_us(p95),
                thousands(row.window.requests()),
                row.window.p50_us().map(fmt_us).unwrap_or_default()
            ),
            metric: fmt_us(p95),
            metric_note: "p95".to_string(),
            action: Action::None,
            score: score(
                &Signals {
                    last_hour: row.window.requests(),
                    last_24h: row.window.requests(),
                    last_seen: Some(now),
                    ..Signals::default()
                },
                now,
            ),
            last_seen: now,
        });
    }
    items
}

/// `0.42%`, `12%`, `<0.01%`.
pub(crate) fn fmt_rate(rate: f64) -> String {
    let pct = rate * 100.0;
    if pct <= 0.0 {
        "0%".to_string()
    } else if pct < 0.01 {
        "<0.01%".to_string()
    } else if pct < 1.0 {
        format!("{pct:.2}%")
    } else if pct < 10.0 {
        format!("{pct:.1}%")
    } else {
        format!("{}%", pct.round() as u64)
    }
}

// --- the trackers themselves ------------------------------------------------------------

/// A tracker that lost or failed to write what it recorded, since the process
/// started: **Warning**.
pub(crate) fn recording_item(
    what: &str,
    page: &str,
    stats: &StatsSnapshot,
    now: i64,
) -> Option<Item> {
    let lost = stats.dropped + stats.lost;
    if lost == 0 && stats.failed == 0 {
        return None;
    }
    let mut parts = Vec::new();
    if stats.dropped > 0 {
        parts.push(format!("{} dropped (queue full)", thousands(stats.dropped)));
    }
    if stats.lost > 0 {
        parts.push(format!("{} lost", thousands(stats.lost)));
    }
    if stats.failed > 0 {
        parts.push(format!("{} failed writes", thousands(stats.failed)));
    }
    Some(Item {
        tier: Tier::Warning,
        source: Source::Recording,
        title: format!("Not every {what} was stored"),
        detail: parts.join(" \u{b7} "),
        why: "Since this process started \u{b7} the stderr lines say why".to_string(),
        metric: thousands(lost + stats.failed),
        metric_note: "missed".to_string(),
        action: if page.is_empty() {
            Action::None
        } else {
            Action::Link {
                href: page.to_string(),
                label: "Open page",
            }
        },
        score: 0.0,
        last_seen: now,
    })
}

/// A store a source could not be read from: **Warning**, with the error.
pub(crate) fn unreadable_item(what: &str, page: &str, error: &str, now: i64) -> Item {
    Item {
        tier: Tier::Warning,
        source: Source::Recording,
        title: format!("Could not read the {what}"),
        detail: cut(error, 300),
        why: "Its tile and rows are missing from this page until it answers".to_string(),
        metric: "\u{2013}".to_string(),
        metric_note: "unread".to_string(),
        action: Action::Link {
            href: page.to_string(),
            label: "Open page",
        },
        score: 0.0,
        last_seen: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 2025-11-06 14:30:00 UTC.
    const NOW: i64 = 1_762_439_400;

    fn iso(unix: i64) -> String {
        chrono::DateTime::from_timestamp(unix, 0)
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_default()
    }

    fn item(tier: Tier, score: f64, last_seen: i64, title: &str) -> Item {
        Item {
            tier,
            source: Source::Error,
            title: title.to_string(),
            detail: String::new(),
            why: String::new(),
            metric: String::new(),
            metric_note: String::new(),
            action: Action::None,
            score,
            last_seen,
        }
    }

    #[test]
    fn tiers_come_first_then_score() {
        let mut items = vec![
            item(Tier::Info, 99.0, 9, "i"),
            item(Tier::Warning, 10.0, 9, "w-low"),
            item(Tier::Critical, 1.0, 9, "c"),
            item(Tier::Warning, 50.0, 9, "w-high"),
        ];
        rank(&mut items);
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["c", "w-high", "w-low", "i"]);
    }

    #[test]
    fn ties_go_to_the_most_recent_then_the_title() {
        let mut items = vec![
            item(Tier::Warning, 5.0, 100, "b"),
            item(Tier::Warning, 5.0, 100, "a"),
            item(Tier::Warning, 5.0, 200, "z"),
        ];
        rank(&mut items);
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(titles, ["z", "a", "b"]);
        let mut empty: Vec<Item> = Vec::new();
        rank(&mut empty);
        assert!(empty.is_empty());
    }

    #[test]
    fn the_score_weighs_recency_growth_and_volume() {
        let quiet = Signals {
            last_hour: 0,
            previous_hours: 0,
            last_24h: 0,
            last_seen: None,
        };
        assert_eq!(score(&quiet, NOW), 0.0);
        let burst = Signals {
            last_hour: 9_999,
            previous_hours: 0,
            last_24h: 9_999,
            last_seen: Some(NOW),
        };
        assert!((score(&burst, NOW) - 100.0).abs() < 0.01);
        // Same volume, same recency: the one growing ranks higher.
        let growing = Signals {
            last_hour: 50,
            previous_hours: 50,
            last_24h: 100,
            last_seen: Some(NOW - 60),
        };
        let steady = Signals {
            last_hour: 4,
            previous_hours: 96,
            last_24h: 100,
            last_seen: Some(NOW - 60),
        };
        assert!(score(&growing, NOW) > score(&steady, NOW));
        assert_eq!(steady.growth(), 0.0, "{}", steady.growth());
        // Older is lower, all else equal.
        let old = Signals {
            last_seen: Some(NOW - 10 * 3600),
            ..growing
        };
        assert!(score(&growing, NOW) > score(&old, NOW));
    }

    fn hourly(pairs: &[(i64, u64)]) -> serde_json::Value {
        json!(pairs
            .iter()
            .map(|(back, n)| json!([hour_label(NOW - back * 3600), n]))
            .collect::<Vec<_>>())
    }

    fn error_row(
        key: &str,
        first_back: i64,
        last_back: i64,
        hours: &[(i64, u64)],
    ) -> serde_json::Value {
        json!({
            "_key": key,
            "message": "NoMethodError: undefined method 'total' for nil",
            "location": "app/controllers/orders_controller.sl:42",
            "status": "open",
            "count": hours.iter().map(|(_, n)| n).sum::<u64>() + 5,
            "first_seen": iso(NOW - first_back),
            "last_seen": iso(NOW - last_back),
            "hourly": hourly(hours),
        })
    }

    #[test]
    fn error_tiers() {
        let rows = vec![
            error_row("aaaaaaaaaaaaaaaa", 600, 30, &[(0, 3)]), // new
            error_row("bbbbbbbbbbbbbbbb", 5 * 86_400, 60, &[(0, 40), (5, 2)]), // rising
            error_row("cccccccccccccccc", 5 * 86_400, 7_200, &[(2, 3), (3, 3)]), // recurring
            error_row("dddddddddddddddd", 9 * 86_400, 3 * 86_400, &[]), // quiet
            error_row("not-a-key", 60, 60, &[(0, 99)]),        // skipped
        ];
        let (mut items, summary) = error_items(&rows, 200, NOW);
        assert_eq!(summary.open, 5);
        assert_eq!(summary.new_last_hour, 1);
        assert!(!summary.capped);
        assert_eq!(items.len(), 4);
        rank(&mut items);
        let tiers: Vec<Tier> = items.iter().map(|i| i.tier).collect();
        assert_eq!(
            tiers,
            [Tier::Critical, Tier::Critical, Tier::Warning, Tier::Info]
        );
        assert!(items.iter().any(|i| i
            .title
            .starts_with("New error in app/controllers/orders_controller.sl")));
        assert!(items.iter().any(|i| i.title.starts_with("Error is rising")));
        assert_eq!(
            items[0].action,
            Action::Link {
                href: format!(
                    "/__soli/errors/{}",
                    if items[0].title.starts_with("New") {
                        "aaaaaaaaaaaaaaaa"
                    } else {
                        "bbbbbbbbbbbbbbbb"
                    }
                ),
                label: "Open error"
            }
        );
        let regressed = json!({
            "_key": "eeeeeeeeeeeeeeee", "message": "boom", "location": "",
            "count": 9, "first_seen": iso(NOW - 9 * 86_400), "last_seen": iso(NOW - 120),
            "regressed_at": iso(NOW - 600), "hourly": hourly(&[(0, 1)]),
        });
        let (items, _) = error_items(&[regressed], 200, NOW);
        assert_eq!(items[0].tier, Tier::Critical);
        assert_eq!(items[0].title, "Error came back after it was resolved");
    }

    #[test]
    fn hourly_signals_spread_the_last_hour_over_two_buckets() {
        // Half way through the hour: the current bucket plus half the one before.
        let map =
            super::super::error_tracker::parse_hourly(Some(&hourly(&[(0, 10), (1, 20), (5, 7)])));
        let s = hourly_signals(&map, NOW, Some(NOW));
        assert_eq!(s.last_hour, 20);
        assert_eq!(s.last_24h, 37);
        assert_eq!(s.previous_hours, 17);
    }

    #[test]
    fn job_tiers_and_actions() {
        let rows = vec![
            json!({"_key": "j-1", "state": "dead", "handler": "InvoiceMailerJob", "queue": "default",
                   "finished_at": iso(NOW - 600), "run_at": iso(NOW - 900), "attempts": 6, "max_retries": 5,
                   "last_error": "SMTP connection refused"}),
            json!({"_key": "j-2", "state": "dead", "handler": "ReportJob", "finished_at": iso(NOW - 5 * 3600)}),
            json!({"_key": "j-3", "state": "dead", "handler": "ReportJob", "finished_at": iso(NOW - 6 * 3600)}),
            json!({"_key": "j-4", "state": "dead", "handler": "OldJob", "finished_at": iso(NOW - 3 * 86_400)}),
            json!({"_key": "j-5", "state": "pending", "queue": "default", "run_at": iso(NOW - 2_400)}),
            json!({"_key": "j-6", "state": "scheduled", "queue": "default", "run_at": iso(NOW + 600)}),
            json!({"_key": "j-7", "state": "running", "locked_until": iso(NOW - 600)}),
            json!({"_key": "j-8", "state": "failed", "handler": "SyncJob", "run_at": iso(NOW + 120),
                   "attempts": 2, "max_retries": 5, "last_error": "timeout"}),
        ];
        let (mut items, summary) = job_items(&rows, 500, NOW);
        assert_eq!(summary.dead_24h, 3, "the 3-day-old one is history");
        assert_eq!(summary.queued, 1, "a future run is not queued");
        assert_eq!(summary.oldest_wait, Some(2_400));
        rank(&mut items);
        // Dead within the hour and a 40-minute wait are critical.
        assert_eq!(items[0].tier, Tier::Critical);
        assert_eq!(items[1].tier, Tier::Critical);
        let dead_one = items
            .iter()
            .find(|i| i.title.contains("InvoiceMailerJob"))
            .expect("the mailer group");
        assert_eq!(
            dead_one.action,
            Action::Post {
                href: "/__soli/jobs/j-1/retry".to_string(),
                label: "Retry"
            }
        );
        let reports = items
            .iter()
            .find(|i| i.title.contains("ReportJob"))
            .expect("the report group");
        assert_eq!(reports.tier, Tier::Warning);
        assert!(matches!(reports.action, Action::Link { .. }));
        assert!(items.iter().any(|i| i.metric_note == "stuck"));
        assert!(items.iter().any(|i| i.metric_note == "failing"));
        assert!(!items.iter().any(|i| i.title.contains("OldJob")));
        assert_eq!(job_items(&[], 500, NOW).0, Vec::new());
    }

    #[test]
    fn spam_spikes_are_warnings_and_quiet_days_nothing() {
        let mut rows: Vec<serde_json::Value> = (0..30)
            .map(|i| json!({"created_at": iso(NOW - 60 * i), "path": "/posts/42/comments", "ip": format!("10.0.0.{}", i % 3), "reason": "honeypot"}))
            .collect();
        let (items, summary) = spam_items(&rows, NOW);
        assert_eq!(summary.last_hour, 30);
        assert_eq!(items[0].tier, Tier::Warning);
        assert!(
            items[0].detail.starts_with("/posts/:id/comments"),
            "{}",
            items[0].detail
        );
        assert!(items[0].detail.contains("3 source IPs"));
        rows.truncate(2);
        assert_eq!(spam_items(&rows, NOW).0[0].tier, Tier::Info);
        let old = vec![json!({"created_at": iso(NOW - 2 * 86_400), "path": "/x"})];
        assert!(spam_items(&old, NOW).0.is_empty());
    }

    #[test]
    fn slow_queries_keep_the_day_and_cap_the_rows() {
        let row = |key: String, first_back: i64, last_back: i64, mean: f64| {
            json!({"_key": key, "query": "FOR doc IN posts FILTER doc.author_id == @id RETURN doc",
                   "count": 10, "total_ms": mean * 10.0, "max_ms": mean * 2.0,
                   "first_seen": iso(NOW - first_back), "last_seen": iso(NOW - last_back),
                   "last_context": "GET /posts/7 \u{2192} posts#index", "hourly": hourly(&[(0, 4)])})
        };
        let mut rows: Vec<serde_json::Value> = (0..8)
            .map(|i| row(format!("{:016x}", i + 1), 5 * 86_400, 60, 250.0))
            .collect();
        rows.push(row("ffffffffffffffff".into(), 600, 60, 300.0)); // new
        rows.push(row(
            "eeeeeeeeeeeeeeee".into(),
            9 * 86_400,
            3 * 86_400,
            900.0,
        )); // stale
        let (items, summary) = slow_query_items(&rows, 200, NOW);
        assert_eq!(summary.seen_24h, 9);
        assert_eq!(summary.new_24h, 1);
        assert_eq!(items.len(), MAX_QUERY_ITEMS);
        assert_eq!(items[0].title, "New slow query in posts#index");
        assert_eq!(items[0].tier, Tier::Warning);
        assert!(
            !items.iter().any(|i| i.why.contains("/posts/7")),
            "no raw path"
        );
    }

    #[test]
    fn engine_events_become_rows() {
        let event = |kind, last_at| EngineEvent {
            tenant: crate::serve::tenant::TenantId::PRIMARY,
            kind,
            subject: "reports#export".to_string(),
            detail: "unsupported opcode".to_string(),
            count: 2,
            first_at: NOW - 7_200,
            last_at,
        };
        let items = engine_items(
            &[
                event(EngineKind::Panic, NOW - 60),
                event(EngineKind::Panic, NOW - 7_200),
                event(EngineKind::Refused, NOW - 60),
            ],
            NOW,
        );
        assert_eq!(items[0].tier, Tier::Critical);
        assert_eq!(items[1].tier, Tier::Warning);
        assert_eq!(items[2].title, "A handler runs on the slower engine");
        assert_eq!(items[2].detail, "reports#export \u{b7} unsupported opcode");
    }

    #[test]
    fn helpers() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(182_430), "182,430");
        assert_eq!(thousands(1_000_000), "1,000,000");
        assert_eq!(fmt_ms(0.42), "0.4 ms");
        assert_eq!(fmt_ms(184.2), "184 ms");
        assert_eq!(fmt_ms(1_840.0), "1.8 s");
        assert_eq!(fmt_rate(0.0042), "0.42%");
        assert_eq!(fmt_rate(0.0), "0%");
        assert_eq!(fmt_rate(0.000_01), "<0.01%");
        assert_eq!(
            pattern_like("/posts/42/comments?x=1"),
            "/posts/:id/comments"
        );
        assert_eq!(
            pattern_like("/u/550e8400-e29b-41d4-a716-446655440000"),
            "/u/:id"
        );
        assert_eq!(pattern_like("/users/sign_up"), "/users/sign_up");
        assert_eq!(pattern_like("/deadbeef"), "/deadbeef", "no digit, a word");
        assert_eq!(file_of("app/x.sl:42"), "app/x.sl");
        assert_eq!(file_of("weird"), "weird");
        assert_eq!(ago(30), "just now");
        assert_eq!(ago(38 * 60), "38 min ago");
        assert_eq!(span(9 * 60), "9 min");
    }

    #[test]
    fn traffic_rows_follow_the_5xx_rate() {
        use super::super::usage::{Range, RouteRow, UsageView, Window};
        let view = |ok: u64, bad: u64| {
            let window = Window::default().with(200, 2_000, ok).with(500, 2_000, bad);
            UsageView {
                range: Range::Hour,
                now: NOW as u64,
                bars: Vec::new(),
                current: window,
                previous: None,
                routes: vec![RouteRow {
                    label: "GET /orders/:id".to_string(),
                    window,
                }],
            }
        };
        assert!(traffic_items(&view(1_000, 0), true, NOW).is_empty());
        let warn = traffic_items(&view(990, 10), true, NOW);
        assert_eq!(warn[0].tier, Tier::Warning);
        assert_eq!(warn[0].detail, "GET /orders/:id \u{b7} 10 5xx of 1.0k");
        let crit = traffic_items(&view(900, 100), false, NOW);
        assert_eq!(crit[0].tier, Tier::Critical);
        assert_eq!(crit[0].action, Action::None, "no errors page, no link");
        let slow = UsageView {
            routes: vec![RouteRow {
                label: "GET /report".to_string(),
                window: Window::default().with(200, 3_000_000, 60),
            }],
            ..view(60, 0)
        };
        assert!(traffic_items(&slow, true, NOW)
            .iter()
            .any(|i| i.title == "A route is slow"));
    }
}
