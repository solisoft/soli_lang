//! Operator notifications: tell someone when production starts failing or
//! slowing down, without them watching `/__soli/errors`.
//!
//! The trackers raise four events, each from their writer thread, after the
//! group is stored:
//!
//! - `error.new` — an error with a fingerprint never seen before;
//! - `error.regressed` — an error marked resolved has come back;
//! - `error.spike` — one error group reached `SOLI_NOTIFY_SPIKE` occurrences
//!   (default `50/5m`) within the window;
//! - `slow_query.new` — a query shape crossed `SOLI_SLOW_QUERY_MS` for the
//!   first time.
//!
//! Each goes to every configured destination:
//!
//! - `SOLI_NOTIFY_EMAILS` — comma-separated addresses, sent through the app's
//!   own mailer settings (`SOLI_SMTP_*`; in `--dev` the dev inbox).
//! - `SOLI_NOTIFY_WEBHOOKS` — comma-separated URLs. Slack, Microsoft Teams,
//!   Discord and Google Chat URLs are recognised by host and sent the message
//!   shape each expects; any other URL gets the event as JSON, signed with
//!   `X-Soli-Signature` (HMAC-SHA256 of the body) when `SOLI_NOTIFY_SECRET` is
//!   set. URLs go through the same SSRF guard as `Webhook.enqueue`.
//! - `app/jobs/soli_notification_job.sl` — when the app has this job, every
//!   event is also enqueued to `SoliNotificationJob.perform(event)`, for any
//!   destination the first two do not cover (PagerDuty, SMS, a ticket).
//!
//! `SOLI_NOTIFY_EVENTS` narrows which events are sent. One event per group is
//! sent at most once per `SOLI_NOTIFY_THROTTLE` (default `15m`), so a burst of
//! the same failure is one message. Sending happens on a per-application
//! notifier thread: a slow Slack never delays the trackers, and never a
//! request.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::tenant::TenantId;
use super::tenant_writer::{self, Stats, StatsSnapshot, Writers};

/// Events waiting to be sent. A burst beyond this is dropped and counted.
const QUEUE_CAP: usize = 256;

const DEFAULT_THROTTLE: Duration = Duration::from_secs(15 * 60);
const DEFAULT_SPIKE: (u64, Duration) = (50, Duration::from_secs(5 * 60));
const JOB_CLASS: &str = "SoliNotificationJob";
const JOB_FILE: &str = "app/jobs/soli_notification_job.sl";

/// Every event name, in the order the docs list them.
pub(crate) const EVENT_NAMES: [&str; 4] = [
    "error.new",
    "error.regressed",
    "error.spike",
    "slow_query.new",
];

static WRITERS: Writers<Event> = Writers::new("notify", QUEUE_CAP);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Kind {
    ErrorNew,
    ErrorRegressed,
    ErrorSpike,
    SlowQueryNew,
}

impl Kind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::ErrorNew => "error.new",
            Kind::ErrorRegressed => "error.regressed",
            Kind::ErrorSpike => "error.spike",
            Kind::SlowQueryNew => "slow_query.new",
        }
    }

    fn headline(self) -> &'static str {
        match self {
            Kind::ErrorNew => "New error",
            Kind::ErrorRegressed => "Error is back",
            Kind::ErrorSpike => "Error spike",
            Kind::SlowQueryNew => "New slow query",
        }
    }
}

/// One thing worth telling an operator.
#[derive(Clone, Debug)]
pub(crate) struct Event {
    pub(crate) kind: Kind,
    pub(crate) fingerprint: String,
    /// The error message, or the query shape.
    pub(crate) summary: String,
    /// Where it was raised, or how slow it was.
    pub(crate) detail: String,
    /// The request or job it came from.
    pub(crate) context: String,
    pub(crate) count: Option<u64>,
    pub(crate) at: String,
}

impl Event {
    pub(crate) fn error(
        kind: Kind,
        fingerprint: &str,
        message: &str,
        location: &str,
        request: &str,
        count: Option<u64>,
    ) -> Event {
        Event {
            kind,
            fingerprint: fingerprint.to_string(),
            summary: message.to_string(),
            detail: if location.is_empty() {
                String::new()
            } else {
                format!("raised in {location}")
            },
            context: request.to_string(),
            count,
            at: crate::jobs::now_iso(),
        }
    }

