//! `ES` — the driver for es, the Kafka-shaped event log (`../es`,
//! github.com/solisoft/event_stream).
//!
//! Two transports, one class:
//!
//! * **Produce, consume and ping** go over the broker's binary protocol when
//!   `ES_BINARY` names its listener (`es-broker --bind-binary`): one
//!   long-lived connection per worker, raw bytes, no JSON. Without it they go
//!   over HTTP, so an app works against a broker that has no binary listener.
//! * **Everything else** — topics, consumer groups, commits — exists only on
//!   the HTTP API, and goes there.
//!
//! The binary client is `es_client::Client`, blocking `std::net`: Soli's
//! builtins are synchronous and each worker is its own thread.
//!
//! Settings are per application (`TenantValue`), like SoliKV's: two apps in
//! one server talk to their own brokers, with their own tokens.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{json, Value as Json};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{json_to_value, Class, HashKey, HashPairs, NativeFunction, Value};
use crate::serve::tenant::TenantValue;

const DEFAULT_URL: &str = "http://127.0.0.1:9000";
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const DEFAULT_MAX_RECORDS: u64 = 100;
const DEFAULT_MAX_BYTES: u64 = 1 << 20;
/// Idle binary connections kept per application. A worker holds one only
/// for the length of a call, so this is about the number of workers.
const MAX_IDLE: usize = 32;
/// An idle connection older than this is closed rather than reused: the
/// broker drops idle connections after `--binary-idle-timeout` (10 minutes
/// by default), and `is_closed` cannot see a close still in flight.
const MAX_IDLE_AGE: Duration = Duration::from_secs(300);

/// Where the broker is, as the application configured it.
#[derive(Clone)]
struct EsConfig {
    /// HTTP base URL, no trailing slash.
    url: String,
    /// `host:port` of the binary listener, when there is one.
    binary: Option<String>,
    token: Option<String>,
    gzip: bool,
    /// PEM file of the CA that signed the broker's certificate.
    ca_file: Option<String>,
    timeout: Duration,
}

impl EsConfig {
    fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Self {
            url: var("ES_BROKER")
                .map(|u| u.trim_end_matches('/').to_string())
                .unwrap_or_else(|| DEFAULT_URL.to_string()),
            binary: var("ES_BINARY"),
            token: var("ES_AUTH"),
            gzip: var("ES_GZIP").is_some_and(|v| !matches!(v.as_str(), "0" | "false" | "no")),
            ca_file: var("ES_CA_FILE"),
            timeout: Duration::from_secs(
                var("ES_TIMEOUT")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_TIMEOUT_SECS),
            ),
        }
    }

    fn tls(&self) -> bool {
        self.url.starts_with("https://")
    }
}

/// One application's broker: its settings, and what is built from them.
struct EsState {
    config: EsConfig,
    /// Bumped by `ES.configure`, so a connection made under the old settings
    /// is not handed back to the pool.
    generation: u64,
    agent: Option<ureq::Agent>,
    /// Binary connection options, TLS roots included; built once.
    options: Option<es_client::Options>,
    idle: Vec<(es_client::Client, Instant, u64)>,
}

static STATE: TenantValue<EsState> = TenantValue::new(|| EsState {
    config: EsConfig::from_env(),
    generation: 0,
    agent: None,
    options: None,
    idle: Vec::new(),
});

