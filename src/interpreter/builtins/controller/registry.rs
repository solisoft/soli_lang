//! Controller registry and scanner for OOP controllers.
//!
//! This module handles:
//! - Scanning controllers directory for controller files
//! - Parsing controller files to extract metadata
//! - Registering controller actions with the router
//! - Instantiating controllers per request

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::RwLock;

use super::controller::{AfterAction, BeforeAction, ControllerAction, ControllerInfo};
use crate::interpreter::value::{Instance, Value};
use crate::interpreter::Interpreter;

// Global registry of all controllers.
// Uses RwLock to allow concurrent reads (most operations) while only blocking for writes.
lazy_static::lazy_static! {
    pub static ref CONTROLLER_REGISTRY: RwLock<ControllerRegistry> = RwLock::new(ControllerRegistry::new());
}

// Thread-local controller instances for current request.
thread_local! {
    static CURRENT_CONTROLLER: RefCell<Option<Value>> = RefCell::new(None);
}

// Thread-local cache for pre-compiled handler programs.
// Key: handler source string, Value: parsed Program
thread_local! {
    static HANDLER_PROGRAM_CACHE: RefCell<HashMap<String, crate::ast::Program>> = RefCell::new(HashMap::new());
}

/// Controller registry - stores metadata about all controllers.
#[derive(Debug, Clone)]
pub struct ControllerRegistry {
    controllers: HashMap<String, ControllerInfo>,
}

impl ControllerRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            controllers: HashMap::new(),
        }
    }

    /// Register a controller.
    pub fn register(&mut self, info: ControllerInfo) {
        self.controllers.insert(info.class_name.clone(), info);
    }

    /// Get a controller by its class name (e.g., "posts" for PostsController).
    pub fn get(&self, class_name: &str) -> Option<&ControllerInfo> {
        self.controllers.get(class_name)
    }

    /// Get a controller by its full name (e.g., "PostsController").
    pub fn get_by_name(&self, name: &str) -> Option<&ControllerInfo> {
        self.controllers.values().find(|c| c.name == name)
    }

    /// Get all controllers.
    pub fn all(&self) -> Vec<&ControllerInfo> {
        self.controllers.values().collect()
    }

    /// Get all action names for a controller.
    pub fn get_actions(&self, class_name: &str) -> Vec<String> {
        self.controllers
            .get(class_name)
            .map(|c| c.actions.iter().map(|a| a.action_name.clone()).collect())
            .unwrap_or_default()
    }
}

