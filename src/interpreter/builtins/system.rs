//! System command execution builtin.
//!
//! Provides async shell command execution with auto-resolving futures.

use indexmap::IndexMap;
use std::cell::RefCell;
use std::collections::HashMap;
use std::process::Command;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, NativeFunction, Value};

pub fn register_system_builtins(env: &mut Environment) {
    // System class
    env.define("System".to_string(), system_class());
}

fn system_class() -> Value {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // System.run(command) - Run command asynchronously
    #[allow(clippy::arc_with_non_send_sync)]
    methods.insert(
        "run".to_string(),
        Rc::new(NativeFunction::new("System.run", Some(1), |args| {
            let cmd = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "System.run() expects string command, got {}",
                        other.type_name()
                    ))
                }
            };

            // Parse command - detect if shell command or direct executable
            let (program, args_vec) = parse_command(&cmd);

            // Spawn background thread and return Future
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
            Ok(Value::Future(Arc::new(Mutex::new(future_state))))
        })),
    );

    // System.run_sync(command) - Run command synchronously (blocking)
    methods.insert(
        "run_sync".to_string(),
        Rc::new(NativeFunction::new("System.run_sync", Some(1), |args| {
            let cmd = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "System.run_sync() expects string command, got {}",
                        other.type_name()
                    ))
                }
            };

            let (program, args) = parse_command(&cmd);
            let result = execute_command(&program, &args)?;

            // Create a Hash with the result data using IndexMap
            let mut hash: IndexMap<HashKey, Value> = IndexMap::new();
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

            Ok(Value::Hash(Rc::new(RefCell::new(hash))))
        })),
    );

    let class = Class {
        name: "System".to_string(),
        superclass: None,
        methods: HashMap::new(),
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

/// Parse a command string into program and arguments.
/// If the command contains shell metacharacters, use "sh" as the program.
pub fn parse_command(cmd: &str) -> (String, Vec<String>) {
    let has_shell_chars = cmd.contains('|')
        || cmd.contains('>')
        || cmd.contains('<')
        || cmd.contains('&')
        || cmd.contains(';')
        || cmd.contains('$')
        || cmd.contains('(')
        || cmd.contains(')')
        || cmd.contains('`')
        || cmd.contains('\'')
        || cmd.contains('"')
        || cmd.contains('*')
        || cmd.contains('?')
        || cmd.contains('[')
        || cmd.contains(']')
        || cmd.contains('{')
        || cmd.contains('}')
        || cmd.contains('~');

    if has_shell_chars {
        // Use shell to execute
        ("sh".to_string(), vec!["-c".to_string(), cmd.to_string()])
    } else {
        // Direct executable - split into program and args
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        if parts.is_empty() {
            ("".to_string(), vec![])
        } else {
            let program = parts[0].to_string();
            let args = parts[1..].iter().map(|s| s.to_string()).collect();
            (program, args)
        }
    }
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