fn rustls_config(config: &EsConfig) -> Result<std::sync::Arc<rustls::ClientConfig>, String> {
    let mut roots = rustls::RootCertStore::empty();
    match &config.ca_file {
        Some(path) => {
            use rustls_pki_types::pem::PemObject;
            let certs = rustls_pki_types::CertificateDer::pem_file_iter(path)
                .map_err(|e| format!("ES: cannot read ca_file {path}: {e}"))?;
            for cert in certs {
                let cert = cert.map_err(|e| format!("ES: ca_file {path}: {e}"))?;
                roots
                    .add(cert)
                    .map_err(|e| format!("ES: ca_file {path}: {e}"))?;
            }
        }
        None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
    }
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("ES: TLS setup: {e}"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(std::sync::Arc::new(tls))
}

/// The HTTP agent and settings for this application, built on first use.
fn http_agent() -> Result<(ureq::Agent, EsConfig), String> {
    STATE.write(|state| {
        if state.agent.is_none() {
            let mut builder = ureq::AgentBuilder::new().timeout(state.config.timeout);
            if state.config.ca_file.is_some() {
                builder = builder.tls_config(rustls_config(&state.config)?);
            }
            state.agent = Some(builder.build());
        }
        Ok((
            state.agent.clone().unwrap_or_else(ureq::agent),
            state.config.clone(),
        ))
    })
}

// ----- HTTP -----

/// Percent-encode one path segment.
fn seg(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

/// Call the HTTP API. `Ok(None)` for a `204 No Content`.
fn http(op: &str, method: &str, path: &str, body: Option<Json>) -> Result<Option<Json>, String> {
    let (agent, config) = http_agent()?;
    let url = format!("{}{}", config.url, path);
    let mut request = agent.request(method, &url);
    if let Some(token) = &config.token {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let result = match body {
        Some(body) => request.send_json(body),
        None => request.call(),
    };
    let response = match result {
        Ok(response) => response,
        Err(ureq::Error::Status(status, response)) => {
            let text = response.into_string().unwrap_or_default();
            let message = serde_json::from_str::<Json>(&text)
                .ok()
                .and_then(|j| j.get("error").and_then(Json::as_str).map(str::to_string))
                .unwrap_or(text);
            return Err(status_error(op, status, &message, &config));
        }
        Err(e) => return Err(format!("{op}: cannot reach es at {}: {e}", config.url)),
    };
    if response.status() == 204 {
        return Ok(None);
    }
    let text = response
        .into_string()
        .map_err(|e| format!("{op}: reading the response: {e}"))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("{op}: the broker answered something that is not JSON: {e}"))
}

fn status_error(op: &str, status: u16, message: &str, config: &EsConfig) -> String {
    match status {
        401 if config.token.is_none() => {
            format!("{op}: the broker requires a token — set ES_AUTH ({message})")
        }
        401 => format!("{op}: the token was refused — check ES_AUTH ({message})"),
        403 => format!("{op}: forbidden — the key lacks a grant ({message})"),
        _ => format!("{op}: {message} ({status})"),
    }
}

fn http_json(op: &str, method: &str, path: &str, body: Option<Json>) -> Result<Json, String> {
    http(op, method, path, body)?.ok_or_else(|| format!("{op}: the broker answered with no body"))
}

// ----- binary -----

/// Run `call` on a pooled binary connection, or a new one.
///
/// A connection goes back to the pool after success or a server error (an
/// unknown topic leaves it usable) and is dropped on anything else. An
/// idempotent call — consume, ping — that fails on a reused connection is
/// tried once more on a fresh one; a produce is not, because the broker may
/// have appended before the connection broke.
fn with_binary<R>(
    op: &str,
    idempotent: bool,
    mut call: impl FnMut(&mut es_client::Client) -> es_client::Result<R>,
) -> Result<R, String> {
    let (pooled, options, addr, generation) = STATE.write(|state| {
        let addr = state.config.binary.clone().unwrap_or_default();
        if state.options.is_none() {
            let tls = if state.config.tls() {
                let host = addr.rsplit_once(':').map_or(addr.as_str(), |(h, _)| h);
                Some(es_client::Tls {
                    config: rustls_config(&state.config)?,
                    server_name: host
                        .trim_start_matches('[')
                        .trim_end_matches(']')
                        .to_string(),
                })
            } else {
                None
            };
            state.options = Some(es_client::Options {
                token: state.config.token.clone().unwrap_or_default(),
                gzip: state.config.gzip,
                tls,
                connect_timeout: Duration::from_secs(5),
                io_timeout: Some(state.config.timeout),
            });
        }
        let mut pooled = None;
        while let Some((mut client, since, gen)) = state.idle.pop() {
            if gen == state.generation && since.elapsed() < MAX_IDLE_AGE && !client.is_closed() {
                pooled = Some(client);
                break;
            }
        }
        Ok::<_, String>((pooled, state.options.clone(), addr, state.generation))
    })?;
    let options = options.unwrap_or_default();
    let connect = || {
        es_client::Client::connect(addr.as_str(), &options)
            .map_err(|e| format!("{op}: cannot connect to the es binary listener at {addr}: {e}"))
    };

    let reused = pooled.is_some();
    let mut client = match pooled {
        Some(client) => client,
        None => connect()?,
    };
    let result = match call(&mut client) {
        Err(e) if reused && idempotent && !e.connection_usable() => {
            client = connect()?;
            call(&mut client)
        }
        other => other,
    };
    match result {
        Ok(value) => {
            give_back(client, generation);
            Ok(value)
        }
        Err(e) if e.connection_usable() => {
            give_back(client, generation);
            Err(format!("{op}: {e}"))
        }
        Err(e) => Err(format!("{op}: {e} (binary listener {addr})")),
    }
}

fn give_back(client: es_client::Client, generation: u64) {
    STATE.write(|state| {
        if state.generation == generation && state.idle.len() < MAX_IDLE {
            state.idle.push((client, Instant::now(), generation));
        }
    });
}

fn binary_configured() -> bool {
    STATE.read(|state| state.config.binary.is_some())
}

// ----- arguments -----

fn string_arg(op: &str, args: &[Value], i: usize, name: &str) -> Result<String, String> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.to_string()),
        Some(other) => Err(format!(
            "{op}: {name} must be a String, got {}",
            other.type_name()
        )),
        None => Err(format!("{op}: missing argument {name}")),
    }
}

