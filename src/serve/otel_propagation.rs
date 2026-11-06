//! Outbound W3C Trace Context: the trace a worker is serving, and the
//! `traceparent` / `tracestate` its outgoing `HTTP.*` calls carry.
//!
//! The trace being served — a request (`activate` in `serve`) or a job
//! (`otel_jobs`) — is kept in a request-local, so a call made anywhere in the
//! handler can find it. Each outgoing call reserves the span_log id of its
//! CLIENT span *before* it is sent and names that span's exported id as the
//! parent in `traceparent`; the downstream service's SERVER span then nests
//! under the call in Datadog / Jaeger / Tempo, not beside it. The inbound
//! `tracestate` is forwarded as it came.
//!
//! Like the OTel SDKs' default, the headers go to every host. A caller that
//! sets `traceparent` or `tracestate` itself keeps its value; nothing is added
//! when tracing is off; `SOLI_OTEL_PROPAGATE=0` turns injection off while
//! keeping export.
//!
//! The context is read on the calling thread: a call that runs elsewhere (the
//! parallel fan-outs) takes an [`OutboundCall`] here first and carries it.

use super::otel::{child_span_hex, TraceContext};
use super::span_log::{self, Reserved};
use std::cell::RefCell;
use std::sync::OnceLock;

/// The trace this thread is serving, as outbound calls need it.
#[derive(Clone, Debug, PartialEq)]
pub struct Active {
    pub trace_id: String,
    /// The root span's exported id (SERVER for a request, CONSUMER for a job).
    pub span_id: String,
    pub flags: u8,
    /// Inbound `tracestate`, forwarded unchanged.
    pub tracestate: Option<String>,
}

crate::request_local! {
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
}

/// Make `ctx` the trace this thread serves (`None` forgets it). `tracestate`
/// is the inbound header, kept for outgoing calls.
pub fn activate(ctx: Option<&TraceContext>, tracestate: Option<&str>) {
    let active = ctx.map(|c| Active {
        trace_id: c.trace_id.clone(),
        span_id: c.span_id.clone(),
        flags: c.flags,
        tracestate: clean_tracestate(tracestate),
    });
    ACTIVE.with(|a| *a.borrow_mut() = active);
}

/// A `tracestate` worth forwarding: trimmed, non-empty, and within the
/// spec's 512-character limit (a longer one is dropped rather than cut).
fn clean_tracestate(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 512)
        .map(str::to_string)
}

pub fn active() -> Option<Active> {
    ACTIVE.with(|a| a.borrow().clone())
}

/// `SOLI_OTEL_PROPAGATE=0` / `false` / `no` / `off`: no headers on outgoing
/// calls. On by default.
pub fn propagate_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        !crate::platform::env::var("SOLI_OTEL_PROPAGATE").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            )
        })
    })
}

/// The exported id of the innermost open span — the request or job root when
/// nothing is nested.
fn current_span_hex(active: &Active) -> String {
    match span_log::current_span() {
        Some(local) => span_hex(active, local),
        None => active.span_id.clone(),
    }
}

/// The exported id of span_log span `local`: the root keeps the context's id.
fn span_hex(active: &Active, local: u32) -> String {
    if local == 0 {
        // `open_request_root` is the first span of a request or job: id 0.
        return active.span_id.clone();
    }
    child_span_hex(&active.span_id, local)
}

/// `traceparent` naming the innermost open span — what a job stores when it is
/// enqueued, so its own trace can link back to the code that enqueued it.
pub fn current_traceparent() -> Option<String> {
    let active = active()?;
    Some(format!(
        "00-{}-{}-{:02x}",
        active.trace_id,
        current_span_hex(&active),
        active.flags
    ))
}

/// One outgoing call's trace headers, and the CLIENT span id they name.
#[derive(Clone, Debug, PartialEq)]
pub struct OutboundCall {
    /// The span_log id the call's CLIENT span must be recorded under.
    pub span: Option<Reserved>,
    pub traceparent: String,
    pub tracestate: Option<String>,
}

impl OutboundCall {
    /// Reserve the CLIENT span for a call about to be made from this thread.
    /// `None` when tracing is off, nothing is being traced here, or
    /// propagation is turned off.
    pub fn begin() -> Option<Self> {
        let active = active()?;
        if !propagate_enabled() {
            return None;
        }
        Some(Self::under(&active, span_log::reserve()))
    }

    /// Headers for a call that records no span of its own (`HTTP.stream_to`,
    /// which outlives the request): the parent is the innermost open span.
    pub fn detached() -> Option<Self> {
        let active = active()?;
        if !propagate_enabled() {
            return None;
        }
        Some(Self::under(&active, None))
    }

