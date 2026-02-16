//! Built-in functions for Soli.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Write};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

// Re-export submodules
pub mod assertions;
pub mod assigns_helpers;
pub mod cache;
pub mod clock;
pub mod collections;
pub mod controller;
pub mod crypto;
pub mod datetime;
pub mod datetime_class;
pub mod dotenv;
pub mod env;
pub mod factories;
pub mod file;
pub mod hash;
pub mod html;
pub mod http;
pub mod http_class;
pub mod i18n;
pub mod json;
pub mod jwt;
pub mod math;
pub mod model;
pub mod rate_limit;
pub mod regex;
pub mod request_helpers;
pub mod response_helpers;
pub mod router;
pub mod security_headers;
pub mod server;
pub mod session;
pub mod session_helpers;
pub mod soap;
pub mod solidb;
pub mod strings;
pub mod system;
pub mod template;
pub mod test_dsl;
pub mod test_server;
pub mod types;
pub mod uploads;
pub mod validation;

/// Register all built-in functions in the given environment.
pub fn register_builtins(env: &mut Environment) {
    // ===== Core I/O functions =====

    // print(...) - Print values to stdout (auto-resolves Futures)
    env.define(
        "print".to_string(),
        Value::NativeFunction(NativeFunction::new("print", None, |args| {
            for (i, arg) in args.into_iter().enumerate() {
                if i > 0 {
                    print!(" ");
                }
                // Auto-resolve futures before printing
                let resolved = arg.resolve()?;
                print!("{}", resolved);
            }
            println!();
            Ok(Value::Null)
        })),
    );

    // println(...) - Same as print, alias
    env.define(
        "println".to_string(),
        Value::NativeFunction(NativeFunction::new("println", None, |args| {
            for (i, arg) in args.into_iter().enumerate() {
                if i > 0 {
                    print!(" ");
                }
                // Auto-resolve futures before printing
                let resolved = arg.resolve()?;
                print!("{}", resolved);
            }
            println!();
            Ok(Value::Null)
        })),
    );

    // break() - Trigger a breakpoint for debugging (opens dev page with REPL)
    env.define(
        "break".to_string(),
        Value::NativeFunction(NativeFunction::new("break", Some(0), |_args| {
            Ok(Value::Breakpoint)
        })),
    );

    // await(future) - Await a Future value and return the resolved result
    env.define(
        "await".to_string(),
        Value::NativeFunction(NativeFunction::new("await", Some(1), |args| {
            if args.is_empty() {
                return Err("await() requires one argument".to_string());
            }
            args[0].clone().resolve()
        })),
    );

    // input(prompt?) - Read a line from stdin
    env.define(
        "input".to_string(),
        Value::NativeFunction(NativeFunction::new("input", None, |args| {
            if let Some(Value::String(prompt)) = args.first() {
                print!("{}", prompt);
                io::stdout().flush().ok();
            }
            let mut line = String::new();
            io::stdin()
                .read_line(&mut line)
                .map_err(|e| e.to_string())?;
            Ok(Value::String(line.trim_end().to_string()))
        })),
    );

    // ===== Universal collection functions =====

    // len(array|string|hash) - Get length (auto-resolves Futures)
    env.define(
        "len".to_string(),
        Value::NativeFunction(NativeFunction::new("len", Some(1), |args| {
            let resolved = args.into_iter().next().unwrap().resolve()?;
            match &resolved {
                Value::Array(arr) => Ok(Value::Int(arr.borrow().len() as i64)),
                Value::String(s) => Ok(Value::Int(s.chars().count() as i64)),
                Value::Hash(hash) => Ok(Value::Int(hash.borrow().len() as i64)),
                other => Err(format!(
                    "len() expects array, string, or hash, got {}",
                    other.type_name()
                )),
            }
        })),
    );

    // ===== Time function =====

    // clock() - Current time in seconds since epoch
    env.define(
        "clock".to_string(),
        Value::NativeFunction(NativeFunction::new("clock", Some(0), |_| {
            let duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
            Ok(Value::Float(duration.as_secs_f64()))
        })),
    );

    // ===== Register themed submodule builtins =====

    // Type conversion functions (str, int, float, type)
    types::register_type_builtins(env);

    // Math functions (range, abs, min, max, sqrt, pow)
    math::register_math_builtins(env);

    // Hash functions (keys, values, has_key, delete, merge, entries, from_entries, clear)
    hash::register_hash_builtins(env);

    // File I/O functions (barf, slurp)
    file::register_file_builtins(env);

    // String functions (split, join, contains, index_of, substring, upcase, downcase, trim, replace)
    strings::register_string_builtins(env);

    // HTML functions (html_escape, html_unescape, sanitize_html, strip_html)
    html::register_html_builtins(env);

    // Register JSON class (replaces json_parse and json_stringify functions)
    json::register_json_class(env);

    // ===== Register other submodule builtins =====

    // Register HTTP class
    http_class::register_http_class(env);

    // Register SOAP class
    soap::register_soap_class(env);

    // Register HTTP server functions
    server::register_server_builtins(env);

    // Register WebSocket server functions
    server::register_websocket_builtins(env);

    // Register cryptographic functions
    crypto::register_crypto_builtins(env);

    // Register SoliDB functions
    solidb::register_solidb_builtins(env);

    // Register Model/ORM functions
    model::register_model_builtins(env);

    // Register dotenv functions
    dotenv::register_dotenv_builtins(env);

    // Register env functions
    env::register_env_builtins(env);

    // Register template functions
    template::register_template_builtins(env);

    // Register Regex class
    regex::register_regex_class(env);

    // Register router functions
    router::register_router_builtins(env);

    // Register controller functions
    controller::register_controller_builtins(env);

    // Register datetime functions (helper functions)
    datetime::register_datetime_builtins(env);

    // Register DateTime and Duration classes
    datetime_class::register_datetime_and_duration_classes(env);

    // Register I18n class
    i18n::register_i18n_class(env);

    // Register validation system (V class and validate function)
    validation::register_validation_builtins(env);

    // Register session management builtins
    session::register_session_builtins(env);

    // Register JWT builtins
    jwt::register_jwt_builtins(env);

    // Register factory builtins
    factories::register_factories(env);

    // Register assertion builtins
    assertions::register_assertions(env);

    // Register test DSL builtins
    test_dsl::register_test_builtins(env);

    // Register test server builtins
    test_server::register_test_server_builtins(env);

    // Register request helper builtins
    request_helpers::register_request_helpers(env);

    // Register response helper builtins
    response_helpers::register_response_helpers(env);

    // Register session helper builtins (disabled due to type complexity)
    // session_helpers::register_session_helpers(env);

    // Register assigns helper builtins (disabled due to type complexity)
    // assigns_helpers::register_assigns_helpers(env);

    // Register Error class and error types
    register_error_classes(env);

    // Register cache builtins
    cache::register_cache_builtins(env);

    // Register rate limit builtins
    rate_limit::register_rate_limit_builtins(env);

    // Register security headers builtins
    security_headers::register_security_headers_builtins(env);

    // Register upload builtins
    uploads::register_upload_builtins(env);

    // Register clock builtins (sleep, microtime)
    clock::register_clock_builtins(env);

    // Register collection classes (String, Array, Hash, Set, Range, Base64)
    collections::register_collection_classes(env);

    // Register system builtins (System.run, System.run_sync)
    system::register_system_builtins(env);
}

