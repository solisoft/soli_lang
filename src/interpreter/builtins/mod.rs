//! Built-in functions for Soli.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Write};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

thread_local! {
    /// The controller action name ("index", "update", ...) for the request the
    /// current worker thread is handling. Set by the server in `call_handler`
    /// from the "controller#action" handler string and read by the
    /// `current_action()` builtin so the auth Policy layer can infer the policy
    /// method in `authorize(record)` without an explicit argument.
    static CURRENT_ACTION: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Record the action name for the in-flight request on this worker thread.
pub fn set_current_action(action: &str) {
    CURRENT_ACTION.with(|cell| {
        *cell.borrow_mut() = action.to_string();
    });
}

/// Clear the recorded action name (called when a request finishes).
pub fn clear_current_action() {
    CURRENT_ACTION.with(|cell| cell.borrow_mut().clear());
}

/// The action name recorded for the current request, or "" outside a request.
pub fn current_action_name() -> String {
    CURRENT_ACTION.with(|cell| cell.borrow().clone())
}

// Re-export submodules
pub mod assertions;
pub mod assigns_helpers;
pub mod body_limit;
pub mod cache;
pub mod clock;
pub mod collections;
pub mod controller;
pub mod cookie_jar;
pub mod crypto;
pub mod datetime;
pub mod datetime_class;
pub mod deflate;
pub mod dotenv;
pub mod encoding;
pub mod env;
pub mod expectations;
pub mod factories;
pub mod file;
pub mod hash;
pub mod hex;
pub mod html;
pub mod http_class;
pub mod http_log;
pub mod i18n;
pub mod image;
pub mod jobs;
pub mod json;
pub mod jwt;
pub mod kv;
pub mod kv_log;
pub mod mailer;
pub mod markdown;
pub mod math;
pub mod mock_http;
pub mod model;
pub mod named_routes;
pub mod nanoid;
pub mod pades;
pub mod pades_tsa;
pub mod pdf;
pub mod pdf_markdown;
pub mod permit;
pub mod pop3;
pub mod primitives;
pub mod rate_limit;
pub mod regex;
pub mod request_helpers;
pub mod resp;
pub mod respond_to;
pub mod response_helpers;
pub mod router;
pub mod rsa_key;
pub mod s3;
pub mod secure_cookies;
pub mod security_headers;
pub mod server;
pub mod session;
pub mod session_cookie;
pub mod session_disk;
pub mod session_helpers;
pub mod session_solidb;
pub mod session_solikv;
pub mod soap;
pub mod solidb;
pub mod solikv;
pub mod spreadsheet;
pub mod streaming;
pub mod strings;
pub mod system;
pub mod template;
pub mod test_dsl;
pub mod test_server;
pub mod trust_proxy;
pub mod types;
pub mod ulid;
pub mod uploads;
pub mod uuid;
pub mod validation;
pub mod vapid;
pub mod x509;
pub mod xml_c14n;

thread_local! {
    /// When `Some`, Soli's `print`/`println` builtins write here instead of
    /// the process stdout. Used by the parallel test runner so each worker
    /// thread captures its own output without racing on the global stdout fd
    /// (which is what `gag::BufferRedirect` does — incompatible with
    /// `--jobs > 1` because the OS pipe deadlocks once it fills).
    static STDOUT_CAPTURE: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

/// RAII guard: while alive, the current thread's `print`/`println` output is
/// buffered into a thread-local Vec instead of being written to stdout.
/// On drop, the captured bytes are returned via the closure passed to
/// `with_captured_stdout`.
pub struct StdoutCaptureGuard {
    _private: (),
}

impl StdoutCaptureGuard {
    /// Begin capturing the current thread's `print`/`println` output.
    pub fn start() -> Self {
        STDOUT_CAPTURE.with(|c| *c.borrow_mut() = Some(Vec::new()));
        Self { _private: () }
    }

    /// Stop capturing and return the buffered bytes.
    pub fn finish(self) -> Vec<u8> {
        STDOUT_CAPTURE
            .with(|c| c.borrow_mut().take())
            .unwrap_or_default()
    }
}

impl Drop for StdoutCaptureGuard {
    fn drop(&mut self) {
        STDOUT_CAPTURE.with(|c| {
            let _ = c.borrow_mut().take();
        });
    }
}

fn write_captured_or_stdout(s: &str) {
    let captured = STDOUT_CAPTURE.with(|c| {
        if let Some(ref mut buf) = *c.borrow_mut() {
            buf.extend_from_slice(s.as_bytes());
            true
        } else {
            false
        }
    });
    if !captured {
        let stdout = io::stdout();
        let mut handle = stdout.lock();
        let _ = handle.write_all(s.as_bytes());
    }
}

/// Register all built-in functions in the given environment.
/// When `include_test_builtins` is false, test-only modules (factories, assertions,
/// test_dsl, test_server) are skipped to save memory in serve mode.
pub fn register_builtins(env: &mut Environment, include_test_builtins: bool) {
    // ===== Core I/O functions =====

    // print(...) - Print values to stdout (auto-resolves Futures)
    env.define(
        "print".to_string(),
        Value::NativeFunction(NativeFunction::new("print", None, |args| {
            for (i, arg) in args.into_iter().enumerate() {
                if i > 0 {
                    write_captured_or_stdout(" ");
                }
                // Auto-resolve futures before printing
                let resolved = arg.resolve()?;
                write_captured_or_stdout(&format!("{}", resolved));
            }
            write_captured_or_stdout("\n");
            Ok(Value::Null)
        })),
    );

    // println / puts — both print space-joined args followed by a newline.
    fn println_impl(args: Vec<Value>) -> Result<Value, String> {
        for (i, arg) in args.into_iter().enumerate() {
            if i > 0 {
                write_captured_or_stdout(" ");
            }
            let resolved = arg.resolve()?;
            write_captured_or_stdout(&format!("{}", resolved));
        }
        write_captured_or_stdout("\n");
        Ok(Value::Null)
    }
    env.define(
        "println".to_string(),
        Value::NativeFunction(NativeFunction::new("println", None, println_impl)),
    );
    env.define(
        "puts".to_string(),
        Value::NativeFunction(NativeFunction::new("puts", None, println_impl)),
    );

    // grouped(fn() { ... }) — coalesce the DB reads inside the block into a
    // single round-trip. The real work happens in the `evaluate_call`
    // interceptor (it needs `&mut Interpreter` to run the block); this
    // placeholder only catches misuse (a non-block argument).
    env.define(
        "grouped".to_string(),
        Value::NativeFunction(NativeFunction::new("grouped", Some(1), |_args| {
            Err("grouped() expects a function block: grouped(fn() { ... })".to_string())
        })),
    );

    // __sdql_exec(query, binds) — runtime backing for `@sdbql{ ... }` blocks.
    // The tree-walking interpreter executes the block inline; the VM compiler
    // lowers a block to a call to this global instead (it cannot inline the
    // query against the live environment). `binds` is a hash of bare
    // interpolation variable name -> value. Both paths funnel into the shared
    // `run_sdql_block` so the two runtimes stay identical.
    env.define(
        "__sdql_exec".to_string(),
        Value::NativeFunction(NativeFunction::new("__sdql_exec", None, |args| {
            let query = match args.first() {
                Some(Value::String(s)) => s.as_ref().to_string(),
                _ => {
                    return Err(
                        "__sdql_exec expects an SDBQL query string as its first argument"
                            .to_string(),
                    )
                }
            };
            let mut binds: Vec<(String, Value)> = Vec::new();
            if let Some(Value::Hash(hash)) = args.get(1) {
                for (key, value) in hash.borrow().iter() {
                    if let crate::interpreter::value::HashKey::String(name) = key {
                        binds.push((name.to_string(), value.clone()));
                    }
                }
            }
            Ok(crate::interpreter::executor::literals::run_sdql_block(
                &query, &binds,
            ))
        })),
    );

    // break() - Trigger a breakpoint for debugging (opens dev page with REPL)
    env.define(
        "break".to_string(),
        Value::NativeFunction(NativeFunction::new("break", Some(0), |_args| {
            Ok(Value::Breakpoint)
        })),
    );

    // next() - Alias for continue() in loops
    env.define(
        "next".to_string(),
        Value::NativeFunction(NativeFunction::new("next", Some(0), |_args| {
            Ok(Value::Continue)
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
            Ok(Value::String(line.trim_end().to_string().into()))
        })),
    );

    // defined(name) - Check if a variable is defined in the current scope chain
    env.define(
        "defined".to_string(),
        Value::NativeFunction(NativeFunction::new("defined", Some(1), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "defined() expects a string, got {}",
                        other.type_name()
                    ))
                }
            };
            let exists = crate::interpreter::executor::is_defined(&name);
            Ok(Value::Bool(exists))
        })),
    );

    // const_get(name) - Resolve a string name to its value (class, function, variable, etc.)
    env.define(
        "const_get".to_string(),
        Value::NativeFunction(NativeFunction::new("const_get", Some(1), |args| {
            let name = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "const_get() expects a string, got {}",
                        other.type_name()
                    ))
                }
            };
            Ok(crate::interpreter::executor::current_env_lookup(&name).unwrap_or(Value::Null))
        })),
    );

    // current_action() - The controller action name for the in-flight request
    // ("index", "update", ...), or "" outside a request. Lets the auth Policy
    // layer infer the policy method in `authorize(record)`.
    env.define(
        "current_action".to_string(),
        Value::NativeFunction(NativeFunction::new("current_action", Some(0), |_args| {
            Ok(Value::String(current_action_name().into()))
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
                Value::QueryBuilder(qb) => Ok(
                    crate::interpreter::builtins::model::execute_query_builder_count(&qb.borrow()),
                ),
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
            // Trip the data-cache dirty flag: the returned value flows
            // into the controller's data hash, so a data-signature
            // key would otherwise group requests with different
            // timestamps under one cached body.
            crate::template::response_cache::mark_data_dirty();
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

    // Markdown class (Markdown.to_html)
    markdown::register_markdown_builtins(env);

    // Register JSON class (replaces json_parse and json_stringify functions)
    json::register_json_class(env);

    // Register Image class
    image::register_image_class(env);

    // ===== Register other submodule builtins =====

    // Register HTTP class
    http_class::register_http_class(env);

    // Register S3 class
    s3::register_s3_class(env);

    // Register SOAP class
    soap::register_soap_class(env);

    // Register Xml class (exclusive XML canonicalization for XML-DSig)
    xml_c14n::register_xml_builtins(env);

    // Register X509 class (certificate public-key extraction for SAML)
    x509::register_x509_builtins(env);

    // Register Deflate class (raw DEFLATE for the SAML HTTP-Redirect binding)
    deflate::register_deflate_builtins(env);

    // Register Hex class (hex<->bytes bridge between Crypto.* and Base64)
    hex::register_hex_class(env);

    // Register Encoding class (charset decode/encode: Latin-1, etc. <-> UTF-8)
    encoding::register_encoding_class(env);

    // Register RsaKey class (PEM private-key parsing for envelope signing)
    rsa_key::register_rsa_key_builtins(env);

    // Register Pop3 email-reading class
    pop3::register_pop3_class(env);

    // Register outbound-email (Mailer) native builtins. The `Mailer`/`Message`
    // classes themselves are defined by a Soli prelude (mailer::ensure_prelude).
    mailer::register_mailer_builtins(env);

    // Register streaming/SSE builtins (sse / stream) + the StreamOut emitter.
    streaming::register_streaming_builtins(env);

    // Register url_encode/url_decode
    server::register_server_builtins(env);

    // Register WebSocket server functions
    server::register_websocket_builtins(env);

    // Register cryptographic functions
    crypto::register_crypto_builtins(env);

    // Register UUID generators (uuid_v4, uuid_v7, UUID class)
    uuid::register_uuid_builtins(env);

    // Register ULID generator (ulid(), ULID class)
    ulid::register_ulid_builtins(env);

    // Register NanoID generator (nanoid(size?, alphabet?), NanoID class)
    nanoid::register_nanoid_builtins(env);

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
    permit::register_permit_builtins(env);

    // Register Job and Cron classes (SolidB-backed background jobs)
    jobs::register_jobs_builtins(env);

    // Register JWT builtins
    jwt::register_jwt_builtins(env);

    // Register VAPID / Web Push builtins
    vapid::register_vapid_builtins(env);

    // Register PDF / Factur-X builtins
    pdf::register_pdf_builtins(env);

    // Register test-only builtins (skipped in serve mode)
    if include_test_builtins {
        factories::register_factories(env);
        assertions::register_assertions(env);
        expectations::register_expectation_class(env);
        test_dsl::register_test_builtins(env);
        test_server::register_test_server_builtins(env);
        mock_http::register_mock_http_builtins(env);
    }

    // Register request helper builtins
    request_helpers::register_request_helpers(env);

    // Register response helper builtins
    response_helpers::register_response_helpers(env);

    // Register session + view-introspection helper builtins. Registered after
    // response_helpers so the real view_path/render_template implementations
    // win over any earlier definitions. Test-only.
    if include_test_builtins {
        session_helpers::register_session_helpers(env);
        assigns_helpers::register_assigns_helpers(env);
    }

    // Register Error class and error types
    register_error_classes(env);

    // Register cache builtins
    cache::register_cache_builtins(env);

    // Register KV builtins
    kv::register_kv_builtins(env);

    // Register rate limit builtins
    rate_limit::register_rate_limit_builtins(env);

    // Register security headers builtins
    security_headers::register_security_headers_builtins(env);

    // Register trust-proxy gate (off by default; opt in only when the
    // deployment terminates X-Forwarded-* at a trusted proxy hop)
    trust_proxy::register_trust_proxy_builtins(env);

    // SEC-028: register the force-Secure-cookies gate. Off by default;
    // opt in via SOLI_FORCE_SECURE_COOKIES=1 or `enable_force_secure_cookies()`
    // when the deployment is always on TLS but the proxy doesn't forward
    // `X-Forwarded-Proto: https` (or the operator hasn't enabled trust_proxy).
    secure_cookies::register_secure_cookies_builtins(env);

    // Register body-size limit (default 8 MiB; raise via set_max_body_size
    // when accepting larger file uploads)
    body_limit::register_body_limit_builtins(env);

    // Register upload builtins
    uploads::register_upload_builtins(env);

    // Register clock builtins (sleep, microtime)
    clock::register_clock_builtins(env);

    // Register collection classes (String, Array, Hash, Set, Range, Base64)
    collections::register_collection_classes(env);

    // Register value-type primitive classes (Int, Float, Bool, Null, Decimal, Symbol)
    primitives::register_primitive_classes(env);

    // Register system builtins (System.run, System.run_sync)
    system::register_system_builtins(env);

    // Register spreadsheet builtins (Spreadsheet.csv, Spreadsheet.excel, etc.)
    spreadsheet::register_spreadsheet_builtins(env);

    // Register cookie builtins (set_cookie) — registered last so they win over
    // any test helpers with the same name (e.g. set_cookie in request_helpers).
    session::register_cookie_builtins(env);

    // Signed/encrypted cookie jar reader (read_cookie); the write side rides
    // set_cookie's options ("signed"/"encrypted") above.
    cookie_jar::register_cookie_jar_builtins(env);
}

/// Register the Error class and built-in error types.
fn register_error_classes(env: &mut Environment) {
    use crate::interpreter::value::Class;

    // Create the Error base class (shared by all subclasses)
    let error_class = Rc::new(Class {
        name: "Error".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    });
    env.define("Error".to_string(), Value::Class(error_class.clone()));

    // Helper to create an error subclass sharing the same Error base Rc
    let mut define_error_subclass = |name: &str| {
        let subclass = Class {
            name: name.to_string(),
            superclass: Some(error_class.clone()),
            methods: Rc::new(RefCell::new(HashMap::new())),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields: HashMap::new(),
            constructor: None,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
            ..Default::default()
        };
        env.define(name.to_string(), Value::Class(Rc::new(subclass)));
    };

    define_error_subclass("ValueError");
    define_error_subclass("TypeError");
    define_error_subclass("KeyError");
    define_error_subclass("IndexError");
    define_error_subclass("RuntimeError");
}
