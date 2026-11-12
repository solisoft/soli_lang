//! The tree-walker's side of the step debugger (`crate::debug::session`).
//!
//! [`Interpreter::debugger_statement`] runs before every statement on the
//! debuggee thread: it records where each frame is, asks the session whether
//! to stop, and while stopped serves the adapter's commands. The inspection
//! helpers below (stack, scopes, variables, evaluate) run inside those
//! commands, on this thread.
//!
//! Variables handed to the adapter are kept in a handle table that lives
//! until the program resumes, as DAP's `variablesReference` requires.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use serde_json::{json, Value as Json};

use super::{ControlFlow, Interpreter};
use crate::debug::session::{self, Command, Event, Location, StepMode};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::Value;

/// Where a frame's variables come from.
#[derive(Clone)]
pub(crate) enum FrameScope {
    /// The tree-walker's live scope chain.
    Env(Rc<RefCell<Environment>>),
    /// The VM's slots, read when the program stopped.
    Values(Vec<(String, Value)>),
}

/// What one frame of the paused program looked like at its last statement.
#[derive(Clone)]
pub(crate) struct FrameState {
    pub name: String,
    pub file: String,
    pub line: u32,
    pub scope: FrameScope,
}

/// The engine-specific part of a paused program: running code in it.
/// Everything else (stack, scopes, variables) reads the recorded frames.
pub trait Paused {
    /// DAP `evaluate` in the frame at `depth` (the innermost when `None`).
    fn evaluate(&mut self, source: &str, depth: Option<usize>) -> Result<Json, String>;
}

/// Something the adapter can expand.
#[derive(Clone)]
enum Handle {
    /// The bindings of `env` and its enclosing scopes, up to the global one
    /// (`locals`), or of the global scope alone, without the builtins.
    Scope {
        env: Rc<RefCell<Environment>>,
        locals: bool,
    },
    /// A fixed list of bindings (a VM frame's slots, the VM's globals).
    List(Vec<(String, Value)>),
    Value(Value),
}

struct DebuggerState {
    mode: StepMode,
    last: Option<Location>,
    /// By depth: `frames[0]` is the script's top level.
    frames: Vec<FrameState>,
    handles: Vec<Handle>,
    /// Names the global scope held before the program ran: builtins, left
    /// out of the Globals scope.
    builtins: Option<HashSet<String>>,
    /// Set while `evaluate` runs code, so it never stops inside itself.
    evaluating: bool,
    /// The VM's globals when it stopped (the tree-walker reads its global
    /// scope instead).
    vm_globals: Option<Vec<(String, Value)>>,
}

thread_local! {
    static STATE: RefCell<DebuggerState> = const {
        RefCell::new(DebuggerState {
            mode: StepMode::Run,
            last: None,
            frames: Vec::new(),
            handles: Vec::new(),
            builtins: None,
            evaluating: false,
            vm_globals: None,
        })
    };
}

/// Stop at the program's first statement (`stopOnEntry`).
pub fn stop_on_entry() {
    STATE.with(|state| state.borrow_mut().mode = StepMode::In);
}

fn global_env(env: &Rc<RefCell<Environment>>) -> Rc<RefCell<Environment>> {
    let mut current = env.clone();
    loop {
        let next = current.borrow().enclosing();
        match next {
            Some(parent) => current = parent,
            None => return current,
        }
    }
}

