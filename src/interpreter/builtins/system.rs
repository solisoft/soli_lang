//! System command execution builtin.
//!
//! Provides async shell command execution with auto-resolving futures.

use std::cell::RefCell;
use std::collections::HashMap;
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFunction, Value};

pub fn register_system_builtins(env: &mut Environment) {
    // System class
    env.define("System".to_string(), system_class());
}

fn system_class() -> Value {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // System.run(command) - Run command asynchronously, no auto-shell.
    //
    // SEC-074: The Arc<Mutex<FutureState>> is sent across thread::spawn
    // boundaries. FutureState::Pending holds a Receiver which is !Send. This
    // compiles because the receiver lives entirely on the spawned thread
    // and Arc/Mutex add Sync — but the lint is a real concern: if a future
    // is polled from a different thread or FutureState gains Send fields,
    // this becomes UB. Tracking in SEC-074.
    #[allow(clippy::arc_with_non_send_sync)]
    methods.insert(
        "run".to_string(),
        Rc::new(NativeFunction::new("System.run", Some(1), |args| {
            let (program, args_vec) = parse_argv(&args[0], "System.run")?;
            Ok(spawn_future(program, args_vec))
        })),
    );

    // System.run_sync(command) - Run command synchronously (blocking), no auto-shell.
    methods.insert(
        "run_sync".to_string(),
        Rc::new(NativeFunction::new("System.run_sync", Some(1), |args| {
            let (program, args_vec) = parse_argv(&args[0], "System.run_sync")?;
            let result = execute_command(&program, &args_vec)?;
            Ok(result_to_hash(result))
        })),
    );

    // System.shell(command) - Run command via `sh -c` (explicit opt-in to shell).
    // SEC-074: Same Arc<Mutex<FutureState>> cross-thread concern as System.run.
    #[allow(clippy::arc_with_non_send_sync)]
    methods.insert(
        "shell".to_string(),
        Rc::new(NativeFunction::new("System.shell", Some(1), |args| {
            let (program, args_vec) = parse_shell(&args[0], "System.shell")?;
            Ok(spawn_future(program, args_vec))
        })),
    );

    // System.shell_sync(command) - Run command via `sh -c` synchronously.
    methods.insert(
        "shell_sync".to_string(),
        Rc::new(NativeFunction::new("System.shell_sync", Some(1), |args| {
            let (program, args_vec) = parse_shell(&args[0], "System.shell_sync")?;
            let result = execute_command(&program, &args_vec)?;
            Ok(result_to_hash(result))
        })),
    );

    let class = Class {
        name: "System".to_string(),
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
    };

    Value::Class(Rc::new(class))
}

fn result_to_hash(result: SystemResultData) -> Value {
    let mut hash: HashPairs = HashPairs::default();
    hash.insert(
        HashKey::String("stdout".to_string()),
        Value::String(result.stdout),
    );
    hash.insert(
        HashKey::String("stderr".to_string()),
        Value::String(result.stderr),
    );
    hash.insert(
        HashKey::String("exit_code".to_string()),
        Value::Int(result.exit_code as i64),
    );
    Value::Hash(Rc::new(RefCell::new(hash)))
}

#[allow(clippy::arc_with_non_send_sync)]
fn spawn_future(program: String, args_vec: Vec<String>) -> Value {
    let (tx, rx): (Sender<Result<String, String>>, Receiver<_>) = mpsc::channel();

    thread::spawn(move || {
        let result = execute_command(&program, &args_vec);
        let json = match result {
            Ok(data) => serde_json::to_string(&data).map_err(|e| e.to_string()),
            Err(e) => Err(e),
        };
        tx.send(json).ok();
    });

    let future_state = crate::interpreter::value::FutureState::Pending {
        receiver: rx,
        kind: crate::interpreter::value::HttpFutureKind::SystemResult,
    };
    Value::Future(Arc::new(Mutex::new(future_state)))
}

