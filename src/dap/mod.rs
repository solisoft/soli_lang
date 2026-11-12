//! `soli dap`: a Debug Adapter Protocol server on stdin/stdout, so VS Code,
//! Zed, nvim-dap and Helix can set breakpoints in a Soli script, step through
//! it, and look at its frames and variables.
//!
//! One process per debugging session. The editor sends `launch` with the
//! script; the program starts at `configurationDone`, on its own thread, on
//! the tree-walker (the engine the debugger can stop). Stopping, stepping
//! and inspecting go through `crate::debug::session`; everything that reads
//! the paused program runs on the program's thread.
//!
//! The protocol owns stdout, so the program's own output is captured (fd 1
//! is pointed at a pipe) and forwarded as `output` events.

use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value as Json};

use crate::debug::session::{self, Command, Event};

/// How long an inspection waits for the program's thread to answer.
const INSPECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The one thread the adapter reports: the program's.
const THREAD_ID: i64 = 1;

type Out = Arc<Mutex<Box<dyn Write + Send>>>;

/// Writes protocol messages, numbering them.
#[derive(Clone)]
struct Wire {
    out: Out,
    seq: Arc<AtomicI64>,
}

impl Wire {
    fn send(&self, mut message: Json) {
        message["seq"] = json!(self.seq.fetch_add(1, Ordering::Relaxed));
        let body = message.to_string();
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        let _ = write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body);
        let _ = out.flush();
    }

    fn event(&self, event: &str, body: Json) {
        self.send(json!({"type": "event", "event": event, "body": body}));
    }

    fn respond(&self, request: &Json, result: Result<Json, String>) {
        let command = request["command"].clone();
        let request_seq = request["seq"].clone();
        match result {
            Ok(body) => self.send(json!({
                "type": "response", "request_seq": request_seq, "success": true,
                "command": command, "body": body,
            })),
            Err(message) => self.send(json!({
                "type": "response", "request_seq": request_seq, "success": false,
                "command": command, "message": message,
                "body": {"error": {"id": 1, "format": message}},
            })),
        }
    }
}

/// Read one `Content-Length`-framed message.
fn read_message(input: &mut impl BufRead) -> Option<Json> {
    let mut length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = value.trim().parse().ok();
        }
    }
    let mut body = vec![0u8; length?];
    input.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

/// What `launch` asked for.
#[derive(Default, Clone)]
struct Launch {
    program: String,
    stop_on_entry: bool,
    /// `"test"`: run `program` (a spec file or directory) with `soli test`.
    test: bool,
    /// `soli test --filter`.
    filter: Option<String>,
}

/// Runs specs for `launch` in test mode: `soli test` lives in the CLI, which
/// hands it over before starting the adapter. Arguments: the spec path and
/// the optional name filter.
pub type TestRunner = fn(&str, Option<&str>);

static TEST_RUNNER: std::sync::OnceLock<TestRunner> = std::sync::OnceLock::new();

/// Let `launch` with `"mode": "test"` run specs (called by the CLI).
pub fn set_test_runner(runner: TestRunner) {
    let _ = TEST_RUNNER.set(runner);
}

struct Adapter {
    wire: Wire,
    commands: Sender<Command>,
    launch: Option<Launch>,
    /// Set while the program is stopped: inspections are answered only then.
    stopped: Arc<AtomicBool>,
    started: bool,
}

impl Adapter {
    fn handle(&mut self, request: &Json) -> Option<Result<Json, String>> {
        let args = &request["arguments"];
        let result = match request["command"].as_str().unwrap_or_default() {
            "initialize" => {
                let capabilities = json!({
                    "supportsConfigurationDoneRequest": true,
                    "supportsEvaluateForHovers": true,
                    "supportsTerminateRequest": true,
                });
                self.wire.respond(request, Ok(capabilities));
                self.wire.event("initialized", json!({}));
                return None;
            }
            "launch" => self.launch(args),
            "attach" => Err("soli dap launches a script; attaching is not supported yet".into()),
            "setBreakpoints" => Ok(set_breakpoints(args)),
            "setExceptionBreakpoints" => Ok(json!({"breakpoints": []})),
            "configurationDone" => {
                self.start();
                Ok(json!({}))
            }
            "threads" => Ok(json!({"threads": [{"id": THREAD_ID, "name": "main"}]})),
            "stackTrace" => self.inspect(|interpreter| Ok(interpreter.debugger_stack_trace())),
            "scopes" => {
                let depth = args["frameId"].as_u64().unwrap_or(0) as usize;
                self.inspect(move |interpreter| Ok(interpreter.debugger_scopes(depth)))
            }
            "variables" => {
                let reference = args["variablesReference"].as_u64().unwrap_or(0) as usize;
                self.inspect(move |interpreter| Ok(interpreter.debugger_variables(reference)))
            }
            "evaluate" => {
                let expression = args["expression"].as_str().unwrap_or_default().to_string();
                let depth = args["frameId"].as_u64().map(|d| d as usize);
                self.inspect(move |interpreter| interpreter.debugger_evaluate(&expression, depth))
            }
            "continue" => self
                .resume(Command::Continue)
                .map(|_| json!({"allThreadsContinued": true})),
            "next" => self.resume(Command::Next),
            "stepIn" => self.resume(Command::StepIn),
            "stepOut" => self.resume(Command::StepOut),
            "pause" => {
                session::request_pause();
                Ok(json!({}))
            }
            "disconnect" | "terminate" => {
                self.wire.respond(request, Ok(json!({})));
                std::process::exit(0);
            }
            other => Err(format!("unsupported request {other}")),
        };
        Some(result)
    }