impl Interpreter {
    /// Before a statement on the debuggee thread: note where the frame is,
    /// and stop if the session says so. Callers check
    /// `crate::debug::is_debuggee()` first.
    pub(crate) fn debugger_statement(&mut self, file: Option<String>, line: usize) {
        let evaluating = STATE.with(|state| state.borrow().evaluating);
        if evaluating {
            return;
        }
        // A statement with no file is the engine's own prelude (the mailer
        // and event classes), never the program's: not a place to stop.
        let Some(file) = file else {
            return;
        };
        let depth = self.call_stack.len();
        let line = u32::try_from(line).unwrap_or(u32::MAX);
        let name = match depth {
            0 => "<main>".to_string(),
            _ => {
                let name = &self.call_stack[depth - 1].function_name;
                if name.is_empty() {
                    "<anonymous>".to_string()
                } else {
                    name.clone()
                }
            }
        };
        let here = Location {
            file: file.clone(),
            line,
            depth,
        };
        let (last, mode) = STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.builtins.is_none() {
                let globals = global_env(&self.environment);
                let names: HashSet<String> = globals
                    .borrow()
                    .own_bindings()
                    .into_iter()
                    .map(|(name, _)| name)
                    .collect();
                state.builtins = Some(names);
            }
            state.frames.truncate(depth + 1);
            let frame = FrameState {
                name,
                file,
                line,
                scope: FrameScope::Env(self.environment.clone()),
            };
            if state.frames.len() == depth + 1 {
                state.frames[depth] = frame;
            } else {
                // A frame entered without a statement in between (an
                // expression-bodied lambda): pad with the caller's view.
                while state.frames.len() < depth {
                    let filler = state
                        .frames
                        .last()
                        .cloned()
                        .unwrap_or_else(|| frame.clone());
                    state.frames.push(filler);
                }
                state.frames.push(frame);
            }
            let last = state.last.replace(here.clone());
            (last, state.mode)
        });
        if let Some(reason) = session::should_stop(&here, last.as_ref(), mode) {
            self.debugger_pause(reason, format!("{}:{}", here.file, here.line));
        }
    }

    /// `debug()` reached with a debugger attached: stop here instead of
    /// unwinding to the dev error page.
    pub(crate) fn debugger_breakpoint_call(&mut self) {
        let description = STATE.with(|state| {
            state
                .borrow()
                .last
                .as_ref()
                .map(|l| format!("debug() at {}:{}", l.file, l.line))
                .unwrap_or_else(|| "debug()".to_string())
        });
        self.debugger_pause("breakpoint", description);
    }

    fn debugger_pause(&mut self, reason: &'static str, description: String) {
        let depth = self.call_stack.len();
        serve_pause(self, depth, reason, description);
    }

    /// DAP `evaluate` in the frame at `depth` (the innermost when `None`):
    /// the source runs with that frame's scope, and its last expression's
    /// value is the result. It can assign: that is how a variable is changed
    /// from the debug console.
    pub fn debugger_evaluate(
        &mut self,
        source: &str,
        depth: Option<usize>,
    ) -> Result<Json, String> {
        let env = STATE.with(|state| {
            let state = state.borrow();
            let frame = match depth {
                Some(depth) => state.frames.get(depth),
                None => state.frames.last(),
            };
            match frame.map(|f| &f.scope) {
                Some(FrameScope::Env(env)) => Some(env.clone()),
                _ => None,
            }
        });
        let env = env.unwrap_or_else(|| self.environment.clone());
        let value = run_source(self, env, source)?;
        Ok(describe_result(&value))
    }
}

impl Paused for Interpreter {
    fn evaluate(&mut self, source: &str, depth: Option<usize>) -> Result<Json, String> {
        self.debugger_evaluate(source, depth)
    }
}

/// Stopped: tell the adapter, then serve its commands until one resumes.
/// `depth` is the stopped frame's, for step over and step out.
fn serve_pause(target: &mut dyn Paused, depth: usize, reason: &'static str, description: String) {
    session::send(Event::Stopped {
        reason,
        description,
    });
    let mode = loop {
        match session::next_command() {
            None | Some(Command::Continue) => break StepMode::Run,
            Some(Command::Next) => break StepMode::Over(depth),
            Some(Command::StepIn) => break StepMode::In,
            Some(Command::StepOut) => break StepMode::Out(depth),
            Some(Command::Inspect(inspect)) => inspect(target),
        }
    };
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.mode = mode;
        state.handles.clear();
    });
}

/// Run `source` in `env` on `interpreter`, without stopping inside it; the
/// value of its last statement.
fn run_source(
    interpreter: &mut Interpreter,
    env: Rc<RefCell<Environment>>,
    source: &str,
) -> Result<Value, String> {
    let tokens = crate::lexer::Scanner::new(source)
        .scan_tokens()
        .map_err(|e| e.to_string())?;
    let program = crate::parser::Parser::new(tokens)
        .parse()
        .map_err(|e| e.to_string())?;
    let saved = std::mem::replace(&mut interpreter.environment, env);
    STATE.with(|state| state.borrow_mut().evaluating = true);
    let mut result = Ok(Value::Null);
    for stmt in &program.statements {
        match interpreter.execute(stmt) {
            Ok(ControlFlow::Normal(value)) | Ok(ControlFlow::Return(value)) => result = Ok(value),
            Ok(ControlFlow::Throw(value)) => {
                result = Err(format!("{}", value));
                break;
            }
            Ok(_) => {}
            Err(e) => {
                result = Err(e.to_string());
                break;
            }
        }
    }
    STATE.with(|state| state.borrow_mut().evaluating = false);
    interpreter.environment = saved;
    result
}

