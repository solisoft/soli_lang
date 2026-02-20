//! Application Loader
//!
//! This module handles loading controllers, models, middleware, and executing files.
//! It also provides view file tracking for hot reload.

use std::path::{Path, PathBuf};

use crate::error::RuntimeError;
use crate::interpreter::builtins::router::register_controller_action;
use crate::interpreter::{Interpreter, Value};
use crate::serve::hot_reload::FileTracker;
use crate::serve::middleware::{
    clear_middleware, extract_middleware_functions, register_middleware_with_options,
    scan_middleware_files,
};
use crate::serve::router::{derive_routes_from_controller, to_pascal_case_controller};
use crate::span::Span;

/// Scan for all controller files in the controllers directory.
pub(crate) fn scan_controllers(controllers_dir: &Path) -> Result<Vec<PathBuf>, RuntimeError> {
    let mut controllers = Vec::new();

    for entry in std::fs::read_dir(controllers_dir).map_err(|e| RuntimeError::General {
        message: format!("Failed to read controllers directory: {}", e),
        span: Span::default(),
    })? {
        let entry = entry.map_err(|e| RuntimeError::General {
            message: format!("Failed to read directory entry: {}", e),
            span: Span::default(),
        })?;

        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "sl") {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.ends_with("_controller.sl") {
                    controllers.push(path);
                }
            }
        }
    }

    Ok(controllers)
}

/// Load all model files.
pub(crate) fn load_models(
    interpreter: &mut Interpreter,
    models_dir: &Path,
) -> Result<(), RuntimeError> {
    for entry in std::fs::read_dir(models_dir)
        .map_err(|e| RuntimeError::General {
            message: format!("Failed to read models directory: {}", e),
            span: Span::default(),
        })?
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "sl") {
            println!("Loading model: {}", path.display());
            execute_file(interpreter, &path)?;
        }
    }
    Ok(())
}

/// Load all middleware files and register middleware functions.
pub(crate) fn load_middleware(
    interpreter: &mut Interpreter,
    middleware_dir: &Path,
    file_tracker: &mut FileTracker,
) -> Result<(), RuntimeError> {
    // Clear existing middleware
    clear_middleware();

    let middleware_files = scan_middleware_files(middleware_dir)?;

    if middleware_files.is_empty() {
        return Ok(());
    }

    println!("Loading middleware:");

    for middleware_path in middleware_files {
        // Track file for hot reload
        file_tracker.track(&middleware_path);

        // Read source to extract function names and orders
        let source =
            std::fs::read_to_string(&middleware_path).map_err(|e| RuntimeError::General {
                message: format!("Failed to read middleware file: {}", e),
                span: Span::default(),
            })?;

        let functions = extract_middleware_functions(&source);

        // Execute the middleware file to define functions
        execute_file(interpreter, &middleware_path)?;

        // Register each middleware function
        for (func_name, order, global_only, scope_only) in functions {
            let func_value = interpreter
                .environment
                .borrow()
                .get(&func_name)
                .ok_or_else(|| RuntimeError::General {
                    message: format!(
                        "Middleware function '{}' not found in {}",
                        func_name,
                        middleware_path.display()
                    ),
                    span: Span::default(),
                })?;

            let flags = if global_only {
                " [global_only]".to_string()
            } else if scope_only {
                " [scope_only]".to_string()
            } else {
                "".to_string()
            };
            println!(
                "  [{}] {} (order: {}){}",
                middleware_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown"),
                func_name,
                order,
                flags
            );

            register_middleware_with_options(
                &func_name,
                func_value,
                order,
                global_only,
                scope_only,
            );
        }
    }

    Ok(())
}

/// Load a controller file and register its routes.
pub(crate) fn load_controller(
    interpreter: &mut Interpreter,
    controller_path: &Path,
    file_tracker: &mut FileTracker,
) -> Result<(), RuntimeError> {
    let controller_name = controller_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    println!("Loading controller: {}", controller_name);

    // Track file for hot reload
    file_tracker.track(controller_path);

    // Read and parse the controller to extract function names
    let source = std::fs::read_to_string(controller_path).map_err(|e| RuntimeError::General {
        message: format!("Failed to read controller file: {}", e),
        span: Span::default(),
    })?;

    // Derive routes from the controller
    let routes = derive_routes_from_controller(controller_name, &source)?;

    // Execute the controller file to define functions
    execute_file(interpreter, controller_path)?;

    // Check if this is an OOP controller (class-based)
    let controller_key = controller_name.trim_end_matches("_controller");
    let class_name = to_pascal_case_controller(controller_key);
    let is_oop_controller = interpreter
        .environment
        .borrow()
        .get(&class_name)
        .map(|v| matches!(v, Value::Class(_)))
        .unwrap_or(false);

    // Register routes using the interpreter's environment
    for route in routes {
        // Create full handler name: controller#action
        let full_handler_name = format!("{}#{}", controller_key, route.function_name);

        if is_oop_controller {
            // OOP controller: methods are inside the class, resolved at runtime
        } else {
            // Function-based controller: look up the function in the environment
            let func_value = interpreter
                .environment
                .borrow()
                .get(&route.function_name)
                .ok_or_else(|| RuntimeError::General {
                    message: format!(
                        "Function '{}' not found in controller {}",
                        route.function_name, controller_name
                    ),
                    span: Span::default(),
                })?;

            // Register action in global registry for DSL lookup
            register_controller_action(controller_key, &route.function_name, func_value.clone());
        }

        crate::interpreter::builtins::server::register_route_with_handler(
            &route.method,
            &route.path,
            full_handler_name,
        );
    }

    Ok(())
}

