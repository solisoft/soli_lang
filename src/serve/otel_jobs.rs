//! OpenTelemetry spans for background jobs.
//!
//! A job run on the pool is a trace of its own: a CONSUMER root span named
//! `job <ClassName>` with the `messaging.*` attributes, and under it the same
//! db / http / view spans a request records — the pool thread uses the same
//! span_log tree, so nothing is instrumented twice. Its outgoing `HTTP.*`
//! calls carry its `traceparent`, as a request's do.
//!
//! A job is not a child of the request that enqueued it: it may run minutes or
//! days later, on another host. When it was enqueued inside a traced request
//! or job, `JobDoc.traceparent` holds the enqueuing span and the job's root
//! span carries an OTLP **link** to it, which Datadog, Jaeger and Tempo show as
//! "linked from". Sampling is the root sampler's decision for a new trace
//! (a link does not make a parent, as the OTel spec has it).
//!
//! The OTLP body is the request exporter's (`otel::build_otlp_body`, so
//! resource attributes — `service.name`, the Datadog `DD_*` tags — and span
//! conversion stay in one place), with the root span then turned from a
//! SERVER span into this CONSUMER one. A failed job's span is marked with
//! `otel::ExceptionInfo`, as a request's uncaught error is.

use super::{otel, otel_propagation, span_log};
use serde_json::{json, Value};

/// What is known about a job when it starts.
pub struct JobInfo<'a> {
    pub class_name: &'a str,
    /// The queue row's key; `None` for a fire-and-forget hand-off.
    pub job_id: Option<&'a str>,
    pub queue: Option<&'a str>,
    /// 1-based: the claim counts the attempt before the job runs.
    pub attempt: Option<i64>,
    /// `traceparent` of the span that enqueued the job.
    pub enqueued_by: Option<&'a str>,
}

/// A job's trace, from [`begin`] to [`finish`].
pub struct JobTrace {
    ctx: otel::TraceContext,
    span: JobSpan,
}

/// What the job's root span says about it.
struct JobSpan {
    /// The root span's exported id.
    span_id: String,
    class_name: String,
    job_id: Option<String>,
    queue: Option<String>,
    attempt: Option<i64>,
    link: Option<(String, String)>,
}

/// Start the trace of a job about to run on this thread. `None` when tracing
/// is off.
pub fn begin(info: JobInfo<'_>) -> Option<JobTrace> {
    // Fresh ids and the root sampler's decision, exactly as for a request
    // that arrives without a `traceparent`.
    let ctx = otel::begin_request("JOB", info.class_name, None)?;
    // Pool threads are not web workers: their span log starts off.
    span_log::set_enabled(true);
    span_log::begin_request(ctx.instant_start);
    span_log::open_request_root(format!("job {}", info.class_name));
    otel_propagation::activate(Some(&ctx), None);
    let link = info
        .enqueued_by
        .and_then(otel::parse_traceparent)
        .map(|(trace_id, span_id, _)| (trace_id, span_id));
    let span = JobSpan {
        span_id: ctx.span_id.clone(),
        class_name: info.class_name.to_string(),
        job_id: info.job_id.map(str::to_string),
        queue: info.queue.map(str::to_string),
        attempt: info.attempt,
        link,
    };
    Some(JobTrace { ctx, span })
}

/// End the job's trace and queue its export. `error` is the job's failure.
pub fn finish(trace: Option<JobTrace>, error: Option<&str>) {
    let Some(trace) = trace else {
        return;
    };
    span_log::close_request_root();
    let spans = span_log::snapshot();
    otel_propagation::activate(None, None);
    span_log::clear();
    // Not sampled: ids and propagation only.
    if trace.ctx.flags & 0x01 == 0 {
        return;
    }
    let cfg = otel::config();
    if cfg.traces_endpoint.is_none() {
        return;
    }
    let status = if error.is_some() { 500 } else { 200 };
    let body = otel::build_otlp_body(cfg, &trace.ctx, &spans, status, None);
    otel::enqueue_export(as_job_body(&body, &trace.span, error));
}

