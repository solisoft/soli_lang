//! Web Push for the operator pages: `/__soli/errors` and
//! `/__soli/slow_queries` notify a subscribed browser, page closed or not.
//!
//! A destination of [`super::notify`] like the e-mails and webhooks: the same
//! events (`SOLI_NOTIFY_EVENTS`), the same throttle, sent from the same
//! notifier thread. What differs is who chooses to receive them — an operator
//! presses "Notify this device" on the page, per topic:
//!
//! - `errors` — `error.new`, `error.regressed`, `error.spike`;
//! - `slow_queries` — `slow_query.new`.
//!
//! Subscriptions are stored in the app's own database
//! ([`SUBSCRIPTIONS`], through [`internal_store`]), one document per browser
//! endpoint holding its topics. A push service answering 404 or 410 means the
//! browser dropped it, and the document is deleted.
//!
//! The VAPID identity is `VAPID_PUBLIC_KEY` / `VAPID_PRIVATE_KEY` /
//! `VAPID_SUBJECT` when the app sets them (the names `WebPush` reads), else a
//! pair generated once and kept in [`KEYS`]. Rotating it orphans every
//! subscription: browsers must subscribe again.
//!
//! Routes, under `/__soli/push/`:
//!
//! - `GET sw.js` — the service worker, public (it holds no secret, and a
//!   browser fetches it again on its own to check for updates), served with
//!   `Service-Worker-Allowed: /__soli/`;
//! - `POST subscribe`, `unsubscribe`, `status`, `test` — JSON, behind the
//!   gate of the topic's page (`SOLI_ERRORS_*` or `SOLI_SLOW_QUERIES_*`,
//!   `SOLI_ADMIN_*`) and the Origin/Referer gate of every operator page.
//!
//! [`internal_store`]: super::internal_store

use std::net::SocketAddr;

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};
use hyper::body::Incoming;
use hyper::header::{HeaderName, HeaderValue};
use hyper::{Request, Response, StatusCode};
use sha2::{Digest, Sha256};

use super::internal_store;
use super::notify::{Event, Kind};
use super::{admin_auth, full, ResponseBody};

/// Browser subscriptions, one document per push endpoint.
pub(crate) const SUBSCRIPTIONS: &str = "_soli_push_subscriptions";
/// The generated VAPID key pair, when the app sets none.
const KEYS: &str = "_soli_push_keys";
const KEYS_DOC: &str = "vapid";

const BASE: &str = "/__soli/push";
/// A subscription is a few hundred bytes.
const MAX_BODY: usize = 16 * 1024;
/// Subscriptions read per notification. An operator page has a handful.
const MAX_SUBSCRIPTIONS: usize = 200;

/// What a subscription asked to be told about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Topic {
    Errors,
    SlowQueries,
}

impl Topic {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Topic::Errors => "errors",
            Topic::SlowQueries => "slow_queries",
        }
    }

    fn parse(value: &str) -> Option<Topic> {
        match value {
            "errors" => Some(Topic::Errors),
            "slow_queries" => Some(Topic::SlowQueries),
            _ => None,
        }
    }

    pub(crate) fn of(kind: Kind) -> Topic {
        match kind {
            Kind::SlowQueryNew => Topic::SlowQueries,
            _ => Topic::Errors,
        }
    }

    /// The `admin_auth` scope of the page this topic belongs to.
    fn scope(self) -> &'static str {
        match self {
            Topic::Errors => "ERRORS",
            Topic::SlowQueries => "SLOW_QUERIES",
        }
    }

    fn page(self) -> &'static str {
        match self {
            Topic::Errors => "/__soli/errors",
            Topic::SlowQueries => "/__soli/slow_queries",
        }
    }
}

// --- keys --------------------------------------------------------------------

pub(crate) struct Keys {
    pub(crate) public: String,
    pub(crate) private: String,
    pub(crate) subject: String,
}

