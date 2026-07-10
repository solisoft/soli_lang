//! Tree-walking interpreter for Solilang.

mod expressions;
pub(crate) mod literals;
mod loop_capture;
mod operators;
mod pattern_matching;
mod statements;
mod variables;

pub use variables::{
    clear_current_env, current_env_lookup, enter_template_lenient_vars, is_defined,
    set_current_env, template_lenient_vars_enabled, TemplateLenientVarsGuard,
};

pub mod access;
pub mod calls;
pub mod control;
pub mod objects;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::ast::*;
use crate::coverage::CoverageTracker;
use crate::error::RuntimeError;
use crate::interpreter::builtins::register_builtins;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{value_matches_type, Function, HashKey, Value};
use crate::span::Span;

pub(crate) type RuntimeResult<T> = Result<T, RuntimeError>;

/// Represents a single frame in the call stack.
#[derive(Debug, Clone)]
pub struct StackFrame {
    pub function_name: String,
    pub file_path: Option<String>,
    pub line: usize,
    pub column: usize,
}

/// Internal result type that can carry return values and exceptions.
pub(crate) enum ControlFlow {
    Normal(Value),
    Return(Value),
    Throw(Value),
    Continue,
}

/// The Solilang interpreter.
pub struct Interpreter {
    pub(crate) environment: Rc<RefCell<Environment>>,
    pub(crate) coverage_tracker: Option<Arc<Mutex<CoverageTracker>>>,
    pub(crate) current_source_path: Option<PathBuf>,
    pub(crate) call_stack: Vec<StackFrame>,
    pub assertion_count: i64,
}

impl Interpreter {
    pub fn new() -> Self {
        let globals = Rc::new(RefCell::new(Environment::with_builtins_capacity()));
        register_builtins(&mut globals.borrow_mut(), true);

        Self {
            environment: globals,
            coverage_tracker: None,
            current_source_path: None,
            call_stack: Vec::new(),
            assertion_count: 0,
        }
    }

    /// Create an interpreter for serve mode (skips test builtins to save memory).
    pub fn new_for_serve() -> Self {
        let globals = Rc::new(RefCell::new(Environment::with_builtins_capacity()));
        register_builtins(&mut globals.borrow_mut(), false);

        Self {
            environment: globals,
            coverage_tracker: None,
            current_source_path: None,
            call_stack: Vec::new(),
            assertion_count: 0,
        }
    }

    /// Create an interpreter with a pre-built environment (skips register_builtins).
    /// Used by the template engine with a cached builtins environment.
    pub fn with_environment(environment: Rc<RefCell<Environment>>) -> Self {
        Self {
            environment,
            coverage_tracker: None,
            current_source_path: None,
            call_stack: Vec::new(),
            assertion_count: 0,
        }
    }

    pub fn with_coverage_tracker(tracker: Arc<Mutex<CoverageTracker>>) -> Self {
        let globals = Rc::new(RefCell::new(Environment::with_builtins_capacity()));
        register_builtins(&mut globals.borrow_mut(), true);

        Self {
            environment: globals,
            coverage_tracker: Some(tracker),
            current_source_path: None,
            call_stack: Vec::new(),
            assertion_count: 0,
        }
    }

    pub fn set_coverage_tracker(&mut self, tracker: Arc<Mutex<CoverageTracker>>) {
        self.coverage_tracker = Some(tracker);
    }

    pub fn set_source_path(&mut self, path: PathBuf) {
        let absolute_path = if path.is_absolute() {
            path
        } else {
            std::fs::canonicalize(&path).unwrap_or(path)
        };
        self.current_source_path = Some(absolute_path);
    }

    #[inline(always)]
    pub fn record_coverage(&self, line: usize) {
        if let Some(ref path) = self.current_file_path() {
            if let Some(ref tracker) = self.coverage_tracker {
                if let Ok(guard) = tracker.lock() {
                    guard.record_line_hit(path, line);
                }
            } else if let Some(global) = crate::coverage::get_global_coverage_tracker() {
                if let Ok(guard) = global.lock() {
                    guard.record_line_hit(path, line);
                }
            }
        }
    }

