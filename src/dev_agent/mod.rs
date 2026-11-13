//! `soli agent`: a local MCP server, on stdin/stdout, that lets a coding agent
//! see the running `soli serve --dev` process of the app it is started in.
//!
//! Transport is MCP's stdio transport: newline-delimited JSON-RPC 2.0, one
//! message per line. Only `initialize`, `notifications/initialized`, `ping`,
//! `tools/list` and `tools/call` are implemented — no resources, prompts or
//! sampling, and no socket of its own.
//!
//! Every tool but `graph`, `graph_note` and `test` is a call to the dev server's own
//! `/__solidev/*` or `/__dev/repl` endpoints on `127.0.0.1`, so the gates in
//! `serve::dev_routes::dispatch` (trusted peer, local `Host`, same-origin
//! POST, REPL token) apply to the agent exactly as they do to the browser.
//! Nothing here writes into the application; `graph_note` writes a note into
//! the code graph's own store (`soli_graph_insights`), beside the graph.

pub mod discovery;
pub mod http;

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use http::{DevClient, HttpResponse};

/// The version answered when the client asks for one this server does not know.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// How long a `test` run may take before it is killed.
const TEST_TIMEOUT: Duration = Duration::from_secs(120);
/// At most this many failure blocks, and this many bytes, come back from `test`.
const MAX_TEST_FAILURES: usize = 20;
const MAX_TEST_BYTES: usize = 8 * 1024;
/// Replays and REPL calls wait on the app's own handler.
const HTTP_TIMEOUT: Duration = Duration::from_secs(120);

/// How `soli agent` was started.
pub struct Options {
    /// The directory holding `.soli/dev-server.json` (the current directory).
    pub dir: PathBuf,
    /// Tests only: talk to this port and skip the discovery file.
    pub port: Option<u16>,
    /// Tests only, with `port`: the REPL token.
    pub dev_token: Option<String>,
}

/// Entry point for `soli agent`. Returns the process exit code.
pub fn run(opts: Options) -> i32 {
    let (port, dev_token, app_root) = match opts.port {
        Some(port) => (port, opts.dev_token, opts.dir.clone()),
        None => match discovery::read(&opts.dir) {
            Ok(info) => (info.port, info.dev_token, PathBuf::from(info.app_root)),
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        },
    };

    let protocol_out = match take_stdout() {
        Ok(out) => out,
        Err(e) => {
            eprintln!("soli agent: {e}");
            return 1;
        }
    };
    eprintln!("soli agent: attached to the dev server on 127.0.0.1:{port}");

    let mut agent = Agent::new(
        DevClient {
            port,
            dev_token,
            timeout: HTTP_TIMEOUT,
        },
        app_root,
    );
    let stdin = std::io::stdin();
    match serve(&mut agent, stdin.lock(), protocol_out) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("soli agent: {e}");
            1
        }
    }
}

/// Keep the real stdout for the protocol and point fd 1 at stderr, so a stray
/// `println!` anywhere in the code a tool reaches (the graph query, env
/// loading) lands in the log instead of corrupting a JSON-RPC line.
#[cfg(unix)]
fn take_stdout() -> Result<Box<dyn Write>, String> {
    use std::os::unix::io::FromRawFd;
    std::io::stdout().flush().ok();
    let fd = unsafe { libc::dup(1) };
    if fd < 0 {
        return Err("could not duplicate stdout".into());
    }
    if unsafe { libc::dup2(2, 1) } < 0 {
        return Err("could not redirect stdout to stderr".into());
    }
    Ok(Box::new(unsafe { std::fs::File::from_raw_fd(fd) }))
}

#[cfg(not(unix))]
fn take_stdout() -> Result<Box<dyn Write>, String> {
    Ok(Box::new(std::io::stdout()))
}