    pub(crate) fn slow_query(fingerprint: &str, shape: &str, max_ms: f64, context: &str) -> Event {
        Event {
            kind: Kind::SlowQueryNew,
            fingerprint: fingerprint.to_string(),
            summary: shape.to_string(),
            detail: format!(
                "took {:.0} ms (threshold {} ms)",
                max_ms,
                super::slow_queries::threshold_ms()
            ),
            context: context.to_string(),
            count: None,
            at: crate::jobs::now_iso(),
        }
    }

    /// The dashboard page this event is about.
    fn page_path(&self) -> String {
        match self.kind {
            Kind::SlowQueryNew => format!("/__soli/slow_queries/{}", self.fingerprint),
            _ => format!("/__soli/errors/{}", self.fingerprint),
        }
    }

    fn title(&self, app: &str) -> String {
        let mut summary: String = self
            .summary
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(120)
            .collect();
        if summary.len() < self.summary.len() {
            summary.push('\u{2026}');
        }
        format!("[{app}] {}: {summary}", self.kind.headline())
    }

    fn to_json(&self, app: &str, url: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "event": self.kind.name(),
            "app": app,
            "fingerprint": self.fingerprint,
            "summary": self.summary,
            "detail": self.detail,
            "context": self.context,
            "count": self.count,
            "at": self.at,
            "url": url,
        })
    }
}

/// This application's notifier counters, since the process started.
pub(crate) fn stats() -> StatsSnapshot {
    WRITERS.stats(super::tenant::current_id())
}

/// Queue an event for the application's notifier. Cheap and silent when
/// nothing is configured to receive it.
pub(crate) fn emit(event: Event) {
    let config = Config::from_env();
    if !config.wants(event.kind) || !config.has_destination() {
        return;
    }
    WRITERS.deliver(super::tenant::current_id(), event, &spawn_notifier);
}

fn spawn_notifier(tenant: TenantId, receiver: Receiver<Event>, stats: Arc<Stats>) -> bool {
    tenant_writer::spawn_thread("notify", tenant, move || {
        let mut throttle = Throttle::default();
        while let Ok(event) = receiver.recv() {
            let config = Config::from_env();
            if !throttle.allow(&event, config.throttle, Instant::now()) {
                continue;
            }
            for failure in send(&event, &config) {
                stats.failed.fetch_add(1, Ordering::Relaxed);
                eprintln!(
                    "[notify] {} {}: {failure}",
                    event.kind.name(),
                    event.fingerprint
                );
            }
        }
    })
}

/// At most one message per event kind and group per window.
#[derive(Default)]
struct Throttle {
    last: HashMap<(Kind, String), Instant>,
}

impl Throttle {
    fn allow(&mut self, event: &Event, window: Duration, now: Instant) -> bool {
        let key = (event.kind, event.fingerprint.clone());
        if let Some(at) = self.last.get(&key) {
            if now.duration_since(*at) < window {
                return false;
            }
        }
        // Forget what is out of every window, so the map stays small.
        self.last.retain(|_, at| now.duration_since(*at) < window);
        self.last.insert(key, now);
        true
    }
}

/// What to send and where, read from the environment each time: the notifier
/// is quiet most of the time, and an operator changing `.env` should not need
/// to know which values were cached.
struct Config {
    emails: Vec<String>,
    webhooks: Vec<String>,
    job: bool,
    events: Vec<String>,
    throttle: Duration,
    from: Option<String>,
    secret: Option<String>,
    base_url: Option<String>,
    app: String,
}