/// Register the Error class and built-in error types.
fn register_error_classes(env: &mut Environment) {
    use crate::interpreter::value::Class;

    // Create the Error base class
    let error_class = Class {
        name: "Error".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define("Error".to_string(), Value::Class(Rc::new(error_class)));

    // ValueError class
    let value_error_class = Class {
        name: "ValueError".to_string(),
        superclass: Some(Rc::new(Class {
            name: "Error".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields: HashMap::new(),
            constructor: None,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
            ..Default::default()
        })),
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define(
        "ValueError".to_string(),
        Value::Class(Rc::new(value_error_class)),
    );

    // TypeError class
    let type_error_class = Class {
        name: "TypeError".to_string(),
        superclass: Some(Rc::new(Class {
            name: "Error".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields: HashMap::new(),
            constructor: None,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
            ..Default::default()
        })),
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define(
        "TypeError".to_string(),
        Value::Class(Rc::new(type_error_class)),
    );

    // KeyError class (for hash key not found)
    let key_error_class = Class {
        name: "KeyError".to_string(),
        superclass: Some(Rc::new(Class {
            name: "Error".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields: HashMap::new(),
            constructor: None,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
            ..Default::default()
        })),
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define(
        "KeyError".to_string(),
        Value::Class(Rc::new(key_error_class)),
    );

    // IndexError class (for array index out of bounds)
    let index_error_class = Class {
        name: "IndexError".to_string(),
        superclass: Some(Rc::new(Class {
            name: "Error".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields: HashMap::new(),
            constructor: None,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
            ..Default::default()
        })),
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define(
        "IndexError".to_string(),
        Value::Class(Rc::new(index_error_class)),
    );

    // RuntimeError class
    let runtime_error_class = Class {
        name: "RuntimeError".to_string(),
        superclass: Some(Rc::new(Class {
            name: "Error".to_string(),
            superclass: None,
            methods: HashMap::new(),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields: HashMap::new(),
            constructor: None,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
            ..Default::default()
        })),
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define(
        "RuntimeError".to_string(),
        Value::Class(Rc::new(runtime_error_class)),
    );
}