    /// The headers for `span` of the trace `active`. Without a span (the span
    /// log is off) the parent is the innermost open span.
    fn under(active: &Active, span: Option<Reserved>) -> Self {
        let parent = match span {
            Some(reserved) => span_hex(active, reserved.id),
            None => current_span_hex(active),
        };
        OutboundCall {
            span,
            traceparent: format!("00-{}-{}-{:02x}", active.trace_id, parent, active.flags),
            tracestate: active.tracestate.clone(),
        }
    }

    /// The headers this call adds: those the caller did not set already.
    /// `has` answers whether the caller set a header (case-insensitively).
    pub fn headers_missing(&self, has: impl Fn(&str) -> bool) -> Vec<(&'static str, String)> {
        let mut out = Vec::with_capacity(2);
        if !has("traceparent") {
            out.push(("traceparent", self.traceparent.clone()));
        }
        if let Some(state) = &self.tracestate {
            if !has("tracestate") {
                out.push(("tracestate", state.clone()));
            }
        }
        out
    }

    /// Add the headers to a `(name, value)` list, keeping the caller's own.
    pub fn inject_into(&self, headers: &mut Vec<(String, String)>) {
        let missing = self.headers_missing(|name| {
            headers
                .iter()
                .any(|(existing, _)| existing.eq_ignore_ascii_case(name))
        });
        headers.extend(missing.into_iter().map(|(k, v)| (k.to_string(), v)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active(tracestate: Option<&str>) -> Active {
        Active {
            trace_id: "0af7651916cd43dd8448eb211c80319c".into(),
            span_id: "b7ad6b7169203331".into(),
            flags: 1,
            tracestate: tracestate.map(str::to_string),
        }
    }

    #[test]
    fn the_traceparent_names_the_reserved_client_span() {
        let call = OutboundCall::under(
            &active(None),
            Some(Reserved {
                id: 3,
                parent: Some(0),
            }),
        );
        let expected = child_span_hex("b7ad6b7169203331", 3);
        assert_eq!(
            call.traceparent,
            format!("00-0af7651916cd43dd8448eb211c80319c-{expected}-01")
        );
        let (tid, pid, flags) = super::super::otel::parse_traceparent(&call.traceparent).unwrap();
        assert_eq!(tid, "0af7651916cd43dd8448eb211c80319c");
        assert_eq!(pid, expected);
        assert_eq!(flags, 1);
    }

    #[test]
    fn an_unsampled_trace_propagates_flags_00() {
        let mut unsampled = active(None);
        unsampled.flags = 0;
        let call = OutboundCall::under(&unsampled, None);
        assert!(call.traceparent.ends_with("-00"), "{}", call.traceparent);
        // No span reserved: the parent is the root.
        assert!(call.traceparent.contains("-b7ad6b7169203331-"));
    }

    #[test]
    fn the_callers_own_headers_are_kept() {
        let call = OutboundCall::under(&active(Some("dd=s:1,foo=bar")), None);
        let mut headers = vec![("TraceParent".to_string(), "00-mine".to_string())];
        call.inject_into(&mut headers);
        assert_eq!(
            headers,
            vec![
                ("TraceParent".to_string(), "00-mine".to_string()),
                ("tracestate".to_string(), "dd=s:1,foo=bar".to_string()),
            ]
        );
        let mut both = vec![
            ("traceparent".to_string(), "a".to_string()),
            ("TRACESTATE".to_string(), "b".to_string()),
        ];
        call.inject_into(&mut both);
        assert_eq!(both.len(), 2);
    }

    #[test]
    fn no_tracestate_header_without_an_inbound_one() {
        let call = OutboundCall::under(&active(None), None);
        let mut headers = Vec::new();
        call.inject_into(&mut headers);
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "traceparent");
    }

    #[test]
    fn nothing_is_injected_when_nothing_is_traced() {
        activate(None, None);
        assert!(OutboundCall::begin().is_none());
        assert!(current_traceparent().is_none());
    }

    #[test]
    fn an_active_trace_is_propagated_and_forgotten() {
        let mut traced = active(None);
        traced.tracestate = clean_tracestate(Some(" dd=s:1 "));
        ACTIVE.with(|a| *a.borrow_mut() = Some(traced));
        let call = OutboundCall::begin().expect("active");
        assert!(call
            .traceparent
            .starts_with("00-0af7651916cd43dd8448eb211c80319c-"));
        assert_eq!(call.tracestate.as_deref(), Some("dd=s:1"));
        assert!(current_traceparent().is_some());
        activate(None, None);
        assert!(OutboundCall::begin().is_none());
    }
}