fn describe_result(value: &Value) -> Json {
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let (text, kind, reference) = describe(&mut state, value);
        json!({"result": text, "type": kind, "variablesReference": reference})
    })
}

// -- the VM's side --------------------------------------------------------

/// A statement of the VM's at `here`: record it, and say whether and why to
/// stop. `globals` names what the program found defined (builtins), asked
/// for once.
pub(crate) fn vm_statement(
    here: Location,
    builtins: impl FnOnce() -> HashSet<String>,
) -> Option<(&'static str, String)> {
    let (last, mode, evaluating) = STATE.with(|state| {
        let mut state = state.borrow_mut();
        if state.builtins.is_none() {
            state.builtins = Some(builtins());
        }
        let evaluating = state.evaluating;
        if evaluating {
            return (None, state.mode, true);
        }
        (state.last.replace(here.clone()), state.mode, false)
    });
    if evaluating {
        return None;
    }
    session::should_stop(&here, last.as_ref(), mode)
        .map(|reason| (reason, format!("{}:{}", here.file, here.line)))
}

/// The names defined before the program ran (builtins), as first seen.
pub(crate) fn builtin_names() -> HashSet<String> {
    STATE.with(|state| state.borrow().builtins.clone().unwrap_or_default())
}

/// The description of a `debug()` stop, at the last statement seen.
pub(crate) fn breakpoint_description() -> String {
    STATE.with(|state| {
        state
            .borrow()
            .last
            .as_ref()
            .map(|l| format!("debug() at {}:{}", l.file, l.line))
            .unwrap_or_else(|| "debug()".to_string())
    })
}

/// The VM stopped: `frames` (outermost first) and `globals` are what it
/// read off its stack. Serve the adapter until it resumes.
pub(crate) fn vm_pause(
    frames: Vec<FrameState>,
    globals: Vec<(String, Value)>,
    reason: &'static str,
    description: String,
) {
    let depth = frames.len().saturating_sub(1);
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        state.frames = frames;
        state.vm_globals = Some(globals);
    });
    serve_pause(&mut SnapshotEvaluator, depth, reason, description);
    STATE.with(|state| state.borrow_mut().vm_globals = None);
}

/// `evaluate` on a stopped VM: a scratch tree-walker gets the VM's globals
/// and the frame's slots, so reading works; an assignment changes the copy,
/// not the program.
struct SnapshotEvaluator;

impl Paused for SnapshotEvaluator {
    fn evaluate(&mut self, source: &str, depth: Option<usize>) -> Result<Json, String> {
        let (locals, globals) = STATE.with(|state| {
            let state = state.borrow();
            let frame = match depth {
                Some(depth) => state.frames.get(depth),
                None => state.frames.last(),
            };
            let locals = match frame.map(|f| &f.scope) {
                Some(FrameScope::Values(values)) => values.clone(),
                _ => Vec::new(),
            };
            (locals, state.vm_globals.clone().unwrap_or_default())
        });
        let mut interpreter = Interpreter::new();
        {
            let mut global = interpreter.environment.borrow_mut();
            for (name, value) in globals {
                global.define(name, value);
            }
        }
        let mut scope = Environment::with_enclosing(interpreter.environment.clone());
        for (name, value) in locals {
            scope.define(name, value);
        }
        let value = run_source(&mut interpreter, Rc::new(RefCell::new(scope)), source)?;
        Ok(describe_result(&value))
    }
}

// -- inspection, run inside `Command::Inspect` ------------------------------

/// DAP `stackTrace`: innermost first; a frame's id is its depth.
pub fn stack_trace() -> Json {
    {
        STATE.with(|state| {
            let state = state.borrow();
            let frames: Vec<Json> = state
                .frames
                .iter()
                .enumerate()
                .rev()
                .map(|(depth, frame)| {
                    let name = std::path::Path::new(&frame.file)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    json!({
                        "id": depth,
                        "name": frame.name,
                        "line": frame.line,
                        "column": 1,
                        "source": {"name": name, "path": frame.file},
                    })
                })
                .collect();
            json!({"stackFrames": frames, "totalFrames": frames.len()})
        })
    }
}