/// The VAPID identity: the app's own `VAPID_*`, else the stored pair, else a
/// new pair, stored.
pub(crate) fn keys() -> Result<Keys, String> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let subject = env("VAPID_SUBJECT").unwrap_or_else(|| "mailto:admin@localhost".to_string());
    if let (Some(public), Some(private)) = (env("VAPID_PUBLIC_KEY"), env("VAPID_PRIVATE_KEY")) {
        return Ok(Keys {
            public,
            private,
            subject,
        });
    }
    let stored = |doc: &serde_json::Value| -> Option<Keys> {
        Some(Keys {
            public: doc.get("public_key")?.as_str()?.to_string(),
            private: doc.get("private_key")?.as_str()?.to_string(),
            subject: subject.clone(),
        })
    };
    if let Some(found) = read_keys()?.as_ref().and_then(stored) {
        return Ok(found);
    }
    let (public, private) = crate::interpreter::builtins::vapid::generate_key_pair();
    internal_store::ensure(KEYS, "created_at")?;
    let doc = serde_json::json!({
        "public_key": public,
        "private_key": private,
        "created_at": crate::jobs::now_iso(),
    });
    // Two pages generating at once: the second insert loses, and its caller
    // takes the pair that won.
    if internal_store::insert(KEYS, KEYS_DOC, doc).is_err() {
        if let Some(found) = read_keys()?.as_ref().and_then(stored) {
            return Ok(found);
        }
    }
    Ok(Keys {
        public,
        private,
        subject,
    })
}