/// Shell metacharacters that must not silently re-route a string to `sh -c`.
const SHELL_METACHARS: &[char] = &[
    '|', '>', '<', '&', ';', '$', '(', ')', '`', '\'', '"', '*', '?', '[', ']', '{', '}', '~',
];

fn contains_shell_metachars(s: &str) -> bool {
    s.chars().any(|c| SHELL_METACHARS.contains(&c))
}

/// Parse the argument to `System.run` / `System.run_sync` into (program, argv).
///
/// Strings are split on whitespace and executed directly — never auto-promoted
/// to `sh -c`. Strings containing shell metacharacters are rejected with a
/// clear error pointing to `System.shell` or the array form.
///
/// Arrays of strings are taken verbatim: `[program, arg1, arg2, ...]`.
pub fn parse_argv(value: &Value, fn_name: &str) -> Result<(String, Vec<String>), String> {
    match value {
        Value::Array(arr) => {
            let arr = arr.borrow();
            if arr.is_empty() {
                return Err(format!("{}() received an empty array", fn_name));
            }
            let mut parts: Vec<String> = Vec::with_capacity(arr.len());
            for (i, v) in arr.iter().enumerate() {
                match v {
                    Value::String(s) => parts.push(s.clone()),
                    other => {
                        return Err(format!(
                            "{}() array must contain only strings, got {} at index {}",
                            fn_name,
                            other.type_name(),
                            i
                        ))
                    }
                }
            }
            let program = parts.remove(0);
            Ok((program, parts))
        }
        Value::String(s) => {
            if contains_shell_metachars(s) {
                return Err(format!(
                    "{}() refuses to auto-shell a command containing shell metacharacters: {:?}. \
                     Use System.shell() for explicit shell execution, or pass an argv array \
                     like [\"program\", \"arg1\", ...] to bypass the shell.",
                    fn_name, s
                ));
            }
            let parts: Vec<&str> = s.split_whitespace().collect();
            if parts.is_empty() {
                return Err(format!("{}() received an empty command", fn_name));
            }
            let program = parts[0].to_string();
            let args = parts[1..].iter().map(|s| s.to_string()).collect();
            Ok((program, args))
        }
        other => Err(format!(
            "{}() expects a string or array of strings, got {}",
            fn_name,
            other.type_name()
        )),
    }
}

/// Parse the argument to `System.shell` / `System.shell_sync` — always wraps
/// the string in `sh -c <string>`.
pub fn parse_shell(value: &Value, fn_name: &str) -> Result<(String, Vec<String>), String> {
    match value {
        Value::String(s) => Ok(("sh".to_string(), vec!["-c".to_string(), s.clone()])),
        other => Err(format!(
            "{}() expects a string command, got {}",
            fn_name,
            other.type_name()
        )),
    }
}

/// Build the (program, argv) pair for backtick command substitution `` `cmd` ``.
///
/// Backtick literals are authored in source code (no string interpolation is
/// supported by the lexer), so they're treated as explicit shell commands —
/// equivalent to `System.shell(s)`.
pub fn parse_backtick(cmd: &str) -> (String, Vec<String>) {
    ("sh".to_string(), vec!["-c".to_string(), cmd.to_string()])
}

/// Internal data structure for system result
#[derive(serde::Serialize)]
pub struct SystemResultData {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl SystemResultData {
    pub fn new(stdout: String, stderr: String, exit_code: i32) -> Self {
        Self {
            stdout,
            stderr,
            exit_code,
        }
    }
}

/// Execute a command and return the result.
pub fn execute_command(program: &str, args: &[String]) -> Result<SystemResultData, String> {
    if program.is_empty() {
        return Err("Empty command".to_string());
    }

    let mut cmd = Command::new(program);
    cmd.args(args);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute: {}", e))?;

    Ok(SystemResultData {
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        exit_code: output.status.code().unwrap_or(-1),
    })
}