/// DAP `scopes` for the frame at `depth`.
pub fn scopes(depth: usize) -> Json {
    {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let Some(frame) = state.frames.get(depth).cloned() else {
                return json!({"scopes": []});
            };
            let (locals, globals) = match &frame.scope {
                FrameScope::Env(env) => (
                    Handle::Scope {
                        env: env.clone(),
                        locals: true,
                    },
                    Handle::Scope {
                        env: global_env(env),
                        locals: false,
                    },
                ),
                FrameScope::Values(values) => (
                    Handle::List(values.clone()),
                    Handle::List(state.vm_globals.clone().unwrap_or_default()),
                ),
            };
            let locals = push_handle(&mut state, locals);
            let globals = push_handle(&mut state, globals);
            json!({"scopes": [
                {"name": "Locals", "presentationHint": "locals", "variablesReference": locals, "expensive": false},
                {"name": "Globals", "variablesReference": globals, "expensive": false},
            ]})
        })
    }
}

/// DAP `variables` for a handle from `scopes` or an earlier `variables`.
pub fn variables(reference: usize) -> Json {
    {
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let Some(handle) = reference
                .checked_sub(1)
                .and_then(|i| state.handles.get(i))
                .cloned()
            else {
                return json!({"variables": []});
            };
            let entries: Vec<(String, Value)> = match handle {
                Handle::Scope { env, locals } => {
                    scope_entries(&env, locals, state.builtins.as_ref())
                }
                Handle::List(values) => values,
                Handle::Value(value) => children(&value),
            };
            let variables: Vec<Json> = entries
                .into_iter()
                .map(|(name, value)| variable_json(&mut state, name, value))
                .collect();
            json!({"variables": variables})
        })
    }
}

fn push_handle(state: &mut DebuggerState, handle: Handle) -> usize {
    state.handles.push(handle);
    state.handles.len()
}

/// The bindings of one scope view, sorted by name.
fn scope_entries(
    env: &Rc<RefCell<Environment>>,
    locals: bool,
    builtins: Option<&HashSet<String>>,
) -> Vec<(String, Value)> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    let mut current = Some(env.clone());
    while let Some(scope) = current {
        let parent = scope.borrow().enclosing();
        // The global scope is the one with no parent: Locals stop before
        // it, Globals are only it.
        if locals && parent.is_none() {
            break;
        }
        for (name, value) in scope.borrow().own_bindings() {
            if seen.insert(name.clone()) {
                if !locals && builtins.is_some_and(|b| b.contains(&name)) {
                    continue;
                }
                if matches!(value, Value::NativeFunction(_)) {
                    continue;
                }
                entries.push((name, value));
            }
        }
        if !locals {
            break;
        }
        current = parent;
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

/// The parts of a value the adapter can expand into.
fn children(value: &Value) -> Vec<(String, Value)> {
    match value {
        Value::Array(items) => items
            .borrow()
            .iter()
            .enumerate()
            .map(|(i, v)| (format!("[{}]", i), v.clone()))
            .collect(),
        Value::Hash(pairs) => pairs
            .borrow()
            .iter()
            .map(|(k, v)| (format!("{}", k), v.clone()))
            .collect(),
        Value::Instance(instance) => {
            let mut fields: Vec<(String, Value)> = instance
                .borrow()
                .fields
                .iter()
                .map(|(k, v)| (format!("@{}", k), v.clone()))
                .collect();
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            fields
        }
        _ => Vec::new(),
    }
}

/// How a value shows in the Variables view: its text, its type, and a
/// handle to expand it (0 when it has no parts).
fn describe(state: &mut DebuggerState, value: &Value) -> (String, String, usize) {
    let kind = value.type_name().to_string();
    match value {
        Value::Array(items) => {
            let len = items.borrow().len();
            let reference = push_handle(state, Handle::Value(value.clone()));
            (format!("Array({})", len), kind, reference)
        }
        Value::Hash(pairs) => {
            let len = pairs.borrow().len();
            let reference = push_handle(state, Handle::Value(value.clone()));
            (format!("Hash({})", len), kind, reference)
        }
        Value::Instance(instance) => {
            let class = instance.borrow().class.name.clone();
            let reference = push_handle(state, Handle::Value(value.clone()));
            (format!("#<{}>", class), class, reference)
        }
        Value::String(s) => (format!("{:?}", s.as_str()), kind, 0),
        other => (format!("{}", other), kind, 0),
    }
}

fn variable_json(state: &mut DebuggerState, name: String, value: Value) -> Json {
    let shown = if crate::redaction::looks_sensitive(&name) {
        Value::String(crate::redaction::REDACTED.into())
    } else {
        value
    };
    let (text, kind, reference) = describe(state, &shown);
    json!({"name": name, "value": text, "type": kind, "variablesReference": reference})
}