/// Read JSON-RPC messages, one per line, and answer each request on one line.
pub fn serve<R: BufRead, W: Write>(agent: &mut Agent, input: R, mut out: W) -> Result<(), String> {
    for line in input.lines() {
        let line = line.map_err(|e| format!("reading stdin failed: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => agent.handle(&msg),
            Err(e) => Some(rpc_error(Value::Null, -32700, &format!("parse error: {e}"))),
        };
        if let Some(reply) = reply {
            let mut text = reply.to_string();
            text.push('\n');
            out.write_all(text.as_bytes())
                .and_then(|_| out.flush())
                .map_err(|e| format!("writing stdout failed: {e}"))?;
        }
    }
    Ok(())
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A tool's answer: JSON as text, and whether it is a failure.
struct ToolOutcome {
    value: Value,
    is_error: bool,
}

impl ToolOutcome {
    fn ok(value: Value) -> Self {
        Self {
            value,
            is_error: false,
        }
    }

    fn err(message: impl Into<String>) -> Self {
        Self {
            value: json!({ "error": message.into() }),
            is_error: true,
        }
    }

    fn into_result(self) -> Value {
        let text = serde_json::to_string_pretty(&self.value).unwrap_or_default();
        json!({ "content": [{ "type": "text", "text": text }], "isError": self.is_error })
    }
}

pub struct Agent {
    client: DevClient,
    app_root: PathBuf,
    soli_exe: Option<PathBuf>,
    test_timeout: Duration,
    graph_env_loaded: bool,
}

impl Agent {
    pub fn new(client: DevClient, app_root: PathBuf) -> Self {
        Self {
            client,
            app_root,
            soli_exe: std::env::current_exe().ok(),
            test_timeout: TEST_TIMEOUT,
            graph_env_loaded: false,
        }
    }

    /// Answer one message, or `None` for a notification.
    pub fn handle(&mut self, msg: &Value) -> Option<Value> {
        if msg.is_array() {
            return Some(rpc_error(
                Value::Null,
                -32600,
                "batched requests are not supported",
            ));
        }
        let method = msg.get("method").and_then(Value::as_str);
        let id = msg.get("id").cloned();
        let Some(id) = id else {
            // A notification (`notifications/initialized`, a cancellation):
            // nothing to answer.
            return None;
        };
        let Some(method) = method else {
            return Some(rpc_error(id, -32600, "missing method"));
        };
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        Some(match method {
            "initialize" => rpc_result(id, initialize_result(&params)),
            "ping" => rpc_result(id, json!({})),
            "tools/list" => rpc_result(id, json!({ "tools": tool_definitions() })),
            "tools/call" => {
                let Some(name) = params.get("name").and_then(Value::as_str) else {
                    return Some(rpc_error(id, -32602, "tools/call needs a tool name"));
                };
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                match self.call_tool(name, &args) {
                    Some(outcome) => rpc_result(id, outcome.into_result()),
                    None => rpc_error(id, -32602, &format!("unknown tool: {name}")),
                }
            }
            other => rpc_error(id, -32601, &format!("method not found: {other}")),
        })
    }

    fn call_tool(&mut self, name: &str, args: &Value) -> Option<ToolOutcome> {
        Some(match name {
            "requests" => self.http_json(self.client.get("/__solidev/requests")),
            "request" => match str_arg(args, "id") {
                Some(id) => self.http_json(
                    self.client
                        .get(&format!("/__solidev/requests/{}", urlencoding::encode(id))),
                ),
                None => ToolOutcome::err("`id` is required"),
            },
            "replay" => self.replay(args),
            "errors" => {
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20);
                self.http_json(self.client.get(&format!("/__solidev/errors?limit={limit}")))
            }
            "graph" => self.graph(args),
            "graph_note" => self.graph_note(args),
            "test" => self.test(args),
            "repl" => self.repl(args),
            _ => return None,
        })
    }

    /// A dev endpoint's JSON body, or its failure as a tool error.
    fn http_json(&self, resp: Result<HttpResponse, String>) -> ToolOutcome {
        let resp = match resp {
            Ok(r) => r,
            Err(e) => return ToolOutcome::err(e),
        };
        let parsed = serde_json::from_slice::<Value>(&resp.body).ok();
        if (200..300).contains(&resp.status) {
            return match parsed {
                Some(v) => ToolOutcome::ok(v),
                None => ToolOutcome::err(format!(
                    "the dev server answered {} with a non-JSON body",
                    resp.status
                )),
            };
        }
        let message = parsed
            .as_ref()
            .and_then(|v| v.get("error"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| {
                if resp.status == 404 {
                    "not found — either the id is unknown, or this dev server predates `soli agent` (restart it with the same soli binary)".to_string()
                } else {
                    resp.text()
                }
            });
        ToolOutcome {
            value: json!({ "error": message, "status": resp.status }),
            is_error: true,
        }
    }

    fn replay(&self, args: &Value) -> ToolOutcome {
        let Some(id) = str_arg(args, "id") else {
            return ToolOutcome::err("`id` is required");
        };
        let encoded = urlencoding::encode(id);
        // Learn the method first: a read of the snapshot, never the replay.
        let snapshot = self.http_json(self.client.get(&format!("/__solidev/requests/{encoded}")));
        if snapshot.is_error {
            return snapshot;
        }
        let method = snapshot
            .value
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_uppercase();
        let confirmed = args.get("confirm").and_then(Value::as_bool) == Some(true);
        if !matches!(method.as_str(), "GET" | "HEAD") && !confirmed {
            return ToolOutcome::err(format!(
                "request {id} is a {method}: replaying it runs the handler again with its original cookies and body. Call replay again with `confirm: true` to do it."
            ));
        }
        match self
            .client
            .post(&format!("/__solidev/replay/{encoded}"), &[], None)
        {
            Ok(resp) if resp.header("x-soli-replay").is_some() => ToolOutcome::ok(json!({
                "replayed": id,
                "method": method,
                "path": snapshot.value.get("path"),
                "new_request_id": resp.header("x-soli-request-id"),
                "status": resp.status,
            })),
            Ok(resp) => self.http_json(Ok(resp)),
            Err(e) => ToolOutcome::err(e),
        }
    }

    fn repl(&self, args: &Value) -> ToolOutcome {
        let Some(code) = str_arg(args, "code") else {
            return ToolOutcome::err("`code` is required");
        };
        let Some(token) = self.client.dev_token.as_deref() else {
            return ToolOutcome::err(
                "this dev server did not publish a REPL token in .soli/dev-server.json; restart it with the same soli binary",
            );
        };
        let body = json!({
            "code": code,
            "repl_session_id": str_arg(args, "repl_session_id").unwrap_or(""),
        })
        .to_string();
        self.http_json(self.client.post(
            "/__dev/repl",
            &[
                ("X-Soli-Dev-Token", token),
                ("Content-Type", "application/json"),
            ],
            Some(&body),
        ))
    }

    /// Load the app's `.env` and DB config once, for the graph tools.
    fn load_graph_env(&mut self) {
        if !self.graph_env_loaded {
            crate::serve::env_loader::load_env_files(&self.app_root);
            crate::interpreter::builtins::model::init_db_config();
            self.graph_env_loaded = true;
        }
    }

    fn graph_note(&mut self, args: &Value) -> ToolOutcome {
        let Some(text) = str_arg(args, "text").filter(|t| !t.trim().is_empty()) else {
            return ToolOutcome::err("`text` is required");
        };
        let anchors: Vec<String> = match args.get("on") {
            Some(Value::Array(list)) => list
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            Some(Value::String(list)) => crate::graph::parse_kinds(list),
            _ => Vec::new(),
        };
        self.load_graph_env();
        match crate::graph::add_note(text, &anchors, str_arg(args, "question"), None) {
            Ok(note) => match serde_json::to_value(&note) {
                Ok(v) => ToolOutcome::ok(v),
                Err(e) => ToolOutcome::err(format!("note was not JSON: {e}")),
            },
            Err(e) => ToolOutcome::err(e),
        }
    }

    fn graph(&mut self, args: &Value) -> ToolOutcome {
        let Some(question) = str_arg(args, "question") else {
            return ToolOutcome::err("`question` is required");
        };
        self.load_graph_env();
        let opts = crate::graph::QueryOptions {
            database: None,
            limit: args.get("limit").and_then(Value::as_u64).unwrap_or(6) as usize,
            hops: args.get("hops").and_then(Value::as_u64).unwrap_or(1) as usize,
            path: str_arg(args, "path").map(str::to_string),
            kinds: str_arg(args, "kind")
                .map(crate::graph::parse_kinds)
                .unwrap_or_default(),
        };
        match crate::graph::run_query(question, &opts) {
            Ok(result) => match serde_json::from_str(&result.to_pretty_json()) {
                Ok(v) => ToolOutcome::ok(v),
                Err(e) => ToolOutcome::err(format!("graph result was not JSON: {e}")),
            },
            Err(e) => ToolOutcome::err(e),
        }
    }

    fn test(&self, args: &Value) -> ToolOutcome {
        let Some(path) = str_arg(args, "path").filter(|p| !p.trim().is_empty()) else {
            return ToolOutcome::err(
                "`path` is required: a spec file or directory, never the whole suite",
            );
        };
        if let Err(e) = check_test_path(&self.app_root, path) {
            return ToolOutcome::err(e);
        }
        let Some(exe) = self.soli_exe.as_ref() else {
            return ToolOutcome::err("could not locate the soli binary");
        };
        let mut cmd = Command::new(exe);
        cmd.arg("test")
            .arg(path)
            .arg("--no-coverage")
            .current_dir(&self.app_root)
            .env("NO_COLOR", "1")
            .stdin(Stdio::null());
        if let Some(filter) = str_arg(args, "filter") {
            cmd.arg("--filter").arg(filter);
        }
        let run = match run_with_timeout(cmd, self.test_timeout) {
            Ok(r) => r,
            Err(e) => return ToolOutcome::err(e),
        };
        if run.timed_out {
            return ToolOutcome::err(format!(
                "`soli test {path}` did not finish within {}s and was stopped",
                self.test_timeout.as_secs()
            ));
        }
        let report = summarize_test_output(&run.output, MAX_TEST_FAILURES, MAX_TEST_BYTES);
        let passed = run.success;
        ToolOutcome {
            value: json!({
                "path": path,
                "passed": passed,
                "exit_code": run.code,
                "summary": report.summary,
                "failures": if passed { Value::Null } else { Value::String(report.failures) },
                "truncated": report.truncated,
            }),
            is_error: !passed,
        }
    }
}

fn str_arg<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str)
}