    pub fn get_assertion_count(&self) -> i64 {
        self.assertion_count
    }

    pub fn global_env(&self) -> &Rc<RefCell<Environment>> {
        &self.environment
    }

    pub fn increment_assertion_count(&mut self) {
        self.assertion_count += 1;
    }

    /// Serialize the current environment for debugging.
    /// Returns a JSON string with all variables (excluding functions/classes for simplicity).
    /// Also includes view context data if a template error occurred.
    /// Futures are resolved before serialization to capture their actual values.
    pub fn serialize_environment_for_debug(&self) -> String {
        let vars = self.environment.borrow().get_all_variables();
        let mut json_parts = Vec::new();

        for (name, value) in vars {
            // Skip functions and classes - they're not useful in the debug view
            match &value {
                Value::Function(_) | Value::NativeFunction(_) | Value::Class(_) => continue,
                _ => {}
            }

            // Resolve futures before serialization to get their actual values
            let resolved_value = if value.is_future() {
                match value.resolve() {
                    Ok(v) => v,
                    Err(e) => Value::String(format!("<future error: {}>", e).into()),
                }
            } else {
                value
            };

            let json_value = self.value_to_json(&resolved_value);
            json_parts.push(format!(r#""{}": {}"#, name, json_value));
        }

        // Fold in the active view (`render()`) locals so the dev error page can
        // surface the data passed to a template that errored mid-render.
        self.append_view_debug_context(&mut json_parts);

        format!("{{{}}}", json_parts.join(", "))
    }

    /// Serialize a HashMap of variables to JSON string.
    pub fn serialize_environment(&self, vars: &std::collections::HashMap<String, Value>) -> String {
        let mut json_parts = Vec::new();

        for (name, value) in vars {
            // Skip functions and classes
            match value {
                Value::Function(_) | Value::NativeFunction(_) | Value::Class(_) => continue,
                _ => {}
            }

            // Resolve futures before serialization
            let resolved_value = if value.is_future() {
                match value.clone().resolve() {
                    Ok(v) => v,
                    Err(e) => Value::String(format!("<future error: {}>", e).into()),
                }
            } else {
                value.clone()
            };

            let json_value = self.value_to_json(&resolved_value);
            json_parts.push(format!(r#""{}": {}"#, name, json_value));
        }

        // A view error is captured here (the failing controller frame unwinds
        // through `serialize_environment`, not `serialize_environment_for_debug`),
        // so the render locals must be folded in on this path too — otherwise the
        // dev error page shows only the controller env and the `render()` data is
        // lost. The throwaway template interpreter that held those locals is gone
        // by now; the view debug context is the only surviving copy.
        self.append_view_debug_context(&mut json_parts);

        format!("{{{}}}", json_parts.join(", "))
    }

    /// Append the active view (`render()`) data to a list of serialized
    /// `"key": value` JSON fragments, when a template error left a view debug
    /// context set. The render locals appear both as a `_view_data` object and
    /// as individually hoisted top-level keys, skipping any name already present
    /// in `json_parts` (the real environment wins on a collision; the view value
    /// remains reachable under `_view_data`). Shared by both serializers so the
    /// locals survive whichever path captured the environment.
    fn append_view_debug_context(&self, json_parts: &mut Vec<String>) {
        let Some(view_data) = crate::interpreter::builtins::template::get_view_debug_context()
        else {
            return;
        };

        // Collect existing variable names to avoid duplicates
        let existing_names: std::collections::HashSet<String> = json_parts
            .iter()
            .filter_map(|part| {
                // Extract key name from "\"key\": value" format
                if part.starts_with('"') {
                    part.split(':').next().and_then(|k| {
                        let k = k.trim().trim_matches('"');
                        if !k.is_empty() {
                            Some(k.to_string())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            })
            .collect();

        // Add view data as a special "_view_data" variable (always add this)
        if !existing_names.contains("_view_data") {
            let view_json = self.value_to_json(&view_data);
            json_parts.push(format!(r#""_view_data": {}"#, view_json));
        }

        // Also extract individual keys from the view data hash for easy access
        // But ONLY if they don't already exist in the environment
        if let Value::Hash(hash) = &view_data {
            for (key, value) in hash.borrow().iter() {
                if let HashKey::String(key_str) = key {
                    // Skip if this key already exists in the environment
                    if existing_names.contains(&**key_str) {
                        continue;
                    }
                    // Skip functions and classes
                    match value {
                        Value::Function(_) | Value::NativeFunction(_) | Value::Class(_) => continue,
                        _ => {}
                    }
                    let value_json = self.value_to_json(value);
                    json_parts.push(format!(r#""{}": {}"#, key_str, value_json));
                }
            }
        }
    }

    /// Convert a Value to a JSON string representation.
    #[allow(clippy::only_used_in_recursion)]
    fn value_to_json(&self, value: &Value) -> String {
        match value {
            Value::Null => "null".to_string(),
            Value::Bool(b) => b.to_string(),
            Value::Int(n) => n.to_string(),
            Value::Float(n) => n.to_string(),
            Value::Decimal(d) => d.to_string(),
            Value::String(s) => {
                // Escape string for JSON
                let escaped = s
                    .replace("\\", "\\\\")
                    .replace("\"", "\\\"")
                    .replace("\n", "\\n")
                    .replace("\r", "\\r")
                    .replace("\t", "\\t");
                format!("\"{}\"", escaped)
            }
            Value::Array(arr) => {
                let items: Vec<String> =
                    arr.borrow().iter().map(|v| self.value_to_json(v)).collect();
                format!("[{}]", items.join(", "))
            }
            Value::Hash(hash) => {
                let pairs: Vec<String> = hash
                    .borrow()
                    .iter()
                    .map(|(k, v)| {
                        let key = match k {
                            HashKey::String(s) => s.clone(),
                            other => format!("{}", other).into(),
                        };
                        let escaped_key = key.replace("\\", "\\\\").replace("\"", "\\\"");
                        format!(r#""{}": {}"#, escaped_key, self.value_to_json(v))
                    })
                    .collect();
                format!("{{{}}}", pairs.join(", "))
            }
            Value::Instance(inst) => {
                let inst = inst.borrow();
                let fields: Vec<String> = inst
                    .fields
                    .iter()
                    .map(|(k, v)| format!(r#""{}": {}"#, k, self.value_to_json(v)))
                    .collect();
                if fields.is_empty() {
                    format!(r#"{{"__class__": "{}"}}"#, inst.class.name)
                } else {
                    format!(
                        r#"{{"__class__": "{}", {}}}"#,
                        inst.class.name,
                        fields.join(", ")
                    )
                }
            }
            Value::Function(_) => "\"<function>\"".to_string(),
            Value::NativeFunction(_) => "\"<native function>\"".to_string(),
            Value::Class(c) => format!("\"<class {}>\"", c.name),
            Value::Future(_) => "\"<future>\"".to_string(),
            Value::Method(_) => "\"<method>\"".to_string(),
            Value::Breakpoint => "\"<breakpoint>\"".to_string(),
            Value::Continue => "\"<continue>\"".to_string(),
            Value::QueryBuilder(_) => "\"<query builder>\"".to_string(),
            Value::Super(c) => format!("\"<super of {}>\"", c.name),
            Value::VmClosure(c) => format!("\"<fn {}>\"", c.proto.name),
            Value::Symbol(s) => format!("\"{}\"", s),
            Value::Image(_) => "\"<Image>\"".to_string(),
            Value::ImagePlan(_) => "\"<ImagePlan>\"".to_string(),
            // Resolve a `grouped {}` deferred to its query result before
            // serialising.
            Value::Deferred(_) => self.value_to_json(&value.force_deferred()),
        }
    }

    /// Push a frame onto the call stack.
    /// If `source_path` is provided, it takes precedence over `current_source_path`.
    pub(crate) fn push_frame(
        &mut self,
        function_name: &str,
        span: Span,
        source_path: Option<String>,
    ) {
        let file_path = source_path.or_else(|| {
            self.current_source_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
        });
        self.call_stack.push(StackFrame {
            function_name: function_name.to_string(),
            file_path: file_path.clone(),
            line: span.line,
            column: span.column,
        });
        // Deep-mode flamegraph hook: every Soli function call gets a span.
        // No-op when --dev is off (gated inside push_fn). The source
        // location goes into `meta` so anonymous lambdas (which carry an
        // empty `function_name`) still show *where* in the source they
        // came from in the flamegraph tooltip.
        let meta = file_path.as_ref().map(|p| {
            if span.line > 0 {
                format!("{}:{}", p, span.line)
            } else {
                p.clone()
            }
        });
        crate::serve::span_log::push_fn(function_name, meta);
    }

    /// Pop a frame from the call stack.
    pub(crate) fn pop_frame(&mut self) {
        self.call_stack.pop();
        crate::serve::span_log::pop_fn();
    }

    /// Get the current file path from the call stack (top frame) or fallback to current_source_path.
    fn current_file_path(&self) -> Option<PathBuf> {
        self.call_stack
            .last()
            .and_then(|frame| frame.file_path.as_ref())
            .map(PathBuf::from)
            .or_else(|| self.current_source_path.clone())
    }

    /// Get the current call stack as formatted strings.
    /// Returns frames from outermost (entry point) to innermost (most recent call).
    pub fn get_stack_trace(&self) -> Vec<String> {
        self.call_stack
            .iter()
            .map(|frame| {
                let file = frame
                    .file_path
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                format!("{} at {}:{}", frame.function_name, file, frame.line)
            })
            .collect()
    }

    /// Interpret a complete program.
    pub fn interpret(&mut self, program: &Program) -> RuntimeResult<()> {
        for stmt in &program.statements {
            self.execute(stmt)?;
        }
        Ok(())
    }

    /// Execute `statements` in an environment already wrapped in Rc<RefCell<>>.
    ///
    /// Reuses the caller's `Rc<RefCell<Environment>>` — no per-call allocation
    /// of the Rc or the inner HashMaps. Intended for hot iterator callbacks
    /// (array_map/filter/each/reduce) where the same lambda env is used across
    /// many iterations; callers are expected to update the loop-variable slot
    /// in-place via `define_or_update` between calls.
    pub(crate) fn execute_block_in(
        &mut self,
        statements: &[Stmt],
        env: Rc<RefCell<Environment>>,
    ) -> RuntimeResult<ControlFlow> {
        let previous = std::mem::replace(&mut self.environment, env);
        let mut result = Ok(ControlFlow::Normal(Value::Null));
        for stmt in statements {
            result = self.execute(stmt);
            match result {
                Err(_) => break,
                Ok(ControlFlow::Return(_)) => break,
                Ok(ControlFlow::Throw(_)) => break,
                Ok(ControlFlow::Normal(_)) => {}
                Ok(ControlFlow::Continue) => break,
            }
        }
        self.environment = previous;
        result
    }

    pub(crate) fn execute_block(
        &mut self,
        statements: &[Stmt],
        env: Environment,
    ) -> RuntimeResult<ControlFlow> {
        let previous = std::mem::replace(&mut self.environment, Rc::new(RefCell::new(env)));

        let mut result = Ok(ControlFlow::Normal(Value::Null));
        for stmt in statements {
            result = self.execute(stmt);
            match result {
                Err(_) => break,
                Ok(ControlFlow::Return(_)) => break,
                Ok(ControlFlow::Throw(_)) => break,
                Ok(ControlFlow::Normal(_)) => {}
                Ok(ControlFlow::Continue) => break,
            }
        }

        // Capture environment and stack trace BEFORE restoring if there's an error
        // This preserves local variables for debugging
        let result = match result {
            Err(e) if !e.is_breakpoint() && e.breakpoint_env_json().is_none() => {
                let captured_env = self.environment.borrow().get_all_variables();
                let env_json = self.serialize_environment(&captured_env);

                // Get current file path for error location.
                // Prefer the top stack frame's file (e.g., a helper's source
                // path) over `current_source_path`, which is the entry
                // script and would otherwise mislabel errors thrown from
                // helpers/imported functions with the caller's filename.
                let file_path = self
                    .current_file_path()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                // Capture stack trace and update the last frame with actual error line
                let mut stack_trace = self.get_stack_trace();

                // Get the current function name from the last stack frame
                let func_name = self
                    .call_stack
                    .last()
                    .map(|f| f.function_name.clone())
                    .unwrap_or_else(|| "unknown".to_string());

                // Replace the last frame with one that has the actual error line number
                if !stack_trace.is_empty() {
                    stack_trace.pop();
                }
                stack_trace.push(format!("{} at {}:{}", func_name, file_path, e.span().line));

                Err(RuntimeError::with_env(
                    e.to_string(),
                    e.span(),
                    env_json,
                    stack_trace,
                ))
            }
            other => other,
        };

        self.environment = previous;
        result
    }

    /// Call a function with the given arguments and return the result.
    pub(crate) fn call_function(
        &mut self,
        func: &Function,
        arguments: Vec<Value>,
    ) -> RuntimeResult<Value> {
        self.call_function_with_this(func, None, arguments)
    }

    /// Like [`Self::call_function`], but optionally binds `this` directly in
    /// the call environment. This is the direct instance-method invocation
    /// path: the caller passes the class's method `Rc<Function>` as-is plus
    /// the receiver, instead of allocating a bound `Function` whose
    /// construction deep-clones the entire method body AST per call (the old
    /// `instance_member_access` → `call_value` route, still used when a
    /// method is accessed as a value rather than called).
    pub(crate) fn call_function_with_this(
        &mut self,
        func: &Function,
        this: Option<Value>,
        arguments: Vec<Value>,
    ) -> RuntimeResult<Value> {
        // Push stack frame with the function's source path (where it was defined)
        let span = func.span.unwrap_or_else(|| Span::new(0, 0, 1, 1));
        self.push_frame(&func.name, span, func.source_path.clone());

        // Try to take the cached call env; on a recursive call the slot is
        // None and we fall back to allocating a fresh one. For instance
        // methods the slot lives on the class's shared method `Rc`, so the
        // env is reused across receivers — `reset_for_call` wipes all
        // bindings (including the previous `this`) before rebinding.
        let call_env_rc = match func.cached_env.borrow_mut().take() {
            Some(cached) => {
                cached.borrow_mut().reset_for_call();
                cached
            }
            None => Rc::new(RefCell::new(Environment::with_enclosing(
                func.closure.clone(),
            ))),
        };

        {
            let mut call_env_inner = call_env_rc.borrow_mut();
            if let Some(this_val) = this {
                call_env_inner.define("this".to_string(), this_val);
            }
            for (param, value) in func.params.iter().zip(arguments) {
                call_env_inner.define(param.name.clone(), value);
            }

            // Store defining_superclass for super calls
            if let Some(ref sc) = func.defining_superclass {
                call_env_inner.define(
                    "__defining_superclass__".to_string(),
                    Value::Class(sc.clone()),
                );
            }
        }

        // Store reference to capture environment on error and to re-cache after.
        let env_for_capture = call_env_rc.clone();

        // Execute the function body — reuse call_env_rc directly rather than
        // cloning the inner Environment (which would allocate 2 fresh HashMaps
        // per call only to throw them away).
        let result = match self.execute_block_in(&func.body, call_env_rc) {
            Ok(ControlFlow::Normal(v)) => Ok(v),
            Ok(ControlFlow::Return(return_value)) => Ok(return_value),
            Ok(ControlFlow::Continue) => Ok(Value::Null),
            Ok(ControlFlow::Throw(e)) => Err(RuntimeError::General {
                message: format!("Unhandled exception: {}", e),
                span: Span::default(),
            }),
            Err(e) => {
                // Preserve errors that already have captured environment (breakpoint or WithEnv)
                if e.is_breakpoint() || e.breakpoint_env_json().is_some() {
                    Err(e)
                } else {
                    // Capture the local environment before it's lost
                    let captured_env = env_for_capture.borrow().get_all_variables();
                    let env_json = self.serialize_environment(&captured_env);

                    // Capture stack trace before popping frame and rewrite the
                    // deepest frame so it points at the actual error line
                    // rather than the function-definition line. Without this
                    // the dev error page highlights the `def` line instead of
                    // the offending statement inside the function.
                    let mut stack_trace = self.get_stack_trace();
                    if let Some(frame) = self.call_stack.last() {
                        let file = frame
                            .file_path
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string());
                        if !stack_trace.is_empty() {
                            stack_trace.pop();
                        }
                        stack_trace.push(format!(
                            "{} at {}:{}",
                            frame.function_name,
                            file,
                            e.span().line
                        ));
                    }

                    Err(RuntimeError::with_env(
                        e.to_string(),
                        e.span(),
                        env_json,
                        stack_trace,
                    ))
                }
            }
        };

        // Validate return type if annotated
        let result = match result {
            Ok(ref value) => {
                if let Some(ref expected_type) = func.return_type {
                    if !value_matches_type(value, expected_type) {
                        Err(RuntimeError::General {
                            message: format!(
                                "function '{}' expected to return {}, got {}",
                                func.name,
                                expected_type,
                                value.type_name()
                            ),
                            span,
                        })
                    } else {
                        result
                    }
                } else {
                    result
                }
            }
            _ => result,
        };

        // Pop stack frame
        self.pop_frame();

        // Return the env to the per-Function cache for the next call, but
        // only if no one else holds a reference to it. A closure created
        // inside this call captures the env via Rc::clone — reusing it
        // would corrupt the captured state (`makeAdder(5)` then
        // `makeAdder(10)` must produce two independent `n` bindings).
        //
        // strong_count == 1 means `env_for_capture` is the sole remaining Rc
        // (self.environment was restored, dropping that reference). A nested
        // (recursive) call may have already populated the slot — in that case
        // we simply drop env_for_capture and keep the slot's current value.
        if func.cached_env.borrow().is_none() && Rc::strong_count(&env_for_capture) == 1 {
            *func.cached_env.borrow_mut() = Some(env_for_capture);
        }

        result
    }

    /// Execute a constructor body with a call-stack frame carrying the
    /// constructor's defining file, so coverage, stack traces, and the
    /// flamegraph attribute constructor-body lines to the class's source
    /// file — exactly like `call_function_with_this` does for methods.
    /// Without the frame, constructor hits attribute to the *caller* and
    /// are dropped from coverage as test-directory lines.
    pub(crate) fn execute_constructor_body(&mut self, ctor: &Function, ctor_env: Environment) {
        let span = ctor.span.unwrap_or_else(|| Span::new(0, 0, 1, 1));
        self.push_frame(&ctor.name, span, ctor.source_path.clone());
        // Result intentionally discarded (pre-existing constructor behavior);
        // no `?` between push and pop, so the frame is always popped.
        let _ = self.execute_block(&ctor.body, ctor_env);
        self.pop_frame();
    }
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod return_type_enforcement_tests {
    use super::*;
    use crate::lexer::Scanner;
    use crate::parser::Parser;

    fn run(source: &str) -> Result<(), String> {
        let tokens = Scanner::new(source)
            .scan_tokens()
            .map_err(|e| e.to_string())?;
        let program = Parser::new(tokens).parse().map_err(|e| e.to_string())?;
        let mut interpreter = Interpreter::new();
        interpreter.interpret(&program).map_err(|e| e.to_string())
    }

    #[test]
    fn test_correct_return_type_string() {
        let src = r#"
fn greet() -> String
  return "hello"
end
greet()
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_correct_return_type_int() {
        let src = r#"
fn add() -> Int
  return 42
end
add()
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_wrong_return_type_int_instead_of_string() {
        let src = r#"
fn greet() -> String
  return 42
end
greet()
"#;
        let result = run(src);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("expected to return String"),
            "Error was: {}",
            err
        );
        assert!(err.contains("got int"), "Error was: {}", err);
    }

    #[test]
    fn test_nullable_return_type_allows_null() {
        let src = r#"
fn maybe_name() -> String?
  return null
end
maybe_name()
"#;
        let result = run(src);
        assert!(result.is_ok(), "Expected Ok, got Err: {:?}", result.err());
    }

    #[test]
    fn test_nullable_return_type_allows_value() {
        let src = r#"
fn maybe_name() -> String?
  return "Alice"
end
maybe_name()
"#;
        let result = run(src);
        assert!(result.is_ok(), "Expected Ok, got Err: {:?}", result.err());
    }

    #[test]
    fn test_nullable_return_type_rejects_wrong_type() {
        let src = r#"
fn maybe_name() -> String?
  return 42
end
maybe_name()
"#;
        let result = run(src);
        assert!(result.is_err());
    }

    #[test]
    fn test_unannotated_function_allows_anything() {
        let src = r#"
fn flexible()
  return 42
end
flexible()
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_lambda_with_return_type() {
        let src = r#"
let f = fn(x) -> Int
  return x + 1
end
f(5)
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_lambda_with_wrong_return_type() {
        let src = r#"
let f = fn(x) -> Int
  return "not an int"
end
f(5)
"#;
        let result = run(src);
        assert!(result.is_err());
    }

    #[test]
    fn test_bool_return_type() {
        let src = r#"
fn is_even(n) -> Bool
  return n % 2 == 0
end
is_even(4)
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_array_return_type() {
        let src = r#"
fn get_list() -> Array
  return [1, 2, 3]
end
get_list()
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_hash_return_type() {
        let src = r#"
fn get_map() -> Hash
  return { "a" => 1 }
end
get_map()
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_implicit_null_return_fails_for_typed_function() {
        let src = r#"
fn greet() -> String
  let x = 1
end
greet()
"#;
        let result = run(src);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("expected to return String"),
            "Error was: {}",
            err
        );
    }

    #[test]
    fn test_void_return_type_allows_null() {
        let src = r#"
fn do_stuff() -> Void
  let x = 1
end
do_stuff()
"#;
        assert!(run(src).is_ok());
    }

    #[test]
    fn test_void_return_type_rejects_value() {
        let src = r#"
fn do_stuff() -> Void
  return 42
end
do_stuff()
"#;
        let result = run(src);
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod helper_error_path_tests {
    use super::*;
    use crate::lexer::Scanner;
    use crate::parser::Parser;

    /// Regression test: an error thrown from inside a helper function (loaded
    /// from a different file than the entry script) must report the helper's
    /// path, not the entry script's path. Prior to the fix, `execute_block`
    /// derived the file_path from `current_source_path` (the entry file),
    /// which mislabeled errors raised inside `if`/`while`/block constructs
    /// nested in a helper as belonging to the caller's file.
    #[test]
    fn error_in_helper_block_reports_helper_path_not_entry_path() {
        // A helper with the offending reference inside an `if` block — that
        // block is dispatched through `execute_block`, the path with the bug.
        let helper_src = "fn buggy_helper { if true { return missing_var; } }";
        let helper_path = "/fake/app/helpers/buggy_helper.sl".to_string();
        let helper_tokens = Scanner::new(helper_src).scan_tokens().unwrap();
        let helper_program = Parser::new(helper_tokens).parse().unwrap();

        let mut interpreter = Interpreter::new();
        let helper_func = helper_program
            .statements
            .iter()
            .find_map(|s| match &s.kind {
                StmtKind::Function(decl) => Some(Function::from_decl(
                    decl,
                    interpreter.environment.clone(),
                    Some(helper_path.clone()),
                )),
                _ => None,
            })
            .expect("helper function should parse");

        interpreter.environment.borrow_mut().define(
            "buggy_helper".to_string(),
            Value::Function(Rc::new(helper_func)),
        );

        // Pretend the script being executed is a middleware/controller — the
        // same situation as the bug report (cors.sl as entry, helper.sl as
        // the file containing the actual offending line).
        interpreter.set_source_path(PathBuf::from("/fake/app/middleware/cors.sl"));

        let caller_tokens = Scanner::new("buggy_helper();").scan_tokens().unwrap();
        let caller_program = Parser::new(caller_tokens).parse().unwrap();
        let err = interpreter
            .interpret(&caller_program)
            .expect_err("expected an UndefinedVariable error from inside the helper");

        let stack_trace = err
            .breakpoint_stack_trace()
            .expect("error should carry a captured stack trace");

        let any_helper = stack_trace.iter().any(|f| f.contains("buggy_helper.sl"));
        let any_entry = stack_trace.iter().any(|f| f.contains("cors.sl"));
        assert!(
            any_helper,
            "stack trace should mention the helper file, got: {:?}",
            stack_trace
        );
        assert!(
            !any_entry,
            "stack trace should not mention the entry file, got: {:?}",
            stack_trace
        );
    }
}

#[cfg(test)]
mod safe_navigation_tests {
    use crate::interpreter::value::Value;
    use crate::lexer::Scanner;
    use crate::parser::Parser;

    use super::Interpreter;

    fn eval(source: &str) -> Value {
        let tokens = Scanner::new(source).scan_tokens().unwrap();
        let program = Parser::new(tokens).parse().unwrap();
        let mut interpreter = Interpreter::new();
        interpreter.interpret(&program).unwrap();
        // Read the result variable from the environment
        let val = interpreter
            .environment
            .borrow()
            .get("result")
            .unwrap_or(Value::Null);
        val
    }

    #[test]
    fn test_safe_nav_null_returns_null() {
        let val = eval("let x = null; let result = x&.name;");
        assert_eq!(val, Value::Null);
    }

    #[test]
    fn test_safe_nav_non_null_returns_field() {
        // Use hash for field access since it's simpler and well-tested
        let val = eval(
            r#"
let u = { "name" => "Alice" }
let result = u&.name
"#,
        );
        assert_eq!(val, Value::String("Alice".into()));
    }

    #[test]
    fn test_safe_nav_method_null() {
        let val = eval(
            r#"
let x = null
let result = x&.greet()
"#,
        );
        assert_eq!(val, Value::Null);
    }

    #[test]
    fn test_safe_nav_method_non_null() {
        // Use array with a method to test non-null safe nav method call
        let val = eval(
            r#"
let arr = [3, 1, 2]
let result = arr&.length()
"#,
        );
        assert_eq!(val, Value::Int(3));
    }

    #[test]
    fn test_safe_nav_chained_null_at_first() {
        let val = eval("let x = null; let result = x&.inner&.field;");
        assert_eq!(val, Value::Null);
    }

    #[test]
    fn test_safe_nav_chained_non_null() {
        let val = eval(
            r#"
let u = { "address" => { "city" => "Paris" } }
let result = u&.address&.city
"#,
        );
        assert_eq!(val, Value::String("Paris".into()));
    }

    #[test]
    fn test_safe_nav_with_nullish_coalescing() {
        let val = eval(r#"let x = null; let result = x&.name ?? "default";"#);
        assert_eq!(val, Value::String("default".into()));
    }

    #[test]
    fn test_safe_nav_on_non_null_with_nullish() {
        let val = eval(
            r#"
let u = { "name" => "Eve" }
let result = u&.name ?? "default"
"#,
        );
        assert_eq!(val, Value::String("Eve".into()));
    }
}