fn uint_arg(op: &str, args: &[Value], i: usize, name: &str) -> Result<u64, String> {
    match args.get(i) {
        Some(Value::Int(n)) if *n >= 0 => Ok(*n as u64),
        Some(Value::Int(n)) => Err(format!("{op}: {name} must not be negative, got {n}")),
        Some(other) => Err(format!(
            "{op}: {name} must be an Int, got {}",
            other.type_name()
        )),
        None => Err(format!("{op}: missing argument {name}")),
    }
}

fn arity(op: &str, args: &[Value], min: usize, max: usize, shape: &str) -> Result<(), String> {
    if args.len() < min || args.len() > max {
        return Err(format!(
            "{op}{shape} takes {min} to {max} arguments, got {}",
            args.len()
        ));
    }
    Ok(())
}

/// An options hash as JSON, refusing keys outside `allowed`.
fn options(
    op: &str,
    value: Option<&Value>,
    allowed: &[&str],
) -> Result<serde_json::Map<String, Json>, String> {
    let map = match value {
        None | Some(Value::Null) => return Ok(serde_json::Map::new()),
        Some(v @ Value::Hash(_)) => match crate::interpreter::value::value_to_json(v)? {
            Json::Object(map) => map,
            _ => serde_json::Map::new(),
        },
        Some(other) => {
            return Err(format!(
                "{op}: options must be a Hash, got {}",
                other.type_name()
            ))
        }
    };
    if let Some(key) = map.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!(
            "{op}: unknown option \"{key}\" (expected {})",
            allowed
                .iter()
                .map(|k| format!("\"{k}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(map)
}

fn opt_u64(
    op: &str,
    map: &serde_json::Map<String, Json>,
    key: &str,
) -> Result<Option<u64>, String> {
    match map.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("{op}: \"{key}\" must be a non-negative Int, got {v}")),
    }
}

fn opt_str(
    op: &str,
    map: &serde_json::Map<String, Json>,
    key: &str,
) -> Result<Option<String>, String> {
    match map.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::String(s)) => Ok(Some(s.clone())),
        Some(v) => Err(format!("{op}: \"{key}\" must be a String, got {v}")),
    }
}

/// `"base64"` → true; absent or `"utf8"` → false.
fn wants_base64(op: &str, map: &serde_json::Map<String, Json>) -> Result<bool, String> {
    match opt_str(op, map, "encoding")?.as_deref() {
        None | Some("utf8") | Some("utf-8") => Ok(false),
        Some("base64") => Ok(true),
        Some(other) => Err(format!(
            "{op}: unknown encoding \"{other}\" (use \"utf8\" or \"base64\")"
        )),
    }
}