/// Execute a Soli file with the given interpreter.
pub(crate) fn execute_file(interpreter: &mut Interpreter, path: &Path) -> Result<(), RuntimeError> {
    let source = std::fs::read_to_string(path).map_err(|e| RuntimeError::General {
        message: format!("Failed to read file '{}': {}", path.display(), e),
        span: Span::default(),
    })?;

    // Lex
    let tokens = crate::lexer::Scanner::new(&source)
        .scan_tokens()
        .map_err(|e| RuntimeError::General {
            message: format!("Lexer error in {}: {}", path.display(), e),
            span: Span::default(),
        })?;

    // Parse
    let mut program =
        crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| RuntimeError::General {
                message: format!("Parser error in {}: {}", path.display(), e),
                span: Span::default(),
            })?;

    // Module resolution (if the file has imports)
    if crate::has_imports(&program) {
        let base_dir = path.parent().unwrap_or(std::path::Path::new("."));
        let mut resolver = crate::module::ModuleResolver::new(base_dir);
        program = resolver
            .resolve(program, path)
            .map_err(|e| RuntimeError::General {
                message: format!("Module resolution error in {}: {}", path.display(), e),
                span: Span::default(),
            })?;
    }

    // Execute (skip type checking for flexibility)
    interpreter.set_source_path(path.to_path_buf());
    interpreter.interpret(&program)
}

/// Recursively track view files for hot reload.
pub(crate) fn track_view_files(
    views_dir: &Path,
    file_tracker: &mut FileTracker,
) -> Result<(), RuntimeError> {
    fn track_recursive(dir: &Path, file_tracker: &mut FileTracker) -> Result<(), RuntimeError> {
        if !dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(dir)
            .map_err(|e| RuntimeError::General {
                message: format!("Failed to read views directory: {}", e),
                span: Span::default(),
            })?
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                track_recursive(&path, file_tracker)?;
            } else if path.extension().is_some_and(|ext| ext == "erb") {
                file_tracker.track(&path);
            }
        }
        Ok(())
    }

    track_recursive(views_dir, file_tracker)
}

