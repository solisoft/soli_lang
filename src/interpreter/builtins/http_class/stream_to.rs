//! `HTTP.stream_to(method, url, options?, body?, to)`: a stream read by the
//! HTTP runtime instead of a Soli thread.
//!
//! `HTTP.stream` hands each line to Soli code, so the thread that reads it is
//! held for as long as the answer takes — ten minutes for a model that thinks
//! before it writes, one job worker gone for all of them. Here the call
//! returns an id at once and the stream is read by a task on
//! [`user_http_runtime`]: the text of each event is taken out in Rust
//! (`format`), sent to the pages subscribed to `topic` (`sse_broadcast`), kept
//! for `HTTP.stream_status`, and handed at the end to job `job`, failure
//! included. No Soli code runs and no thread waits while the model thinks.
//!
//! The task carries the application it was started for (`task_scope`), so its
//! broadcasts and its job reach that application when several share a server.
//! What it keeps lives in this process only: `stream_status` and
//! `stream_cancel` see the streams this process started, and a stream dies
//! with its process — it is not resumed, and its job is not enqueued.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::serve::tenant::{self, TenantId, TenantValue};

/// How long a finished stream stays readable by `HTTP.stream_status`.
const KEEP_FINISHED: Duration = Duration::from_secs(600);

/// The most of a non-2xx answer kept as its error.
const ERROR_BODY_CAP: usize = 64 * 1024;

/// Streams running in this process, for the cap and for a script's exit.
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// The streams this process started, by id, per application.
static STREAMS: TenantValue<HashMap<String, Entry>> = TenantValue::new(HashMap::new);

/// How the text is taken out of the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    /// Anthropic Messages events: `content_block_delta` text.
    Anthropic,
    /// OpenAI chat completion chunks: `choices[0].delta.content`.
    OpenAi,
    /// Any server-sent events: each event's data.
    Sse,
    /// Each line, as `next_line` gives it.
    Lines,
}

impl Format {
    fn parse(name: &str) -> Result<Self, String> {
        match name {
            "anthropic" => Ok(Self::Anthropic),
            "openai" => Ok(Self::OpenAi),
            "sse" => Ok(Self::Sse),
            "lines" => Ok(Self::Lines),
            other => Err(format!(
                "HTTP.stream_to(): unknown format {other:?} (anthropic, openai, sse or lines)"
            )),
        }
    }

