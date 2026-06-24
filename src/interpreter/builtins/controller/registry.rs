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

use super::controller::{AfterAction, BeforeAction, ControllerAction, ControllerInfo, LayoutRule};
use crate::interpreter::builtins::template as template_module;
use crate::interpreter::value::{Instance, Value};
use crate::interpreter::Interpreter;

// Global registry of all controllers.
// Uses RwLock to allow concurrent reads (most operations) while only blocking for writes.
lazy_static::lazy_static! {
    pub static ref CONTROLLER_REGISTRY: RwLock<ControllerRegistry> = RwLock::new(ControllerRegistry::new());
}

// Thread-local controller instances for current request.
thread_local! {
    static CURRENT_CONTROLLER: RefCell<Option<Value>> = const { RefCell::new(None) };
}

// Thread-local cache for pre-compiled handler programs.
// Key: handler source string, Value: parsed Program
thread_local! {
    #[allow(clippy::missing_const_for_thread_local)]
    static HANDLER_PROGRAM_CACHE: RefCell<HashMap<String, crate::ast::Program>> = RefCell::new(HashMap::new());
}

/// Controller registry - stores metadata about all controllers.
#[derive(Debug, Clone)]
pub struct ControllerRegistry {
    controllers: HashMap<String, ControllerInfo>,
}

impl Default for ControllerRegistry {
    fn default() -> Self {
        Self::new()
    }
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

