//! Built-in functions for Soli.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Write};
use std::rc::Rc;
use web_time::{SystemTime, UNIX_EPOCH};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

crate::request_local! {
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

/// The action name recorded for the current request, or "" outside a request.
pub fn current_action_name() -> String {
    CURRENT_ACTION.with(|cell| cell.borrow().clone())
}

/// Zero-argument builtins whose bare name means their value, on both engines:
/// `clock - start` reads the time like `clock() - start`. The tree-walker
/// calls any zero-argument function read by name; the VM passes a bare
/// function as a value, so it calls these by name only, and the type checker
/// gives them their return type. A program that defines its own `clock` keeps
/// it as written.
pub const BARE_CALL_BUILTINS: &[&str] = &["clock"];

/// Whether a bare read of `name` (no local of that name) calls the builtin.
pub fn is_bare_call_builtin(name: &str) -> bool {
    BARE_CALL_BUILTINS.contains(&name)
}

// Re-export submodules
pub mod apns;
pub mod app_links;
pub(crate) mod args;
pub mod assertions;
pub mod assigns_helpers;
pub mod attachments;
pub mod body_limit;
#[cfg(feature = "mail")]
/// Reading an IMAP `BODYSTRUCTURE`: which parts of a message are the
/// letter and which are the freight, so neither is fetched twice.
pub mod bodystructure;
pub mod browser;
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
#[cfg(feature = "es")]
pub mod es;
pub mod factories;
pub mod fcm;
pub mod file;
pub mod flash;
pub mod geo;
pub mod hash;
pub mod hex;
pub mod html;
pub mod http_class;
pub mod http_log;
pub mod i18n;
pub mod image;
#[cfg(feature = "mail")]
pub mod imap;
pub mod job_log;
pub mod jobs;
pub mod json;
pub mod jwt;
pub mod kv;
pub mod kv_log;
pub mod logger;
pub mod mail_outbox;
#[cfg(feature = "mail")]
pub mod mail_parse;
#[cfg(feature = "mail")]
pub mod mailer;

/// Without the `mail` feature there is no mailer, and the prelude it would
/// install is empty — so the four places that install it, and the job
/// worker, need no `cfg` of their own.
#[cfg(not(feature = "mail"))]
pub mod mailer {
    use crate::interpreter::environment::Environment;
    use crate::interpreter::Interpreter;

    /// Nothing to install, hence nothing for the type checker to learn from
    /// it either (see `types::runtime_globals`).
    pub const MAILER_PRELUDE: &str = "";

    /// No prelude to install.
    pub fn ensure_prelude(_interpreter: &mut Interpreter) {}

    /// No builtins to register.
    pub fn register_mailer_builtins(_env: &mut Environment) {}

    /// The notifier's mail channel, in a build that cannot send mail: report
    /// it rather than pretend the mail went out.
    pub(crate) fn send_operator_mail(
        _to: &[String],
        _from: Option<&str>,
        _subject: &str,
        _text_body: &str,
        _html_body: &str,
    ) -> Result<(), String> {
        Err("this soli was built without the `mail` feature".to_string())
    }
}
pub mod markdown;
pub mod math;
pub mod mock;
pub mod mock_http;
pub mod model;
pub mod money;
pub mod named_routes;
pub mod nanoid;
pub mod native;
#[cfg(feature = "pdf")]
pub mod pades;
#[cfg(feature = "pdf")]
pub mod pades_tsa;
#[cfg(feature = "paseto")]
pub mod paseto;
#[cfg(feature = "pdf")]
pub mod pdf;
#[cfg(feature = "pdf")]
pub mod pdf_markdown;
pub mod permit;
#[cfg(feature = "mail")]
pub mod pop3;
pub mod primitives;
pub mod push;
pub mod rate_limit;
pub mod regex;
pub mod request_helpers;
pub mod resilience;
pub mod resp;
pub mod respond_to;
pub mod response_helpers;
pub mod retry;
pub mod router;
pub mod rsa_key;
#[cfg(feature = "cloud")]
pub mod s3;
#[cfg(feature = "cloud")]
pub(crate) mod s3_client;
pub mod secure_cookies;
pub mod security_headers;
pub mod server;
pub mod session;
pub mod session_cookie;
pub mod session_disk;
pub mod session_helpers;
pub mod session_request_cache;
pub mod session_solidb;
pub mod session_solikv;
pub mod soap;
pub mod solidb;
pub mod solikv;
pub mod spam;
#[cfg(feature = "office")]
pub mod spreadsheet;
pub mod sql_ddl;
pub mod streaming;
pub mod strings;
pub mod system;
pub mod template;
pub mod tenancy;
pub mod test_dsl;
pub mod test_helpers;
pub mod test_progress;
pub mod test_server;
pub mod toml_yaml;
pub mod trust_proxy;
pub mod types;
pub mod ulid;
pub mod updater;
pub mod uploads;
pub mod url_class;
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
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    write_captured_or_stdout(" ");
                }
                // Auto-resolve futures before printing
                let resolved = arg.clone().resolve()?;
                write_captured_or_stdout(&format!("{}", resolved));
            }
            write_captured_or_stdout("\n");
            Ok(Value::Null)
        })),
    );

    // println / puts — both print space-joined args followed by a newline.
    fn println_impl(args: &[Value]) -> Result<Value, String> {
        for (i, arg) in args.iter().enumerate() {
            if i > 0 {
                write_captured_or_stdout(" ");
            }
            let resolved = arg.clone().resolve()?;
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

    // debug() - Trigger a breakpoint for debugging (opens dev page with REPL)
    env.define(
        "debug".to_string(),
        Value::NativeFunction(NativeFunction::new("debug", Some(0), |_args| {
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
            let resolved = args[0].clone().resolve()?;
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

    // switch_tenant / with_tenant / current_tenant (one database per tenant)
    tenancy::register_tenancy_builtins(env);

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
    #[cfg(feature = "cloud")]
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

    // Register Url class (parse/build/join/query params without string surgery)
    url_class::register_url_class(env);

    // Register Logger class (leveled structured logging to stderr)
    logger::register_logger_class(env);

    // Register Toml/Yaml classes (config-format parse/stringify)
    toml_yaml::register_toml_yaml_classes(env);

    // Register CircuitBreaker and Semaphore (process-global concurrency guards)
    resilience::register_circuit_breaker_class(env);
    resilience::register_semaphore_class(env);

    // Register Money class (currency-aware amounts over rust_decimal)
    money::register_money_class(env);

    // Register RsaKey class (PEM private-key parsing for envelope signing)
    rsa_key::register_rsa_key_builtins(env);

    // Register Pop3 email-reading class
    #[cfg(feature = "mail")]
    pop3::register_pop3_class(env);

    // Register Imap email-reading class
    #[cfg(feature = "mail")]
    imap::register_imap_class(env);

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

    // SQL migration DDL helpers (create_table / drop_table) for the SQL adapters.
    let sql_create = Value::NativeFunction(NativeFunction::new(
        "__soli_sql_create_table",
        Some(1),
        |args| {
            let name = match args.first() {
                Some(Value::String(s)) => s.to_string(),
                _ => return Err("__soli_sql_create_table(name) expects a string".into()),
            };
            if !crate::db::is_sql() {
                return Err(
                    "__soli_sql_create_table requires SOLI_DB_ADAPTER=postgres|mysql|sqlite".into(),
                );
            }
            crate::db::ddl::assert_user_table(&name)?;
            crate::db::sql::ensure_table(&name)?;
            Ok(Value::Bool(true))
        },
    ));
    let sql_drop = Value::NativeFunction(NativeFunction::new(
        "__soli_sql_drop_table",
        Some(1),
        |args| {
            let name = match args.first() {
                Some(Value::String(s)) => s.to_string(),
                _ => return Err("__soli_sql_drop_table(name) expects a string".into()),
            };
            if !crate::db::is_sql() {
                return Err(
                    "__soli_sql_drop_table requires SOLI_DB_ADAPTER=postgres|mysql|sqlite".into(),
                );
            }
            crate::db::ddl::assert_user_table(&name)?;
            crate::db::sql::drop_table(&name)?;
            Ok(Value::Bool(true))
        },
    ));
    env.define("__soli_sql_create_table".to_string(), sql_create.clone());
    env.define("__soli_sql_drop_table".to_string(), sql_drop.clone());
    // Column-table schema helpers (create_table with columns, add_column,
    // add_index, execute, …) are registered only on the migration interpreter
    // — see `Interpreter::new_for_migrations`. They must not be callable from
    // request handlers, jobs, or templates.
    // Back-compat aliases from Phase 1.
    env.define("__soli_pg_create_table".to_string(), sql_create);
    env.define("__soli_pg_drop_table".to_string(), sql_drop);

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
    // eui_window(name, handler, view) — a script's own EUI window.
    #[cfg(feature = "eui")]
    crate::serve::eui::script::register_eui_window(env);

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
    flash::register_flash_builtins(env);
    permit::register_permit_builtins(env);

    // Register Job and Cron classes (SolidB-backed background jobs)
    jobs::register_jobs_builtins(env);

    // Register JWT builtins
    jwt::register_jwt_builtins(env);

    // Register VAPID / Web Push builtins
    vapid::register_vapid_builtins(env);

    // Register PDF / Factur-X builtins
    #[cfg(feature = "pdf")]
    pdf::register_pdf_builtins(env);

    // Register test-only builtins (skipped in serve mode)
    if include_test_builtins {
        factories::register_factories(env);
        test_helpers::register_test_helpers(env);
        assertions::register_assertions(env);
        test_dsl::register_test_builtins(env);
        test_server::register_test_server_builtins(env);
        mock_http::register_mock_http_builtins(env);
        // Browser helpers are test-only: `visit`/`click` are ordinary enough
        // names that leaking them into a served app would be a real hazard.
        browser::register_browser_helpers(env);
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
    register_value_object_class(env);
    register_event_builtins(env);

    // Register cache builtins
    cache::register_cache_builtins(env);

    // Register KV builtins
    kv::register_kv_builtins(env);
    #[cfg(feature = "es")]
    es::register_es_builtins(env);
    native::register_native_builtins(env);
    app_links::register_app_links_builtins(env);
    apns::register_apns_builtins(env);
    fcm::register_fcm_builtins(env);
    geo::register_geo_builtins(env);
    push::register_push_builtins(env);
    updater::register_updater_builtins(env);

    // Register rate limit builtins
    rate_limit::register_rate_limit_builtins(env);

    // Spam defences for form posts: spam_fields, pow_field, spam_check,
    // client_ip and the SpamFilter model.
    spam::register_spam_builtins(env);

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
    attachments::register_attachment_builtins(env);
    crate::serve::tus::register(env);
    crate::live::update::register(env);

    // Register clock builtins (sleep, microtime)
    clock::register_clock_builtins(env);

    // Register collection classes (String, Array, Hash, Set, Range, Base64)
    collections::register_collection_classes(env);

    // Register value-type primitive classes (Int, Float, Bool, Null, Decimal, Symbol)
    primitives::register_primitive_classes(env);

    // Register system builtins (System.run, System.run_sync)
    system::register_system_builtins(env);

    // Register spreadsheet builtins (Spreadsheet.csv, Spreadsheet.excel, etc.)
    #[cfg(feature = "office")]
    spreadsheet::register_spreadsheet_builtins(env);

    // Register cookie builtins (set_cookie) — registered last so they win over
    // any test helpers with the same name (e.g. set_cookie in request_helpers).
    session::register_cookie_builtins(env);

    // Signed/encrypted cookie jar reader (read_cookie); the write side rides
    // set_cookie's options ("signed"/"encrypted") above.
    cookie_jar::register_cookie_jar_builtins(env);

    // PASETO v4 tokens (the `Paseto` class: local encrypt/decrypt and public
    // sign/verify, with PASERK-serialized keys). Feature-gated (`paseto`, on
    // by default) so slim builds can drop the pasetors crate.
    #[cfg(feature = "paseto")]
    paseto::register_paseto_builtins(env);
}

/// `ValueObject`: the base class of value objects — compared by value, frozen
/// once constructed, checked against their invariants (see
/// `interpreter::domain`). `to_h` is native; `with` and `from_h` need the
/// engine to run a constructor, so each engine answers them at the call.
fn register_value_object_class(env: &mut Environment) {
    use crate::interpreter::value::{Class, NativeFunction};

    let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    native_methods.insert(
        "to_h".to_string(),
        Rc::new(NativeFunction::new("to_h", Some(0), |args| {
            match args.first() {
                Some(Value::Instance(inst)) => {
                    Ok(crate::interpreter::domain::to_hash(&inst.borrow()))
                }
                _ => Err("to_h must be called on a value object".to_string()),
            }
        })),
    );
    // `Amount(cents: 500, currency: "EUR")` — what `print` shows.
    native_methods.insert(
        "to_s".to_string(),
        Rc::new(NativeFunction::new("to_s", Some(0), |args| {
            match args.first() {
                Some(value @ Value::Instance(_)) => Ok(Value::String(value.to_string().into())),
                _ => Err("to_s must be called on a value object".to_string()),
            }
        })),
    );
    let value_object = Rc::new(Class {
        name: "ValueObject".to_string(),
        native_methods,
        ..Default::default()
    });
    env.define(
        "ValueObject".to_string(),
        Value::Class(value_object.clone()),
    );

    // `class InvoicePaid < DomainEvent`: an event is a value object — built
    // from its fields, immutable — that `publish` hands to its subscribers.
    let domain_event = Class {
        name: crate::interpreter::events::DOMAIN_EVENT.to_string(),
        superclass: Some(value_object),
        ..Default::default()
    };
    env.define(
        crate::interpreter::events::DOMAIN_EVENT.to_string(),
        Value::Class(Rc::new(domain_event)),
    );
}

/// `publish(event)`, `subscribe(EventClass, handler)` and `after_commit`.
/// `after_commit { … }` itself is run by the engines (its block may be one of
/// their closures); this placeholder answers a call without a block.
fn register_event_builtins(env: &mut Environment) {
    use crate::interpreter::value::NativeFunction;
    env.define(
        "publish".to_string(),
        Value::NativeFunction(NativeFunction::new("publish", Some(1), |args| {
            crate::interpreter::events::publish(&args[0]).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        })),
    );
    env.define(
        "subscribe".to_string(),
        Value::NativeFunction(NativeFunction::new("subscribe", Some(2), |args| {
            crate::interpreter::events::subscribe(args)
        })),
    );
    // The job a stream subscription enqueues: appends the event to its ES
    // topic once the transaction that published it has committed.
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    statics.insert(
        "perform".to_string(),
        Rc::new(NativeFunction::new(
            "EventStreamJob.perform",
            Some(1),
            crate::interpreter::events::perform_event_stream,
        )),
    );
    // `EventStream.consume(topic, {"group": …})`: events from an es topic,
    // published to this application's subscribers.
    let mut stream_statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    stream_statics.insert(
        "consume".to_string(),
        Rc::new(NativeFunction::new(
            "EventStream.consume",
            None,
            crate::interpreter::events::consume_stream,
        )),
    );
    env.define(
        "EventStream".to_string(),
        Value::Class(Rc::new(crate::interpreter::value::Class {
            name: "EventStream".to_string(),
            native_static_methods: stream_statics,
            ..Default::default()
        })),
    );
    env.define(
        crate::interpreter::events::EVENT_STREAM_JOB.to_string(),
        Value::Class(Rc::new(crate::interpreter::value::Class {
            name: crate::interpreter::events::EVENT_STREAM_JOB.to_string(),
            native_static_methods: statics,
            ..Default::default()
        })),
    );
    env.define(
        "__rebuild_event".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__rebuild_event",
            None,
            crate::interpreter::events::rebuild_event_native,
        )),
    );
    env.define(
        "__event_version".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__event_version",
            Some(1),
            crate::interpreter::events::event_version_native,
        )),
    );
    env.define(
        "after_commit".to_string(),
        Value::NativeFunction(NativeFunction::new("after_commit", None, |_args| {
            Err("after_commit expects a block: after_commit { … }".to_string())
        })),
    );
}

/// Register the Error class and built-in error types.
///
/// `Error` carries a `message` and optional `details`, set by its native
/// constructor (`new InsufficientFunds("balance too low", {"needed": 30})`) and
/// read by `catch` blocks (`e.message`) and by the request handler when an
/// error goes uncaught. A subclass with its own constructor keeps it, and
/// `super(message)` from there reaches this one.
///
/// `DomainError` is the base for errors a domain rule raises — an outcome of
/// the request, not a bug: uncaught in a served app it answers 422 with its
/// message. `InvariantViolation` is the `DomainError` a value object's broken
/// invariant raises.
fn register_error_classes(env: &mut Environment) {
    use crate::interpreter::value::{Class, NativeFunction, NATIVE_CONSTRUCTOR};

    let mut native_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    native_methods.insert(
        NATIVE_CONSTRUCTOR.to_string(),
        Rc::new(NativeFunction::new("Error", None, |args| {
            let this = match args.first() {
                Some(Value::Instance(inst)) => inst,
                _ => return Err("Error(message, details) must be called on a class".to_string()),
            };
            if args.len() > 3 {
                return Err(format!(
                    "{}(message, details) takes at most 2 arguments, got {}",
                    this.borrow().class.name,
                    args.len() - 1
                ));
            }
            let mut this = this.borrow_mut();
            this.set("message", args.get(1).cloned().unwrap_or(Value::Null));
            this.set("details", args.get(2).cloned().unwrap_or(Value::Null));
            Ok(Value::Null)
        })),
    );
    // "InsufficientFunds: balance too low", or the class name alone.
    native_methods.insert(
        "to_s".to_string(),
        Rc::new(NativeFunction::new("to_s", Some(0), |args| {
            let Some(Value::Instance(inst)) = args.first() else {
                return Err("to_s must be called on an error".to_string());
            };
            let inst = inst.borrow();
            let name = inst.class.name.clone();
            Ok(Value::String(match inst.get("message") {
                Some(Value::Null) | None => name.into(),
                Some(Value::String(message)) => format!("{name}: {message}").into(),
                Some(other) => format!("{name}: {other}").into(),
            }))
        })),
    );

    let error_class = Rc::new(Class {
        name: "Error".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods,
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    });
    env.define("Error".to_string(), Value::Class(error_class.clone()));

    let subclass = |name: &str, superclass: &Rc<Class>| {
        Rc::new(Class {
            name: name.to_string(),
            superclass: Some(superclass.clone()),
            methods: Rc::new(RefCell::new(HashMap::new())),
            static_methods: HashMap::new(),
            native_static_methods: HashMap::new(),
            native_methods: HashMap::new(),
            static_fields: Rc::new(RefCell::new(HashMap::new())),
            fields: HashMap::new(),
            constructor: None,
            nested_classes: Rc::new(RefCell::new(HashMap::new())),
            ..Default::default()
        })
    };

    for name in [
        "ValueError",
        "TypeError",
        "KeyError",
        "IndexError",
        "RuntimeError",
    ] {
        env.define(name.to_string(), Value::Class(subclass(name, &error_class)));
    }
    let domain_error = subclass("DomainError", &error_class);
    env.define(
        "DomainError".to_string(),
        Value::Class(domain_error.clone()),
    );
    let invariant_violation = subclass("InvariantViolation", &domain_error);
    env.define(
        "InvariantViolation".to_string(),
        Value::Class(invariant_violation.clone()),
    );
    // `throw StaleRecord.new("…")` when a locked save was refused: a 409.
    let stale_record = subclass("StaleRecord", &domain_error);
    stale_record
        .static_fields
        .borrow_mut()
        .insert("http_status".to_string(), Value::Int(409));
    env.define("StaleRecord".to_string(), Value::Class(stale_record));
    crate::interpreter::domain::remember_error_classes(domain_error, invariant_violation);
}