impl Config {
    fn from_env() -> Config {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let list = |k: &str| -> Vec<String> {
            env(k)
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };
        let root = super::tenant::app_root();
        let events = list("SOLI_NOTIFY_EVENTS");
        Config {
            emails: list("SOLI_NOTIFY_EMAILS"),
            webhooks: list("SOLI_NOTIFY_WEBHOOKS"),
            job: root.join(JOB_FILE).is_file(),
            events: if events.is_empty() {
                EVENT_NAMES.iter().map(|s| s.to_string()).collect()
            } else {
                events
            },
            throttle: env("SOLI_NOTIFY_THROTTLE")
                .and_then(|v| parse_duration(&v))
                .unwrap_or(DEFAULT_THROTTLE),
            from: env("SOLI_NOTIFY_FROM"),
            secret: env("SOLI_NOTIFY_SECRET"),
            base_url: env("SOLI_NOTIFY_URL")
                .or_else(|| {
                    env("SOLI_APP_HOSTS")
                        .and_then(|hosts| hosts.split(',').next().map(|h| h.trim().to_string()))
                        .filter(|h| !h.is_empty())
                        .map(|h| format!("https://{h}"))
                })
                .map(|u| u.trim_end_matches('/').to_string()),
            app: env("SOLI_NOTIFY_APP_NAME").unwrap_or_else(|| {
                root.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "soli".to_string())
            }),
        }
    }

    fn wants(&self, kind: Kind) -> bool {
        self.events.iter().any(|e| e == kind.name())
    }

    fn has_destination(&self) -> bool {
        !self.emails.is_empty() || !self.webhooks.is_empty() || self.job
    }
}

/// Where events go, one short name per destination — never a URL, which
/// carries the webhook's secret.
fn destinations(config: &Config) -> Vec<String> {
    let mut out: Vec<String> = config
        .webhooks
        .iter()
        .map(|hook| match hook_kind(hook) {
            HookKind::Slack => "Slack".to_string(),
            HookKind::Teams => "Teams".to_string(),
            HookKind::Discord => "Discord".to_string(),
            HookKind::GoogleChat => "Google Chat".to_string(),
            HookKind::Generic => "webhook".to_string(),
        })
        .collect();
    match config.emails.len() {
        0 => {}
        1 => out.push("1 email".to_string()),
        n => out.push(format!("{n} emails")),
    }
    if config.job {
        out.push(JOB_CLASS.to_string());
    }
    out
}

/// One line for the operator pages: where notifications go, or how to turn
/// them on, and whether sending has been failing.
pub(crate) fn status_html() -> String {
    let config = Config::from_env();
    let destinations = destinations(&config);
    let mut out = if destinations.is_empty() {
        "<p class=\"summary\">Notifications are off \u{2014} set <code>SOLI_NOTIFY_WEBHOOKS</code> \
(Slack, Teams, Discord, Google Chat or any URL) or <code>SOLI_NOTIFY_EMAILS</code>.</p>"
            .to_string()
    } else {
        format!(
            "<p class=\"summary\">Notifications: {} \u{b7} {}</p>",
            super::dev_bar::html_escape(&destinations.join(", ")),
            super::dev_bar::html_escape(&config.events.join(", "))
        )
    };
    let failed = stats().failed;
    if failed > 0 {
        out.push_str(&format!(
            "<p class=\"notice bad\">{failed} notification(s) could not be sent since this process \
started (see the <code>[notify]</code> lines on stderr).</p>"
        ));
    }
    out
}

/// The spike rule: `count` occurrences within `window`. `None` when off.
pub(crate) fn spike_rule() -> Option<(u64, Duration)> {
    let Ok(value) = std::env::var("SOLI_NOTIFY_SPIKE") else {
        return Some(DEFAULT_SPIKE);
    };
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        return Some(DEFAULT_SPIKE);
    }
    if matches!(value.as_str(), "off" | "0" | "false" | "no") {
        return None;
    }
    let (count, window) = value.split_once('/')?;
    let count = count.trim().parse::<u64>().ok().filter(|n| *n > 0)?;
    Some((count, parse_duration(window)?))
}