    /// Pieces of a raw format are lines or events, kept apart in the text;
    /// a model's deltas are parts of one text.
    fn separator(self) -> &'static str {
        match self {
            Self::Anthropic | Self::OpenAi => "",
            Self::Sse | Self::Lines => "\n",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Running,
    Done,
    Failed,
    Cancelled,
}

impl State {
    fn name(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// What `HTTP.stream_status` reads.
struct Entry {
    state: State,
    text: String,
    status: Option<u16>,
    error: Option<String>,
    stop_reason: Option<String>,
    cancel: Arc<tokio::sync::Notify>,
    finished_at: Option<Instant>,
}

/// Everything the task needs, owned, so it can leave the calling thread.
struct Spec {
    id: String,
    call: StreamCall,
    format: Format,
    topic: Option<String>,
    job: Option<String>,
    args: serde_json::Map<String, serde_json::Value>,
    app: TenantId,
    db_tenant: Option<crate::db::tenancy::Tenant>,
}

/// How a stream ended.
#[derive(Debug, Default)]
struct Outcome {
    state: Option<State>,
    text: String,
    status: Option<u16>,
    error: Option<String>,
    stop_reason: Option<String>,
    usage: Option<serde_json::Value>,
}

// ---------------------------------------------------------------- parsing

/// One server-sent event.
#[derive(Debug, PartialEq)]
struct SseEvent {
    event: Option<String>,
    data: String,
}

/// Server-sent events, line by line: `event:` and `data:` fields, `:`
/// comments, a blank line ending an event (one without data is dropped).
#[derive(Default)]
struct SseParser {
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    fn line(&mut self, line: &str) -> Option<SseEvent> {
        if line.is_empty() {
            let event = self.event.take();
            if self.data.is_empty() {
                return None;
            }
            return Some(SseEvent {
                event,
                data: std::mem::take(&mut self.data).join("\n"),
            });
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => self.data.push(value.to_string()),
            _ => {}
        }
        None
    }
}

/// What one event says.
#[derive(Debug, Default, PartialEq)]
struct Step {
    delta: Option<String>,
    stop_reason: Option<String>,
    usage: Option<serde_json::Value>,
    end: bool,
    error: Option<String>,
}

fn error_message(json: &serde_json::Value, raw: &str) -> String {
    json["error"]["message"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| raw.to_string())
}

fn step(format: Format, event: &SseEvent) -> Step {
    match format {
        Format::Lines | Format::Sse => Step {
            delta: Some(event.data.clone()),
            ..Default::default()
        },
        Format::OpenAi => {
            if event.data.trim() == "[DONE]" {
                return Step {
                    end: true,
                    ..Default::default()
                };
            }
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&event.data) else {
                return Step::default();
            };
            if !json["error"].is_null() {
                return Step {
                    error: Some(error_message(&json, &event.data)),
                    ..Default::default()
                };
            }
            let choice = &json["choices"][0];
            Step {
                delta: choice["delta"]["content"].as_str().map(str::to_string),
                stop_reason: choice["finish_reason"].as_str().map(str::to_string),
                usage: json.get("usage").filter(|u| u.is_object()).cloned(),
                ..Default::default()
            }
        }
        Format::Anthropic => {
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&event.data) else {
                return Step::default();
            };
            let kind = json["type"]
                .as_str()
                .or(event.event.as_deref())
                .unwrap_or("");
            match kind {
                "content_block_delta" if json["delta"]["type"] == "text_delta" => Step {
                    delta: json["delta"]["text"].as_str().map(str::to_string),
                    ..Default::default()
                },
                "message_start" => Step {
                    usage: json["message"]
                        .get("usage")
                        .filter(|u| u.is_object())
                        .cloned(),
                    ..Default::default()
                },
                "message_delta" => Step {
                    stop_reason: json["delta"]["stop_reason"].as_str().map(str::to_string),
                    usage: json.get("usage").filter(|u| u.is_object()).cloned(),
                    ..Default::default()
                },
                "message_stop" => Step {
                    end: true,
                    ..Default::default()
                },
                "error" => Step {
                    error: Some(error_message(&json, &event.data)),
                    ..Default::default()
                },
                _ => Step::default(),
            }
        }
    }
}

/// Later usage counts replace earlier ones key by key: Anthropic sends the
/// input tokens at the start and the output tokens at the end.
fn merge_usage(into: &mut Option<serde_json::Value>, more: serde_json::Value) {
    match (into.as_mut(), more) {
        (Some(serde_json::Value::Object(old)), serde_json::Value::Object(new)) => {
            old.extend(new);
        }
        (_, more) => *into = Some(more),
    }
}

/// What the lines read so far have said, and where the text is.
struct Reader {
    format: Format,
    parser: SseParser,
    outcome: Outcome,
    /// What this chunk added, sent to the topic once the chunk is read.
    fresh: String,
    pieces: usize,
}

impl Reader {
    fn new(format: Format) -> Self {
        Self {
            format,
            parser: SseParser::default(),
            outcome: Outcome::default(),
            fresh: String::new(),
            pieces: 0,
        }
    }

    /// Reads one line; true once the stream says it is over.
    fn line(&mut self, line: &str) -> bool {
        let event = match self.format {
            Format::Lines => SseEvent {
                event: None,
                data: line.to_string(),
            },
            _ => match self.parser.line(line) {
                Some(event) => event,
                None => return false,
            },
        };
        let step = step(self.format, &event);
        if let Some(delta) = step.delta {
            if self.pieces > 0 {
                self.fresh.push_str(self.format.separator());
            }
            self.pieces += 1;
            self.fresh.push_str(&delta);
        }
        if step.stop_reason.is_some() {
            self.outcome.stop_reason = step.stop_reason;
        }
        if let Some(usage) = step.usage {
            merge_usage(&mut self.outcome.usage, usage);
        }
        if let Some(error) = step.error {
            self.outcome.error = Some(error);
            self.outcome.state = Some(State::Failed);
            return true;
        }
        step.end
    }