/// A record's value: a String as it is, anything else as JSON.
fn payload(op: &str, value: &Value) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.to_string()),
        Value::Null => Err(format!("{op}: a record needs a value")),
        other => serde_json::to_string(&crate::interpreter::value::value_to_json(other)?)
            .map_err(|e| format!("{op}: {e}")),
    }
}

struct Record {
    key: Option<String>,
    value: String,
    partition: Option<u32>,
    sequence: Option<i64>,
}

fn record(op: &str, value: &Value) -> Result<Record, String> {
    let Value::Hash(hash) = value else {
        return Ok(Record {
            key: None,
            value: payload(op, value)?,
            partition: None,
            sequence: None,
        });
    };
    let hash = hash.borrow();
    let mut out = Record {
        key: None,
        value: String::new(),
        partition: None,
        sequence: None,
    };
    let mut has_value = false;
    for (k, v) in hash.iter() {
        let HashKey::String(name) = k else {
            return Err(format!("{op}: record keys are strings, got {k}"));
        };
        match (&**name, v) {
            (_, Value::Null) => {}
            ("value", v) => {
                out.value = payload(op, v)?;
                has_value = true;
            }
            ("key", Value::String(s)) => out.key = Some(s.to_string()),
            ("key", other) => out.key = Some(format!("{other}")),
            ("partition", Value::Int(n)) if *n >= 0 && *n <= u32::MAX as i64 => {
                out.partition = Some(*n as u32)
            }
            ("sequence", Value::Int(n)) => out.sequence = Some(*n),
            ("partition" | "sequence", other) => {
                return Err(format!(
                    "{op}: a record's \"{name}\" must be an Int, got {}",
                    other.type_name()
                ))
            }
            (other, _) => {
                return Err(format!(
                    "{op}: unknown record field \"{other}\" (expected \"key\", \"value\", \"partition\", \"sequence\")"
                ))
            }
        }
    }
    if !has_value {
        return Err(format!("{op}: a record needs a \"value\""));
    }
    Ok(out)
}

// ----- results -----

fn to_value(json: Json) -> Result<Value, String> {
    json_to_value(json)
}

fn bytes_to_text(bytes: Vec<u8>, base64: bool) -> String {
    if base64 {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    } else {
        String::from_utf8(bytes)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    }
}

// ----- operations -----

fn produce(
    op: &str,
    topic: &str,
    records: Vec<Record>,
    producer_id: Option<String>,
) -> Result<Json, String> {
    if let Some(pid) = &producer_id {
        if records.iter().any(|r| r.sequence.is_none()) {
            return Err(format!(
                "{op}: with producer_id \"{pid}\" every record needs a \"sequence\""
            ));
        }
    }
    if binary_configured() {
        let wire: Vec<es_client::WireProduceRecord> = records
            .into_iter()
            .map(|r| es_client::WireProduceRecord {
                key: r.key.map(String::into_bytes),
                value: r.value.into_bytes(),
                partition: r.partition,
                sequence: r.sequence,
            })
            .collect();
        let results = with_binary(op, false, |client| {
            client.produce(topic, producer_id.as_deref(), wire.clone())
        })?;
        return Ok(Json::Array(
            results
                .into_iter()
                .map(|r| json!({"partition": r.partition, "offset": r.offset, "duplicate": r.duplicate}))
                .collect(),
        ));
    }
    let body = json!({
        "records": records.into_iter().map(|r| {
            let mut rec = json!({"value": r.value});
            if let Some(k) = r.key { rec["key"] = json!(k); }
            if let Some(p) = r.partition { rec["partition"] = json!(p); }
            if let Some(s) = r.sequence { rec["sequence"] = json!(s); }
            rec
        }).collect::<Vec<_>>(),
        "producer_id": producer_id,
    });
    let response = http_json(
        op,
        "POST",
        &format!("/topics/{}/produce", seg(topic)),
        Some(body),
    )?;
    let mut results = response.get("results").cloned().unwrap_or(json!([]));
    // HTTP leaves `duplicate` out when false; the binary path always says.
    if let Some(list) = results.as_array_mut() {
        for r in list {
            if r.get("duplicate").is_none() {
                r["duplicate"] = json!(false);
            }
        }
    }
    Ok(results)
}

