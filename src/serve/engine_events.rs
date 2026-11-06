//! What the engines did that an operator should know about, for the
//! `/__soli/` overview: handlers the VM refused (they run on the slower
//! tree-walker), handlers the two engines disagree on, and handler panics.
//!
//! `/_metrics` already counts each of these. This keeps *which* handler and
//! when, so the overview can say it. They are rare — a demotion fires once per
//! handler per worker — so a mutex around a short list is enough; the list is
//! capped at [`MAX_EVENTS`], dropping the one seen least recently.

use std::sync::Mutex;

use super::tenant::TenantId;

/// Distinct (application, kind, subject) entries kept.
pub(crate) const MAX_EVENTS: usize = 64;
const MAX_DETAIL_CHARS: usize = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EngineKind {
    /// The VM refused the handler; it runs on the tree-walker from now on.
    Refused,
    /// The handler failed on the VM and succeeded on the tree-walker.
    Divergence,
    /// The handler panicked; the request got a 500 and the worker survived.
    Panic,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EngineEvent {
    pub tenant: TenantId,
    pub kind: EngineKind,
    /// The handler (`posts#show`) or the route (`GET /posts/:id`).
    pub subject: String,
    /// The latest reason, cut to [`MAX_DETAIL_CHARS`].
    pub detail: String,
    pub count: u64,
    /// Unix seconds.
    pub first_at: i64,
    pub last_at: i64,
}

static EVENTS: Mutex<Vec<EngineEvent>> = Mutex::new(Vec::new());

/// Note one event for the current application.
pub(crate) fn record(kind: EngineKind, subject: &str, detail: &str) {
    let now = chrono::Utc::now().timestamp();
    let mut events = EVENTS.lock().unwrap_or_else(|e| e.into_inner());
    add(
        &mut events,
        super::tenant::current_id(),
        kind,
        subject,
        detail,
        now,
    );
}

fn add(
    events: &mut Vec<EngineEvent>,
    tenant: TenantId,
    kind: EngineKind,
    subject: &str,
    detail: &str,
    now: i64,
) {
    let detail = super::error_tracker::truncate_chars(detail, MAX_DETAIL_CHARS);
    if let Some(event) = events
        .iter_mut()
        .find(|e| e.tenant == tenant && e.kind == kind && e.subject == subject)
    {
        event.count = event.count.saturating_add(1);
        event.last_at = now;
        event.detail = detail;
        return;
    }
    if events.len() >= MAX_EVENTS {
        if let Some(stalest) = events
            .iter()
            .enumerate()
            .min_by_key(|(_, e)| e.last_at)
            .map(|(i, _)| i)
        {
            events.swap_remove(stalest);
        }
    }
    events.push(EngineEvent {
        tenant,
        kind,
        subject: super::error_tracker::truncate_chars(subject, MAX_DETAIL_CHARS),
        detail,
        count: 1,
        first_at: now,
        last_at: now,
    });
}

/// The current application's events.
pub(crate) fn snapshot() -> Vec<EngineEvent> {
    let tenant = super::tenant::current_id();
    EVENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|e| e.tenant == tenant)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: TenantId = TenantId(0xE4E_0001);

    #[test]
    fn repeats_count_on_one_entry() {
        let mut events = Vec::new();
        add(&mut events, T, EngineKind::Refused, "posts#show", "a", 10);
        add(&mut events, T, EngineKind::Refused, "posts#show", "b", 20);
        add(
            &mut events,
            T,
            EngineKind::Divergence,
            "posts#show",
            "c",
            30,
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].count, 2);
        assert_eq!((events[0].first_at, events[0].last_at), (10, 20));
        assert_eq!(events[0].detail, "b");
    }

    #[test]
    fn the_list_is_capped_by_dropping_the_stalest() {
        let mut events = Vec::new();
        for i in 0..MAX_EVENTS as i64 + 5 {
            add(
                &mut events,
                T,
                EngineKind::Panic,
                &format!("GET /r{i}"),
                "",
                100 + i,
            );
        }
        assert_eq!(events.len(), MAX_EVENTS);
        assert!(!events.iter().any(|e| e.subject == "GET /r0"));
        assert!(events
            .iter()
            .any(|e| e.subject == format!("GET /r{}", MAX_EVENTS + 4)));
    }
}