fn initialize_result(params: &Value) -> Value {
    let asked = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or("");
    let version = if SUPPORTED_VERSIONS.contains(&asked) {
        asked
    } else {
        PROTOCOL_VERSION
    };
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "soli", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Attached to this app's running `soli serve --dev`. Start with `requests` or `errors`, open one with `request`, edit the code yourself, then `replay` the same request and run `test` on the spec you touched.",
    })
}

/// The tools, with JSON Schemas for their arguments.
pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "requests",
            "description": "List the last 64 requests the running dev server handled, newest first: id, method, path, status, elapsed_us, route and query_count. Read-only.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "request",
            "description": "One request's dev-bar snapshot: route, phase timings, every database query with its statement, binds and duration, outgoing HTTP and KV calls, and `n_plus_one` when the dev bar detects a repeated query. Read-only.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string", "description": "A request id from `requests`." } },
                "required": ["id"]
            }
        },
        {
            "name": "replay",
            "description": "Re-run a captured request through the app with its original cookies and body, and return the new request id and status. A request that is not GET or HEAD is replayed only with `confirm: true`.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "A request id from `requests`." },
                    "confirm": { "type": "boolean", "description": "Required to replay a POST, PUT, PATCH or DELETE." }
                },
                "required": ["id"]
            }
        },
        {
            "name": "errors",
            "description": "Open error groups (500s grouped by cause), most recent first, each with its latest sample: message, stack, a curl line to reproduce it and the handler's locals, all redacted when stored. Read-only.",
            "inputSchema": {
                "type": "object",
                "properties": { "limit": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Groups to return (default 20)." } }
            }
        },
        {
            "name": "graph",
            "description": "Find the code most relevant to a question in the app's code graph (`soli graph query --json`): each result with its callers, callees, routes, views and the collections it reads or writes, plus the notes earlier sessions left (`insights`, `stale` when the code changed since) and earlier questions that share its words. Needs `soli graph build` and SolidB; when they are not set up, the error says what is missing. Records the question in the graph's query log.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "description": "Seed results (default 6)." },
                    "hops": { "type": "integer", "minimum": 1, "maximum": 3, "description": "Graph expansion depth (default 1)." },
                    "kind": { "type": "string", "description": "Comma-separated node kinds, e.g. `method,controller`." },
                    "path": { "type": "string", "description": "Keep only files under this path prefix." }
                },
                "required": ["question"]
            }
        },
        {
            "name": "graph_note",
            "description": "Keep a finding about the code in the code graph (`soli graph note`) so later `graph` queries return it: what a flow really does, why a method exists, where a question led. Anchor it on the qualified names a `graph` result printed (`PostsController#create`, `posts`). Writes to the graph's note store only, never to the app.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "The finding, in a sentence or two." },
                    "on": { "type": "array", "items": { "type": "string" }, "description": "Qualified names (or graph keys) of the nodes it is about." },
                    "question": { "type": "string", "description": "The question it answers, if any." }
                },
                "required": ["text", "on"]
            }
        },
        {
            "name": "test",
            "description": "Run `soli test <path>` for one spec file or directory (never the whole suite), with a 120 s timeout. Returns failures only, truncated to 20 failures and 8 KB.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Spec file or directory, relative to the app root." },
                    "filter": { "type": "string", "description": "Run only tests whose name contains this text." }
                },
                "required": ["path"]
            }
        },
        {
            "name": "repl",
            "description": "Execute Soli code inside the running dev process, with its models, configuration and database connection, and return the result. Not read-only: the code can write. Pass the returned `repl_session_id` back to keep variables between calls.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "code": { "type": "string" },
                    "repl_session_id": { "type": "string", "description": "From a previous `repl` result, to continue that session." }
                },
                "required": ["code"]
            }
        }
    ])
}