/// `90`, `90s`, `15m`, `2h` → a duration.
pub(crate) fn parse_duration(value: &str) -> Option<Duration> {
    let value = value.trim();
    let (number, unit) = match value.find(|c: char| !c.is_ascii_digit()) {
        Some(i) => value.split_at(i),
        None => (value, "s"),
    };
    let n: u64 = number.parse().ok()?;
    let secs = match unit.trim() {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}

/// Occurrences per error group over the spike window, kept by the error
/// tracker's writer between flushes.
#[derive(Default)]
pub(crate) struct SpikeWatch {
    recent: HashMap<String, VecDeque<(Instant, u64)>>,
}

impl SpikeWatch {
    /// Add `count` occurrences of `fingerprint` seen at `now`. `Some(total)`
    /// when the window now holds at least the rule's count.
    pub(crate) fn add(
        &mut self,
        fingerprint: &str,
        count: u64,
        now: Instant,
        rule: (u64, Duration),
    ) -> Option<u64> {
        let (threshold, window) = rule;
        let seen = self.recent.entry(fingerprint.to_string()).or_default();
        seen.push_back((now, count));
        while seen
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > window)
        {
            seen.pop_front();
        }
        let total: u64 = seen.iter().map(|(_, n)| n).sum();
        // Groups that went quiet are dropped, so the map tracks what is live.
        self.recent.retain(|_, s| {
            s.back()
                .is_some_and(|(at, _)| now.duration_since(*at) <= window)
        });
        (total >= threshold).then_some(total)
    }
}

// --- sending -----------------------------------------------------------------

/// Send `event` everywhere it should go. Returns one message per failure.
fn send(event: &Event, config: &Config) -> Vec<String> {
    let url = config
        .base_url
        .as_ref()
        .map(|base| format!("{base}{}", event.page_path()));
    let mut failures = Vec::new();
    if !config.emails.is_empty() {
        let (subject, text, html) = email_parts(event, &config.app, url.as_deref());
        if let Err(e) = crate::interpreter::builtins::mailer::send_operator_mail(
            &config.emails,
            config.from.as_deref(),
            &subject,
            &text,
            &html,
        ) {
            failures.push(format!("email: {e}"));
        }
    }
    for hook in &config.webhooks {
        if let Err(e) = post_webhook(hook, event, config, url.as_deref()) {
            failures.push(format!(
                "webhook {}: {e}",
                crate::redaction::redact_url_query(hook)
            ));
        }
    }
    if config.job {
        let doc = crate::jobs::JobDoc::new(
            JOB_CLASS,
            event.to_json(&config.app, url.as_deref()),
            &crate::jobs::config().default_queue,
            crate::jobs::now_iso(),
        );
        if let Err(e) = crate::jobs::store::enqueue(&doc) {
            failures.push(format!("{JOB_CLASS}: {e}"));
        }
    }
    failures
}

/// The plain lines every text format shares.
fn text_lines(event: &Event) -> Vec<String> {
    let mut lines = vec![event.summary.clone()];
    if !event.detail.is_empty() {
        lines.push(event.detail.clone());
    }
    if !event.context.is_empty() {
        lines.push(format!("from {}", event.context));
    }
    if let Some(count) = event.count {
        lines.push(format!("{count} occurrence(s)"));
    }
    lines
}

fn email_parts(event: &Event, app: &str, url: Option<&str>) -> (String, String, String) {
    let subject = event.title(app);
    let mut text = text_lines(event).join("\n");
    if let Some(url) = url {
        text.push_str(&format!("\n\n{url}\n"));
    }
    let esc = super::dev_bar::html_escape;
    let mut html = format!(
        "<p><b>{}</b></p><pre style=\"white-space:pre-wrap;font-family:ui-monospace,monospace\">{}</pre>",
        esc(&format!("{} in {app}", event.kind.headline())),
        esc(&event.summary)
    );
    for line in text_lines(event).iter().skip(1) {
        html.push_str(&format!("<p>{}</p>", esc(line)));
    }
    if let Some(url) = url {
        html.push_str(&format!("<p><a href=\"{0}\">{0}</a></p>", esc(url)));
    }
    (subject, text, html)
}

/// Which chat product a webhook URL belongs to, by its host.
#[derive(Debug, PartialEq, Eq)]
enum HookKind {
    Slack,
    Teams,
    Discord,
    GoogleChat,
    Generic,
}