    /// Get a mutable reference to a controller by its full name.
    pub fn get_by_name_mut(&mut self, name: &str) -> Option<&mut ControllerInfo> {
        self.controllers.values_mut().find(|c| c.name == name)
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
/// After registration, inherits before/after actions and layout from parent controllers.
pub fn scan_controllers(controllers_dir: &Path) -> Result<(), String> {
    let mut registry = CONTROLLER_REGISTRY.write().unwrap();

    if !controllers_dir.exists() {
        return Ok(());
    }

    // Track superclass relationships for inheritance resolution
    let mut superclass_map: HashMap<String, String> = HashMap::new(); // class_name -> parent_class_name

    fn walk(
        dir: &Path,
        root: &Path,
        registry: &mut ControllerRegistry,
        superclass_map: &mut HashMap<String, String>,
    ) -> Result<(), String> {
        for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();

            if path.is_dir() {
                walk(&path, root, registry, superclass_map)?;
                continue;
            }

            if path.is_file() && path.extension().is_some_and(|ext| ext == "sl") {
                if let Some(file_name) = path.file_stem().and_then(|n| n.to_str()) {
                    // Skip non-controller files
                    if !file_name.ends_with("_controller") {
                        continue;
                    }

                    // Build the registry key from the path relative to the
                    // controllers root, using `/` separators — matching the
                    // route handler key (e.g. `admin/categories`). Deriving
                    // the key from the class name instead produced
                    // `admin_categories`, which silently broke before_action
                    // lookups for any controller in a subdirectory.
                    let route_key = relative_route_key(&path, root);

                    match parse_controller_file(&path, file_name, &route_key) {
                        Ok(info) => {
                            if let Ok(source) = std::fs::read_to_string(&path) {
                                if let Some(parent) = extract_superclass_name(&source) {
                                    if parent != "Controller" {
                                        superclass_map.insert(info.name.clone(), parent);
                                    }
                                }
                            }

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

    walk(
        controllers_dir,
        controllers_dir,
        &mut registry,
        &mut superclass_map,
    )?;

    // Inherit before/after actions and layout from parent controllers
    resolve_controller_inheritance(&mut registry, &superclass_map);

    Ok(())
}

/// Resolve inheritance: copy before/after actions and layout from parent controllers
/// to children that don't define their own.
/// Inherited hooks from a parent controller: before-actions, after-actions,
/// the default layout, and per-action layout rules.
type InheritedHooks = (
    Vec<BeforeAction>,
    Vec<AfterAction>,
    Option<String>,
    Vec<LayoutRule>,
);

fn resolve_controller_inheritance(
    registry: &mut ControllerRegistry,
    superclass_map: &HashMap<String, String>,
) {
    // Collect inherited hooks from parents
    let mut inherited: HashMap<String, InheritedHooks> = HashMap::new();

    for (child_name, parent_name) in superclass_map {
        if let Some(parent_info) = registry.get_by_name(parent_name) {
            let mut before_actions = parent_info.before_actions.clone();
            let mut after_actions = parent_info.after_actions.clone();
            let mut layout = parent_info.layout.clone();
            let mut action_layouts = parent_info.action_layouts.clone();

            // If the parent also inherits, chain up
            if let Some((parent_before, parent_after, parent_layout, parent_action_layouts)) =
                inherited.get(parent_name)
            {
                let mut combined_before = parent_before.clone();
                combined_before.extend(before_actions);
                before_actions = combined_before;

                let mut combined_after = parent_after.clone();
                combined_after.extend(after_actions);
                after_actions = combined_after;

                if layout.is_none() {
                    layout = parent_layout.clone();
                }

                // Grandparent rules sit after the parent's so nearer ancestors
                // win; child rules are prepended later in the apply step.
                let mut combined_action_layouts = action_layouts;
                combined_action_layouts.extend(parent_action_layouts.clone());
                action_layouts = combined_action_layouts;
            }

            inherited.insert(
                child_name.clone(),
                (before_actions, after_actions, layout, action_layouts),
            );
        }
    }

    // Apply inherited hooks to child controllers
    for (child_name, (parent_before, parent_after, parent_layout, parent_action_layouts)) in
        inherited
    {
        if let Some(child_info) = registry.get_by_name_mut(&child_name) {
            // Prepend parent before_actions (parent hooks run first)
            let mut combined = parent_before;
            combined.append(&mut child_info.before_actions);
            child_info.before_actions = combined;

            // Prepend parent after_actions
            let mut combined = parent_after;
            combined.append(&mut child_info.after_actions);
            child_info.after_actions = combined;

            // Inherit layout if child doesn't define one
            if child_info.layout.is_none() {
                child_info.layout = parent_layout;
            }

            // Append parent per-action rules after the child's own, so a
            // child rule for the same action overrides the inherited one
            // (`layout_for` returns the first match).
            child_info.action_layouts.extend(parent_action_layouts);
        }
    }
}

/// Parse a controller file and extract metadata.
///
/// `route_key` is the routing key the framework expects on lookups (e.g.
/// `admin/categories` for `app/controllers/admin/categories_controller.sl`).
/// It must be derived from the file's path so it matches what the router
/// uses — deriving from the class name produces `admin_categories`, which
/// silently breaks `before_action`/`after_action` resolution for nested
/// controllers.
fn parse_controller_file(
    path: &Path,
    file_name: &str,
    route_key: &str,
) -> Result<ControllerInfo, String> {
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;

    // Controller class name (e.g., "posts_controller" -> "PostsController")
    let class_name = to_class_name(file_name);

    // Extract class name from file (e.g., "class PostsController extends Controller")
    let actual_class_name = extract_class_name(&source).unwrap_or_else(|| class_name.clone());

    let mut info = ControllerInfo::new(&actual_class_name, route_key);

    // Parse static block for configuration
    parse_controller_static_block(&source, &mut info)?;

    // Extract public methods (actions)
    extract_actions(&source, &actual_class_name, &mut info);

    Ok(info)
}

/// Compute the registry/route key for a controller file relative to the
/// controllers directory. Strips `_controller` from the file stem and joins
/// any subdirectory segments with `/`.
///
/// - `controllers_dir/posts_controller.sl` → `posts`
/// - `controllers_dir/admin/categories_controller.sl` → `admin/categories`
fn relative_route_key(path: &Path, root: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let no_ext = rel.with_extension("");
    let mut segments: Vec<String> = no_ext
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(str::to_string))
        .collect();
    if let Some(last) = segments.last_mut() {
        if let Some(stripped) = last.strip_suffix("_controller") {
            *last = stripped.to_string();
        }
    }
    segments.join("/")
}

/// Convert "posts_controller" to "PostsController"
fn to_class_name(file_name: &str) -> String {
    let without_suffix = file_name.strip_suffix("_controller").unwrap_or(file_name);

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

/// Extract the class name from "class X extends Controller"
fn extract_class_name(source: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(after_class) = trimmed.strip_prefix("class ") {
            // Parse "class ClassName extends ..."
            let class_name = if let Some(pos) = after_class.find(" extends ") {
                &after_class[..pos]
            } else if let Some(pos) = after_class.find(' ') {
                &after_class[..pos]
            } else {
                after_class
            };
            return Some(class_name.trim().to_string());
        }
    }
    None
}

/// Extract the superclass name from "class X extends SuperClass"
fn extract_superclass_name(source: &str) -> Option<String> {
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(after_class) = trimmed.strip_prefix("class ") {
            if let Some(pos) = after_class.find(" extends ") {
                let after_extends = &after_class[pos + 9..];
                return after_extends
                    .split_whitespace()
                    .next()
                    .map(|s| s.to_string());
            }
            if let Some(pos) = after_class.find(" < ") {
                let after_lt = &after_class[pos + 3..];
                return after_lt.split_whitespace().next().map(|s| s.to_string());
            }
        }
    }
    None
}

/// Parse the static block for controller configuration.
fn parse_controller_static_block(source: &str, info: &mut ControllerInfo) -> Result<(), String> {
    // Find static { ... } block, along with the file line where its body
    // starts. We need that offset so each extracted hook's `source_line`
    // (line of `fn` within the static block) can be translated back to the
    // original file's line — which is what coverage hits must be attributed
    // to.
    let (static_block, body_start_line) = extract_static_block_with_line(source)?;

    if static_block.is_empty() {
        return Ok(());
    }

    // `extract_function_source` returns a 1-based line counted within the
    // static block; adding `body_start_line - 1` shifts it into file-line
    // space.
    let line_shift = body_start_line.saturating_sub(1);

    // Parse this.layout = "..." (the controller-wide default)
    if let Some(layout) = extract_quoted_value(&static_block, "this.layout") {
        info.layout = Some(layout);
    }

    // Parse this.layout("name", only: [:a, :b]) / except: [:c] — per-action
    // overrides. May appear multiple times; checked in order, first match wins.
    for rule in extract_all_layout_rules(&static_block) {
        info.action_layouts.push(rule);
    }

    // Parse this.before_action = fn(req) { ... }
    if let Some((handler_source, local_line)) =
        extract_function_source(&static_block, "this.before_action")
    {
        info.before_actions.push(BeforeAction {
            actions: Vec::new(), // Empty = all actions
            handler_source,
            source_line: local_line + line_shift,
        });
    }

    // Parse this.before_action(:action1, :action2) = fn(req) { ... } — may appear multiple times
    for (actions, handler_source, local_line) in
        extract_all_action_specific_function_sources(&static_block, "this.before_action")
    {
        info.before_actions.push(BeforeAction {
            actions,
            handler_source,
            source_line: local_line + line_shift,
        });
    }

    // Parse this.after_action = fn(req, response) { ... }
    if let Some((handler_source, local_line)) =
        extract_function_source(&static_block, "this.after_action")
    {
        info.after_actions.push(AfterAction {
            actions: Vec::new(),
            handler_source,
            source_line: local_line + line_shift,
        });
    }

    // Parse this.after_action(:action1, :action2) = fn(req, response) { ... } — may appear multiple times
    for (actions, handler_source, local_line) in
        extract_all_action_specific_function_sources(&static_block, "this.after_action")
    {
        info.after_actions.push(AfterAction {
            actions,
            handler_source,
            source_line: local_line + line_shift,
        });
    }

    Ok(())
}

/// Extract the `static { ... }` block from source. Returns an empty string if absent.
/// Also returns the 1-based line where the block body starts in the file, so
/// extracted handler line numbers can be translated back from static-block-local
/// coordinates to file coordinates.
fn extract_static_block_with_line(source: &str) -> Result<(String, usize), String> {
    let bytes = source.as_bytes();
    let mut search_from = 0;

    while let Some(rel) = source[search_from..].find("static") {
        let kw_start = search_from + rel;
        let kw_end = kw_start + "static".len();

        // Require a word boundary before and after so we don't match e.g. `ecstatic`.
        let before_ok = kw_start == 0 || !is_ident_byte(bytes[kw_start - 1]);
        let after_ok = kw_end >= bytes.len() || !is_ident_byte(bytes[kw_end]);
        if !(before_ok && after_ok) {
            search_from = kw_end;
            continue;
        }

        // Find the `{` that opens the block (skipping whitespace).
        let mut i = kw_end;
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] as char != '{' {
            search_from = kw_end;
            continue;
        }

        // Walk the block body, tracking string literals, line comments, and
        // nested braces. Line comments matter because a Soli `#` comment
        // containing an apostrophe (e.g. `doesn't`) would otherwise open a
        // phantom string literal that never closes, leaving the scanner stuck
        // in string mode and misreporting "Unclosed static block".
        let body_start = i + 1;
        let body_start_line = source[..body_start].bytes().filter(|&b| b == b'\n').count() + 1;
        let mut depth = 1;
        let mut in_string = false;
        let mut string_char = 0u8;
        let mut j = body_start;
        while j < bytes.len() {
            let b = bytes[j];
            if in_string {
                if b == string_char && (j == 0 || bytes[j - 1] != b'\\') {
                    in_string = false;
                }
            } else {
                match b {
                    b'#' => {
                        // `#` starts a line comment; skip to end of line.
                        while j < bytes.len() && bytes[j] != b'\n' {
                            j += 1;
                        }
                        // Don't consume the newline — let the outer loop see it.
                        continue;
                    }
                    b'/' if j + 1 < bytes.len() && bytes[j + 1] == b'/' => {
                        // `//` line comment (Soli also accepts these).
                        while j < bytes.len() && bytes[j] != b'\n' {
                            j += 1;
                        }
                        continue;
                    }
                    b'"' | b'\'' => {
                        in_string = true;
                        string_char = b;
                    }
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return Ok((source[body_start..j].to_string(), body_start_line));
                        }
                    }
                    _ => {}
                }
            }
            j += 1;
        }

        return Err("Unclosed static block".to_string());
    }

    Ok((String::new(), 0))
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Extract a quoted string value like this.layout = "value"
fn extract_quoted_value(source: &str, key: &str) -> Option<String> {
    let key_pattern = format!("{} = ", key);
    if let Some(pos) = source.find(&key_pattern) {
        let after = &source[pos + key_pattern.len()..];
        if let Some(stripped) = after.strip_prefix('"') {
            if let Some(end) = stripped.find('"') {
                return Some(stripped[..end].to_string());
            }
        }
    }
    None
}

/// Extract per-action layout rules of the form:
///   this.layout("name")                     # applies to all actions
///   this.layout("name", only: [:a, :b])     # only these actions
///   this.layout("name", except: [:c])       # all actions but these
/// Returns one `LayoutRule` per occurrence, in source order. The assignment
/// form `this.layout = "..."` is handled separately by `extract_quoted_value`
/// and is not matched here (it has no `(`).
fn extract_all_layout_rules(source: &str) -> Vec<LayoutRule> {
    let pattern = "this.layout(";
    let mut results = Vec::new();
    let mut cursor = 0;

    while let Some(rel_pos) = source[cursor..].find(pattern) {
        let pos = cursor + rel_pos;
        let args_start = pos + pattern.len();
        let after = &source[args_start..];

        // Find the ')' that closes this call, skipping string literals and
        // `[...]` arrays so a delimiter inside them can't terminate early.
        let Some(close) = find_call_close_paren(after) else {
            break;
        };
        let args = &after[..close];
        cursor = args_start + close + 1;

        // The first quoted string is the layout name; without one the call is
        // malformed — skip it rather than registering a blank layout.
        let Some(layout) = first_quoted_string(args) else {
            continue;
        };

        results.push(LayoutRule {
            layout,
            only: extract_symbol_list(args, "only"),
            except: extract_symbol_list(args, "except"),
        });
    }

    results
}

/// Find the byte offset of the `)` that closes a call's argument list. `s`
/// must start just after the opening `(`. Skips string literals and `[...]`
/// arrays so their contents can't terminate the scan early.
fn find_call_close_paren(s: &str) -> Option<usize> {
    let mut bracket_depth = 0i32;
    let mut in_string = false;
    let mut string_char = ' ';
    let mut prev = '\0';

    for (i, c) in s.char_indices() {
        if in_string {
            if c == string_char && prev != '\\' {
                in_string = false;
            }
            prev = c;
            continue;
        }
        match c {
            '"' | '\'' => {
                in_string = true;
                string_char = c;
            }
            '[' => bracket_depth += 1,
            ']' => bracket_depth -= 1,
            ')' if bracket_depth == 0 => return Some(i),
            _ => {}
        }
        prev = c;
    }
    None
}

/// Return the contents of the first single- or double-quoted string in `s`.
fn first_quoted_string(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let quote = bytes[i];
        if quote == b'"' || quote == b'\'' {
            let start = i + 1;
            if let Some(rel_end) = s[start..].find(quote as char) {
                return Some(s[start..start + rel_end].to_string());
            }
            return None;
        }
        i += 1;
    }
    None
}

/// Extract a symbol/identifier list like `only: [:a, :b]` → `["a", "b"]`.
/// Returns empty when `key` is absent. Accepts `:`-prefixed symbols, bare
/// identifiers, and quoted names.
fn extract_symbol_list(args: &str, key: &str) -> Vec<String> {
    let needle = format!("{}:", key);
    let Some(kpos) = args.find(&needle) else {
        return Vec::new();
    };
    let after = &args[kpos + needle.len()..];
    let Some(open) = after.find('[') else {
        return Vec::new();
    };
    let Some(rel_close) = after[open..].find(']') else {
        return Vec::new();
    };
    after[open + 1..open + rel_close]
        .split(',')
        .map(|item| {
            item.trim()
                .trim_start_matches(':')
                .trim_matches('"')
                .trim_matches('\'')
                .to_string()
        })
        .filter(|item| !item.is_empty())
        .collect()
}

/// Extract a function definition source code like this.before_action = fn(req) { ... }
fn extract_function_source(source: &str, key: &str) -> Option<(String, usize)> {
    let key_pattern = format!("{} = ", key);
    if let Some(pos) = source.find(&key_pattern) {
        let fn_byte = pos + key_pattern.len();
        let after = &source[fn_byte..];

        // Look for fn(...) { pattern
        if after.starts_with("fn") {
            // Count to matching brace - start from fn, not from (
            let fn_start = after.find('(')?;
            let fn_end = find_matching_brace(&after[fn_start..])?;
            // Include "fn" prefix in the result (index 0 to matching brace)
            let fn_source = &after[..fn_start + fn_end + 1];

            // 1-based line number where `fn` appears in the original file.
            // The +1 makes it match how editors display lines.
            let source_line = source[..fn_byte].bytes().filter(|&b| b == b'\n').count() + 1;
            return Some((fn_source.to_string(), source_line));
        }
    }
    None
}

/// Extract every `this.before_action(:a, :b) = fn(...) { ... }` occurrence in order.
/// Returns the list of action names, the function source, and the 1-based line
/// number where `fn(...)` appears in the original file (needed for coverage
/// span alignment in `execute_handler_source`).
fn extract_all_action_specific_function_sources(
    source: &str,
    key: &str,
) -> Vec<(Vec<String>, String, usize)> {
    let pattern = format!("{}(:", key);
    let mut results = Vec::new();
    let mut cursor = 0;

    while let Some(rel_pos) = source[cursor..].find(&pattern) {
        let pos = cursor + rel_pos;
        // Include the colon at the start so action parsing below sees `:name, :name) = ...`.
        let after = &source[pos + pattern.len() - 1..];

        let Some(actions_end) = after.find(") = ") else {
            break;
        };
        let actions_str = &after[1..actions_end]; // Skip leading ':'

        let actions: Vec<String> = actions_str
            .split(',')
            .map(|s| s.trim().trim_start_matches(':').to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let after_fn = &after[actions_end + 4..]; // Skip ") = "

        if !after_fn.starts_with("fn") {
            // Not a function definition — skip this occurrence and keep looking.
            cursor = pos + pattern.len();
            continue;
        }

        let Some(fn_start) = after_fn.find('(') else {
            break;
        };
        let Some(fn_end) = find_matching_brace(&after_fn[fn_start..]) else {
            break;
        };

        let fn_source = &after_fn[..fn_start + fn_end + 1];
        // Byte offset of `fn` in the original source; line is the count of
        // preceding newlines + 1.
        let fn_byte = (after.as_ptr() as usize - source.as_ptr() as usize) + actions_end + 4;
        let source_line = source[..fn_byte].bytes().filter(|&b| b == b'\n').count() + 1;

        let consumed_end = fn_byte + fn_start + fn_end + 1;

        results.push((actions, fn_source.to_string(), source_line));
        cursor = consumed_end;
    }

    results
}

/// Find matching brace position (assumes starting at opening brace).
///
/// Returns a **byte** offset into `s` so the result can be used directly in
/// `&s[..pos + 1]` slicing. `char_indices()` yields the byte position of each
/// char's first byte, which is what we need: with multi-byte sequences in
/// string literals (e.g. `"Accès refusé"`), a char-index–based result would
/// truncate the extracted handler before its closing `}`, leaving the parser
/// to fail with `Unexpected token 'EOF', expected }`.
fn find_matching_brace(s: &str) -> Option<usize> {
    let mut depth = 0;
    let mut in_string = false;
    let mut string_char = ' ';
    let mut prev_char = '\0';

    for (i, c) in s.char_indices() {
        if in_string {
            if c == string_char && prev_char != '\\' {
                in_string = false;
            }
            prev_char = c;
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
        prev_char = c;
    }
    None
}

/// Extract public methods (actions) from controller source.
fn extract_actions(source: &str, class_name: &str, info: &mut ControllerInfo) {
    for line in source.lines() {
        let trimmed = line.trim();

        // Accept both `fn name(...)` and `def name(...)` — the lexer treats
        // them as the same keyword, so users writing Ruby-style function-only
        // controllers should not be silently skipped here.
        let after_kw = trimmed
            .strip_prefix("fn ")
            .or_else(|| trimmed.strip_prefix("def "));

        if let Some(rest) = after_kw {
            if let Some(fn_name) = extract_fn_name(rest) {
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

/// Extract function name from the post-keyword remainder, e.g. for
/// `fn name(req: Any) -> Any {` the caller passes `name(req: Any) -> Any {`.
fn extract_fn_name(rest: &str) -> Option<String> {
    let trimmed = rest.trim_start();
    let name_end = trimmed.find('(')?;
    Some(trimmed[..name_end].to_string())
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
/// Work around a parser quirk where `fn(x) { }; <next_stmt>` fails to parse
/// when the body is empty. If the body between the first `{` after `fn(...)`
/// and the matching closing `}` is only whitespace, substitute `{ null }`.
/// An empty hook is effectively a no-op anyway, so this preserves semantics.
fn normalize_empty_handler_body(src: &str) -> String {
    let Some(open) = src.find('{') else {
        return src.to_string();
    };
    let Some(close) = src.rfind('}') else {
        return src.to_string();
    };
    if close <= open {
        return src.to_string();
    }
    let body = &src[open + 1..close];
    if body.chars().all(|c| c.is_whitespace()) {
        let mut out = String::with_capacity(src.len() + 6);
        out.push_str(&src[..open]);
        out.push_str("{ null }");
        out.push_str(&src[close + 1..]);
        return out;
    }
    src.to_string()
}

pub fn execute_handler_source(
    handler_source: &str,
    source_line: usize,
    interpreter: &mut Interpreter,
    req: Value,
) -> Result<Value, String> {
    // Pad the wrapped source with `source_line - 1` newlines so that the
    // `fn(req) { ... }` body lands on the same line numbers it occupied in
    // the original controller file. Without padding, parsed spans start at
    // line 1 and get recorded against the controller's path by coverage —
    // polluting the controller's hit map with phantom hits on comment/blank
    // lines.
    let padding = "\n".repeat(source_line.saturating_sub(1));
    let wrapped_source = format!(
        "{}let __handler = {}; let __result = __handler(req);",
        padding,
        normalize_empty_handler_body(handler_source)
    );

    {
        let mut env = interpreter.environment.borrow_mut();
        env.define("req".to_string(), req);
        // Bind `this` to the current controller instance so `@foo = ...` inside
        // the hook writes to the controller (and then reaches the view via the
        // render-time auto-injection). Without this bind, the free `fn(req)`
        // closure has no `this` in scope and `@foo = x` would fail at runtime.
        if let Some(ctrl) = get_current_controller() {
            env.define("this".to_string(), ctrl);
        }
        // Clear any prior `__result` so a hook that errors mid-flight doesn't
        // silently return the previous request's result on this worker thread.
        env.define("__result".to_string(), Value::Null);
    }

    // Get cached or compile the handler program
    let program = get_or_compile_handler(&wrapped_source)?;

    // Execute — surface errors so a broken hook 500s instead of falling through
    // to the action with a stale/Null `__result` (which `check_for_response`
    // can't recognize as a short-circuit).
    interpreter
        .interpret(&program)
        .map_err(|e| format!("Handler execution error: {}", e))?;

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
    source_line: usize,
    interpreter: &mut Interpreter,
    req: Value,
    response: Value,
) -> Result<Value, String> {
    // See `execute_handler_source` for why we pad with leading newlines.
    let padding = "\n".repeat(source_line.saturating_sub(1));
    let wrapped_source = format!(
        "{}let __handler = {}; let __result = __handler(req, response);",
        padding,
        normalize_empty_handler_body(handler_source)
    );

    {
        let mut env = interpreter.environment.borrow_mut();
        env.define("req".to_string(), req);
        env.define("response".to_string(), response);
        // Bind `this` to the current controller instance — same reasoning as
        // `execute_handler_source`: fields set via `@foo = ...` in the hook should
        // reach the view and be readable as `this.foo` in subsequent code.
        if let Some(ctrl) = get_current_controller() {
            env.define("this".to_string(), ctrl);
        }
        env.define("__result".to_string(), Value::Null);
    }

    // Get cached or compile the handler program
    let program = get_or_compile_handler(&wrapped_source)?;

    // Execute — surface errors rather than swallow them so a broken hook
    // returns a proper 500 instead of silently leaving the response unchanged.
    interpreter
        .interpret(&program)
        .map_err(|e| format!("After handler execution error: {}", e))?;

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
/// This injects req, params, session, headers, cookies into the controller.
pub fn setup_controller_context(
    controller: &Value,
    req: &Value,
    params: &Value,
    session: &Value,
    headers: &Value,
    cookies: &Value,
) {
    if let Value::Instance(inst_rc) = controller {
        let mut inst = inst_rc.borrow_mut();
        inst.fields.insert("req".to_string(), req.clone());
        inst.fields.insert("params".to_string(), params.clone());
        inst.fields.insert("session".to_string(), session.clone());
        inst.fields.insert("headers".to_string(), headers.clone());
        inst.fields.insert("cookies".to_string(), cookies.clone());
    }

    // Also set the current request context for view rendering
    template_module::set_current_request(req.clone());
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
            Err("Function calls not yet implemented for OOP controllers".to_string())
        }
        Value::NativeFunction(native_func) => (native_func.func)(args.to_vec()),
        _ => Err("Cannot call non-function value".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Parser quirk: `fn(x) { }; <next_stmt>` fails to parse when the body is
    // empty, so a hook stub written as `fn(req) { }` used to 500 every
    // request with "Unexpected token 'EOF'". Normalize empty bodies to
    // `{ null }` so they parse and behave as a no-op.
    #[test]
    fn normalize_empty_handler_body_substitutes_null() {
        assert_eq!(
            normalize_empty_handler_body("fn(req) { }"),
            "fn(req) { null }"
        );
        assert_eq!(
            normalize_empty_handler_body("fn(req) {\n    \n}"),
            "fn(req) { null }"
        );
        // Non-empty body is preserved verbatim.
        assert_eq!(
            normalize_empty_handler_body("fn(req) { return req; }"),
            "fn(req) { return req; }"
        );
        // Body containing only a hash literal isn't "empty" — leave it alone.
        assert_eq!(
            normalize_empty_handler_body("fn(req) { {\"a\": 1} }"),
            "fn(req) { {\"a\": 1} }"
        );
    }

    #[test]
    fn relative_route_key_uses_slash_for_subdirs() {
        use std::path::PathBuf;
        let root = PathBuf::from("/app/controllers");
        assert_eq!(
            relative_route_key(&root.join("posts_controller.sl"), &root),
            "posts"
        );
        assert_eq!(
            relative_route_key(&root.join("admin/categories_controller.sl"), &root),
            "admin/categories"
        );
        assert_eq!(
            relative_route_key(&root.join("admin/users/sessions_controller.sl"), &root),
            "admin/users/sessions"
        );
    }

    // Regression: a before_action that does `@foo = x` must actually write to
    // the current controller instance so the auto-view-injection at render-time
    // can expose it. This is the end-to-end guarantee behind the
    // `@current_user = req["current_user"]` pattern in hooks.
    #[test]
    fn handler_binds_this_to_current_controller() {
        use crate::interpreter::value::{Class, HashKey, HashPairs, Instance};

        let class = Rc::new(Class {
            name: "TestController".to_string(),
            ..Default::default()
        });
        let instance_rc = Rc::new(RefCell::new(Instance::new(class)));
        let instance_val = Value::Instance(instance_rc.clone());
        set_current_controller(instance_val);

        let mut interp = crate::interpreter::Interpreter::new();
        let req_hash = Rc::new(RefCell::new({
            let mut h = HashPairs::default();
            h.insert(HashKey::String("uid".into()), Value::Int(7));
            h
        }));
        let req = Value::Hash(req_hash);

        // Handler: reads from req, writes to the controller via @sigil.
        let handler_source = "fn(req) { @uid_from_hook = req[\"uid\"]; req }";
        let result = execute_handler_source(handler_source, 1, &mut interp, req);

        // The hook should return the req (not an error).
        assert!(
            result.is_ok(),
            "handler should execute cleanly: {:?}",
            result
        );

        // The controller instance should now hold the field set by the hook.
        let fields = &instance_rc.borrow().fields;
        assert_eq!(
            fields.get("uid_from_hook"),
            Some(&Value::Int(7)),
            "@uid_from_hook must be written to the instance the hook's `this` is bound to. \
             Without the bind, the free fn has no this, and the write silently fails or \
             scribbles on some other object."
        );
        clear_current_controller();
    }

    #[test]
    fn registers_all_action_specific_before_actions() {
        let source = r#"
class UsersController extends Controller {
    static {
        this.layout = "application";

        this.before_action(:index) = fn(req) {
            return { "status": 403, "body": "Forbidden" };
            req
        }

        this.before_action(:new, :create, :edit, :update, :destroy) = fn(req) {
            return { "status": 401, "body": "Unauthorized" };
            req
        }
    }
}
"#;
        let mut info = ControllerInfo::new("UsersController", "users");
        parse_controller_static_block(source, &mut info).unwrap();

        assert_eq!(info.before_actions.len(), 2, "both hooks must register");
        assert_eq!(info.before_actions[0].actions, vec!["index"]);
        assert_eq!(
            info.before_actions[1].actions,
            vec!["new", "create", "edit", "update", "destroy"]
        );
    }

    #[test]
    fn registers_per_action_layout_rules() {
        let source = r#"
class ReportsController extends Controller {
    static {
        this.layout = "admin";
        this.layout("print", only: [:invoice, :receipt]);
        this.layout("blank", except: [:index]);
    }
}
"#;
        let mut info = ControllerInfo::new("ReportsController", "reports");
        parse_controller_static_block(source, &mut info).unwrap();

        assert_eq!(info.layout.as_deref(), Some("admin"), "default layout");
        assert_eq!(info.action_layouts.len(), 2, "two per-action rules");

        // `only` rule wins for its listed actions.
        assert_eq!(info.layout_for("invoice").as_deref(), Some("print"));
        assert_eq!(info.layout_for("receipt").as_deref(), Some("print"));

        // The `except` rule (registered second) covers everything but `index`;
        // it only takes effect for actions the `print` rule didn't claim.
        assert_eq!(info.layout_for("show").as_deref(), Some("blank"));

        // `index` is excluded from the blank rule and not in the print rule,
        // so it falls through to the controller-wide default.
        assert_eq!(info.layout_for("index").as_deref(), Some("admin"));
    }

    #[test]
    fn layout_call_without_filters_applies_to_all_actions() {
        let source = r#"
class PagesController extends Controller {
    static {
        this.layout("marketing");
    }
}
"#;
        let mut info = ControllerInfo::new("PagesController", "pages");
        parse_controller_static_block(source, &mut info).unwrap();

        assert_eq!(info.action_layouts.len(), 1);
        assert_eq!(info.layout_for("anything").as_deref(), Some("marketing"));
    }

    // `def` is a synonym for `fn` at the lexer level, so function-style
    // controllers written with `def` must register actions too — otherwise
    // routes silently 404 with no visible parse error.
    #[test]
    fn extract_actions_accepts_def_keyword() {
        let source = r#"
def index(req)
  return render("posts/index")
end

def show(req)
  return render("posts/show")
end

def _private_helper(req)
  return null
end
"#;
        let mut info = ControllerInfo::new("PostsController", "posts");
        extract_actions(source, "PostsController", &mut info);

        let names: Vec<&str> = info
            .actions
            .iter()
            .map(|a| a.action_name.as_str())
            .collect();
        assert_eq!(names, vec!["index", "show"]);
    }
}