/// Load all controllers in a worker thread
pub(crate) fn load_controllers_in_worker(
    worker_id: usize,
    interpreter: &mut Interpreter,
    controllers_dir: &Path,
) {
    if let Ok(entries) = std::fs::read_dir(controllers_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "sl") {
                if let Err(e) = execute_file(interpreter, &path) {
                    eprintln!(
                        "Worker {}: Error loading {}: {}",
                        worker_id,
                        path.display(),
                        e
                    );
                }

                // Also register controller actions in this worker (only for function-based controllers)
                if let Some(name) = path.file_stem().and_then(|n| n.to_str()) {
                    if name.ends_with("_controller") {
                        let controller_key = name.trim_end_matches("_controller");
                        let class_name = to_pascal_case_controller(controller_key);

                        // Check if this is an OOP controller (class-based)
                        let is_oop_controller = interpreter
                            .environment
                            .borrow()
                            .get(&class_name)
                            .map(|v| matches!(v, Value::Class(_)))
                            .unwrap_or(false);

                        // Only register actions for function-based controllers
                        // OOP controllers have their methods resolved at runtime
                        if !is_oop_controller {
                            let source = std::fs::read_to_string(&path).unwrap_or_default();
                            let routes =
                                derive_routes_from_controller(name, &source).unwrap_or_default();
                            for route in routes {
                                if let Some(func_value) =
                                    interpreter.environment.borrow().get(&route.function_name)
                                {
                                    register_controller_action(
                                        controller_key,
                                        &route.function_name,
                                        func_value.clone(),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Define DSL helpers for routes in the interpreter.
/// This must be called before routes.sl can be executed.
pub(crate) fn define_routes_dsl(interpreter: &mut Interpreter) -> Result<(), RuntimeError> {
    let dsl_source = r#"
        fn resources(name: Any, block: Any = null) {
            router_resource_enter(name, null);
            if (block != null) { block(); }
            router_resource_exit();
        }

        fn namespace(name: Any, block: Any) {
            router_namespace_enter(name);
            if (block != null) { block(); }
            router_namespace_exit();
        }

        fn member(block: Any) {
            router_member_enter();
            if (block != null) { block(); }
            router_member_exit();
        }

        fn collection(block: Any) {
            router_collection_enter();
            if (block != null) { block(); }
            router_collection_exit();
        }

        fn middleware(mw_names: Any, block: Any) {
            router_middleware_scope(mw_names);
            if (block != null) { block(); }
            router_middleware_scope_exit();
        }

        fn get(path: Any, action: Any) { router_match("GET", path, action); }
        fn post(path: Any, action: Any) { router_match("POST", path, action); }
        fn put(path: Any, action: Any) { router_match("PUT", path, action); }
        fn delete(path: Any, action: Any) { router_match("DELETE", path, action); }
        fn patch(path: Any, action: Any) { router_match("PATCH", path, action); }

        fn websocket(path: Any, action: Any) { router_websocket(path, action); }
    "#;

    let tokens = crate::lexer::Scanner::new(dsl_source)
        .scan_tokens()
        .map_err(|e| RuntimeError::General {
            message: format!("DSL Lexer error: {}", e),
            span: Span::default(),
        })?;
    let program =
        crate::parser::Parser::new(tokens)
            .parse()
            .map_err(|e| RuntimeError::General {
                message: format!("DSL Parser error: {}", e),
                span: Span::default(),
            })?;
    interpreter.interpret(&program)
}

/// Reload all controllers in a worker thread.
/// This ensures OOP controllers are properly registered in the environment
/// before routes are reloaded.
pub(crate) fn reload_controllers_in_worker(
    worker_id: usize,
    interpreter: &mut Interpreter,
    controllers_dir: &Path,
    file_tracker: &mut FileTracker,
) {
    let controller_files = match scan_controllers(controllers_dir) {
        Ok(files) => files,
        Err(e) => {
            eprintln!(
                "Worker {}: Error scanning controllers directory: {}",
                worker_id, e
            );
            return;
        }
    };

    for controller_path in &controller_files {
        if let Err(e) = load_controller(interpreter, controller_path, file_tracker) {
            eprintln!(
                "Worker {}: Error reloading controller {}: {}",
                worker_id,
                controller_path.display(),
                e
            );
        }
    }
}

/// Reload routes in a worker thread.
/// Clears existing routes, resets router context, reloads controllers, and re-executes routes.sl.
/// This ensures that OOP controllers are properly loaded before routes are registered.
pub(crate) fn reload_routes_in_worker(
    worker_id: usize,
    interpreter: &mut Interpreter,
    routes_file: &Path,
    controllers_dir: &Path,
    file_tracker: &mut FileTracker,
) {
    // 1. Save current routes before clearing (for rollback on failure)
    let saved_routes = crate::interpreter::builtins::server::take_routes();
    let saved_ws_routes = crate::serve::websocket::take_websocket_routes();

    // 2. Reset router context
    crate::interpreter::builtins::router::reset_router_context();

    // 3. Reload controllers to ensure OOP controller classes are available
    reload_controllers_in_worker(worker_id, interpreter, controllers_dir, file_tracker);

    // 4. Clear old routes and re-execute routes.sl
    crate::interpreter::builtins::server::clear_routes();
    crate::serve::websocket::clear_websocket_routes();

    // Define route DSL functions before executing routes.sl
    if let Err(e) = define_routes_dsl(interpreter) {
        eprintln!(
            "Worker {}: Error defining route DSL: {} - restoring previous routes",
            worker_id, e
        );
        crate::interpreter::builtins::server::restore_routes(saved_routes);
        crate::serve::websocket::restore_websocket_routes(saved_ws_routes);
        crate::interpreter::builtins::server::rebuild_route_index();
        return;
    }

    let reload_result = if routes_file.exists() {
        execute_file(interpreter, routes_file)
    } else {
        Ok(())
    };

    // 5. Rebuild route index
    crate::interpreter::builtins::server::rebuild_route_index();

    // 6. If reload failed, restore previous routes
    if let Err(e) = reload_result {
        eprintln!(
            "Worker {}: Error reloading routes: {} - restoring previous routes",
            worker_id, e
        );
        crate::interpreter::builtins::server::restore_routes(saved_routes);
        crate::serve::websocket::restore_websocket_routes(saved_ws_routes);
        crate::interpreter::builtins::server::rebuild_route_index();
    }
}