fn consume(
    op: &str,
    topic: &str,
    partition: u64,
    offset: u64,
    opts: &serde_json::Map<String, Json>,
) -> Result<Json, String> {
    let max_records = opt_u64(op, opts, "max_records")?.unwrap_or(DEFAULT_MAX_RECORDS);
    let max_bytes = opt_u64(op, opts, "max_bytes")?.unwrap_or(DEFAULT_MAX_BYTES);
    let base64 = wants_base64(op, opts)?;
    let partition_u32 = u32::try_from(partition)
        .map_err(|_| format!("{op}: partition {partition} is out of range"))?;
    if binary_configured() {
        let request = es_client::WireConsumeRequest {
            topic: topic.to_string(),
            partition: partition_u32,
            offset,
            max_records: u32::try_from(max_records).unwrap_or(u32::MAX),
            max_bytes: u32::try_from(max_bytes).unwrap_or(u32::MAX),
        };
        let page = with_binary(op, true, |client| client.consume(&request))?;
        let records: Vec<Json> = page
            .records
            .into_iter()
            .map(|r| {
                json!({
                    "partition": r.partition,
                    "offset": r.offset,
                    "timestamp_ms": r.timestamp_ms,
                    "key": r.key.map(|k| bytes_to_text(k, base64)),
                    "value": bytes_to_text(r.value, base64),
                })
            })
            .collect();
        return Ok(json!({
            "records": records,
            "next_offset": page.next_offset,
            "high_watermark": page.high_watermark,
        }));
    }
    let mut path = format!(
        "/topics/{}/consume?partition={partition}&offset={offset}&max_records={max_records}&max_bytes={max_bytes}",
        seg(topic)
    );
    if base64 {
        path.push_str("&encoding=base64");
    }
    page_shape(http_json(op, "GET", &path, None)?)
}

/// A consume page as `ES` returns it: every record has a `key`, `nil` when
/// absent, and the HTTP-only `encoding` marker is dropped.
fn page_shape(mut page: Json) -> Result<Json, String> {
    if let Some(obj) = page.as_object_mut() {
        obj.remove("encoding");
    }
    if let Some(records) = page.get_mut("records").and_then(Json::as_array_mut) {
        for r in records {
            if r.get("key").is_none() {
                r["key"] = Json::Null;
            }
        }
    }
    Ok(page)
}

/// `{"orders": {"0": 12}}` from the broker, as `{"orders": {0: 12}}`: a
/// partition is an Int everywhere else in `ES`.
fn offsets_value(json: Json) -> Result<Value, String> {
    let mut topics = HashPairs::default();
    if let Some(map) = json.get("offsets").and_then(Json::as_object) {
        for (topic, parts) in map {
            let mut partitions = HashPairs::default();
            if let Some(parts) = parts.as_object() {
                for (p, offset) in parts {
                    let key = p
                        .parse::<i64>()
                        .map(HashKey::Int)
                        .unwrap_or_else(|_| HashKey::String(p.as_str().into()));
                    partitions.insert(key, to_value(offset.clone())?);
                }
            }
            topics.insert(
                HashKey::String(topic.as_str().into()),
                Value::Hash(Rc::new(RefCell::new(partitions))),
            );
        }
    }
    Ok(Value::Hash(Rc::new(RefCell::new(topics))))
}

fn config_value() -> Result<Value, String> {
    let config = STATE.read(|state| state.config.clone());
    to_value(json!({
        "url": config.url,
        "binary": config.binary,
        "token_set": config.token.is_some(),
        "gzip": config.gzip,
        "ca_file": config.ca_file,
        "timeout": config.timeout.as_secs(),
        "transport": if config.binary.is_some() { "binary" } else { "http" },
    }))
}