/// Scan controllers directory and register all controllers.
pub fn scan_controllers(controllers_dir: &Path) -> Result<(), String> {
    let mut registry = CONTROLLER_REGISTRY.write().unwrap();

    if !controllers_dir.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(controllers_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();

        if path.is_file() && path.extension().map_or(false, |ext| ext == "soli") {
            if let Some(file_name) = path.file_stem().and_then(|n| n.to_str()) {
                // Skip non-controller files
                if !file_name.ends_with("_controller") {
                    continue;
                }

                match parse_controller_file(&path, file_name) {
                    Ok(info) => {
                        println!(
                            "Registered controller: {} with actions: {:?}",
                            info.name,
                            info.actions
                                .iter()
                                .map(|a| &a.action_name)
                                .collect::<Vec<_>>()
                        );
                        registry.register(info);
                    }
                    Err(e) => {
                        eprintln!(
                            "Warning: Failed to parse controller {}: {}",
                            path.display(),
                            e
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

/// Parse a controller file and extract metadata.
fn parse_controller_file(path: &Path, file_name: &str) -> Result<ControllerInfo, String> {
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;

    // Controller class name (e.g., "posts_controller" -> "PostsController")
    let class_name = to_class_name(file_name);

    // Extract class name from file (e.g., "class PostsController extends Controller")
    let actual_class_name = extract_class_name(&source).unwrap_or_else(|| class_name.clone());

    // Extract the class name part (e.g., "posts" from "PostsController")
    let controller_class_name = to_controller_name(&actual_class_name);

    let mut info = ControllerInfo::new(&actual_class_name, &controller_class_name);

    // Parse static block for configuration
    parse_controller_static_block(&source, &mut info)?;

    // Extract public methods (actions)
    extract_actions(&source, &actual_class_name, &mut info);

    Ok(info)
}

/// Convert "posts_controller" to "PostsController"
fn to_class_name(file_name: &str) -> String {
    let without_suffix = if file_name.ends_with("_controller") {
        &file_name[..file_name.len() - "_controller".len()]
    } else {
        file_name
    };

    let mut result = String::new();
    let mut capitalize = true;
    for c in without_suffix.chars() {
        if c == '_' {
            capitalize = true;
        } else if capitalize {
            result.push(c.to_ascii_uppercase());
            capitalize = false;
        } else {
            result.push(c);
        }
    }
    result
}

/// Convert "PostsController" to "posts"
fn to_controller_name(class_name: &str) -> String {
    let mut result = String::new();
    for (i, c) in class_name.chars().enumerate() {
        if i > 0 && c.is_ascii_uppercase() {
            result.push('_');
        }
        result.push(c.to_ascii_lowercase());
    }
    result
}

/// Extract the class name from "class X extends Controller"
fn extract_class_name(source: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("class ") {
            // Parse "class ClassName extends ..."
            let after_class = &trimmed["class ".len()..];
            let class_name = if let Some(pos) = after_class.find(" extends ") {
                &after_class[..pos]
            } else if let Some(pos) = after_class.find(" ") {
                &after_class[..pos]
            } else {
                after_class
            };
            return Some(class_name.trim().to_string());
        }
    }
    None
}

/// Parse the static block for controller configuration.
fn parse_controller_static_block(source: &str, info: &mut ControllerInfo) -> Result<(), String> {
    // Find static { ... } block
    let static_block = extract_static_block(source)?;

    if static_block.is_empty() {
        return Ok(());
    }

    // Parse this.layout = "..."
    if let Some(layout) = extract_quoted_value(&static_block, "this.layout") {
        info.layout = Some(layout);
    }

    // Parse this.before_action = fn(req) { ... }
    if let Some(handler_source) = extract_function_source(&static_block, "this.before_action") {
        info.before_actions.push(BeforeAction {
            actions: Vec::new(), // Empty = all actions
            handler_source,
        });
    }

    // Parse this.before_action(:action1, :action2) = fn(req) { ... }
    if let Some((actions, handler_source)) =
        extract_action_specific_function_source(&static_block, "this.before_action")
    {
        info.before_actions.push(BeforeAction {
            actions,
            handler_source,
        });
    }

    // Parse this.after_action = fn(req, response) { ... }
    if let Some(handler_source) = extract_function_source(&static_block, "this.after_action") {
        info.after_actions.push(AfterAction {
            actions: Vec::new(),
            handler_source,
        });
    }

    // Parse this.after_action(:action1, :action2) = fn(req, response) { ... }
    if let Some((actions, handler_source)) =
        extract_action_specific_function_source(&static_block, "this.after_action")
    {
        info.after_actions.push(AfterAction {
            actions,
            handler_source,
        });
    }

    Ok(())
}

/// Extract the static { ... } block from source.
fn extract_static_block(source: &str) -> Result<String, String> {
    let mut depth = 0;
    let mut in_static = false;
    let mut result = String::new();
    let mut chars = source.char_indices().peekable();

    while let Some((pos, c)) = chars.next() {
        if !in_static {
            if c == 's' || c == 'S' {
                let rest: String = source[pos..].chars().take(6).collect();
                if rest.to_lowercase() == "static" {
                    // Check if followed by {
                    let mut ahead = chars.clone();
                    while let Some((_, c2)) = ahead.next() {
                        if c2.is_whitespace() {
                            continue;
                        }
                        if c2 == '{' {
                            in_static = true;
                            depth = 1;
                            // Skip past "static" and whitespace
                            for _ in 0..rest.len() {
                                chars.next();
                            }
                            // Skip whitespace
                            while let Some((_, c2)) = chars.peek() {
                                if c2.is_whitespace() {
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                            // Skip opening brace
                            if let Some((_, '{')) = chars.next() {
                                // Now inside static block
                            }
                            break;
                        }
                        break;
                    }
                }
            }
        } else {
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            if in_static && depth > 0 {
                result.push(c);
            }
        }
    }

    Ok(result)
}

/// Extract a quoted string value like this.layout = "value"
fn extract_quoted_value(source: &str, key: &str) -> Option<String> {
    let key_pattern = format!("{} = ", key);
    if let Some(pos) = source.find(&key_pattern) {
        let after = &source[pos + key_pattern.len()..];
        if after.starts_with('"') {
            if let Some(end) = after[1..].find('"') {
                return Some(after[1..=end].to_string());
            }
        }
    }
    None
}

/// Extract a function definition source code like this.before_action = fn(req) { ... }
fn extract_function_source(source: &str, key: &str) -> Option<String> {
    let key_pattern = format!("{} = ", key);
    if let Some(pos) = source.find(&key_pattern) {
        let after = &source[pos + key_pattern.len()..];

        // Look for fn(...) { pattern
        if after.starts_with("fn") {
            // Count to matching brace - start from fn, not from (
            let fn_start = after.find('(')?;
            let fn_end = find_matching_brace(&after[fn_start..])?;
            // Include "fn" prefix in the result (index 0 to matching brace)
            let fn_source = &after[..fn_start + fn_end + 1];

            // Return the complete function source code including "fn"
            return Some(fn_source.to_string());
        }
    }
    None
}

/// Extract a function source code with action list like this.before_action(:show, :edit) = fn(req) { ... }
fn extract_action_specific_function_source(
    source: &str,
    key: &str,
) -> Option<(Vec<String>, String)> {
    let pattern = format!("{}(:", key);
    if let Some(pos) = source.find(&pattern) {
        let after = &source[pos + pattern.len() - 1..]; // Include the colon

        // Parse action list: :action1, :action2) = fn(...) {
        let actions_end = after.find(") = ")?;
        let actions_str = &after[1..actions_end]; // Skip leading :

        let actions: Vec<String> = actions_str
            .split(',')
            .map(|s| s.trim().trim_start_matches(':').to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let after_fn = &after[actions_end + 4..]; // Skip ") = "

        if after_fn.starts_with("fn") {
            let fn_start = after_fn.find('(')?;
            let fn_end = find_matching_brace(&after_fn[fn_start..])?;
            // Include "fn" prefix in the result (index 0 to matching brace)
            let fn_source = &after_fn[..fn_start + fn_end + 1];

            return Some((actions, fn_source.to_string()));
        }
    }
    None
}

/// Find matching brace position (assumes starting at opening brace)
fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth = 0;
    let mut in_string = false;
    let mut string_char = ' ';

    for (i, c) in s.chars().enumerate() {
        if in_string {
            if c == string_char && s.chars().nth(i.wrapping_sub(1)) != Some('\\') {
                in_string = false;
            }
            continue;
        }

        match c {
            '"' | '\'' => {
                in_string = true;
                string_char = c;
            }
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Extract public methods (actions) from controller source.
fn extract_actions(source: &str, class_name: &str, info: &mut ControllerInfo) {
    for line in source.lines() {
        let trimmed = line.trim();

        // Look for "fn name(req: Any)" pattern
        if trimmed.starts_with("fn ") {
            // Check if it's a public method (doesn't start with _)
            if let Some(fn_name) = extract_fn_name(trimmed) {
                if !fn_name.starts_with('_') {
                    info.actions.push(ControllerAction {
                        controller_name: info.class_name.clone(),
                        class_name: class_name.to_string(),
                        action_name: fn_name,
                        is_public: true,
                    });
                }
            }
        }
    }
}

/// Extract function name from "fn name(req: Any) -> Any {"
fn extract_fn_name(line: &str) -> Option<String> {
    let after_fn = line[3..].trim_start();
    let name_end = after_fn.find('(')?;
    Some(after_fn[..name_end].to_string())
}

/// Set the current controller for this thread (for accessing from helpers).
pub fn set_current_controller(controller: Value) {
    CURRENT_CONTROLLER.with(|c| {
        *c.borrow_mut() = Some(controller);
    });
}

/// Get the current controller for this thread.
pub fn get_current_controller() -> Option<Value> {
    CURRENT_CONTROLLER.with(|c| c.borrow().clone())
}

/// Clear the current controller.
pub fn clear_current_controller() {
    CURRENT_CONTROLLER.with(|c| {
        *c.borrow_mut() = None;
    });
}

/// Get or compile a handler program from cache.
fn get_or_compile_handler(wrapped_source: &str) -> Result<crate::ast::Program, String> {
    HANDLER_PROGRAM_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();

        // Check if already cached
        if let Some(program) = cache.get(wrapped_source) {
            return Ok(program.clone());
        }

        // Compile and cache
        let tokens = crate::lexer::Scanner::new(wrapped_source)
            .scan_tokens()
            .map_err(|e| format!("Lexer error in handler: {}", e))?;

        let program = crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| format!("Parser error in handler: {}", e))?;

        cache.insert(wrapped_source.to_string(), program.clone());
        Ok(program)
    })
}

/// Compile and execute a before/after action handler source code.
/// Returns the result of executing the handler.
/// Uses thread-local cache to avoid re-parsing on every request.
pub fn execute_handler_source(
    handler_source: &str,
    interpreter: &mut Interpreter,
    req: Value,
) -> Result<Value, String> {
    // Create a wrapper that defines the function, calls it, and stores the result
    let wrapped_source = format!(
        "let __handler = {}; let __result = __handler(req);",
        handler_source
    );

    // Set req in the environment
    interpreter
        .environment
        .borrow_mut()
        .define("req".to_string(), req);

    // Get cached or compile the handler program
    let program = get_or_compile_handler(&wrapped_source)?;

    // Execute (ignore errors for now)
    let _ = interpreter.interpret(&program);

    // Retrieve the result
    interpreter
        .environment
        .borrow()
        .get("__result")
        .ok_or_else(|| "Handler did not return a value".to_string())
}

/// Compile and execute an after action handler with both req and response.
/// Uses thread-local cache to avoid re-parsing on every request.
pub fn execute_after_handler_source(
    handler_source: &str,
    interpreter: &mut Interpreter,
    req: Value,
    response: Value,
) -> Result<Value, String> {
    // Create a wrapper that defines the function, calls it, and stores the result
    let wrapped_source = format!(
        "let __handler = {}; let __result = __handler(req, response);",
        handler_source
    );

    // Set req and response in the environment
    interpreter
        .environment
        .borrow_mut()
        .define("req".to_string(), req);
    interpreter
        .environment
        .borrow_mut()
        .define("response".to_string(), response);

    // Get cached or compile the handler program
    let program = get_or_compile_handler(&wrapped_source)?;

    // Execute (ignore errors for now)
    let _ = interpreter.interpret(&program);

    // Retrieve the result
    interpreter
        .environment
        .borrow()
        .get("__result")
        .ok_or_else(|| "After handler did not return a value".to_string())
}

/// Create a new controller instance for the given class name.
pub fn create_controller_instance(
    class_name: &str,
    interpreter: &mut Interpreter,
) -> Result<Value, String> {
    // Look up the class
    let class_value = interpreter
        .environment
        .borrow()
        .get(class_name)
        .ok_or_else(|| format!("Controller class '{}' not found", class_name))?
        .clone();

    // Instantiate the class
    instantiate_class(&class_value)
}

/// Instantiate a class value to create an instance.
fn instantiate_class(class_value: &Value) -> Result<Value, String> {
    match class_value {
        Value::Class(class_rc) => {
            // Create instance with empty fields
            let instance = Instance::new(class_rc.clone());
            Ok(Value::Instance(Rc::new(RefCell::new(instance))))
        }
        _ => Err("Cannot instantiate non-class value".to_string()),
    }
}

/// Set up the request context for a controller instance.
/// This injects req, params, session, headers into the controller.
pub fn setup_controller_context(
    controller: &Value,
    req: &Value,
    params: &Value,
    session: &Value,
    headers: &Value,
) {
    if let Value::Instance(inst_rc) = controller {
        let mut inst = inst_rc.borrow_mut();
        inst.fields.insert("req".to_string(), req.clone());
        inst.fields.insert("params".to_string(), params.clone());
        inst.fields.insert("session".to_string(), session.clone());
        inst.fields.insert("headers".to_string(), headers.clone());
    }
}

/// Get a field from a controller instance.
pub fn get_controller_field(controller: &Value, field_name: &str) -> Option<Value> {
    match controller {
        Value::Instance(inst_rc) => {
            let inst = inst_rc.borrow();
            inst.fields.get(field_name).cloned()
        }
        _ => None,
    }
}

/// Set a field in a controller instance.
pub fn set_controller_field(controller: &Value, field_name: &str, value: Value) {
    if let Value::Instance(inst_rc) = controller {
        let mut inst = inst_rc.borrow_mut();
        inst.fields.insert(field_name.to_string(), value);
    }
}

/// Call a controller action method by name.
pub fn call_controller_action(
    controller_class_name: &str,
    action_name: &str,
    interpreter: &mut Interpreter,
) -> Result<Value, String> {
    // Look up the action function in the controller class
    // For OOP controllers, actions are defined as methods on the class
    // We need to look up the function and call it with the controller instance

    // First, try to get the function from the environment (for function-based controllers)
    let func_name = format!("{}_{}", controller_class_name, action_name);
    let func_opt = interpreter.environment.borrow().get(&func_name);

    // Release the borrow before calling call_function_value
    drop(interpreter.environment.borrow());

    if let Some(func) = func_opt {
        // Call the function directly (function-based controller)
        let args = vec![];
        return call_function_value(&func, &args, interpreter);
    }

    // For OOP controllers, we need to look up the method on the class
    // This is a placeholder - actual implementation would involve
    // looking up the method on the controller class and binding it to the instance
    Err(format!(
        "Action '{}' not found in controller '{}'",
        action_name, controller_class_name
    ))
}

/// Call a function value with the given arguments.
fn call_function_value(
    func: &Value,
    args: &[Value],
    _interpreter: &mut Interpreter,
) -> Result<Value, String> {
    match func {
        Value::Function(_func_data) => {
            // For bytecode functions, we'd need to call them properly
            // For now, this is a placeholder
            Err("Function calls not yet implemented for OOP controllers".to_string())
        }
        Value::NativeFunction(native_func) => (native_func.func)(args.to_vec()),
        _ => Err("Cannot call non-function value".to_string()),
    }
}
