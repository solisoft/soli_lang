//! Domain events and after-commit work.
//!
//! `publish(event)` hands a `DomainEvent` to the handlers `subscribe(...)`
//! registered for its class (or an ancestor):
//!
//! * a **job** handler — `subscribe(InvoicePaid, SendReceiptJob)` — is
//!   enqueued at once, with the event's fields as its arguments. Inside a
//!   transaction the job row is written by that transaction (`_jobs` lives on
//!   the default connection), so it commits or rolls back with the data:
//!   the transactional outbox, with the job queue's retries for delivery.
//! * a **block** handler — `subscribe(InvoicePaid) do |event| … end` — runs
//!   in this process once the data is committed: right away outside a
//!   transaction, after the outermost commit inside one, never if it rolls
//!   back.
//!
//! `after_commit { … }` queues a block the same way; the engine that ran the
//! transaction runs those (a block written in a VM-run action is a VM
//! closure), see [`take_after_commit`].
//!
//! A model records events on itself with `record_event(event)`; they are
//! published when the record is next saved, so an aggregate method that
//! raises an event before a refused save publishes nothing.
//!
//! Block handlers come from application files (`app/subscribers/`), which
//! the tree-walker loads, so they are `Function`s this module runs itself.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::executor::{ControlFlow, Interpreter};
use crate::interpreter::value::{Class, Function, Value};
use crate::span::Span;

/// The built-in base class of domain events.
pub const DOMAIN_EVENT: &str = "DomainEvent";

/// What runs when a matching event is published.
#[derive(Clone)]
enum Handler {
    Block(Rc<Function>),
    Job(String),
    /// `subscribe(Event, {"stream": "billing", "key": "number"})`: appended
    /// to an ES topic, through the outbox (`EventStreamJob`).
    Stream {
        topic: String,
        key: Option<String>,
    },
}

/// The built-in job that appends an event to an ES stream.
pub const EVENT_STREAM_JOB: &str = "EventStreamJob";

#[derive(Clone)]
struct Subscription {
    event: String,
    handler: Handler,
}

thread_local! {
    static SUBSCRIBERS: RefCell<Vec<Subscription>> = const { RefCell::new(Vec::new()) };
}