    fn launch(&mut self, args: &Json) -> Result<Json, String> {
        let program = args["program"]
            .as_str()
            .filter(|p| !p.is_empty())
            .ok_or("launch needs \"program\": the .sl script to run")?;
        if let Some(cwd) = args["cwd"].as_str().filter(|c| !c.is_empty()) {
            std::env::set_current_dir(cwd).map_err(|e| format!("cannot use cwd {cwd}: {e}"))?;
        }
        let program = session::normalize_path(program);
        let test = match args["mode"].as_str().unwrap_or("script") {
            "script" => false,
            "test" => true,
            other => return Err(format!("mode is \"script\" or \"test\", not \"{other}\"")),
        };
        let exists = if test {
            std::path::Path::new(&program).exists()
        } else {
            std::path::Path::new(&program).is_file()
        };
        if !exists {
            return Err(format!("{program} is not a file"));
        }
        if test && TEST_RUNNER.get().is_none() {
            return Err("this build cannot run specs under the debugger".into());
        }
        self.launch = Some(Launch {
            program,
            stop_on_entry: args["stopOnEntry"].as_bool().unwrap_or(false),
            test,
            filter: args["filter"].as_str().map(str::to_string),
        });
        Ok(json!({}))
    }

    /// Run the launched program on its own thread.
    fn start(&mut self) {
        if self.started {
            return;
        }
        let Some(launch) = self.launch.clone() else {
            return;
        };
        self.started = true;
        let spawned = std::thread::Builder::new()
            .name("soli-debuggee".to_string())
            .stack_size(crate::serve::server_constants::worker_stack_bytes())
            .spawn(move || {
                if launch.stop_on_entry {
                    crate::interpreter::executor::debugger::stop_on_entry();
                }
                if launch.test {
                    // The runner marks its (single) worker thread as the
                    // debuggee; its own exits come back through
                    // `session::exit`.
                    if let Some(runner) = TEST_RUNNER.get() {
                        runner(&launch.program, launch.filter.as_deref());
                    }
                    let _ = std::io::stdout().flush();
                    session::send(Event::Finished { error: None });
                    return;
                }
                crate::debug::set_thread_listener(crate::debug::DEBUGGEE, true);
                let result = crate::debug_script_file(std::path::Path::new(&launch.program));
                let _ = std::io::stdout().flush();
                session::send(Event::Finished {
                    error: result.err(),
                });
            });
        if let Err(e) = spawned {
            session::send(Event::Finished {
                error: Some(format!("could not start the program: {e}")),
            });
        }
    }

    /// Run `f` on the paused program's thread and wait for its answer.
    fn inspect(
        &self,
        f: impl FnOnce(&mut crate::interpreter::executor::Interpreter) -> Result<Json, String>
            + Send
            + 'static,
    ) -> Result<Json, String> {
        if !self.stopped.load(Ordering::Acquire) {
            return Err("the program is running".into());
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.commands
            .send(Command::Inspect(Box::new(move |interpreter| {
                let _ = tx.send(f(interpreter));
            })))
            .map_err(|_| "the program has ended".to_string())?;
        rx.recv_timeout(INSPECT_TIMEOUT)
            .map_err(|_| "the program did not answer".to_string())?
    }

    fn resume(&self, command: Command) -> Result<Json, String> {
        if !self.stopped.swap(false, Ordering::AcqRel) {
            return Err("the program is not stopped".into());
        }
        self.commands
            .send(command)
            .map_err(|_| "the program has ended".to_string())?;
        Ok(json!({}))
    }
}

fn set_breakpoints(args: &Json) -> Json {
    let path = args["source"]["path"].as_str().unwrap_or_default();
    let lines: Vec<u32> = args["breakpoints"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|b| b["line"].as_u64())
                .map(|l| l as u32)
                .collect()
        })
        .unwrap_or_default();
    session::set_breakpoints(path, lines.iter().copied());
    let breakpoints: Vec<Json> = lines
        .iter()
        .map(|line| json!({"verified": true, "line": line}))
        .collect();
    json!({"breakpoints": breakpoints})
}