fn configure(op: &str, args: &[Value]) -> Result<Value, String> {
    let opts = options(
        op,
        args.first(),
        &["url", "binary", "token", "gzip", "ca_file", "timeout"],
    )?;
    let string_or_nil = |key: &str| -> Result<Option<Option<String>>, String> {
        match opts.get(key) {
            None => Ok(None),
            Some(Json::Null) => Ok(Some(None)),
            Some(Json::String(s)) if s.trim().is_empty() => Ok(Some(None)),
            Some(Json::String(s)) => Ok(Some(Some(s.clone()))),
            Some(v) => Err(format!("{op}: \"{key}\" must be a String or nil, got {v}")),
        }
    };
    let url = string_or_nil("url")?;
    let binary = string_or_nil("binary")?;
    let token = string_or_nil("token")?;
    let ca_file = string_or_nil("ca_file")?;
    let gzip = match opts.get("gzip") {
        None => None,
        Some(Json::Bool(b)) => Some(*b),
        Some(v) => return Err(format!("{op}: \"gzip\" must be true or false, got {v}")),
    };
    let timeout = opt_u64(op, &opts, "timeout")?;
    if let Some(Some(url)) = &url {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!(
                "{op}: \"url\" is the broker's HTTP address (http:// or https://), got \"{url}\""
            ));
        }
    }
    STATE.write(|state| {
        let c = &mut state.config;
        if let Some(url) = url {
            c.url = url
                .map(|u| u.trim_end_matches('/').to_string())
                .unwrap_or_else(|| DEFAULT_URL.to_string());
        }
        if let Some(binary) = binary {
            c.binary = binary;
        }
        if let Some(token) = token {
            c.token = token;
        }
        if let Some(ca_file) = ca_file {
            c.ca_file = ca_file;
        }
        if let Some(gzip) = gzip {
            c.gzip = gzip;
        }
        if let Some(secs) = timeout {
            c.timeout = Duration::from_secs(secs.max(1));
        }
        state.generation += 1;
        state.agent = None;
        state.options = None;
        state.idle.clear();
    });
    config_value()
}

// ----- registration -----

type Op = fn(&str, &[Value]) -> Result<Value, String>;