crate::request_local! {
    /// Events whose block handlers wait for the outermost commit.
    static PENDING_EVENTS: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
    /// `after_commit` blocks waiting for the outermost commit.
    static AFTER_COMMIT: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
    /// Nesting of dispatches: a handler that publishes runs its handlers
    /// inline, and a loop of them stops here instead of overflowing.
    static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

const MAX_DEPTH: usize = 32;

/// Forget every subscription — before the application's subscriber files
/// are loaded again.
pub fn clear_subscribers() {
    SUBSCRIBERS.with(|s| s.borrow_mut().clear());
}

/// Drop what waits for a commit that will not happen: a rollback, a failed
/// commit, the end of a request.
pub fn discard_pending() {
    PENDING_EVENTS.with(|p| p.borrow_mut().clear());
    AFTER_COMMIT.with(|a| a.borrow_mut().clear());
}

/// Whether `class` is `name` or inherits it.
pub fn class_is(class: &Class, name: &str) -> bool {
    class.name == name || class.superclass.as_ref().is_some_and(|s| class_is(s, name))
}

fn event_class(value: &Value) -> Option<Rc<Class>> {
    match value {
        Value::Instance(inst) => {
            let class = inst.borrow().class.clone();
            class_is(&class, DOMAIN_EVENT).then_some(class)
        }
        _ => None,
    }
}

/// `subscribe(EventClass) do |event| … end` / `subscribe(EventClass, JobClass)`.
pub fn subscribe(args: &[Value]) -> Result<Value, String> {
    let usage = "subscribe(EventClass) do |event| … end, or subscribe(EventClass, JobClass)";
    let event = match args.first() {
        Some(Value::Class(c)) if class_is(c, DOMAIN_EVENT) => c.name.clone(),
        Some(Value::Class(c)) => {
            return Err(format!(
                "subscribe: {} is not a DomainEvent — declare it `class {} < DomainEvent`",
                c.name, c.name
            ))
        }
        Some(Value::String(s)) => s.to_string(),
        _ => return Err(usage.to_string()),
    };
    let handler = match args.get(1) {
        Some(Value::Function(f)) => Handler::Block(f.clone()),
        Some(Value::Class(c)) => Handler::Job(c.name.clone()),
        Some(Value::String(s)) => Handler::Job(s.to_string()),
        Some(Value::Hash(options)) => stream_handler(&options.borrow())?,
        Some(Value::VmClosure(_)) => {
            return Err(
                "subscribe: a block handler belongs in app/subscribers/ (it was given from \
                 bytecode-compiled code, which cannot run after a commit)"
                    .to_string(),
            )
        }
        _ => return Err(usage.to_string()),
    };
    SUBSCRIBERS.with(|s| s.borrow_mut().push(Subscription { event, handler }));
    Ok(Value::Null)
}

fn stream_handler(options: &crate::interpreter::value::HashPairs) -> Result<Handler, String> {
    let mut topic = None;
    let mut key = None;
    for (name, value) in options.iter() {
        let crate::interpreter::value::HashKey::String(name) = name else {
            return Err("subscribe: stream options are \"stream\" and \"key\"".to_string());
        };
        match (name.as_ref(), value) {
            ("stream", Value::String(s)) => topic = Some(s.to_string()),
            ("key", Value::String(s)) | ("key", Value::Symbol(s)) => key = Some(s.to_string()),
            ("key", Value::Null) => {}
            (other, _) => {
                return Err(format!(
                "subscribe: unknown stream option \"{other}\" (\"stream\": topic, \"key\": field)"
            ))
            }
        }
    }
    let topic =
        topic.ok_or("subscribe: a stream subscription names its topic: {\"stream\": \"…\"}")?;
    Ok(Handler::Stream { topic, key })
}

fn subscriptions_for(class: &Class) -> Vec<Handler> {
    SUBSCRIBERS.with(|s| {
        s.borrow()
            .iter()
            .filter(|sub| class_is(class, &sub.event))
            .map(|sub| sub.handler.clone())
            .collect()
    })
}

/// `publish(event)`: enqueue its job handlers now, run its block handlers
/// now or after the outermost commit.
pub fn publish(event: &Value) -> Result<(), RuntimeError> {
    publish_with(event, true)
}

/// `publish`, with or without the stream subscriptions — an event consumed
/// from a stream is not appended to one again.
fn publish_with(event: &Value, to_streams: bool) -> Result<(), RuntimeError> {
    let class = event_class(event).ok_or_else(|| {
        RuntimeError::type_error(
            format!(
                "publish expects a DomainEvent (`class X < DomainEvent`), got {}",
                event.type_name()
            ),
            Span::default(),
        )
    })?;
    let handlers = subscriptions_for(&class);
    let mut has_blocks = false;
    for handler in &handlers {
        match handler {
            Handler::Job(job) => enqueue_job(job, event)?,
            Handler::Stream { topic, key } if to_streams => {
                enqueue_stream(&class, topic, key.as_deref(), event)?
            }
            Handler::Stream { .. } => {}
            Handler::Block(_) => has_blocks = true,
        }
    }
    if !has_blocks {
        return Ok(());
    }
    if crate::interpreter::builtins::model::crud::has_active_tx() {
        PENDING_EVENTS.with(|p| p.borrow_mut().push(event.clone()));
        Ok(())
    } else {
        run_blocks(&class, event)
    }
}

fn enqueue_job(job: &str, event: &Value) -> Result<(), RuntimeError> {
    let fields = match event {
        Value::Instance(inst) => crate::interpreter::domain::to_hash(&inst.borrow()),
        _ => Value::Null,
    };
    crate::interpreter::builtins::jobs::enqueue_named(job, fields)
        .map(|_| ())
        .map_err(|e| RuntimeError::new(format!("publish: enqueueing {job}: {e}"), Span::default()))
}

/// Enqueue `EventStreamJob` with the record to append: the event as
/// `{"type", "data", "occurred_at"}`, keyed by the named field.
fn enqueue_stream(
    class: &Class,
    topic: &str,
    key: Option<&str>,
    event: &Value,
) -> Result<(), RuntimeError> {
    let Value::Instance(inst) = event else {
        return Ok(());
    };
    let key_value = key.and_then(|field| inst.borrow().fields.get(field).cloned());
    let mut envelope = crate::interpreter::value::HashPairs::default();
    use crate::interpreter::value::HashKey;
    envelope.insert(
        HashKey::String("type".into()),
        Value::String(class.name.clone().into()),
    );
    envelope.insert(
        HashKey::String("data".into()),
        crate::interpreter::domain::to_hash(&inst.borrow()),
    );
    envelope.insert(
        HashKey::String("version".into()),
        Value::Int(event_version(class)),
    );
    envelope.insert(
        HashKey::String("occurred_at".into()),
        Value::String(crate::jobs::now_iso().into()),
    );
    let mut args = crate::interpreter::value::HashPairs::default();
    args.insert(HashKey::String("topic".into()), Value::String(topic.into()));
    args.insert(
        HashKey::String("key".into()),
        match key_value {
            Some(Value::Null) | None => Value::Null,
            Some(Value::String(s)) => Value::String(s),
            Some(other) => Value::String(other.to_string().into()),
        },
    );
    args.insert(
        HashKey::String("event".into()),
        Value::Hash(Rc::new(RefCell::new(envelope))),
    );
    crate::interpreter::builtins::jobs::enqueue_named(
        EVENT_STREAM_JOB,
        Value::Hash(Rc::new(RefCell::new(args))),
    )
    .map(|_| ())
    .map_err(|e| {
        RuntimeError::new(
            format!("publish: streaming to {topic}: {e}"),
            Span::default(),
        )
    })
}

/// `EventStreamJob.perform(args)`: append the event to its topic.
pub fn perform_event_stream(args: &[Value]) -> Result<Value, String> {
    let Some(Value::Hash(args)) = args.first() else {
        return Err("EventStreamJob.perform expects its arguments hash".to_string());
    };
    let args = args.borrow();
    let get = |name: &str| {
        args.iter()
            .find(|(k, _)| matches!(k, crate::interpreter::value::HashKey::String(s) if s.as_ref() == name))
            .map(|(_, v)| v.clone())
    };
    let topic = match get("topic") {
        Some(Value::String(s)) => s.to_string(),
        _ => return Err("EventStreamJob: no topic".to_string()),
    };
    let key = match get("key") {
        Some(Value::String(s)) => Some(s.to_string()),
        _ => None,
    };
    let event = get("event").unwrap_or(Value::Null);
    let value = serde_json::to_string(&crate::interpreter::value::value_to_json(&event)?)
        .map_err(|e| e.to_string())?;
    emit_to_stream(&topic, key, value)?;
    Ok(Value::Null)
}

#[cfg(feature = "es")]
fn emit_to_stream(topic: &str, key: Option<String>, value: String) -> Result<(), String> {
    crate::interpreter::builtins::es::emit_record(topic, key, value)
}

#[cfg(not(feature = "es"))]
fn emit_to_stream(_topic: &str, _key: Option<String>, _value: String) -> Result<(), String> {
    Err(NO_ES.to_string())
}

#[cfg(not(feature = "es"))]
const NO_ES: &str = "event streams need es support, which this build of soli leaves out";

fn run_blocks(class: &Class, event: &Value) -> Result<(), RuntimeError> {
    let depth = DEPTH.with(|d| d.get());
    if depth >= MAX_DEPTH {
        return Err(RuntimeError::new(
            format!(
                "publish: {} handlers published events {MAX_DEPTH} levels deep — a loop?",
                class.name
            ),
            Span::default(),
        ));
    }
    DEPTH.with(|d| d.set(depth + 1));
    let result = subscriptions_for(class)
        .iter()
        .try_for_each(|handler| match handler {
            Handler::Block(func) => call_block(func, event),
            Handler::Job(_) | Handler::Stream { .. } => Ok(()),
        });
    DEPTH.with(|d| d.set(depth));
    result
}

/// Run a handler block with the event as its parameter. A `throw` in it
/// keeps its value, so an uncaught `DomainError` still answers its status.
fn call_block(func: &Function, event: &Value) -> Result<(), RuntimeError> {
    let mut env = Environment::with_enclosing(func.closure.clone());
    if let Some(param) = func.params.first() {
        env.define(param.name.clone(), event.clone());
    }
    let mut interp = Interpreter::for_bound_body();
    if let Some(ref file) = func.source_path {
        interp.current_source_path = Some(std::path::PathBuf::from(file));
    }
    match interp.execute_block(&func.body, env)? {
        ControlFlow::Throw(value) => Err(RuntimeError::Thrown {
            value,
            span: func.span.unwrap_or_default(),
        }),
        _ => Ok(()),
    }
}

/// After the outermost commit: run the block handlers of the events
/// published inside the transaction. The `after_commit` blocks are the
/// engine's to run, with [`take_after_commit`].
pub fn after_outermost_commit() -> Result<(), RuntimeError> {
    let events = PENDING_EVENTS.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for event in &events {
        if let Some(class) = event_class(event) {
            run_blocks(&class, event)?;
        }
    }
    Ok(())
}

/// Queue an `after_commit` block (inside a transaction).
pub fn queue_after_commit(block: Value) {
    AFTER_COMMIT.with(|a| a.borrow_mut().push(block));
}

/// The `after_commit` blocks of the transaction that just committed, for the
/// engine to call. Empty while a transaction is still open (a nested
/// `transaction` block returned into its outer one).
pub fn take_after_commit() -> Vec<Value> {
    if crate::interpreter::builtins::model::crud::has_active_tx() {
        return Vec::new();
    }
    AFTER_COMMIT.with(|a| std::mem::take(&mut *a.borrow_mut()))
}

/// The field a model keeps its recorded, not yet published events in. The
/// leading `_` keeps it out of what is saved and serialized.
pub const RECORDED_EVENTS_FIELD: &str = "_recorded_events";

/// `record.record_event(event)`.
pub fn record_event(args: &[Value]) -> Result<Value, String> {
    let (Some(Value::Instance(record)), Some(event)) = (args.first(), args.get(1)) else {
        return Err("record_event(event) is called on a model instance".to_string());
    };
    if event_class(event).is_none() {
        return Err(format!(
            "record_event expects a DomainEvent (`class X < DomainEvent`), got {}",
            event.type_name()
        ));
    }
    let mut record = record.borrow_mut();
    match record.fields.get_mut(RECORDED_EVENTS_FIELD) {
        Some(Value::Array(events)) => events.borrow_mut().push(event.clone()),
        _ => {
            record.set(
                RECORDED_EVENTS_FIELD,
                Value::Array(Rc::new(RefCell::new(vec![event.clone()]))),
            );
        }
    }
    Ok(Value::Null)
}

/// Publish what `record` recorded, once it is saved.
pub fn publish_recorded(record: &Value) -> Result<(), String> {
    let Value::Instance(inst) = record else {
        return Ok(());
    };
    let events = match inst.borrow_mut().fields.remove(RECORDED_EVENTS_FIELD) {
        Some(Value::Array(events)) => events.borrow().clone(),
        _ => return Ok(()),
    };
    for event in &events {
        publish(event).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `EventStream.consume(topic, {"group": "shipping", "partitions": 1,
/// "max_records": 100})`: read what consumer group `group` has not read yet
/// from an es topic, rebuild each record as its `DomainEvent` and publish it
/// here — its block and job subscribers run, its stream subscriptions do not
/// — committing the group's offset after each one. A record whose event
/// class this application does not have is skipped. Returns how many events
/// were published. Delivery is at least once: a handler that fails stops the
/// run before its record is committed, so the next run sees it again.
pub fn consume_stream(args: &[Value]) -> Result<Value, String> {
    let usage = "EventStream.consume(topic, {\"group\": …, \"partitions\": n, \"max_records\": n})";
    let Some(Value::String(topic)) = args.first() else {
        return Err(usage.to_string());
    };
    let mut group = None;
    let mut partitions = 1u64;
    let mut max_records = 100u64;
    if let Some(Value::Hash(options)) = args.get(1) {
        for (name, value) in options.borrow().iter() {
            let crate::interpreter::value::HashKey::String(name) = name else {
                return Err(usage.to_string());
            };
            match (name.as_ref(), value) {
                ("group", Value::String(g)) => group = Some(g.to_string()),
                ("partitions", Value::Int(n)) if *n > 0 => partitions = *n as u64,
                ("max_records", Value::Int(n)) if *n > 0 => max_records = *n as u64,
                (other, _) => {
                    return Err(format!(
                        "EventStream.consume: bad option \"{other}\" — {usage}"
                    ))
                }
            }
        }
    }
    let group =
        group.ok_or_else(|| format!("EventStream.consume needs a consumer group: {usage}"))?;
    consume_records(topic, &group, partitions, max_records)
}

#[cfg(not(feature = "es"))]
fn consume_records(_: &str, _: &str, _: u64, _: u64) -> Result<Value, String> {
    Err(NO_ES.to_string())
}

#[cfg(feature = "es")]
fn consume_records(
    topic: &str,
    group: &str,
    partitions: u64,
    max_records: u64,
) -> Result<Value, String> {
    let mut published = 0i64;
    for partition in 0..partitions {
        let page =
            crate::interpreter::builtins::es::group_page(group, topic, partition, max_records)?;
        let records = page
            .get("records")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        for record in records {
            let offset = record.get("offset").and_then(|o| o.as_u64()).unwrap_or(0);
            if let Some(event) = event_from_record(&record)? {
                publish_with(&event, false).map_err(|e| e.to_string())?;
                published += 1;
            }
            crate::interpreter::builtins::es::commit_offset(group, topic, partition, offset + 1)?;
        }
    }
    Ok(Value::Int(published))
}

/// A stream record (`{"type", "data", …}` as a stream subscription writes
/// it) as its domain event, when this application declares that class.
///
/// A record written by an older version of the event (`"version"` below the
/// class's `static version`) goes through the class's
/// `static def upcast(data, from_version)` first; with no `upcast`, or from a
/// newer version than the class knows, it is an error — the run stops before
/// committing the record rather than lose it.
#[cfg_attr(not(feature = "es"), allow(dead_code))]
fn event_from_record(record: &serde_json::Value) -> Result<Option<Value>, String> {
    let Some(value) = record.get("value") else {
        return Ok(None);
    };
    let envelope: serde_json::Value = match value {
        serde_json::Value::String(text) => match serde_json::from_str(text) {
            Ok(json) => json,
            Err(_) => return Ok(None),
        },
        other => other.clone(),
    };
    let (Some(class_name), Some(data)) = (
        envelope.get("type").and_then(|t| t.as_str()),
        envelope.get("data"),
    ) else {
        return Ok(None);
    };
    let data = crate::interpreter::value_json::json_to_value_or_null(data.clone());
    let written = envelope
        .get("version")
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    rebuild_event(class_name, data, written)
}

/// The event `class_name` from its stored fields, written by version
/// `written` of the class: upcast when older, an error when newer or when
/// an older one has no `upcast`. `None` when this application does not
/// declare that event class.
fn rebuild_event(class_name: &str, mut data: Value, written: i64) -> Result<Option<Value>, String> {
    let Some(class) = crate::interpreter::domain::registered_class(class_name) else {
        return Ok(None);
    };
    if !class_is(&class, DOMAIN_EVENT) {
        return Ok(None);
    }
    let current = event_version(&class);
    if written > current {
        return Err(format!(
            "{class_name} version {written} on the stream, but this application knows \
             version {current}: deploy the newer event class first"
        ));
    }
    if written < current {
        let upcast = class.find_static_method("upcast").ok_or_else(|| {
            format!(
                "{class_name} version {written} on the stream, version {current} here, and \
                 no `static def upcast(data, from_version)` to bring it up to date"
            )
        })?;
        data = call_function(&upcast, vec![data, Value::Int(written)])
            .map_err(|e| format!("{class_name}.upcast: {e}"))?;
    }
    Ok(crate::interpreter::domain::hydrate(class_name, &data))
}

/// The `static version` an event class declares (or inherits); 1 without.
fn event_version(class: &Class) -> i64 {
    if let Some(Value::Int(version)) = class.static_fields.borrow().get("version") {
        return *version;
    }
    class.superclass.as_deref().map(event_version).unwrap_or(1)
}

/// Call a tree-walker function with positional arguments.
fn call_function(func: &Function, args: Vec<Value>) -> Result<Value, RuntimeError> {
    let mut env = Environment::with_enclosing(func.closure.clone());
    for (param, arg) in func.params.iter().zip(args) {
        env.define(param.name.clone(), arg);
    }
    let mut interp = Interpreter::for_bound_body();
    match interp.execute_block(&func.body, env)? {
        ControlFlow::Throw(value) => Err(RuntimeError::Thrown {
            value,
            span: func.span.unwrap_or_default(),
        }),
        ControlFlow::Return(v) | ControlFlow::Normal(v) => Ok(v),
        _ => Ok(Value::Null),
    }
}

/// Event-sourced aggregates, in Soli: the state of an `EventSourced`
/// subclass is what its `apply(event)` makes of the events it emitted,
/// replayed from an append-only store. Each event is stored under the key
/// `<stream>.<version>`, so two writers appending the same version collide
/// on the key — optimistic concurrency, answered with `StaleRecord`.
pub const EVENT_SOURCED_PRELUDE: &str = r##"
class SoliEventStoreRecord < Model
end

class EventSourced
  id: Any = nil
  version: Int = 0
  pending_events: Array = []

  # `Account.load(id)`: the aggregate rebuilt from its stream.
  static def load(id)
    this.new().__load(id)
  end

  def __load(id)
    @id = id
    @version = 0
    @pending_events = []
    @__replay
    self
  end

  def apply(event)
    throw "#{self.class} must define apply(event) to change its state from an event"
  end

  def emit(event)
    @apply(event)
    @pending_events = (@pending_events ?? []) + [event]
    event
  end

  def save
    pending = @pending_events ?? []
    return true if pending.length == 0

    @id = uuid_v4() if @id.nil?
    loaded_version = @version
    try
      SoliEventStoreRecord.transaction do
        pending.each do |event|
          @version = @version + 1
          record = SoliEventStoreRecord.create({
            "stream": @__stream,
            "version": @version,
            "event_type": event.class,
            "event_version": __event_version(event),
            "data": event.to_h
          }, {"key": @__event_key})
          @__refuse(record._errors, loaded_version) if record._errors
          publish(event)
        end
      end
    catch StaleRecord error
      @version = loaded_version
      throw error
    catch error
      @version = loaded_version
      # SoliDB reports a key taken by a concurrent writer at commit.
      throw @__stale(loaded_version) if "#{error}".includes?("already exists")

      throw error
    end
    @pending_events = []
    true
  end

  def __event_key -> String
    "#{@__stream}.#{@version}".replace("::", ".").replace(":", ".")
  end

  def __refuse(errors, loaded_version)
    throw @__stale(loaded_version) if errors.any? { |e| e["message"] == "has already been taken" }

    throw "#{@__stream}: event #{@version} could not be stored: #{errors}"
  end

  def __stale(loaded_version)
    StaleRecord.new("#{@__stream} changed since it was loaded at version #{loaded_version}")
  end

  def __stream -> String
    "#{self.class}:#{@id}"
  end

  def __replay
    records = SoliEventStoreRecord.where({"stream": @__stream}).order("version", "asc").all
    records.each do |record|
      event = __rebuild_event(record.event_type, record.data, record.event_version)
      @apply(event) unless event.nil?
      @version = record.version
    end
  end
end
"##;

/// Define `EventSourced` (and its store model) in `interpreter` if not
/// already present.
pub fn ensure_prelude(interpreter: &mut Interpreter) {
    if interpreter
        .global_env()
        .borrow()
        .get("EventSourced")
        .is_some()
    {
        return;
    }
    let program = crate::lexer::Scanner::new(EVENT_SOURCED_PRELUDE)
        .scan_tokens()
        .map_err(|e| e.to_string())
        .and_then(|tokens| {
            crate::parser::Parser::new(tokens)
                .parse()
                .map_err(|e| e.to_string())
        });
    match program {
        Ok(program) => {
            if let Err(e) = interpreter.interpret(&program) {
                eprintln!("event-sourcing prelude: {e}");
            }
        }
        Err(e) => eprintln!("event-sourcing prelude: {e}"),
    }
}

/// `__rebuild_event(type, data, version)`: a stored event back as its
/// `DomainEvent` (upcast when older); `nil` for a class this application
/// does not declare.
pub fn rebuild_event_native(args: &[Value]) -> Result<Value, String> {
    let (Some(Value::String(class_name)), Some(data)) = (args.first(), args.get(1)) else {
        return Err("__rebuild_event(type, data, version)".to_string());
    };
    let written = match args.get(2) {
        Some(Value::Int(v)) => *v,
        _ => 1,
    };
    Ok(rebuild_event(class_name, data.clone(), written)?.unwrap_or(Value::Null))
}

/// `__event_version(event)`: the `static version` of an event's class.
pub fn event_version_native(args: &[Value]) -> Result<Value, String> {
    match args.first().and_then(event_class) {
        Some(class) => Ok(Value::Int(event_version(&class))),
        None => Err("__event_version expects a DomainEvent".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_record_names_its_event_class() {
        // No such class here: the record is skipped, not an error.
        let record = serde_json::json!({
            "offset": 0,
            "key": "F-1",
            "value": r#"{"type":"Billing::InvoicePaid","data":{"number":"F-1"}}"#
        });
        assert!(matches!(event_from_record(&record), Ok(None)));
        assert!(matches!(
            event_from_record(&serde_json::json!({"value": "not json"})),
            Ok(None)
        ));
    }
}
