//! Outbound W3C trace propagation, head sampling and job spans, end to end.
//!
//! Each test serves a throwaway app with `soli serve`, pointed at two loopback
//! servers of its own: an OTLP/HTTP "collector" that keeps every JSON body it
//! is sent, and a "downstream" service that keeps the headers of every request
//! the app's `HTTP.*` calls make. A traced call must reach downstream with a
//! `traceparent` whose trace id is the request's and whose parent id is the
//! CLIENT span the app exported for that call — so the downstream span nests
//! under it in Datadog or Jaeger.

use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const INBOUND_TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const INBOUND_PARENT: &str = "00f067aa0ba902b7";
const TRACESTATE: &str = "dd=s:1;t.dm:-0,congo=t61rcWkgMzE";

/// A request's path and its headers (names lowercased).
type Received = (String, Vec<(String, String)>);

/// A loopback HTTP server that keeps what it receives. `bodies` holds request
/// bodies (the collector), `requests` the lowercased path + headers (downstream).
#[derive(Clone, Default)]
struct Sink {
    port: u16,
    bodies: Arc<Mutex<Vec<String>>>,
    requests: Arc<Mutex<Vec<Received>>>,
}

impl Sink {
    fn start() -> Sink {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let sink = Sink {
            port: listener.local_addr().expect("addr").port(),
            ..Sink::default()
        };
        let accepting = sink.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let sink = accepting.clone();
                thread::spawn(move || sink.serve(stream));
            }
        });
        sink
    }

    fn serve(&self, mut stream: TcpStream) {
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut raw = Vec::new();
        let mut buf = [0u8; 8192];
        let head_end = loop {
            if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => raw.extend_from_slice(&buf[..n]),
            }
        };
        let head = String::from_utf8_lossy(&raw[..head_end]).to_string();
        let mut lines = head.split("\r\n");
        let path = lines
            .next()
            .and_then(|l| l.split(' ').nth(1))
            .unwrap_or("")
            .to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let length = headers
            .iter()
            .find(|(k, _)| k == "content-length")
            .and_then(|(_, v)| v.parse::<usize>().ok())
            .unwrap_or(0);
        while raw.len() < head_end + length {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => raw.extend_from_slice(&buf[..n]),
            }
        }
        let body = String::from_utf8_lossy(&raw[head_end..]).to_string();
        self.bodies.lock().unwrap().push(body);
        self.requests.lock().unwrap().push((path, headers));
        let _ = stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
        );
    }

    /// Headers of the requests made to `path`, once `count` have arrived.
    fn headers_for(&self, path: &str, count: usize) -> Vec<Vec<(String, String)>> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let found: Vec<_> = self
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(p, _)| p == path)
                .map(|(_, h)| h.clone())
                .collect();
            if found.len() >= count {
                return found;
            }
            assert!(
                Instant::now() < deadline,
                "{count} request(s) to {path} never came; got {found:?}"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Every exported span of `trace_id`, across all bodies received so far.
    fn spans_of(&self, trace_id: &str) -> Vec<Value> {
        let bodies = self.bodies.lock().unwrap().clone();
        bodies
            .iter()
            .filter_map(|b| serde_json::from_str::<Value>(b).ok())
            .flat_map(|payload| {
                payload["resourceSpans"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .flat_map(|rs| rs["scopeSpans"].as_array().cloned().unwrap_or_default())
            .flat_map(|ss| ss["spans"].as_array().cloned().unwrap_or_default())
            .filter(|span| span["traceId"] == trace_id)
            .collect()
    }

    /// Wait for a span of `trace_id` that `pick` accepts.
    fn wait_span(&self, trace_id: &str, pick: impl Fn(&Value) -> bool) -> (Value, Vec<Value>) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let spans = self.spans_of(trace_id);
            if let Some(span) = spans.iter().find(|s| pick(s)) {
                return (span.clone(), spans);
            }
            assert!(
                Instant::now() < deadline,
                "no matching span of {trace_id} was exported; got {spans:#?}"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// `(trace_id, parent_id, flags)` of a `traceparent`.
fn parts(traceparent: &str) -> (String, String, String) {
    let p: Vec<&str> = traceparent.split('-').collect();
    assert_eq!(p.len(), 4, "{traceparent}");
    assert_eq!(p[0], "00", "{traceparent}");
    (p[1].to_string(), p[2].to_string(), p[3].to_string())
}

fn write(root: &Path, path: &str, body: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, body).unwrap();
}

fn write_app(root: &Path, down: u16, with_jobs: bool) {
    let target = format!("http://127.0.0.1:{down}");
    let mut routes = String::from(
        "get(\"/ping\", \"home#ping\")\nget(\"/call\", \"home#call\")\n\
         get(\"/explicit\", \"home#explicit\")\nget(\"/fanout\", \"home#fanout\")\n",
    );
    let mut later = String::new();
    if with_jobs {
        routes.push_str("get(\"/later\", \"home#later\")\n");
        later = "\n  # GET /later\n  def later(req)\n    PingJob.perform_later({})\n    \
                 {\"status\": 200, \"body\": \"queued\"}\n  end\n"
            .to_string();
        write(
            root,
            "config/database.toml",
            "default = \"primary\"\n\n[connections.primary]\nadapter = \"sqlite\"\n\
             url = \"sqlite://db/main.sqlite3\"\n",
        );
        write(
            root,
            "app/jobs/ping_job.sl",
            &format!(
                "class PingJob\n  static def perform(args: Hash)\n    \
                 HTTP.get(\"{target}/from-job\")\n  end\nend\n"
            ),
        );
    }
    write(root, "config/routes.sl", &routes);
    write(
        root,
        "app/controllers/home_controller.sl",
        &format!(
            r#"class HomeController < Controller
  # GET /ping
  def ping(req)
    {{"status": 200, "body": "pong"}}
  end

  # GET /call
  def call(req)
    {{"status": 200, "body": HTTP.get("{target}/down")}}
  end

  # GET /explicit
  def explicit(req)
    mine = "00-11111111111111111111111111111111-2222222222222222-01"
    {{"status": 200, "body": HTTP.get("{target}/explicit", {{"headers": {{"traceparent": mine}}}})}}
  end

  # GET /fanout
  def fanout(req)
    HTTP.get_all(["{target}/fan", "{target}/fan"])
    {{"status": 200, "body": "fanned"}}
  end
{later}end
"#
        ),
    );
}

struct Server {
    child: Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn serve(root: &Path, down: u16, env: &[(&str, &str)]) -> Server {
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("a free port");
    let mut command = Command::new(env!("CARGO_BIN_EXE_soli"));
    command
        .arg("serve")
        .arg(root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        .current_dir(root)
        .env("SOLI_HOST", "127.0.0.1")
        .env("SOLIDB_HOST", "http://127.0.0.1:1")
        .env("SOLI_HTTP_ALLOW_HOSTS", format!("127.0.0.1:{down}"))
        .env("SOLI_JOBS_POLL_MS", "100")
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .env_remove("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
        .env_remove("OTEL_TRACES_SAMPLER")
        .env_remove("OTEL_TRACES_SAMPLER_ARG")
        .env_remove("OTEL_SDK_DISABLED")
        .env_remove("SOLI_OTEL")
        .env_remove("SOLI_OTEL_PROPAGATE")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (k, v) in env {
        command.env(k, v);
    }
    let server = Server {
        child: command.spawn().expect("spawn soli serve"),
        port,
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if ureq::get(&format!("http://127.0.0.1:{port}/ping"))
            .timeout(Duration::from_millis(500))
            .call()
            .is_ok()
        {
            return server;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("soli serve did not answer on {port}");
}

/// GET `path`, with optional inbound trace headers; returns the response's
/// `traceparent`.
fn get(
    server: &Server,
    path: &str,
    traceparent: Option<&str>,
    tracestate: Option<&str>,
) -> Option<String> {
    let mut request = ureq::get(&format!("http://127.0.0.1:{}{path}", server.port));
    if let Some(tp) = traceparent {
        request = request.set("traceparent", tp);
    }
    if let Some(ts) = tracestate {
        request = request.set("tracestate", ts);
    }
    let response = request.call().unwrap_or_else(|e| panic!("GET {path}: {e}"));
    assert_eq!(response.status(), 200, "GET {path}");
    response.header("traceparent").map(str::to_string)
}

fn collector_env(collector: &Sink) -> String {
    format!("http://127.0.0.1:{}", collector.port)
}

/// The exported span `id`'s ancestors, ending at a span without a parent in
/// the trace (the SERVER root).
fn root_of<'a>(spans: &'a [Value], id: &str) -> &'a Value {
    let mut current = spans
        .iter()
        .find(|s| s["spanId"] == id)
        .unwrap_or_else(|| panic!("span {id} not exported: {spans:#?}"));
    for _ in 0..64 {
        let Some(parent) = current["parentSpanId"].as_str() else {
            return current;
        };
        match spans.iter().find(|s| s["spanId"] == parent) {
            Some(next) => current = next,
            None => return current,
        }
    }
    panic!("parent cycle in {spans:#?}");
}

#[test]
fn outgoing_calls_carry_the_client_span_and_the_inbound_tracestate() {
    let collector = Sink::start();
    let down = Sink::start();
    let dir = tempfile::tempdir().expect("tempdir");
    write_app(dir.path(), down.port, false);
    let endpoint = collector_env(&collector);
    let server = serve(
        dir.path(),
        down.port,
        &[("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.as_str())],
    );

    // A traced, sampled request.
    let inbound = format!("00-{INBOUND_TRACE}-{INBOUND_PARENT}-01");
    get(&server, "/call", Some(&inbound), Some(TRACESTATE));
    let sent = &down.headers_for("/down", 1)[0];
    let (trace_id, parent_id, flags) = parts(header(sent, "traceparent").expect("traceparent"));
    assert_eq!(trace_id, INBOUND_TRACE, "the request's trace goes on");
    assert_ne!(
        parent_id, INBOUND_PARENT,
        "the parent is our span, not the caller's"
    );
    assert_eq!(flags, "01");
    assert_eq!(
        header(sent, "tracestate"),
        Some(TRACESTATE),
        "forwarded unchanged"
    );

    // The parent id is the exported CLIENT span of that call, under our
    // SERVER root, which is under the caller's span.
    let (client, spans) = collector.wait_span(INBOUND_TRACE, |s| s["spanId"] == parent_id.as_str());
    assert_eq!(client["kind"], 3, "CLIENT: {client}");
    assert!(
        client["name"].as_str().unwrap_or("").ends_with("/down"),
        "{client}"
    );
    let root = root_of(&spans, &parent_id);
    assert_eq!(root["kind"], 2, "SERVER root: {root}");
    assert_eq!(root["parentSpanId"], INBOUND_PARENT);
    let calls = spans
        .iter()
        .filter(|s| s["kind"] == 3 && s["name"].as_str().unwrap_or("").ends_with("/down"))
        .count();
    assert_eq!(calls, 1, "one CLIENT span per call: {spans:#?}");

    // A caller's own traceparent is sent as it is.
    get(&server, "/explicit", Some(&inbound), None);
    let sent = &down.headers_for("/explicit", 1)[0];
    assert_eq!(
        header(sent, "traceparent"),
        Some("00-11111111111111111111111111111111-2222222222222222-01")
    );
    assert_eq!(header(sent, "tracestate"), None, "none came in");

    // Each call of a fan-out names its own CLIENT span.
    let fresh = "0123456789abcdef0123456789abcdef";
    get(
        &server,
        "/fanout",
        Some(&format!("00-{fresh}-{INBOUND_PARENT}-01")),
        None,
    );
    let fanned = down.headers_for("/fan", 2);
    let ids: Vec<String> = fanned
        .iter()
        .map(|h| parts(header(h, "traceparent").expect("traceparent")))
        .map(|(t, p, _)| {
            assert_eq!(t, fresh);
            p
        })
        .collect();
    assert_ne!(ids[0], ids[1]);
    for id in &ids {
        let (span, _) = collector.wait_span(fresh, |s| s["spanId"] == id.as_str());
        assert_eq!(span["kind"], 3, "{span}");
    }

    // Parent-based sampling: an inbound "not sampled" is honoured — the call
    // still propagates, with flags 00, and nothing is exported.
    let quiet = "fedcba9876543210fedcba9876543210";
    let answered = get(
        &server,
        "/call",
        Some(&format!("00-{quiet}-{INBOUND_PARENT}-00")),
        None,
    )
    .expect("a traceparent on the response");
    assert!(answered.ends_with("-00"), "{answered}");
    let sent = &down.headers_for("/down", 2)[1];
    let (trace_id, _, flags) = parts(header(sent, "traceparent").expect("traceparent"));
    assert_eq!((trace_id.as_str(), flags.as_str()), (quiet, "00"));
    thread::sleep(Duration::from_millis(1500));
    assert!(
        collector.spans_of(quiet).is_empty(),
        "an unsampled trace is not exported"
    );
}

#[test]
fn always_off_still_propagates_ids_but_exports_nothing() {
    let collector = Sink::start();
    let down = Sink::start();
    let dir = tempfile::tempdir().expect("tempdir");
    write_app(dir.path(), down.port, false);
    let endpoint = collector_env(&collector);
    let server = serve(
        dir.path(),
        down.port,
        &[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.as_str()),
            ("OTEL_TRACES_SAMPLER", "always_off"),
        ],
    );
    let answered = get(&server, "/call", None, None).expect("a traceparent on the response");
    let (trace_id, _, flags) = parts(&answered);
    assert_eq!(flags, "00");
    let sent = &down.headers_for("/down", 1)[0];
    let (sent_trace, _, sent_flags) = parts(header(sent, "traceparent").expect("traceparent"));
    assert_eq!(sent_trace, trace_id, "logs and downstream share the id");
    assert_eq!(sent_flags, "00");
    thread::sleep(Duration::from_millis(1500));
    assert!(collector.spans_of(&trace_id).is_empty());
}

#[test]
fn nothing_is_propagated_when_tracing_is_off_or_opted_out() {
    let down = Sink::start();
    let dir = tempfile::tempdir().expect("tempdir");
    write_app(dir.path(), down.port, false);

    // Tracing off: no OTEL variable at all.
    {
        let server = serve(dir.path(), down.port, &[]);
        let inbound = format!("00-{INBOUND_TRACE}-{INBOUND_PARENT}-01");
        assert!(get(&server, "/call", Some(&inbound), Some(TRACESTATE)).is_none());
        let sent = &down.headers_for("/down", 1)[0];
        assert_eq!(header(sent, "traceparent"), None);
        assert_eq!(header(sent, "tracestate"), None);
    }

    // Tracing on, propagation off.
    let collector = Sink::start();
    let endpoint = collector_env(&collector);
    let server = serve(
        dir.path(),
        down.port,
        &[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.as_str()),
            ("SOLI_OTEL_PROPAGATE", "0"),
        ],
    );
    let inbound = format!("00-{INBOUND_TRACE}-{INBOUND_PARENT}-01");
    assert!(get(&server, "/call", Some(&inbound), Some(TRACESTATE)).is_some());
    let sent = &down.headers_for("/down", 2)[1];
    assert_eq!(header(sent, "traceparent"), None);
    assert_eq!(header(sent, "tracestate"), None);
    // Export is unaffected.
    collector.wait_span(INBOUND_TRACE, |s| s["kind"] == 3);
}

#[cfg(feature = "sqlite")]
#[test]
fn a_job_is_a_consumer_span_linked_to_the_request_that_enqueued_it() {
    let collector = Sink::start();
    let down = Sink::start();
    let dir = tempfile::tempdir().expect("tempdir");
    write_app(dir.path(), down.port, true);
    let endpoint = collector_env(&collector);
    let server = serve(
        dir.path(),
        down.port,
        &[("OTEL_EXPORTER_OTLP_ENDPOINT", endpoint.as_str())],
    );

    let inbound = format!("00-{INBOUND_TRACE}-{INBOUND_PARENT}-01");
    get(&server, "/later", Some(&inbound), None);

    // The job's call: a trace of its own, under the job's CLIENT span.
    let sent = &down.headers_for("/from-job", 1)[0];
    let (job_trace, client_id, flags) = parts(header(sent, "traceparent").expect("traceparent"));
    assert_ne!(
        job_trace, INBOUND_TRACE,
        "a job is not part of the request's trace"
    );
    assert_eq!(flags, "01");
    let (client, spans) = collector.wait_span(&job_trace, |s| s["spanId"] == client_id.as_str());
    assert_eq!(client["kind"], 3);
    let root = root_of(&spans, &client_id);
    assert_eq!(root["kind"], 5, "CONSUMER: {root}");
    assert_eq!(root["name"], "job PingJob");
    let attrs = root["attributes"].to_string();
    assert!(attrs.contains("messaging.system"), "{attrs}");
    assert!(attrs.contains("messaging.message.id"), "{attrs}");
    assert!(root.get("parentSpanId").is_none(), "{root}");

    // …linked to a span of the request that enqueued it.
    let link = &root["links"][0];
    assert_eq!(link["traceId"], INBOUND_TRACE, "{root}");
    let request_spans = collector.spans_of(INBOUND_TRACE);
    assert!(
        request_spans.iter().any(|s| s["spanId"] == link["spanId"]),
        "the link names an exported request span: {link} / {request_spans:#?}"
    );
}
