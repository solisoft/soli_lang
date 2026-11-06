//! Datadog compatibility of the OpenTelemetry export, end to end: a real
//! `soli serve` exporting to a loopback OTLP/HTTP collector.
//!
//! - every export POST carries `OTEL_EXPORTER_OTLP_HEADERS` (percent-decoded),
//!   which is how Datadog's agentless intake takes its `dd-api-key`;
//! - `DD_SERVICE` / `DD_ENV` / `DD_VERSION` reach the resource;
//! - an uncaught error marks the SERVER span with an `exception` event
//!   carrying the Soli error class, while a deliberate 404 does not;
//! - `SOLI_OTEL_LOGS=1` exports the JSON log events as OTLP LogRecords with
//!   the trace correlation set;
//! - the JSON lines on stdout carry `dd.trace_id` / `dd.span_id` and the
//!   unified service tags.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use serde_json::Value;

struct Export {
    path: String,
    headers: HashMap<String, String>,
    body: Value,
}

fn serve_connection(stream: TcpStream, tx: Sender<Export>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut stream = stream;
    loop {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
            return;
        }
        let path = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or("")
            .to_string();
        let mut headers = HashMap::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
            }
        }
        let len: usize = headers
            .get("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).is_err() {
            return;
        }
        let _ = stream.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}",
        );
        let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let _ = tx.send(Export {
            path,
            headers,
            body,
        });
    }
}

/// A loopback OTLP/HTTP collector: every POST is answered 200 and handed
/// to the returned receiver.
fn collector() -> (u16, Receiver<Export>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || serve_connection(stream, tx));
        }
    });
    (port, rx)
}

fn write(root: &Path, path: &str, body: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, body).unwrap();
}

fn app(root: &Path) {
    write(
        root,
        "config/routes.sl",
        r#"get("/up", "probe#up")
get("/boom", "probe#boom")
get("/missing", "probe#missing")
"#,
    );
    write(
        root,
        "app/controllers/probe_controller.sl",
        r#"class PaymentError < Error
end

class ProbeController < Controller
  # GET /up
  def up(req)
    "ok"
  end

  # GET /boom — an uncaught error: a 500 and an exception on the trace
  def boom(req)
    throw PaymentError.new("card declined")
  end

  # GET /missing — a deliberate 404: not an exception
  def missing(req)
    halt(404, "nope")
  end
end
"#,
    );
}

struct Server(std::process::Child, u16);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn serve(root: &Path, collector_port: u16, stdout: std::fs::File) -> Server {
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(28791);
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("serve")
        .arg(root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        .current_dir(root)
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
        .env(
            "OTEL_EXPORTER_OTLP_ENDPOINT",
            format!("http://127.0.0.1:{collector_port}"),
        )
        .env("OTEL_EXPORTER_OTLP_HEADERS", "dd-api-key=s3cr3t%2Fkey")
        .env("OTEL_EXPORTER_OTLP_TRACES_HEADERS", "x-signal=traces")
        .env("SOLI_OTEL_LOGS", "1")
        .env("SOLI_LOG", "access")
        .env("SOLI_LOG_FORMAT", "json")
        .env("DD_SERVICE", "shop")
        .env("DD_ENV", "test")
        .env("DD_VERSION", "9.9.9")
        .env_remove("OTEL_SERVICE_NAME")
        .env_remove("OTEL_RESOURCE_ATTRIBUTES")
        .env_remove("OTEL_SDK_DISABLED")
        .stdout(stdout)
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn soli serve");
    let server = Server(child, port);
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if ureq::get(&format!("http://127.0.0.1:{port}/up"))
            .call()
            .is_ok()
        {
            return server;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("soli serve did not answer on {port}");
}

fn get(server: &Server, path: &str) -> u16 {
    match ureq::get(&format!("http://127.0.0.1:{}{path}", server.1)).call() {
        Ok(r) => r.status(),
        Err(ureq::Error::Status(code, _)) => code,
        Err(e) => panic!("GET {path}: {e}"),
    }
}

fn attrs_get<'a>(attrs: &'a Value, key: &str) -> Option<&'a Value> {
    attrs
        .as_array()?
        .iter()
        .find(|a| a["key"] == key)
        .map(|a| &a["value"])
}

fn str_attr(attrs: &Value, key: &str) -> Option<String> {
    attrs_get(attrs, key).and_then(|v| v["stringValue"].as_str().map(str::to_string))
}

/// All spans received, with their resource attributes.
fn spans_of(export: &Export) -> Vec<(Value, Value)> {
    let mut out = Vec::new();
    for rs in export.body["resourceSpans"]
        .as_array()
        .into_iter()
        .flatten()
    {
        for ss in rs["scopeSpans"].as_array().into_iter().flatten() {
            for span in ss["spans"].as_array().into_iter().flatten() {
                out.push((rs["resource"]["attributes"].clone(), span.clone()));
            }
        }
    }
    out
}

fn log_records_of(export: &Export) -> Vec<(Value, Value)> {
    let mut out = Vec::new();
    for rl in export.body["resourceLogs"].as_array().into_iter().flatten() {
        for sl in rl["scopeLogs"].as_array().into_iter().flatten() {
            for rec in sl["logRecords"].as_array().into_iter().flatten() {
                out.push((rl["resource"]["attributes"].clone(), rec.clone()));
            }
        }
    }
    out
}