    /// The body is over: its unended last line, then a last event.
    fn finish(&mut self, rest: &[u8]) {
        if !rest.is_empty() && self.line(&String::from_utf8_lossy(rest)) {
            return;
        }
        if self.format != Format::Lines {
            self.line("");
        }
    }

    /// Takes what this chunk added out, into the text.
    fn take_fresh(&mut self) -> Option<String> {
        if self.fresh.is_empty() {
            return None;
        }
        let fresh = std::mem::take(&mut self.fresh);
        self.outcome.text.push_str(&fresh);
        Some(fresh)
    }
}

// ---------------------------------------------------------------- the task

/// Decrements [`IN_FLIGHT`] however the task ends.
struct InFlight;

impl Drop for InFlight {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

fn with_entry(id: &str, f: impl FnOnce(&mut Entry)) {
    STREAMS.write(|streams| {
        if let Some(entry) = streams.get_mut(id) {
            f(entry);
        }
    });
}

fn broadcast(topic: Option<&str>, event: &str, data: serde_json::Value) {
    if let Some(topic) = topic {
        crate::interpreter::builtins::streaming::broadcast_sse(
            topic,
            &data.to_string(),
            Some(event),
        );
    }
}

/// Reads the answer into `reader`, sending each chunk's text to the topic.
async fn read(spec: &Spec, cancel: &tokio::sync::Notify, reader: &mut Reader) {
    let idle = spec.call.idle;
    let fail = |reader: &mut Reader, error: String| {
        reader.outcome.state = Some(State::Failed);
        reader.outcome.error = Some(error);
    };
    let client = user_http_client(spec.call.follow_redirects).clone();
    let request = match stream_request(
        &client,
        &spec.call.method,
        &spec.call.url,
        &spec.call.headers,
        spec.call.body.clone(),
    ) {
        Ok(request) => request,
        Err(e) => return fail(reader, e),
    };
    let mut response = tokio::select! {
        sent = tokio::time::timeout(idle, request.send()) => match sent {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => return fail(reader, describe_request_error(&e)),
            Err(_) => return fail(reader, format!("no answer in {} s", idle.as_secs())),
        },
        _ = cancel.notified() => {
            reader.outcome.state = Some(State::Cancelled);
            return;
        }
    };
    let status = response.status();
    reader.outcome.status = Some(status.as_u16());
    with_entry(&spec.id, |entry| entry.status = Some(status.as_u16()));

    let mut pending: Vec<u8> = Vec::new();
    let mut read = 0usize;
    loop {
        let chunk = tokio::select! {
            chunk = tokio::time::timeout(idle, response.chunk()) => chunk,
            _ = cancel.notified() => {
                reader.outcome.state = Some(State::Cancelled);
                return;
            }
        };
        let bytes = match chunk {
            Ok(Ok(Some(bytes))) => bytes,
            Ok(Ok(None)) => break,
            Ok(Err(e)) => {
                return fail(
                    reader,
                    format!("the answer broke off: {}", describe_request_error(&e)),
                )
            }
            Err(_) => return fail(reader, format!("nothing received for {} s", idle.as_secs())),
        };
        read += bytes.len();
        if !status.is_success() {
            pending.extend_from_slice(&bytes);
            if pending.len() >= ERROR_BODY_CAP {
                break;
            }
            continue;
        }
        if read > http_max_response_bytes() {
            return fail(
                reader,
                format!(
                    "the answer passed {} bytes (SOLI_HTTP_MAX_RESPONSE_BYTES)",
                    http_max_response_bytes()
                ),
            );
        }
        pending.extend_from_slice(&bytes);
        let mut over = false;
        while let Some(line) = take_line(&mut pending) {
            if reader.line(&line) {
                over = true;
                break;
            }
        }
        publish(spec, reader);
        if over {
            return;
        }
    }
    if !status.is_success() {
        pending.truncate(ERROR_BODY_CAP);
        let body = String::from_utf8_lossy(&pending).into_owned();
        let error = if body.trim().is_empty() {
            format!("HTTP {}", status.as_u16())
        } else {
            body
        };
        return fail(reader, error);
    }
    reader.finish(&pending);
    publish(spec, reader);
}

/// Sends what the last chunk added, and keeps it for `stream_status`.
fn publish(spec: &Spec, reader: &mut Reader) {
    if let Some(fresh) = reader.take_fresh() {
        with_entry(&spec.id, |entry| entry.text.push_str(&fresh));
        broadcast(
            spec.topic.as_deref(),
            "delta",
            serde_json::json!({ "id": spec.id, "text": fresh }),
        );
    }
}

/// The arguments the job gets: the caller's, then what the stream gave.
fn job_args(spec: &Spec, outcome: &Outcome, state: State) -> serde_json::Value {
    let mut args = spec.args.clone();
    args.insert("stream_id".into(), spec.id.clone().into());
    args.insert("state".into(), state.name().into());
    args.insert(
        "status".into(),
        outcome.status.map_or(serde_json::Value::Null, Into::into),
    );
    args.insert("text".into(), outcome.text.clone().into());
    args.insert(
        "error".into(),
        outcome
            .error
            .clone()
            .map_or(serde_json::Value::Null, Into::into),
    );
    args.insert(
        "stop_reason".into(),
        outcome
            .stop_reason
            .clone()
            .map_or(serde_json::Value::Null, Into::into),
    );
    args.insert(
        "usage".into(),
        outcome.usage.clone().unwrap_or(serde_json::Value::Null),
    );
    serde_json::Value::Object(args)
}

/// Enqueues the job, on a blocking thread bound to the stream's application
/// and database tenant: enqueuing writes a row, and a task-local does not
/// follow onto that thread.
async fn enqueue(spec: &Spec, job: &str, args: serde_json::Value) -> Result<(), String> {
    let (app, db_tenant, job) = (spec.app, spec.db_tenant.clone(), job.to_string());
    tokio::task::spawn_blocking(move || {
        tenant::scoped(app, || {
            crate::db::tenancy::scoped(db_tenant, || {
                let doc = crate::jobs::JobDoc::new(
                    &job,
                    args,
                    &crate::jobs::config().default_queue,
                    crate::jobs::now_iso(),
                );
                crate::interpreter::builtins::jobs::enqueue_doc(
                    doc,
                    &serde_json::json!({}),
                    "HTTP.stream_to",
                )
                .map(|_| ())
            })
        })
    })
    .await
    .map_err(|e| format!("HTTP.stream_to: the job could not be enqueued: {e}"))?
}

async fn run(spec: Spec, cancel: Arc<tokio::sync::Notify>) {
    let _in_flight = InFlight;
    let mut reader = Reader::new(spec.format);
    read(&spec, &cancel, &mut reader).await;
    let mut outcome = reader.outcome;
    let state = outcome.state.unwrap_or(State::Done);
    if state == State::Cancelled && outcome.error.is_none() {
        outcome.error = Some("cancelled".to_string());
    }
    let mut job_error = None;
    if let Some(job) = &spec.job {
        if let Err(e) = enqueue(&spec, job, job_args(&spec, &outcome, state)).await {
            eprintln!("[http.stream_to] {}: {e}", spec.id);
            job_error = Some(e);
        }
    }
    with_entry(&spec.id, |entry| {
        entry.state = state;
        entry.error = outcome.error.clone().or(job_error);
        entry.stop_reason = outcome.stop_reason.clone();
        entry.finished_at = Some(Instant::now());
    });
    let mut last = serde_json::json!({ "id": spec.id, "text": outcome.text });
    if let Some(error) = &outcome.error {
        last["error"] = error.clone().into();
    }
    broadcast(spec.topic.as_deref(), state.name(), last);
}

// ---------------------------------------------------------------- builtins

fn max_streams() -> usize {
    crate::platform::env::var("SOLI_HTTP_MAX_STREAMS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(256)
}

/// A job handler names a class: `AnswerDoneJob`, `Billing::ChargeJob`.
fn is_class_name(name: &str) -> bool {
    name.split("::").all(|part| {
        part.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn option_string(to: &HashPairs, key: &str) -> Result<Option<String>, String> {
    match to.get(&HashKey::String(key.into())) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if !s.is_empty() => Ok(Some(s.to_string())),
        Some(other) => Err(format!(
            "HTTP.stream_to(): \"{key}\" must be a non-empty string, got {}",
            other.type_name()
        )),
    }
}

/// Where a stream goes: the fifth argument of `HTTP.stream_to`, checked.
struct Target {
    format: Format,
    topic: Option<String>,
    job: Option<String>,
    args: serde_json::Map<String, serde_json::Value>,
}

/// `HTTP.stream_to(method, url, options?, body?, to)`.
fn http_stream_to(args: &[Value]) -> Result<Value, String> {
    let call = stream_call(args, "HTTP.stream_to")?;
    if call.response != ResponseMode::Text {
        return Err(
            "HTTP.stream_to() reads text events; it takes no \"response\" option".to_string(),
        );
    }
    let target = target(args.get(4))?;
    begin(call, target).map(|id| Value::String(id.into()))
}

fn target(to: Option<&Value>) -> Result<Target, String> {
    let to = match to {
        Some(Value::Hash(to)) => to.borrow().clone(),
        _ => {
            return Err(
                "HTTP.stream_to() takes a fifth argument, a hash: {\"format\", \"topic\", \"job\", \"args\"}"
                    .to_string(),
            )
        }
    };
    for (key, _) in to.iter() {
        let HashKey::String(key) = key else { continue };
        if !matches!(key.as_ref(), "format" | "topic" | "job" | "args") {
            return Err(format!(
                "HTTP.stream_to(): unknown key {:?} (format, topic, job, args)",
                key.as_ref()
            ));
        }
    }
    let format = Format::parse(option_string(&to, "format")?.as_deref().unwrap_or("lines"))?;
    let topic = option_string(&to, "topic")?;
    let job = option_string(&to, "job")?;
    if let Some(job) = &job {
        if !is_class_name(job) {
            return Err(format!(
                "HTTP.stream_to(): \"job\" must name a job class, got {job:?}"
            ));
        }
    }
    let args = match to.get(&HashKey::String("args".into())) {
        None | Some(Value::Null) => serde_json::Map::new(),
        Some(v @ Value::Hash(_)) => match crate::interpreter::value::value_to_json(v)? {
            serde_json::Value::Object(map) => map,
            _ => serde_json::Map::new(),
        },
        Some(other) => {
            return Err(format!(
                "HTTP.stream_to(): \"args\" must be a hash, got {}",
                other.type_name()
            ))
        }
    };
    Ok(Target {
        format,
        topic,
        job,
        args,
    })
}

/// Starts the task; the id it is known by.
fn begin(call: StreamCall, target: Target) -> Result<String, String> {
    if !matches!(
        call.method.as_str(),
        "GET" | "POST" | "PUT" | "DELETE" | "PATCH"
    ) {
        return Err(format!(
            "HTTP.stream_to(): unsupported method {}",
            call.method
        ));
    }
    if IN_FLIGHT.load(Ordering::SeqCst) >= max_streams() {
        return Err(format!(
            "HTTP.stream_to(): {} streams are already running (SOLI_HTTP_MAX_STREAMS)",
            max_streams()
        ));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let cancel = Arc::new(tokio::sync::Notify::new());
    let now = Instant::now();
    STREAMS.write(|streams| {
        streams.retain(|_, entry| {
            entry
                .finished_at
                .is_none_or(|at| now.duration_since(at) < KEEP_FINISHED)
        });
        streams.insert(
            id.clone(),
            Entry {
                state: State::Running,
                text: String::new(),
                status: None,
                error: None,
                stop_reason: None,
                cancel: cancel.clone(),
                finished_at: None,
            },
        );
    });
    let app = tenant::current_id();
    let spec = Spec {
        id: id.clone(),
        call,
        format: target.format,
        topic: target.topic,
        job: target.job,
        args: target.args,
        app,
        db_tenant: crate::db::tenancy::current(),
    };
    IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
    user_http_runtime().spawn(tenant::task_scope(app, run(spec, cancel)));
    Ok(id)
}

fn stream_id_arg(args: &[Value], what: &str) -> Result<String, String> {
    match args.first() {
        Some(Value::String(s)) => Ok(s.to_string()),
        _ => Err(format!("{what}() takes the id HTTP.stream_to returned")),
    }
}

/// `HTTP.stream_status(id)`: a hash, or nil for an id this process does not
/// know (never started here, or finished more than ten minutes ago).
fn http_stream_status(args: &[Value]) -> Result<Value, String> {
    let id = stream_id_arg(args, "HTTP.stream_status")?;
    let string = |s: &Option<String>| {
        s.as_ref()
            .map_or(Value::Null, |s| Value::String(s.clone().into()))
    };
    Ok(STREAMS.read(|streams| {
        streams.get(&id).map_or(Value::Null, |entry| {
            let pairs: HashPairs = [
                ("id", Value::String(id.clone().into())),
                ("state", Value::String(entry.state.name().into())),
                ("text", Value::String(entry.text.clone().into())),
                (
                    "status",
                    entry
                        .status
                        .map_or(Value::Null, |s| Value::Int(i64::from(s))),
                ),
                ("error", string(&entry.error)),
                ("stop_reason", string(&entry.stop_reason)),
            ]
            .into_iter()
            .map(|(k, v)| (HashKey::String(k.into()), v))
            .collect();
            Value::Hash(Rc::new(RefCell::new(pairs)))
        })
    }))
}

/// `HTTP.stream_cancel(id)`: true when the stream was running here.
fn http_stream_cancel(args: &[Value]) -> Result<Value, String> {
    let id = stream_id_arg(args, "HTTP.stream_cancel")?;
    let cancel = STREAMS.read(|streams| {
        streams
            .get(&id)
            .filter(|entry| entry.state == State::Running)
            .map(|entry| entry.cancel.clone())
    });
    Ok(Value::Bool(cancel.is_some_and(|cancel| {
        cancel.notify_one();
        true
    })))
}

pub(super) fn register(static_methods: &mut HashMap<String, Rc<NativeFunction>>) {
    static_methods.insert(
        "stream_to".to_string(),
        Rc::new(NativeFunction::new("HTTP.stream_to", None, http_stream_to)),
    );
    static_methods.insert(
        "stream_status".to_string(),
        Rc::new(NativeFunction::new(
            "HTTP.stream_status",
            Some(1),
            http_stream_status,
        )),
    );
    static_methods.insert(
        "stream_cancel".to_string(),
        Rc::new(NativeFunction::new(
            "HTTP.stream_cancel",
            Some(1),
            http_stream_cancel,
        )),
    );
}

/// Waits for the `HTTP.stream_to` streams still running, so that a plain
/// `soli script.sl` does not exit under them: its process would take the
/// runtime, and the streams with it. Bounded by `SOLI_HTTP_STREAM_EXIT_WAIT`
/// (seconds) when set; each stream is bounded by its own idle timeout anyway.
pub fn wait_for_background_streams() {
    let running = IN_FLIGHT.load(Ordering::SeqCst);
    if running == 0 {
        return;
    }
    let limit = crate::platform::env::var("SOLI_HTTP_STREAM_EXIT_WAIT")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    eprintln!("[http.stream_to] waiting for {running} stream(s) to finish");
    let start = Instant::now();
    while IN_FLIGHT.load(Ordering::SeqCst) > 0 {
        if limit.is_some_and(|limit| start.elapsed() >= limit) {
            eprintln!("[http.stream_to] SOLI_HTTP_STREAM_EXIT_WAIT reached, exiting");
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(format: Format, raw: &str) -> Reader {
        let mut reader = Reader::new(format);
        let mut pending = raw.as_bytes().to_vec();
        while let Some(line) = take_line(&mut pending) {
            if reader.line(&line) {
                reader.take_fresh();
                return reader;
            }
        }
        reader.finish(&pending);
        reader.take_fresh();
        reader
    }

    #[test]
    fn sse_events_end_on_a_blank_line_and_join_their_data() {
        let mut parser = SseParser::default();
        assert_eq!(parser.line(": a comment"), None);
        assert_eq!(parser.line("event: note"), None);
        assert_eq!(parser.line("data: one"), None);
        assert_eq!(parser.line("data:two"), None);
        assert_eq!(
            parser.line(""),
            Some(SseEvent {
                event: Some("note".into()),
                data: "one\ntwo".into()
            })
        );
        assert_eq!(parser.line("event: ping"), None);
        assert_eq!(parser.line(""), None, "an event without data is dropped");
    }

    #[test]
    fn anthropic_text_stop_reason_and_usage_are_taken_out() {
        let raw = concat!(
            "event: message_start\r\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"output_tokens\":1}}}\r\n\r\n",
            "event: content_block_start\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: ping\ndata: {\"type\": \"ping\"}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"hmm\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\", world\"}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":15}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"after the end\"}}\n\n",
        );
        let reader = events(Format::Anthropic, raw);
        assert_eq!(reader.outcome.text, "Hello, world");
        assert_eq!(reader.outcome.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(
            reader.outcome.usage,
            Some(serde_json::json!({"input_tokens": 12, "output_tokens": 15}))
        );
        assert_eq!(reader.outcome.state, None, "ended well");
    }

    #[test]
    fn an_anthropic_error_event_fails_the_stream() {
        let raw = "event: error\ndata: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\",\"message\":\"Overloaded\"}}\n\n";
        let reader = events(Format::Anthropic, raw);
        assert_eq!(reader.outcome.state, Some(State::Failed));
        assert_eq!(reader.outcome.error.as_deref(), Some("Overloaded"));
    }

    #[test]
    fn openai_chunks_end_on_done() {
        let raw = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hi\"}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\" there\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"late\"}}]}\n\n",
        );
        let reader = events(Format::OpenAi, raw);
        assert_eq!(reader.outcome.text, "Hi there");
        assert_eq!(reader.outcome.stop_reason.as_deref(), Some("stop"));
        assert_eq!(
            reader.outcome.usage,
            Some(serde_json::json!({"prompt_tokens": 5, "completion_tokens": 2}))
        );
    }

    #[test]
    fn raw_formats_keep_their_pieces_apart() {
        let sse = events(Format::Sse, "data: a\n\ndata: b\ndata: c\n\ndata: last");
        assert_eq!(
            sse.outcome.text, "a\nb\nc\nlast",
            "an unended event still counts"
        );
        let lines = events(Format::Lines, "one\ntwo\r\nthree");
        assert_eq!(lines.outcome.text, "one\ntwo\nthree");
    }

    #[test]
    fn job_handlers_must_name_a_class() {
        assert!(is_class_name("AnswerDoneJob"));
        assert!(is_class_name("Billing::ChargeJob"));
        assert!(!is_class_name("answer_done"));
        assert!(!is_class_name("Answer Done"));
        assert!(!is_class_name(""));
    }

    /// A server answering `status` with `parts`, `gap` apart.
    fn server(status: &'static str, parts: Vec<&'static str>, gap: Duration) -> String {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback");
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 4096];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            );
            for part in parts {
                let _ = stream.write_all(part.as_bytes());
                let _ = stream.flush();
                std::thread::sleep(gap);
            }
        });
        format!("http://{addr}/stream")
    }

    /// Starts a stream past the SSRF check, which refuses the loopback
    /// servers these tests answer from (as `open_stream`'s tests do).
    fn start(url: &str, format: &str) -> String {
        let call = StreamCall {
            method: "POST".into(),
            url: url.into(),
            idle: Duration::from_secs(5),
            headers: Vec::new(),
            body: Some(OutgoingBody::Text("{}".into())),
            follow_redirects: true,
            response: ResponseMode::Text,
        };
        let target = Target {
            format: Format::parse(format).unwrap(),
            topic: None,
            job: None,
            args: serde_json::Map::new(),
        };
        begin(call, target).expect("stream_to starts")
    }

    fn status_of(id: &str) -> (String, String, Option<String>) {
        let Value::Hash(hash) = http_stream_status(&[Value::String(id.into())]).unwrap() else {
            panic!("a status hash");
        };
        let hash = hash.borrow();
        let get = |k: &str| hash.get(&HashKey::String(k.into())).cloned();
        let text = |v: Option<Value>| match v {
            Some(Value::String(s)) => Some(s.to_string()),
            _ => None,
        };
        (
            text(get("state")).unwrap(),
            text(get("text")).unwrap(),
            text(get("error")),
        )
    }

    fn wait_until_finished(id: &str) -> (String, String, Option<String>) {
        let start = Instant::now();
        loop {
            let status = status_of(id);
            if status.0 != "running" || start.elapsed() > Duration::from_secs(10) {
                return status;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_stream_is_read_in_the_background_to_its_end() {
        let url = server(
            "200 OK",
            vec![
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Bon\"}}\n\n",
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"jour\"}}\n\n",
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            ],
            Duration::from_millis(200),
        );
        let begun = Instant::now();
        let id = start(&url, "anthropic");
        assert!(
            begun.elapsed() < Duration::from_millis(150),
            "the call returns before the first chunk"
        );
        assert_eq!(status_of(&id).0, "running");
        assert_eq!(
            wait_until_finished(&id),
            ("done".to_string(), "Bonjour".to_string(), None)
        );
    }

    #[test]
    fn a_failing_answer_keeps_its_body_as_the_error() {
        let url = server(
            "500 Internal Server Error",
            vec!["{\"error\":\"down\"}"],
            Duration::ZERO,
        );
        let id = start(&url, "anthropic");
        assert_eq!(
            wait_until_finished(&id),
            (
                "failed".to_string(),
                String::new(),
                Some("{\"error\":\"down\"}".to_string())
            )
        );
    }

    #[test]
    fn a_cancelled_stream_stops_and_says_so() {
        let url = server(
            "200 OK",
            vec!["data: a\n\n", "data: b\n\n", "data: c\n\n"],
            Duration::from_millis(500),
        );
        let id = start(&url, "sse");
        std::thread::sleep(Duration::from_millis(100));
        assert!(matches!(
            http_stream_cancel(&[Value::String(id.clone().into())]).unwrap(),
            Value::Bool(true)
        ));
        let (state, _, error) = wait_until_finished(&id);
        assert_eq!(state, "cancelled");
        assert_eq!(error.as_deref(), Some("cancelled"));
        assert!(
            matches!(
                http_stream_cancel(&[Value::String(id.into())]).unwrap(),
                Value::Bool(false)
            ),
            "nothing left to cancel"
        );
    }

    #[test]
    fn a_wrong_call_raises_at_once() {
        let to = |pairs: Vec<(&str, Value)>| {
            Value::Hash(Rc::new(RefCell::new(
                pairs
                    .into_iter()
                    .map(|(k, v)| (HashKey::String(k.into()), v))
                    .collect(),
            )))
        };
        let call = |to: Value| target(Some(&to)).err().unwrap();
        assert!(call(to(vec![("format", Value::String("xml".into()))])).contains("unknown format"));
        assert!(call(to(vec![("job", Value::String("not a class".into()))])).contains("job class"));
        assert!(call(to(vec![("topik", Value::String("t".into()))])).contains("unknown key"));
        assert!(call(to(vec![("args", Value::Int(1))])).contains("must be a hash"));
        assert!(call(Value::Null).contains("fifth argument"));
    }
}