/// Forward what the program's thread reports as protocol events.
fn pump_events(wire: Wire, events: std::sync::mpsc::Receiver<Event>, stopped: Arc<AtomicBool>) {
    for event in events {
        match event {
            Event::Stopped {
                reason,
                description,
            } => {
                stopped.store(true, Ordering::Release);
                wire.event(
                    "stopped",
                    json!({
                        "reason": reason, "description": description,
                        "threadId": THREAD_ID, "allThreadsStopped": true,
                    }),
                );
            }
            Event::Finished { error } => {
                // Let the output pump deliver what the program printed last.
                std::thread::sleep(Duration::from_millis(150));
                if let Some(message) = &error {
                    wire.event(
                        "output",
                        json!({"category": "stderr", "output": format!("{message}\n")}),
                    );
                }
                let code = i64::from(error.is_some());
                wire.event("exited", json!({"exitCode": code}));
                wire.event("terminated", json!({}));
            }
        }
    }
}

/// Keep the real stdout for the protocol and route fd 1 into a pipe whose
/// lines become `output` events.
#[cfg(unix)]
fn capture_stdout(wire_out: &mut Option<Out>) -> Result<std::fs::File, String> {
    use std::os::unix::io::FromRawFd;
    let _ = std::io::stdout().flush();
    let protocol = unsafe { libc::dup(1) };
    if protocol < 0 {
        return Err("could not duplicate stdout".into());
    }
    let mut fds = [0i32; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err("could not create a pipe for the program's output".into());
    }
    if unsafe { libc::dup2(fds[1], 1) } < 0 {
        return Err("could not redirect stdout".into());
    }
    unsafe { libc::close(fds[1]) };
    *wire_out = Some(Arc::new(Mutex::new(Box::new(unsafe {
        std::fs::File::from_raw_fd(protocol)
    }))));
    Ok(unsafe { std::fs::File::from_raw_fd(fds[0]) })
}

#[cfg(not(unix))]
fn capture_stdout(_wire_out: &mut Option<Out>) -> Result<std::fs::File, String> {
    Err("soli dap captures the program's output on Unix only".into())
}

/// Run the adapter until the editor disconnects. Returns the exit code.
pub fn run() -> i32 {
    let mut out: Option<Out> = None;
    let program_output = match capture_stdout(&mut out) {
        Ok(reader) => Some(reader),
        Err(e) => {
            eprintln!("soli dap: {e}; the program's output may disturb the protocol");
            None
        }
    };
    let out = out.unwrap_or_else(|| Arc::new(Mutex::new(Box::new(std::io::stdout()))));
    let wire = Wire {
        out,
        seq: Arc::new(AtomicI64::new(1)),
    };
    if let Some(reader) = program_output {
        let wire = wire.clone();
        std::thread::spawn(move || {
            let mut lines = BufReader::new(reader);
            loop {
                let mut line = String::new();
                match lines.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => wire.event("output", json!({"category": "stdout", "output": line})),
                }
            }
        });
    }

    let (commands, events) = match session::open() {
        Ok(channels) => channels,
        Err(e) => {
            eprintln!("soli dap: {e}");
            return 1;
        }
    };
    let stopped = Arc::new(AtomicBool::new(false));
    {
        let wire = wire.clone();
        let stopped = stopped.clone();
        std::thread::spawn(move || pump_events(wire, events, stopped));
    }

    let mut adapter = Adapter {
        wire: wire.clone(),
        commands,
        launch: None,
        stopped,
        started: false,
    };
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    while let Some(request) = read_message(&mut input) {
        if request["type"] != "request" {
            continue;
        }
        if let Some(result) = adapter.handle(&request) {
            wire.respond(&request, result);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_framed_message_is_read_whole() {
        let body = r#"{"seq":1,"type":"request","command":"threads"}"#;
        let framed = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        let mut input = std::io::Cursor::new(framed.into_bytes());
        let message = read_message(&mut input).unwrap();
        assert_eq!(message["command"], "threads");
        assert!(read_message(&mut input).is_none());
    }

    #[test]
    fn breakpoints_answer_verified_lines() {
        let _ = session::open();
        let answer = set_breakpoints(&json!({
            "source": {"path": "/tmp/dap_spec.sl"},
            "breakpoints": [{"line": 3}, {"line": 7}],
        }));
        assert_eq!(
            answer["breakpoints"][1],
            json!({"verified": true, "line": 7})
        );
    }
}