#[test]
fn datadog_ready_traces_and_logs_reach_the_collector() {
    let (collector_port, rx) = collector();
    let dir = tempfile::tempdir().unwrap();
    app(dir.path());
    let stdout_path = dir.path().join("stdout.log");
    let stdout = std::fs::File::create(&stdout_path).unwrap();
    let server = serve(dir.path(), collector_port, stdout);

    assert_eq!(get(&server, "/boom"), 500);
    assert_eq!(get(&server, "/missing"), 404);

    let mut boom_span: Option<(Value, Value)> = None;
    let mut missing_span: Option<Value> = None;
    let mut error_log: Option<(Value, Value)> = None;
    let mut trace_headers: Option<HashMap<String, String>> = None;
    let mut log_headers: Option<HashMap<String, String>> = None;

    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline
        && (boom_span.is_none() || missing_span.is_none() || error_log.is_none())
    {
        let Ok(export) = rx.recv_timeout(Duration::from_millis(500)) else {
            continue;
        };
        if export.path == "/v1/traces" {
            trace_headers.get_or_insert_with(|| export.headers.clone());
            for (resource, span) in spans_of(&export) {
                if span["kind"] != 2 {
                    continue;
                }
                if span["name"] == "GET /boom" {
                    boom_span = Some((resource, span));
                } else if span["name"] == "GET /missing" {
                    missing_span = Some(span);
                }
            }
        } else if export.path == "/v1/logs" {
            log_headers.get_or_insert_with(|| export.headers.clone());
            for (resource, rec) in log_records_of(&export) {
                if rec["body"]["stringValue"] == "request_error" {
                    error_log = Some((resource, rec));
                }
            }
        }
    }

    // ---- headers: generic on both signals, the traces-only one on traces.
    let trace_headers = trace_headers.expect("no traces export received");
    assert_eq!(
        trace_headers.get("dd-api-key").map(String::as_str),
        Some("s3cr3t/key")
    );
    assert_eq!(
        trace_headers.get("x-signal").map(String::as_str),
        Some("traces")
    );
    let log_headers = log_headers.expect("no logs export received");
    assert_eq!(
        log_headers.get("dd-api-key").map(String::as_str),
        Some("s3cr3t/key")
    );
    assert!(!log_headers.contains_key("x-signal"));

    // ---- the failing request's SERVER span.
    let (resource, span) = boom_span.expect("no SERVER span for /boom");
    assert_eq!(str_attr(&resource, "service.name").as_deref(), Some("shop"));
    assert_eq!(
        str_attr(&resource, "deployment.environment.name").as_deref(),
        Some("test")
    );
    assert_eq!(
        str_attr(&resource, "service.version").as_deref(),
        Some("9.9.9")
    );
    assert_eq!(span["status"]["code"], 2, "{span}");
    let event = span["events"]
        .as_array()
        .and_then(|e| e.iter().find(|e| e["name"] == "exception"))
        .unwrap_or_else(|| panic!("no exception event: {span}"));
    assert_eq!(
        str_attr(&event["attributes"], "exception.type").as_deref(),
        Some("PaymentError"),
        "{event}"
    );
    assert!(str_attr(&event["attributes"], "exception.message")
        .unwrap_or_default()
        .contains("card declined"));
    assert!(attrs_get(&event["attributes"], "exception.stacktrace").is_some());
    let trace_id = span["traceId"].as_str().unwrap().to_string();

    // ---- a deliberate 404 is not an exception.
    let missing = missing_span.expect("no SERVER span for /missing");
    assert!(missing.get("events").is_none(), "{missing}");
    assert_ne!(missing["status"]["code"], 2, "{missing}");

    // ---- the error log record, correlated to the trace.
    let (log_resource, rec) = error_log.expect("no request_error log record");
    assert_eq!(rec["traceId"], trace_id.as_str());
    assert_eq!(rec["severityText"], "ERROR");
    assert_eq!(
        str_attr(&log_resource, "service.name").as_deref(),
        Some("shop")
    );
    assert_eq!(
        str_attr(&rec["attributes"], "error_type").as_deref(),
        Some("PaymentError")
    );

    // ---- stdout JSON access lines carry the Datadog correlation.
    drop(server);
    let stdout = std::fs::read_to_string(&stdout_path).unwrap();
    let boom_line: Value = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find(|v| v["msg"] == "request" && v["path"] == "/boom")
        .unwrap_or_else(|| panic!("no access line for /boom in:\n{stdout}"));
    assert_eq!(boom_line["service"], "shop");
    assert_eq!(boom_line["env"], "test");
    assert_eq!(boom_line["version"], "9.9.9");
    assert_eq!(boom_line["trace_id"], trace_id.as_str());
    let low64 = u64::from_str_radix(&trace_id[16..], 16)
        .unwrap()
        .to_string();
    assert_eq!(boom_line["dd"]["trace_id"], low64.as_str());
    let span_hex = boom_line["span_id"].as_str().unwrap();
    assert_eq!(
        boom_line["dd"]["span_id"],
        u64::from_str_radix(span_hex, 16)
            .unwrap()
            .to_string()
            .as_str()
    );
    assert_eq!(boom_line["dd"]["service"], "shop");
    // The API key never reaches stdout.
    assert!(!stdout.contains("s3cr3t"), "{stdout}");
}