fn hook_kind(url: &str) -> HookKind {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or("")
        .split(['/', ':', '?'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if host == "hooks.slack.com" {
        HookKind::Slack
    } else if host.ends_with(".webhook.office.com")
        || host.ends_with(".logic.azure.com")
        || host.ends_with(".powerplatform.com")
    {
        HookKind::Teams
    } else if host == "discord.com" || host == "discordapp.com" {
        HookKind::Discord
    } else if host == "chat.googleapis.com" {
        HookKind::GoogleChat
    } else {
        HookKind::Generic
    }
}

/// The request body a webhook receives, in the shape its product expects.
fn webhook_body(kind: &HookKind, event: &Event, app: &str, url: Option<&str>) -> serde_json::Value {
    let title = event.title(app);
    let lines = text_lines(event);
    match kind {
        HookKind::Slack => {
            let mut text = format!("*{title}*\n```{}```", event.summary);
            for line in lines.iter().skip(1) {
                text.push_str(&format!("\n{line}"));
            }
            if let Some(url) = url {
                text.push_str(&format!("\n<{url}|Open in Soli>"));
            }
            serde_json::json!({ "text": text })
        }
        HookKind::Teams => {
            let mut body = vec![
                serde_json::json!({"type": "TextBlock", "text": title, "weight": "Bolder", "wrap": true}),
                serde_json::json!({"type": "TextBlock", "text": event.summary, "fontType": "Monospace", "wrap": true}),
            ];
            for line in lines.iter().skip(1) {
                body.push(serde_json::json!({"type": "TextBlock", "text": line, "isSubtle": true, "wrap": true}));
            }
            let actions: Vec<serde_json::Value> = url
                .map(|u| serde_json::json!({"type": "Action.OpenUrl", "title": "Open in Soli", "url": u}))
                .into_iter()
                .collect();
            serde_json::json!({
                "type": "message",
                "attachments": [{
                    "contentType": "application/vnd.microsoft.card.adaptive",
                    "content": {
                        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
                        "type": "AdaptiveCard",
                        "version": "1.4",
                        "body": body,
                        "actions": actions,
                    }
                }]
            })
        }
        HookKind::Discord | HookKind::GoogleChat => {
            let mut text = format!("**{title}**\n```\n{}\n```", event.summary);
            if *kind == HookKind::GoogleChat {
                text = format!("*{title}*\n```\n{}\n```", event.summary);
            }
            for line in lines.iter().skip(1) {
                text.push_str(&format!("\n{line}"));
            }
            if let Some(url) = url {
                text.push_str(&format!("\n{url}"));
            }
            let text: String = text.chars().take(1900).collect();
            if *kind == HookKind::Discord {
                serde_json::json!({ "content": text })
            } else {
                serde_json::json!({ "text": text })
            }
        }
        HookKind::Generic => event.to_json(app, url),
    }
}

fn post_webhook(
    hook: &str,
    event: &Event,
    config: &Config,
    url: Option<&str>,
) -> Result<(), String> {
    crate::jobs::engine::validate_webhook_url(hook)?;
    let kind = hook_kind(hook);
    let body = webhook_body(&kind, event, &config.app, url).to_string();
    let client = crate::interpreter::builtins::http_class::get_user_http_client().clone();
    let mut request = client
        .post(hook)
        .header("Content-Type", "application/json")
        .header("X-Soli-Event", event.kind.name())
        .timeout(Duration::from_secs(10));
    if kind == HookKind::Generic {
        if let Some(secret) = &config.secret {
            let mac = crate::interpreter::builtins::crypto::hmac_sha256_bytes(
                body.as_bytes(),
                secret.as_bytes(),
            );
            let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
            request = request.header("X-Soli-Signature", hex);
        }
    }
    let response = crate::interpreter::builtins::http_class::block_on_db(async move {
        request.body(body).send().await
    })
    .map_err(|e| e.to_string())?;
    let status = response.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: Kind) -> Event {
        Event {
            kind,
            fingerprint: "0123456789abcdef".into(),
            summary: "undefined method 'total' for nil".into(),
            detail: "raised in show at app/controllers/orders_controller.sl".into(),
            context: "GET /orders/7".into(),
            count: Some(3),
            at: "2026-09-25T10:00:00Z".into(),
        }
    }

    #[test]
    fn hosts_pick_the_message_shape() {
        assert_eq!(
            hook_kind("https://hooks.slack.com/services/T/B/x"),
            HookKind::Slack
        );
        assert_eq!(
            hook_kind("https://acme.webhook.office.com/webhookb2/abc"),
            HookKind::Teams
        );
        assert_eq!(
            hook_kind("https://prod-01.westus.logic.azure.com:443/workflows/x"),
            HookKind::Teams
        );
        assert_eq!(
            hook_kind("https://discord.com/api/webhooks/1/x"),
            HookKind::Discord
        );
        assert_eq!(
            hook_kind("https://chat.googleapis.com/v1/spaces/x/messages?key=k"),
            HookKind::GoogleChat
        );
        assert_eq!(hook_kind("https://ops.example.com/hook"), HookKind::Generic);
        // A look-alike path is not the product.
        assert_eq!(
            hook_kind("https://evil.example.com/hooks.slack.com"),
            HookKind::Generic
        );
    }

    #[test]
    fn slack_gets_text_and_a_link() {
        let body = webhook_body(
            &HookKind::Slack,
            &event(Kind::ErrorNew),
            "shop",
            Some("https://shop.example.com/__soli/errors/0123456789abcdef"),
        );
        let text = body["text"].as_str().unwrap();
        assert!(
            text.starts_with("*[shop] New error: undefined method"),
            "{text}"
        );
        assert!(
            text.contains("<https://shop.example.com/__soli/errors/0123456789abcdef|Open in Soli>")
        );
        assert!(text.contains("from GET /orders/7"));
    }

    #[test]
    fn teams_gets_an_adaptive_card() {
        let body = webhook_body(
            &HookKind::Teams,
            &event(Kind::ErrorRegressed),
            "shop",
            Some("https://x/y"),
        );
        let card = &body["attachments"][0];
        assert_eq!(
            card["contentType"],
            "application/vnd.microsoft.card.adaptive"
        );
        assert_eq!(card["content"]["type"], "AdaptiveCard");
        assert_eq!(card["content"]["actions"][0]["url"], "https://x/y");
        assert!(card["content"]["body"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Error is back"));
    }

    #[test]
    fn a_generic_hook_gets_the_event() {
        let body = webhook_body(&HookKind::Generic, &event(Kind::ErrorSpike), "shop", None);
        assert_eq!(body["event"], "error.spike");
        assert_eq!(body["app"], "shop");
        assert_eq!(body["count"], 3);
        assert!(body["url"].is_null());
    }

    #[test]
    fn a_repeat_inside_the_window_is_held_back() {
        let mut throttle = Throttle::default();
        let now = Instant::now();
        let window = Duration::from_secs(900);
        assert!(throttle.allow(&event(Kind::ErrorSpike), window, now));
        assert!(!throttle.allow(
            &event(Kind::ErrorSpike),
            window,
            now + Duration::from_secs(60)
        ));
        // Another kind for the same group is its own message.
        assert!(throttle.allow(&event(Kind::ErrorRegressed), window, now));
        assert!(throttle.allow(
            &event(Kind::ErrorSpike),
            window,
            now + Duration::from_secs(901)
        ));
    }

    #[test]
    fn a_spike_is_a_count_within_the_window() {
        let rule = (10, Duration::from_secs(300));
        let mut watch = SpikeWatch::default();
        let t0 = Instant::now();
        assert_eq!(watch.add("a", 4, t0, rule), None);
        assert_eq!(watch.add("a", 5, t0 + Duration::from_secs(60), rule), None);
        assert_eq!(
            watch.add("a", 1, t0 + Duration::from_secs(120), rule),
            Some(10)
        );
        // The first four fall out of the window.
        assert_eq!(watch.add("a", 1, t0 + Duration::from_secs(320), rule), None);
        assert_eq!(watch.add("b", 10, t0, rule), Some(10));
    }

    #[test]
    fn durations_and_rules_parse() {
        assert_eq!(parse_duration("90"), Some(Duration::from_secs(90)));
        assert_eq!(parse_duration("15m"), Some(Duration::from_secs(900)));
        assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7200)));
        assert_eq!(parse_duration("soon"), None);
    }

    #[test]
    fn email_carries_the_link_and_escapes_the_message() {
        let mut e = event(Kind::ErrorNew);
        e.summary = "<script>".into();
        let (subject, text, html) = email_parts(&e, "shop", Some("https://x/y"));
        assert_eq!(subject, "[shop] New error: <script>");
        assert!(text.ends_with("https://x/y\n"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
    }
}
