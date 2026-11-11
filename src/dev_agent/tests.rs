use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use super::*;

/// One request the fake dev server received.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// `(method, path, status, extra headers, body)` the fake server answers with.
type Route = (
    &'static str,
    &'static str,
    u16,
    Vec<(&'static str, &'static str)>,
    String,
);

/// A stand-in for `soli serve --dev`: answers by `(method, path)` and records
/// every request.
fn fake_backend(routes: Vec<Route>) -> (u16, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let mut parts = request_line.split_whitespace();
            let method = parts.next().unwrap_or("").to_string();
            let path = parts.next().unwrap_or("").to_string();
            let mut headers = Vec::new();
            let mut len = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                if let Some((k, v)) = line.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                    headers.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            log.lock().unwrap().push(Seen {
                method: method.clone(),
                path: path.clone(),
                headers,
                body: String::from_utf8_lossy(&body).into_owned(),
            });
            let hit = routes.iter().find(|(m, p, ..)| *m == method && *p == path);
            let (status, extra, body) = match hit {
                Some((_, _, s, h, b)) => (*s, h.clone(), b.clone()),
                None => (404, vec![], "Not Found".to_string()),
            };
            let mut resp = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (k, v) in extra {
                resp.push_str(&format!("{k}: {v}\r\n"));
            }
            resp.push_str("\r\n");
            resp.push_str(&body);
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    (port, seen)
}

fn agent_for(port: u16, token: Option<&str>) -> Agent {
    Agent::new(
        DevClient {
            port,
            dev_token: token.map(str::to_string),
            timeout: Duration::from_secs(5),
        },
        std::env::temp_dir(),
    )
}

/// Run a whole stdio session and return the reply lines.
fn session(agent: &mut Agent, messages: &[Value]) -> Vec<Value> {
    let input: String = messages.iter().map(|m| format!("{m}\n")).collect();
    let mut out = Vec::new();
    serve(agent, input.as_bytes(), &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).expect("one JSON message per line"))
        .collect()
}

fn tool_text(reply: &Value) -> Value {
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

#[test]
fn stdio_handshake_list_and_call() {
    let requests = json!({ "requests": [{ "id": "r_1", "method": "GET", "path": "/orders",
        "status": 200, "elapsed_us": 412000, "route": "orders#index", "query_count": 53 }] });
    let (port, seen) = fake_backend(vec![(
        "GET",
        "/__solidev/requests",
        200,
        vec![("Content-Type", "application/json")],
        requests.to_string(),
    )]);
    let mut agent = agent_for(port, None);
    let replies = session(
        &mut agent,
        &[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": { "name": "t", "version": "0" } } }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": { "name": "requests", "arguments": {} } }),
            json!({ "jsonrpc": "2.0", "id": 4, "method": "ping" }),
            json!({ "jsonrpc": "2.0", "id": 5, "method": "resources/list" }),
        ],
    );

    // The notification gets no reply: five answers for six messages.
    assert_eq!(replies.len(), 5);

    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-06-18");
    assert!(replies[0]["result"]["capabilities"]["tools"].is_object());
    assert_eq!(replies[0]["result"]["serverInfo"]["name"], "soli");

    let names: Vec<&str> = replies[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["requests", "request", "replay", "errors", "graph", "test", "repl"]
    );
    let repl = replies[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "repl")
        .unwrap();
    assert!(repl["description"]
        .as_str()
        .unwrap()
        .contains("inside the running dev process"));

    assert_eq!(replies[2]["result"]["isError"], false);
    assert_eq!(replies[2]["result"]["content"][0]["type"], "text");
    assert_eq!(tool_text(&replies[2])["requests"][0]["id"], "r_1");

    assert_eq!(replies[3]["result"], json!({}));
    assert_eq!(replies[4]["error"]["code"], -32601);

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].path, "/__solidev/requests");
    assert_eq!(
        seen[0].header("host"),
        Some(format!("127.0.0.1:{port}").as_str())
    );
}

#[test]
fn unknown_protocol_version_gets_ours() {
    let mut agent = agent_for(1, None);
    let reply = agent
        .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "protocolVersion": "1999-01-01" } }))
        .unwrap();
    assert_eq!(reply["result"]["protocolVersion"], PROTOCOL_VERSION);
}

#[test]
fn malformed_line_is_a_parse_error() {
    let mut agent = agent_for(1, None);
    let mut out = Vec::new();
    serve(&mut agent, "{not json\n".as_bytes(), &mut out).unwrap();
    let reply: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(reply["error"]["code"], -32700);
}

#[test]
fn replay_of_a_post_without_confirm_sends_no_replay() {
    let snapshot = json!({ "id": "r_9", "method": "POST", "path": "/orders" });
    let (port, seen) = fake_backend(vec![
        (
            "GET",
            "/__solidev/requests/r_9",
            200,
            vec![],
            snapshot.to_string(),
        ),
        (
            "POST",
            "/__solidev/replay/r_9",
            201,
            vec![("X-Soli-Replay", "1"), ("X-Soli-Request-Id", "r_10")],
            String::new(),
        ),
    ]);
    let mut agent = agent_for(port, None);
    let reply = agent
        .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "replay", "arguments": { "id": "r_9" } } }))
        .unwrap();
    assert_eq!(reply["result"]["isError"], true);
    assert!(tool_text(&reply)["error"]
        .as_str()
        .unwrap()
        .contains("confirm: true"));
    assert!(
        seen.lock().unwrap().iter().all(|s| s.method != "POST"),
        "no replay may be sent without confirm"
    );

    let reply = agent
        .handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": { "name": "replay", "arguments": { "id": "r_9", "confirm": true } } }))
        .unwrap();
    assert_eq!(reply["result"]["isError"], false);
    let result = tool_text(&reply);
    assert_eq!(result["new_request_id"], "r_10");
    assert_eq!(result["status"], 201);

    let seen = seen.lock().unwrap();
    let post = seen.iter().find(|s| s.method == "POST").unwrap();
    assert_eq!(
        post.header("origin"),
        Some(format!("http://127.0.0.1:{port}").as_str())
    );
    assert_eq!(
        post.header("host"),
        Some(format!("127.0.0.1:{port}").as_str())
    );
}