/// Refuse a path that is the whole suite (the app root or its `tests/`), or
/// one outside the app.
fn check_test_path(app_root: &Path, path: &str) -> Result<(), String> {
    let root = app_root
        .canonicalize()
        .map_err(|e| format!("app root {}: {e}", app_root.display()))?;
    let target = root
        .join(path)
        .canonicalize()
        .map_err(|_| format!("{path} does not exist under {}", root.display()))?;
    if !target.starts_with(&root) {
        return Err(format!("{path} is outside the app"));
    }
    let whole_suite = target == root
        || ["tests", "test", "spec", "specs"]
            .iter()
            .any(|dir| target == root.join(dir));
    if whole_suite {
        return Err(format!(
            "{path} is the whole suite; pass the spec file or a directory inside it"
        ));
    }
    Ok(())
}

struct RunOutput {
    output: String,
    success: bool,
    code: Option<i32>,
    timed_out: bool,
}

fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<RunOutput, String> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start soli test: {e}"))?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = drain(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = drain(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                timed_out = true;
                break child.wait().ok();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("waiting on soli test failed: {e}")),
        }
    };
    let mut output = String::from_utf8_lossy(&out.join().unwrap_or_default()).into_owned();
    output.push_str(&String::from_utf8_lossy(&err.join().unwrap_or_default()));
    Ok(RunOutput {
        output,
        success: status.is_some_and(|s| s.success()) && !timed_out,
        code: status.and_then(|s| s.code()),
        timed_out,
    })
}