/// Turn the request-shaped root span of `body` into the job's CONSUMER span.
/// Leaves `body` as it is if it does not parse (it always does: it was just
/// serialised).
fn as_job_body(body: &str, trace: &JobSpan, error: Option<&str>) -> String {
    let Ok(mut payload) = serde_json::from_str::<Value>(body) else {
        return body.to_string();
    };
    let spans = payload
        .pointer_mut("/resourceSpans/0/scopeSpans/0/spans")
        .and_then(Value::as_array_mut);
    let root = spans.and_then(|spans| {
        spans
            .iter_mut()
            .find(|s| s.get("spanId").and_then(Value::as_str) == Some(&trace.span_id))
    });
    let Some(root) = root.and_then(Value::as_object_mut) else {
        return body.to_string();
    };

    root.insert("name".into(), json!(format!("job {}", trace.class_name)));
    // OTLP SpanKind 5 = CONSUMER.
    root.insert("kind".into(), json!(5));
    let mut attributes = vec![
        attr_str("soli.span.kind", "job"),
        attr_str("messaging.system", "soli"),
        attr_str("messaging.operation.type", "process"),
        attr_str("soli.job.class", &trace.class_name),
    ];
    if let Some(queue) = &trace.queue {
        attributes.push(attr_str("messaging.destination.name", queue));
    }
    if let Some(id) = &trace.job_id {
        attributes.push(attr_str("messaging.message.id", id));
    }
    if let Some(attempt) = trace.attempt {
        attributes
            .push(json!({"key": "soli.job.attempt", "value": {"intValue": attempt.to_string()}}));
    }
    root.insert("attributes".into(), Value::Array(attributes));
    root.insert("status".into(), json!({"code": 1}));
    if let Some((trace_id, span_id)) = &trace.link {
        root.insert(
            "links".into(),
            json!([{
                "traceId": trace_id,
                "spanId": span_id,
                "attributes": [attr_str("soli.link.kind", "enqueued_by")],
            }]),
        );
    }
    if let Some(message) = error {
        // The job's failure reaches the runner as a string: an `Error` with
        // no frames, marked the way a request's uncaught error is (status
        // ERROR, `error.type`, the `exception` event).
        let end_ns = root
            .get("endTimeUnixNano")
            .and_then(Value::as_str)
            .and_then(|t| t.parse::<u64>().ok())
            .unwrap_or(0);
        let exception = otel::ExceptionInfo::new("Error", message, &[], "");
        let mut span = Value::Object(std::mem::take(root));
        exception.apply(&mut span, end_ns, true);
        if let Value::Object(map) = span {
            *root = map;
        }
    }
    payload.to_string()
}

fn attr_str(key: &str, value: &str) -> Value {
    json!({"key": key, "value": {"stringValue": value}})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace(link: Option<(&str, &str)>) -> JobSpan {
        JobSpan {
            span_id: "b7ad6b7169203331".into(),
            class_name: "SendInvoiceJob".into(),
            job_id: Some("job-1".into()),
            queue: Some("mailers".into()),
            attempt: Some(2),
            link: link.map(|(t, s)| (t.to_string(), s.to_string())),
        }
    }

    fn body() -> String {
        json!({"resourceSpans": [{"resource": {"attributes": []}, "scopeSpans": [{
            "scope": {"name": "soli.serve"},
            "spans": [
                {"spanId": "1111111111111111", "parentSpanId": "b7ad6b7169203331",
                 "name": "GET http://x", "kind": 3, "attributes": []},
                {"spanId": "b7ad6b7169203331", "name": "JOB SendInvoiceJob", "kind": 2,
                 "attributes": [{"key": "http.request.method", "value": {"stringValue": "JOB"}}],
                 "status": {"code": 1}}
            ]
        }]}]})
        .to_string()
    }

    fn root(out: &str) -> Value {
        let payload: Value = serde_json::from_str(out).unwrap();
        payload["resourceSpans"][0]["scopeSpans"][0]["spans"][1].clone()
    }

    #[test]
    fn the_root_becomes_a_consumer_span_with_messaging_attributes() {
        let out = as_job_body(&body(), &trace(None), None);
        let root = root(&out);
        assert_eq!(root["kind"], 5);
        assert_eq!(root["name"], "job SendInvoiceJob");
        let attrs = root["attributes"].to_string();
        for needle in [
            "messaging.system",
            "\"soli\"",
            "messaging.destination.name",
            "mailers",
            "messaging.message.id",
            "job-1",
            "soli.job.attempt",
        ] {
            assert!(attrs.contains(needle), "{needle} missing: {attrs}");
        }
        assert!(!attrs.contains("http.request.method"), "{attrs}");
        assert!(root.get("links").is_none());
        assert_eq!(root["status"]["code"], 1);
        // Child spans are left alone.
        let payload: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            payload["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["kind"],
            3
        );
    }

    #[test]
    fn a_failed_job_has_an_error_status() {
        let out = as_job_body(&body(), &trace(None), Some("boom"));
        let root = root(&out);
        assert_eq!(root["status"]["code"], 2);
        assert_eq!(root["status"]["message"], "boom");
        let attrs = root["attributes"].to_string();
        assert!(attrs.contains("error.type"), "{attrs}");
        assert_eq!(root["events"][0]["name"], "exception");
    }

    #[test]
    fn the_enqueuing_span_is_a_link_not_a_parent() {
        let link = ("4bf92f3577b34da6a3ce929d0e0e4736", "00f067aa0ba902b7");
        let out = as_job_body(&body(), &trace(Some(link)), None);
        let root = root(&out);
        assert_eq!(root["links"][0]["traceId"], link.0);
        assert_eq!(root["links"][0]["spanId"], link.1);
        assert!(root.get("parentSpanId").is_none());
    }
}