#[test]
fn replay_of_a_get_needs_no_confirm() {
    let snapshot = json!({ "id": "r_1", "method": "GET", "path": "/orders" });
    let (port, _seen) = fake_backend(vec![
        (
            "GET",
            "/__solidev/requests/r_1",
            200,
            vec![],
            snapshot.to_string(),
        ),
        (
            "POST",
            "/__solidev/replay/r_1",
            200,
            vec![("X-Soli-Replay", "1"), ("X-Soli-Request-Id", "r_2")],
            "<html></html>".into(),
        ),
    ]);
    let mut agent = agent_for(port, None);
    let reply = agent
        .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "replay", "arguments": { "id": "r_1" } } }))
        .unwrap();
    assert_eq!(reply["result"]["isError"], false);
    assert_eq!(tool_text(&reply)["new_request_id"], "r_2");
}

#[test]
fn evicted_request_is_a_tool_error() {
    let (port, _seen) = fake_backend(vec![(
        "GET",
        "/__solidev/requests/old",
        404,
        vec![],
        json!({ "error": "request old is no longer in the dev buffer" }).to_string(),
    )]);
    let mut agent = agent_for(port, None);
    let reply = agent
        .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "request", "arguments": { "id": "old" } } }))
        .unwrap();
    assert!(
        reply.get("error").is_none(),
        "a tool failure is not a JSON-RPC error"
    );
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(tool_text(&reply)["status"], 404);
}

#[test]
fn repl_sends_the_token_and_session() {
    let (port, seen) = fake_backend(vec![(
        "POST",
        "/__dev/repl",
        200,
        vec![],
        json!({ "result": "3", "error": null, "repl_session_id": "s1" }).to_string(),
    )]);
    let mut agent = agent_for(port, Some("tok-123"));
    let reply = agent
        .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "repl", "arguments": { "code": "1 + 2", "repl_session_id": "s0" } } }))
        .unwrap();
    assert_eq!(tool_text(&reply)["repl_session_id"], "s1");
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].header("x-soli-dev-token"), Some("tok-123"));
    assert!(seen[0].header("origin").is_some());
    let body: Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["code"], "1 + 2");
    assert_eq!(body["repl_session_id"], "s0");
}

#[test]
fn unreachable_server_is_a_tool_error() {
    // Bind then drop: nothing listens on this port any more.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut agent = agent_for(port, None);
    let reply = agent
        .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "requests", "arguments": {} } }))
        .unwrap();
    assert_eq!(reply["result"]["isError"], true);
    assert!(tool_text(&reply)["error"]
        .as_str()
        .unwrap()
        .contains("not reachable"));
}

/// Unix only: elsewhere `pid_alive` cannot tell a dead pid (there is no
/// cheap check), so the connection attempt decides, as `discovery` documents.
#[cfg(unix)]
#[test]
fn dead_pid_fails_before_any_connection() {
    let (port, seen) = fake_backend(vec![]);
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    let info = discovery::DevServerInfo {
        pid,
        port,
        app_root: dir.path().display().to_string(),
        dev_token: None,
    };
    std::fs::create_dir_all(dir.path().join(".soli")).unwrap();
    std::fs::write(
        dir.path().join(discovery::DISCOVERY_FILE),
        serde_json::to_string(&info).unwrap(),
    )
    .unwrap();

    let code = run(Options {
        dir: dir.path().to_path_buf(),
        port: None,
        dev_token: None,
    });
    assert_eq!(code, 1);
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn test_tool_requires_a_path_and_refuses_the_whole_suite() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("tests/models")).unwrap();
    let mut agent = Agent::new(
        DevClient {
            port: 1,
            dev_token: None,
            timeout: Duration::from_secs(1),
        },
        dir.path().to_path_buf(),
    );
    for args in [
        json!({}),
        json!({ "path": "tests" }),
        json!({ "path": "." }),
    ] {
        let reply = agent
            .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "test", "arguments": args } }))
            .unwrap();
        assert_eq!(reply["result"]["isError"], true, "{args}");
    }
    assert!(check_test_path(dir.path(), "tests/models").is_ok());
    assert!(check_test_path(dir.path(), "../").is_err());
}

#[test]
fn test_output_keeps_failures_only() {
    let raw = "\x1b[32mtests/models\x1b[0m\n  user_spec.sl   12ms   4 ✓\n  order_spec.sl  30ms   2 ✗\n  ┌─ 1 test failure\n  │\n  │  ✗ totals\n  │     → expected 3, got 2\n  └─\n\n\n❌ \n  1 file passed, 1 failed (2 total)\n  6 tests, 1 failed\n";
    let report = summarize_test_output(raw, 20, 8 * 1024);
    assert!(report.failures.contains("order_spec.sl"));
    assert!(report.failures.contains("expected 3, got 2"));
    assert!(!report.failures.contains("user_spec.sl"));
    assert!(report.summary.contains("1 failed"));
    assert!(!report.truncated);

    let many: String = (0..30)
        .map(|i| format!("  f{i}_spec.sl 1ms 1 ✗\n  ┌─ ✗ boom\n  └─\n"))
        .collect();
    let report = summarize_test_output(&many, 20, 8 * 1024);
    assert!(report.truncated);
    assert!(report.failures.contains("f19_spec"));
    assert!(!report.failures.contains("f20_spec"));
}