struct TestReport {
    summary: String,
    failures: String,
    truncated: bool,
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Keep the failing rows of `soli test` output (a row ending in `✗` and the
/// `┌─ … └─` box under it) and the closing summary. When a run fails with no
/// such row — a parse error, a server that would not start — keep the tail.
fn summarize_test_output(raw: &str, max_failures: usize, max_bytes: usize) -> TestReport {
    let clean = strip_ansi(raw);
    // Progress redraws use `\r`; keep the last state of each line.
    let lines: Vec<&str> = clean
        .lines()
        .map(|l| l.rsplit('\r').next().unwrap_or(l))
        .collect();

    let mut blocks: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in &lines {
        let trimmed = line.trim_end();
        if let Some(block) = current.as_mut() {
            block.push_str(trimmed);
            block.push('\n');
            if trimmed.trim_start().starts_with("└─") {
                blocks.push(current.take().unwrap());
            }
            continue;
        }
        if trimmed.ends_with(" ✗") {
            current = Some(format!("{trimmed}\n"));
        }
    }
    if let Some(block) = current {
        blocks.push(block);
    }

    let summary_start = lines
        .iter()
        .rposition(|l| matches!(l.trim(), "❌" | "✓"))
        .map(|i| i + 1);
    let summary = summary_start
        .map(|i| {
            lines[i..]
                .iter()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                .take(4)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    if blocks.is_empty() {
        let tail_from = lines.len().saturating_sub(40);
        let tail = lines[tail_from..].join("\n");
        let (text, truncated) = truncate_bytes(tail.trim(), max_bytes);
        return TestReport {
            summary,
            failures: text,
            truncated,
        };
    }

    let mut truncated = blocks.len() > max_failures;
    let joined = blocks
        .into_iter()
        .take(max_failures)
        .collect::<Vec<_>>()
        .join("\n");
    let (text, cut) = truncate_bytes(&joined, max_bytes);
    truncated |= cut;
    TestReport {
        summary,
        failures: text,
        truncated,
    }
}

fn truncate_bytes(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_string(), false);
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}\n… truncated", &s[..end]), true)
}

#[cfg(test)]
mod tests;
