//! `soli dap` driven over stdio, as an editor drives it: launch a script,
//! stop at a breakpoint, read the stack, scopes and variables, evaluate in
//! the paused frame, step, stop at `debug()`, and run to the end.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

const PROGRAM: &str = r#"def add(a, b)
  total = a + b
  total * 2
end

user = {"name": "Ada", "tags": ["x", "y"]}
print("start")
result = add(2, 3)
print("result #{result}")
debug()
print("after debug")
"#;

struct Client {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Value>,
    seq: i64,
    output: String,
}

fn reader(stdout: ChildStdout) -> Receiver<Value> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut input = BufReader::new(stdout);
        loop {
            let mut length = None;
            loop {
                let mut line = String::new();
                if input.read_line(&mut line).unwrap_or(0) == 0 {
                    return;
                }
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                if let Some(value) = line.strip_prefix("Content-Length:") {
                    length = value.trim().parse::<usize>().ok();
                }
            }
            let mut body = vec![0u8; length.expect("a Content-Length header")];
            input.read_exact(&mut body).unwrap();
            if tx.send(serde_json::from_slice(&body).unwrap()).is_err() {
                return;
            }
        }
    });
    rx
}

impl Client {
    fn start() -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_soli"))
            .arg("dap")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("soli dap starts");
        let stdin = child.stdin.take().unwrap();
        let messages = reader(child.stdout.take().unwrap());
        Client {
            child,
            stdin,
            messages,
            seq: 0,
            output: String::new(),
        }
    }

    fn next_message(&mut self) -> Value {
        let message = self
            .messages
            .recv_timeout(Duration::from_secs(20))
            .expect("the adapter answers in time");
        if message["event"] == "output" {
            self.output
                .push_str(message["body"]["output"].as_str().unwrap_or_default());
        }
        message
    }

    fn request(&mut self, command: &str, arguments: Value) -> Value {
        self.seq += 1;
        let body =
            json!({"seq": self.seq, "type": "request", "command": command, "arguments": arguments})
                .to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        self.stdin.flush().unwrap();
        let seq = self.seq;
        loop {
            let message = self.next_message();
            if message["type"] == "response" && message["request_seq"] == seq {
                assert_eq!(message["success"], true, "{command}: {message}");
                return message["body"].clone();
            }
        }
    }

    fn wait_event(&mut self, event: &str) -> Value {
        loop {
            let message = self.next_message();
            if message["type"] == "event" && message["event"] == event {
                return message["body"].clone();
            }
        }
    }

    fn top_frames(&mut self) -> Vec<(String, i64)> {
        self.request("stackTrace", json!({"threadId": 1}))["stackFrames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["name"].as_str().unwrap().to_string(),
                    f["line"].as_i64().unwrap(),
                )
            })
            .collect()
    }

    fn variables(&mut self, reference: &Value) -> Vec<(String, String)> {
        self.request("variables", json!({"variablesReference": reference}))["variables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                (
                    v["name"].as_str().unwrap().to_string(),
                    v["value"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[test]
fn a_script_stops_steps_and_shows_its_variables() {
    script_scenario("tree");
}

/// The same stops, stack, locals and steps on the VM, which a debugger
/// stops through the statement markers it compiles in.
#[test]
fn the_vm_stops_where_the_tree_walker_does() {
    script_scenario("vm");
}

fn script_scenario(engine: &str) {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("prog.sl");
    std::fs::write(&program, PROGRAM).unwrap();
    let program = program.to_string_lossy().to_string();

    let mut client = Client::start();
    let capabilities = client.request("initialize", json!({"adapterID": "soli"}));
    assert_eq!(capabilities["supportsConfigurationDoneRequest"], true);
    client.wait_event("initialized");
    client.request("launch", json!({"program": program, "engine": engine}));
    let set = client.request(
        "setBreakpoints",
        json!({"source": {"path": program}, "breakpoints": [{"line": 2}]}),
    );
    assert_eq!(set["breakpoints"][0]["verified"], true);
    client.request("configurationDone", json!({}));

    let stopped = client.wait_event("stopped");
    assert_eq!(stopped["reason"], "breakpoint");
    assert_eq!(
        client.top_frames(),
        [("add".to_string(), 2), ("<main>".to_string(), 8)]
    );

    let scopes = client.request("scopes", json!({"frameId": 1}));
    let locals = client.variables(&scopes["scopes"][0]["variablesReference"]);
    assert_eq!(
        locals,
        [
            ("a".to_string(), "2".to_string()),
            ("b".to_string(), "3".to_string())
        ]
    );
    let globals = client.variables(&scopes["scopes"][1]["variablesReference"]);
    let names: Vec<&str> = globals.iter().map(|(n, _)| n.as_str()).collect();
    // The VM keeps a script's top-level variables in the `<main>` frame's
    // slots: `user` is a local of that frame there, a global on the
    // tree-walker.
    let user_scope = if engine == "vm" {
        assert_eq!(names, ["add"], "builtins stay out of Globals");
        let main = client.request("scopes", json!({"frameId": 0}));
        main["scopes"][0]["variablesReference"].clone()
    } else {
        assert_eq!(names, ["add", "user"], "builtins stay out of Globals");
        scopes["scopes"][1]["variablesReference"].clone()
    };

    // Expand the hash.
    let globals_raw = client.request("variables", json!({"variablesReference": user_scope}));
    let user = globals_raw["variables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "user")
        .unwrap()["variablesReference"]
        .clone();
    let fields = client.variables(&user);
    assert_eq!(fields[0], ("name".to_string(), "\"Ada\"".to_string()));
    assert_eq!(fields[1], ("tags".to_string(), "Array(2)".to_string()));

    let evaluated = client.request("evaluate", json!({"expression": "a * 10", "frameId": 1}));
    assert_eq!(evaluated["result"], "20");

    client.request("next", json!({"threadId": 1}));
    client.wait_event("stopped");
    assert_eq!(client.top_frames()[0], ("add".to_string(), 3));

    client.request("stepOut", json!({"threadId": 1}));
    client.wait_event("stopped");
    assert_eq!(client.top_frames(), [("<main>".to_string(), 9)]);

    client.request("continue", json!({"threadId": 1}));
    let at_debug = client.wait_event("stopped");
    assert!(
        at_debug["description"]
            .as_str()
            .unwrap()
            .starts_with("debug() at"),
        "{at_debug}"
    );

    client.request("continue", json!({"threadId": 1}));
    let exited = client.wait_event("exited");
    assert_eq!(exited["exitCode"], 0);
    client.wait_event("terminated");
    assert!(
        client.output.contains("start\n") && client.output.contains("after debug\n"),
        "the program's output comes back as output events: {:?}",
        client.output
    );
}

#[test]
fn an_uncaught_error_is_reported_and_ends_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("boom.sl");
    std::fs::write(&program, "x = 1\nprint(nope)\n").unwrap();
    let program = program.to_string_lossy().to_string();

    let mut client = Client::start();
    client.request("initialize", json!({}));
    client.request("launch", json!({"program": program, "stopOnEntry": true}));
    client.request("configurationDone", json!({}));
    let stopped = client.wait_event("stopped");
    assert_eq!(
        stopped["reason"], "step",
        "stopOnEntry stops at the first line"
    );
    assert_eq!(client.top_frames(), [("<main>".to_string(), 1)]);
    client.request("continue", json!({"threadId": 1}));
    let exited = client.wait_event("exited");
    assert_eq!(exited["exitCode"], 1);
    assert!(
        client.output.contains("Undefined variable 'nope'"),
        "{}",
        client.output
    );
}

#[test]
fn launch_refuses_a_missing_program() {
    let mut client = Client::start();
    client.request("initialize", json!({}));
    client.seq += 1;
    let body = json!({"seq": client.seq, "type": "request", "command": "launch",
        "arguments": {"program": "/nonexistent/x.sl"}})
    .to_string();
    write!(
        client.stdin,
        "Content-Length: {}\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
    client.stdin.flush().unwrap();
    loop {
        let message = client.next_message();
        if message["type"] == "response" && message["command"] == "launch" {
            assert_eq!(message["success"], false);
            assert!(message["message"]
                .as_str()
                .unwrap()
                .contains("is not a file"));
            break;
        }
    }
}

/// `"mode": "test"`: the specs run under `soli test` (one worker), the
/// debugger stops in a helper a test calls, and the runner's failing exit
/// comes back as the session's exit code instead of killing the adapter.
#[test]
fn specs_run_under_the_debugger() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env.test"), "SOLIDB_DATABASE=dap_spec\n").unwrap();
    std::fs::create_dir_all(dir.path().join("specs")).unwrap();
    let spec = dir.path().join("specs/math_spec.sl");
    std::fs::write(
        &spec,
        r#"def double(x)
  y = x * 2
  y
end

describe("math") do
  test("doubles") do
    value = double(4)
    assert_eq(value, 8)
  end

  test("fails") do
    assert_eq(double(1), 3)
  end
end
"#,
    )
    .unwrap();
    let spec = spec.to_string_lossy().to_string();

    let mut client = Client::start();
    client.request("initialize", json!({}));
    client.request(
        "launch",
        json!({"program": spec, "mode": "test", "cwd": dir.path().to_string_lossy()}),
    );
    client.request(
        "setBreakpoints",
        json!({"source": {"path": spec}, "breakpoints": [{"line": 2}]}),
    );
    client.request("configurationDone", json!({}));
    client.wait_event("stopped");
    let frames = client.top_frames();
    assert_eq!(frames[0], ("double".to_string(), 2));
    assert_eq!(frames.last().unwrap().0, "<main>");
    let scopes = client.request("scopes", json!({"frameId": frames.len() - 1}));
    let locals = client.variables(&scopes["scopes"][0]["variablesReference"]);
    assert_eq!(locals, [("x".to_string(), "4".to_string())]);

    client.request(
        "setBreakpoints",
        json!({"source": {"path": spec}, "breakpoints": []}),
    );
    client.request("continue", json!({"threadId": 1}));
    let exited = client.wait_event("exited");
    assert_eq!(exited["exitCode"], 1, "one spec fails");
    client.wait_event("terminated");
    assert!(
        client.output.contains("2 tests, 1 failed"),
        "{}",
        client.output
    );
}