pub fn register_es_builtins(env: &mut Environment) {
    let ops: [(&str, Option<usize>, Op); 19] = [
        ("configure", Some(1), configure),
        ("config", Some(0), |_, _| config_value()),
        ("ping", Some(0), |op, _| {
            if binary_configured() {
                with_binary(op, true, |client| client.ping())?;
            } else {
                http_json(op, "GET", "/healthz", None)?;
            }
            Ok(Value::Bool(true))
        }),
        ("produce", None, |op, args| {
            arity(op, args, 2, 3, "(topic, records, options)")?;
            let topic = string_arg(op, args, 0, "topic")?;
            let records = match &args[1] {
                Value::Array(items) => items
                    .borrow()
                    .iter()
                    .map(|v| record(op, v))
                    .collect::<Result<Vec<_>, _>>()?,
                other => {
                    return Err(format!(
                        "{op}: records must be an Array, got {} — ES.emit sends one",
                        other.type_name()
                    ))
                }
            };
            if records.is_empty() {
                return Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))));
            }
            let opts = options(op, args.get(2), &["producer_id"])?;
            let producer_id = opt_str(op, &opts, "producer_id")?;
            to_value(produce(op, &topic, records, producer_id)?)
        }),
        ("emit", None, |op, args| {
            arity(op, args, 3, 4, "(topic, key, value, options)")?;
            let topic = string_arg(op, args, 0, "topic")?;
            let key = match &args[1] {
                Value::Null => None,
                Value::String(s) => Some(s.to_string()),
                other => Some(format!("{other}")),
            };
            let opts = options(op, args.get(3), &["partition", "producer_id", "sequence"])?;
            let partition = opt_u64(op, &opts, "partition")?
                .map(|p| {
                    u32::try_from(p).map_err(|_| format!("{op}: partition {p} is out of range"))
                })
                .transpose()?;
            let sequence = match opts.get("sequence") {
                None | Some(Json::Null) => None,
                Some(v) => Some(
                    v.as_i64()
                        .ok_or_else(|| format!("{op}: \"sequence\" must be an Int, got {v}"))?,
                ),
            };
            let record = Record {
                key,
                value: payload(op, &args[2])?,
                partition,
                sequence,
            };
            let results = produce(op, &topic, vec![record], opt_str(op, &opts, "producer_id")?)?;
            to_value(results.get(0).cloned().unwrap_or(Json::Null))
        }),
        ("consume", None, |op, args| {
            arity(op, args, 3, 4, "(topic, partition, offset, options)")?;
            let topic = string_arg(op, args, 0, "topic")?;
            let partition = uint_arg(op, args, 1, "partition")?;
            let offset = uint_arg(op, args, 2, "offset")?;
            let opts = options(op, args.get(3), &["max_records", "max_bytes", "encoding"])?;
            to_value(consume(op, &topic, partition, offset, &opts)?)
        }),
        ("group_consume", None, |op, args| {
            arity(op, args, 3, 4, "(group, topic, partition, options)")?;
            let group = string_arg(op, args, 0, "group")?;
            let topic = string_arg(op, args, 1, "topic")?;
            let partition = uint_arg(op, args, 2, "partition")?;
            let opts = options(op, args.get(3), &["max_records", "max_bytes", "encoding"])?;
            let max_records = opt_u64(op, &opts, "max_records")?.unwrap_or(DEFAULT_MAX_RECORDS);
            let max_bytes = opt_u64(op, &opts, "max_bytes")?.unwrap_or(DEFAULT_MAX_BYTES);
            let mut path = format!(
                "/groups/{}/consume?topic={}&partition={partition}&max_records={max_records}&max_bytes={max_bytes}",
                seg(&group),
                seg(&topic)
            );
            if wants_base64(op, &opts)? {
                path.push_str("&encoding=base64");
            }
            to_value(page_shape(http_json(op, "GET", &path, None)?)?)
        }),
        ("commit", Some(4), |op, args| {
            let group = string_arg(op, args, 0, "group")?;
            let topic = string_arg(op, args, 1, "topic")?;
            let partition = uint_arg(op, args, 2, "partition")?;
            let offset = uint_arg(op, args, 3, "offset")?;
            http(
                op,
                "POST",
                &format!("/groups/{}/commit", seg(&group)),
                Some(json!({"topic": topic, "partition": partition, "offset": offset})),
            )?;
            Ok(Value::Bool(true))
        }),
        ("offsets", Some(1), |op, args| {
            let group = string_arg(op, args, 0, "group")?;
            offsets_value(http_json(
                op,
                "GET",
                &format!("/groups/{}/offsets", seg(&group)),
                None,
            )?)
        }),
        ("join", None, |op, args| {
            arity(op, args, 2, 3, "(group, topics, member_id)")?;
            let group = string_arg(op, args, 0, "group")?;
            let topics = match &args[1] {
                Value::String(s) => vec![json!(s.to_string())],
                Value::Array(items) => items
                    .borrow()
                    .iter()
                    .map(|v| match v {
                        Value::String(s) => Ok(json!(s.to_string())),
                        other => Err(format!(
                            "{op}: topics must be Strings, got {}",
                            other.type_name()
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                other => {
                    return Err(format!(
                        "{op}: topics must be an Array of Strings, got {}",
                        other.type_name()
                    ))
                }
            };
            let member_id = match args.get(2) {
                None | Some(Value::Null) => Json::Null,
                Some(Value::String(s)) => json!(s.to_string()),
                Some(other) => {
                    return Err(format!(
                        "{op}: member_id must be a String or nil, got {}",
                        other.type_name()
                    ))
                }
            };
            let body = json!({"topics": topics, "member_id": member_id});
            to_value(http_json(
                op,
                "POST",
                &format!("/groups/{}/join", seg(&group)),
                Some(body),
            )?)
        }),
        ("heartbeat", Some(3), |op, args| {
            let group = string_arg(op, args, 0, "group")?;
            let member_id = string_arg(op, args, 1, "member_id")?;
            let generation = uint_arg(op, args, 2, "generation")?;
            let body = json!({"member_id": member_id, "generation": generation});
            to_value(http_json(
                op,
                "POST",
                &format!("/groups/{}/heartbeat", seg(&group)),
                Some(body),
            )?)
        }),
        ("leave", Some(2), |op, args| {
            let group = string_arg(op, args, 0, "group")?;
            let member_id = string_arg(op, args, 1, "member_id")?;
            http(
                op,
                "POST",
                &format!("/groups/{}/leave", seg(&group)),
                Some(json!({"member_id": member_id})),
            )?;
            Ok(Value::Bool(true))
        }),
        ("assignment", Some(2), |op, args| {
            let group = string_arg(op, args, 0, "group")?;
            let member_id = string_arg(op, args, 1, "member_id")?;
            to_value(http_json(
                op,
                "GET",
                &format!(
                    "/groups/{}/assignment?member_id={}",
                    seg(&group),
                    seg(&member_id)
                ),
                None,
            )?)
        }),
        ("topics", Some(0), |op, _| {
            let list = http_json(op, "GET", "/topics", None)?;
            to_value(list.get("topics").cloned().unwrap_or(json!([])))
        }),
        ("create_topic", None, |op, args| {
            arity(op, args, 1, 2, "(name, options)")?;
            let name = string_arg(op, args, 0, "name")?;
            let mut opts = options(
                op,
                args.get(1),
                &[
                    "partitions",
                    "retention_ms",
                    "retention_bytes",
                    "cleanup_policy",
                    "segment_bytes",
                    "tombstone_retention_ms",
                ],
            )?;
            let partitions = opt_u64(op, &opts, "partitions")?.unwrap_or(1);
            opts.remove("partitions");
            let body = json!({
                "name": name,
                "partitions": partitions,
                "config": if opts.is_empty() { Json::Null } else { Json::Object(opts) },
            });
            to_value(http_json(op, "POST", "/topics", Some(body))?)
        }),
        ("topic", Some(1), |op, args| {
            let name = string_arg(op, args, 0, "name")?;
            to_value(http_json(
                op,
                "GET",
                &format!("/topics/{}", seg(&name)),
                None,
            )?)
        }),
        ("delete_topic", Some(1), |op, args| {
            let name = string_arg(op, args, 0, "name")?;
            to_value(http_json(
                op,
                "DELETE",
                &format!("/topics/{}", seg(&name)),
                None,
            )?)
        }),
        ("topic_config", Some(1), |op, args| {
            let name = string_arg(op, args, 0, "name")?;
            to_value(http_json(
                op,
                "GET",
                &format!("/topics/{}/config", seg(&name)),
                None,
            )?)
        }),
        ("update_topic_config", Some(2), |op, args| {
            let name = string_arg(op, args, 0, "name")?;
            let patch = options(
                op,
                args.get(1),
                &[
                    "retention_ms",
                    "retention_bytes",
                    "cleanup_policy",
                    "segment_bytes",
                    "tombstone_retention_ms",
                ],
            )?;
            to_value(http_json(
                op,
                "PUT",
                &format!("/topics/{}/config", seg(&name)),
                Some(Json::Object(patch)),
            )?)
        }),
    ];

    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    for (name, arity, op) in ops {
        let label = format!("ES.{name}");
        let shown = label.clone();
        methods.insert(
            name.to_string(),
            Rc::new(NativeFunction::new(label, arity, move |args| {
                op(&shown, args)
            })),
        );
    }
    let class = Rc::new(Class {
        name: "ES".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    });
    env.define("ES".to_string(), Value::Class(class));
}

/// The static methods `ES` answers, for the type checker.
pub const ES_METHODS: [&str; 19] = [
    "configure",
    "config",
    "ping",
    "produce",
    "emit",
    "consume",
    "group_consume",
    "commit",
    "offsets",
    "join",
    "heartbeat",
    "leave",
    "assignment",
    "topics",
    "create_topic",
    "topic",
    "delete_topic",
    "topic_config",
    "update_topic_config",
];