fn read_keys() -> Result<Option<serde_json::Value>, String> {
    match internal_store::get(KEYS, KEYS_DOC) {
        Ok(doc) => Ok(doc),
        Err(e) if internal_store::is_missing_collection(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

// --- subscriptions -------------------------------------------------------------

/// The document key for an endpoint: endpoints are long URLs, keys are not.
fn key_for(endpoint: &str) -> String {
    let digest = Sha256::digest(endpoint.as_bytes());
    digest.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// A browser's `PushSubscription.toJSON()`, checked: an https endpoint and
/// both keys. The endpoint is replayed on every notification, so nothing
/// else is accepted (the send path checks it against the SSRF blocklist too).
fn valid_subscription(sub: &serde_json::Value) -> Result<serde_json::Value, String> {
    let endpoint = sub
        .get("endpoint")
        .and_then(|v| v.as_str())
        .filter(|e| e.starts_with("https://") && e.len() <= 2048)
        .ok_or("the subscription needs an https endpoint")?;
    let key = |name: &str| {
        sub.get("keys")
            .and_then(|k| k.get(name))
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty() && v.len() <= 256)
            .map(str::to_string)
    };
    let (Some(p256dh), Some(auth)) = (key("p256dh"), key("auth")) else {
        return Err("the subscription needs its p256dh and auth keys".to_string());
    };
    Ok(serde_json::json!({
        "endpoint": endpoint,
        "keys": {"p256dh": p256dh, "auth": auth},
    }))
}

fn topics_of(doc: &serde_json::Value) -> Vec<String> {
    doc.get("topics")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn read_subscription(endpoint: &str) -> Result<Option<serde_json::Value>, String> {
    match internal_store::get(SUBSCRIPTIONS, &key_for(endpoint)) {
        Ok(doc) => Ok(doc),
        Err(e) if internal_store::is_missing_collection(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Add `topic` to this browser's subscription, creating it when new.
pub(crate) fn subscribe(
    subscription: &serde_json::Value,
    topic: Topic,
    user_agent: &str,
) -> Result<(), String> {
    let sub = valid_subscription(subscription)?;
    let endpoint = sub["endpoint"].as_str().unwrap_or_default().to_string();
    internal_store::ensure(SUBSCRIPTIONS, "created_at")?;
    let key = key_for(&endpoint);
    match read_subscription(&endpoint)? {
        Some(doc) => {
            let mut topics = topics_of(&doc);
            if !topics.iter().any(|t| t == topic.name()) {
                topics.push(topic.name().to_string());
            }
            // The keys come again: a browser can rotate them for one endpoint.
            internal_store::patch(
                SUBSCRIPTIONS,
                &key,
                serde_json::json!({"keys": sub["keys"], "topics": topics}),
            )
        }
        None => internal_store::insert(
            SUBSCRIPTIONS,
            &key,
            serde_json::json!({
                "endpoint": endpoint,
                "keys": sub["keys"],
                "topics": [topic.name()],
                "user_agent": user_agent.chars().take(200).collect::<String>(),
                "created_at": crate::jobs::now_iso(),
            }),
        ),
    }
}

/// Remove `topic` from this browser's subscription; the document goes with
/// its last topic. `true` when the browser was subscribed to it.
pub(crate) fn unsubscribe(endpoint: &str, topic: Topic) -> Result<bool, String> {
    let Some(doc) = read_subscription(endpoint)? else {
        return Ok(false);
    };
    let topics = topics_of(&doc);
    if !topics.iter().any(|t| t == topic.name()) {
        return Ok(false);
    }
    let rest: Vec<String> = topics.into_iter().filter(|t| t != topic.name()).collect();
    let key = key_for(endpoint);
    if rest.is_empty() {
        internal_store::delete(SUBSCRIPTIONS, &key)?;
    } else {
        internal_store::patch(SUBSCRIPTIONS, &key, serde_json::json!({"topics": rest}))?;
    }
    Ok(true)
}

/// Whether this browser is subscribed to `topic`.
pub(crate) fn is_subscribed(endpoint: &str, topic: Topic) -> Result<bool, String> {
    Ok(read_subscription(endpoint)?
        .map(|doc| topics_of(&doc).iter().any(|t| t == topic.name()))
        .unwrap_or(false))
}

/// Every stored subscription. A missing collection is none.
fn all_subscriptions() -> Result<Vec<serde_json::Value>, String> {
    match internal_store::list(
        SUBSCRIPTIONS,
        None,
        "created_at",
        false,
        MAX_SUBSCRIPTIONS,
        &["endpoint", "keys", "topics"],
    ) {
        Ok(rows) => Ok(rows),
        Err(e) if internal_store::is_missing_collection(&e) => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Browsers subscribed to `topic`. Zero, not an error, when the store cannot
/// be read: this only decides whether there is anyone to notify.
pub(crate) fn subscriber_count(topic: Topic) -> usize {
    all_subscriptions()
        .map(|rows| {
            rows.iter()
                .filter(|r| topics_of(r).iter().any(|t| t == topic.name()))
                .count()
        })
        .unwrap_or(0)
}

// --- sending -------------------------------------------------------------------

/// The notification a browser shows. `url` is a path on this app: the service
/// worker opens it on its own origin, so no public base URL is needed.
fn payload(event: &Event, title: &str, body: &str) -> String {
    serde_json::json!({
        "title": title,
        "body": body.chars().take(240).collect::<String>(),
        "url": event_path(event),
        "tag": format!("{}:{}", event.kind.name(), event.fingerprint),
    })
    .to_string()
}

fn event_path(event: &Event) -> String {
    match event.kind {
        Kind::SlowQueryNew => format!("/__soli/slow_queries/{}", event.fingerprint),
        _ => format!("/__soli/errors/{}", event.fingerprint),
    }
}

/// Push `event` to every browser subscribed to its topic. Returns one
/// message per failure; subscriptions the push service no longer knows are
/// deleted, not reported.
pub(crate) fn send(event: &Event, title: &str, body: &str) -> Vec<String> {
    let topic = Topic::of(event.kind);
    let rows = match all_subscriptions() {
        Ok(rows) => rows,
        Err(e) => return vec![format!("web push: {e}")],
    };
    let targets: Vec<&serde_json::Value> = rows
        .iter()
        .filter(|r| topics_of(r).iter().any(|t| t == topic.name()))
        .collect();
    if targets.is_empty() {
        return Vec::new();
    }
    let keys = match keys() {
        Ok(keys) => keys,
        Err(e) => return vec![format!("web push keys: {e}")],
    };
    let message = payload(event, title, body);
    let mut failures = Vec::new();
    for row in targets {
        if let Err(e) = push_to(row, &message, &keys) {
            failures.push(e);
        }
    }
    failures
}

/// What became of one push.
#[derive(Debug, PartialEq, Eq)]
enum Delivery {
    Delivered,
    /// The push service no longer knows the browser (404/410): the
    /// subscription has been deleted.
    Gone(i64),
}

/// One subscription.
fn push_to(row: &serde_json::Value, message: &str, keys: &Keys) -> Result<Delivery, String> {
    let endpoint = row.get("endpoint").and_then(|v| v.as_str()).unwrap_or("");
    let subscription = crate::interpreter::value::json_to_value(serde_json::json!({
        "endpoint": endpoint,
        "keys": row.get("keys").cloned().unwrap_or(serde_json::Value::Null),
    }))?;
    let result = crate::interpreter::builtins::vapid::send_to_subscription(
        &subscription,
        message,
        &keys.private,
        &keys.public,
        &keys.subject,
        None,
    )
    .map_err(|e| format!("web push {}: {e}", host_of(endpoint)))?;
    let status = status_of(&result);
    if status == 404 || status == 410 {
        let _ = internal_store::delete(SUBSCRIPTIONS, &key_for(endpoint));
        return Ok(Delivery::Gone(status));
    }
    if !(200..300).contains(&status) {
        return Err(format!("web push {}: HTTP {status}", host_of(endpoint)));
    }
    Ok(Delivery::Delivered)
}

fn status_of(result: &crate::interpreter::value::Value) -> i64 {
    use crate::interpreter::value::{HashKey, Value};
    match result {
        Value::Hash(h) => match h.borrow().get(&HashKey::String("status".into())) {
            Some(Value::Int(n)) => *n,
            _ => 0,
        },
        _ => 0,
    }
}

/// The push service's host, for messages: the endpoint path identifies the
/// browser and is not logged.
fn host_of(endpoint: &str) -> String {
    endpoint
        .split("://")
        .nth(1)
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}

// --- routes --------------------------------------------------------------------

pub(crate) fn is_push_path(path: &str) -> bool {
    path == BASE || path.starts_with("/__soli/push/")
}

/// The service worker and the subscription endpoints; the request back on a
/// miss.
#[allow(clippy::result_large_err)]
pub(super) async fn dispatch(
    req: Request<Incoming>,
    method: &str,
    path: &str,
    peer_addr: SocketAddr,
    dev_mode: bool,
) -> Result<Response<ResponseBody>, Request<Incoming>> {
    if !is_push_path(path) {
        return Err(req);
    }
    let action = path.strip_prefix("/__soli/push/").unwrap_or("");
    if action == "sw.js" && matches!(method, "GET" | "HEAD") {
        return Ok(service_worker());
    }
    if method != "POST" || !matches!(action, "subscribe" | "unsubscribe" | "status" | "test") {
        return Ok(admin_auth::hidden_not_found());
    }
    let headers = req.headers().clone();
    let user_agent = headers
        .get(hyper::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = match Limited::new(req.into_body(), MAX_BODY).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return Ok(json_error(StatusCode::PAYLOAD_TOO_LARGE, "body too large")),
    };
    let Ok(input) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return Ok(json_error(StatusCode::BAD_REQUEST, "expected a JSON body"));
    };
    let Some(topic) = input
        .get("topic")
        .and_then(|t| t.as_str())
        .and_then(Topic::parse)
    else {
        return Ok(json_error(
            StatusCode::BAD_REQUEST,
            "topic must be errors or slow_queries",
        ));
    };
    // The gate of the page the topic belongs to: who may read the errors may
    // be told about them, and nobody else.
    let decision = admin_auth::authorize(&headers, dev_mode, peer_addr.ip(), topic.scope());
    if let Some(refused) = admin_auth::refusal(decision, "Soli push") {
        return Ok(refused);
    }
    Ok(handle(action, topic, &input, &user_agent))
}

fn handle(
    action: &str,
    topic: Topic,
    input: &serde_json::Value,
    user_agent: &str,
) -> Response<ResponseBody> {
    let endpoint = input
        .get("endpoint")
        .or_else(|| input.get("subscription").and_then(|s| s.get("endpoint")))
        .and_then(|e| e.as_str())
        .unwrap_or("")
        .to_string();
    let outcome = match action {
        "subscribe" => {
            let subscription = input
                .get("subscription")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            subscribe(&subscription, topic, user_agent)
                .map(|_| serde_json::json!({"subscribed": true}))
        }
        "unsubscribe" => unsubscribe(&endpoint, topic)
            .map(|was| serde_json::json!({"subscribed": false, "removed": was})),
        "status" => is_subscribed(&endpoint, topic).map(|on| serde_json::json!({"subscribed": on})),
        _ => test_push(&endpoint, topic),
    };
    match outcome {
        Ok(body) => json_response(StatusCode::OK, body),
        Err(e) => json_error(StatusCode::UNPROCESSABLE_ENTITY, &e),
    }
}

/// A notification to this browser only, so the operator sees it work.
fn test_push(endpoint: &str, topic: Topic) -> Result<serde_json::Value, String> {
    let Some(doc) = read_subscription(endpoint)? else {
        return Err("this browser is not subscribed".to_string());
    };
    let keys = keys()?;
    let message = serde_json::json!({
        "title": "Soli notifications are on",
        "body": format!("This device will be told about new {}.", match topic {
            Topic::Errors => "errors",
            Topic::SlowQueries => "slow queries",
        }),
        "url": topic.page(),
        "tag": "soli-push-test",
    })
    .to_string();
    match push_to(&doc, &message, &keys)? {
        Delivery::Delivered => Ok(serde_json::json!({"sent": true})),
        // Not an error for the page: it drops this browser's stale
        // subscription so the next "Notify this device" makes a fresh one.
        Delivery::Gone(status) => Ok(serde_json::json!({
            "sent": false,
            "gone": true,
            "error": format!(
                "the push service no longer knows this browser (HTTP {status}); its subscription \
        was removed \u{2014} press \u{201c}Notify this device\u{201d} again"
            ),
        })),
    }
}

// --- responses -------------------------------------------------------------------

fn with_header(
    mut response: Response<ResponseBody>,
    name: &'static str,
    value: &str,
) -> Response<ResponseBody> {
    if let Ok(value) = HeaderValue::from_str(value) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(name), value);
    }
    response
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response<ResponseBody> {
    let mut response = Response::new(full(Bytes::from(body.to_string())));
    *response.status_mut() = status;
    with_header(response, "content-type", "application/json")
}

fn json_error(status: StatusCode, message: &str) -> Response<ResponseBody> {
    json_response(status, serde_json::json!({ "error": message }))
}

fn service_worker() -> Response<ResponseBody> {
    let response = Response::new(full(Bytes::from_static(SERVICE_WORKER.as_bytes())));
    let response = with_header(
        response,
        "content-type",
        "application/javascript; charset=utf-8",
    );
    let response = with_header(response, "service-worker-allowed", "/__soli/");
    with_header(response, "cache-control", "no-cache")
}

/// Shows each push, and opens (or focuses) its page when clicked.
const SERVICE_WORKER: &str = r#"self.addEventListener('install', function () { self.skipWaiting(); });
self.addEventListener('activate', function (e) { e.waitUntil(self.clients.claim()); });
self.addEventListener('push', function (e) {
  var d = {};
  try { d = e.data ? e.data.json() : {}; } catch (_) { d = { body: e.data ? e.data.text() : '' }; }
  e.waitUntil(self.registration.showNotification(d.title || 'Soli', {
    body: d.body || '', tag: d.tag || undefined, renotify: !!d.tag,
    data: { url: d.url || '/__soli/errors' }
  }));
});
self.addEventListener('notificationclick', function (e) {
  e.notification.close();
  var url = new URL((e.notification.data && e.notification.data.url) || '/__soli/errors', self.location.origin).href;
  e.waitUntil(self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then(function (wins) {
    for (var i = 0; i < wins.length; i++) {
      if (wins[i].url === url && 'focus' in wins[i]) { return wins[i].focus(); }
    }
    return self.clients.openWindow(url);
  }));
});
"#;

// --- the page block ----------------------------------------------------------------

/// The "Notify this device" block for an operator page. Empty when the VAPID
/// identity cannot be read (no database): the page still renders.
pub(crate) fn page_block(topic: Topic) -> String {
    let Ok(keys) = keys() else {
        return String::new();
    };
    let what = match topic {
        Topic::Errors => "a new error, an error that comes back, or a spike",
        Topic::SlowQueries => "a new slow query",
    };
    format!(
        "<section class=\"push\" data-soli-push data-topic=\"{topic}\" data-key=\"{key}\">\
<p class=\"summary\"><b>Push notifications on this device</b> \u{2014} {what}, \
even with this page closed. <span data-push-state></span></p>\
<p><button type=\"button\" data-push-on hidden>Notify this device</button> \
<button type=\"button\" class=\"ghost\" data-push-test hidden>Send a test</button> \
<button type=\"button\" class=\"ghost\" data-push-off hidden>Stop notifying this device</button></p>\
</section><script>{script}</script>",
        topic = topic.name(),
        key = super::dev_bar::html_escape(&keys.public),
        script = PAGE_SCRIPT,
    )
}

const PAGE_SCRIPT: &str = r#"(function () {
  var box = document.querySelector('[data-soli-push]');
  if (!box) { return; }
  var topic = box.dataset.topic, key = box.dataset.key;
  var state = box.querySelector('[data-push-state]');
  var on = box.querySelector('[data-push-on]'), off = box.querySelector('[data-push-off]'), test = box.querySelector('[data-push-test]');
  function say(text) { state.textContent = text; }
  function show(subscribed) { on.hidden = subscribed; off.hidden = !subscribed; test.hidden = !subscribed; }
  if (!('serviceWorker' in navigator) || !('PushManager' in window) || !('Notification' in window)) {
    say('This browser cannot receive push notifications.'); return;
  }
  if (!window.isSecureContext) { say('Push needs HTTPS (or localhost).'); return; }
  function post(action, body) {
    body.topic = topic;
    return fetch('/__soli/push/' + action, { method: 'POST', credentials: 'same-origin',
      headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
      .then(function (r) { return r.json().then(function (j) { if (!r.ok) { throw new Error(j.error || r.status); } return j; }); });
  }
  function keyBytes(b64) {
    var pad = '='.repeat((4 - b64.length % 4) % 4), raw = atob((b64 + pad).replace(/-/g, '+').replace(/_/g, '/'));
    var out = new Uint8Array(raw.length); for (var i = 0; i < raw.length; i++) { out[i] = raw.charCodeAt(i); } return out;
  }
  var ready = navigator.serviceWorker.register('/__soli/push/sw.js', { scope: '/__soli/' })
    .then(function () { return navigator.serviceWorker.ready; });
  function current() { return ready.then(function (reg) { return reg.pushManager.getSubscription(); }); }
  current().then(function (sub) {
    if (!sub) { show(false); return; }
    return post('status', { endpoint: sub.endpoint }).then(function (j) { show(j.subscribed); });
  }).catch(function (e) { say('Unavailable: ' + e.message); });
  on.addEventListener('click', function () {
    say('');
    Notification.requestPermission().then(function (perm) {
      if (perm !== 'granted') { throw new Error('notifications are blocked for this site'); }
      return ready.then(function (reg) {
        return reg.pushManager.getSubscription().then(function (sub) {
          return sub || reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: keyBytes(key) });
        });
      });
    }).then(function (sub) { return post('subscribe', { subscription: sub.toJSON() }); })
      .then(function () { show(true); say('This device will be notified.'); })
      .catch(function (e) { say('Could not subscribe: ' + e.message); });
  });
  off.addEventListener('click', function () {
    current().then(function (sub) { return sub && post('unsubscribe', { endpoint: sub.endpoint }); })
      .then(function () { show(false); say('This device will not be notified.'); })
      .catch(function (e) { say(e.message); });
  });
  test.addEventListener('click', function () {
    current().then(function (sub) {
      return post('test', { endpoint: sub ? sub.endpoint : '' }).then(function (j) {
        if (!j.gone) { say('Test sent.'); return; }
        say(j.error); show(false);
        return sub && sub.unsubscribe();
      });
    }).catch(function (e) { say('Test failed: ' + e.message); });
  });
})();"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(endpoint: &str) -> serde_json::Value {
        serde_json::json!({"endpoint": endpoint, "keys": {"p256dh": "BPk", "auth": "au"}})
    }

    #[test]
    fn a_subscription_needs_an_https_endpoint_and_both_keys() {
        assert!(valid_subscription(&sub("https://fcm.googleapis.com/fcm/send/x")).is_ok());
        assert!(valid_subscription(&sub("http://10.0.0.1/x")).is_err());
        assert!(valid_subscription(&serde_json::json!({"endpoint": "https://a.b/x"})).is_err());
        // Anything beside the endpoint and keys is dropped, not stored.
        let kept = valid_subscription(&serde_json::json!({
            "endpoint": "https://a.b/x", "keys": {"p256dh": "k", "auth": "a"}, "extra": "x"
        }))
        .unwrap_or_default();
        assert!(kept.get("extra").is_none());
    }

    #[test]
    fn events_map_to_topics() {
        assert_eq!(Topic::of(Kind::ErrorNew), Topic::Errors);
        assert_eq!(Topic::of(Kind::ErrorRegressed), Topic::Errors);
        assert_eq!(Topic::of(Kind::ErrorSpike), Topic::Errors);
        assert_eq!(Topic::of(Kind::SlowQueryNew), Topic::SlowQueries);
        assert_eq!(Topic::parse("slow_queries"), Some(Topic::SlowQueries));
        assert_eq!(Topic::parse("jobs"), None);
    }

    #[test]
    fn the_payload_links_to_the_page_on_this_origin() {
        let event = Event::error(Kind::ErrorNew, "abc123", "boom", "app.sl:3", "GET /x", None);
        let json: serde_json::Value =
            serde_json::from_str(&payload(&event, "[app] New error: boom", "boom"))
                .unwrap_or_default();
        assert_eq!(json["url"], "/__soli/errors/abc123");
        assert_eq!(json["tag"], "error.new:abc123");
        assert_eq!(json["title"], "[app] New error: boom");
    }

    #[test]
    fn keys_for_an_endpoint_are_short_and_stable() {
        let a = key_for("https://push.example/one");
        assert_eq!(a.len(), 32);
        assert_eq!(a, key_for("https://push.example/one"));
        assert_ne!(a, key_for("https://push.example/two"));
    }

    #[test]
    fn the_service_worker_may_control_the_operator_pages() {
        let response = service_worker();
        assert_eq!(
            response
                .headers()
                .get("service-worker-allowed")
                .and_then(|v| v.to_str().ok()),
            Some("/__soli/")
        );
    }
}
