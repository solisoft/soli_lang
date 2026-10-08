//! End-to-end server integration tests.
//!
//! Spawns the `soli serve` binary against `tests/fixtures/_e2e_app/` and
//! exercises HTTP endpoints. Also covers `serve/`, `interpreter/builtins/{server,
//! router, request_helpers, response_helpers}`, and the production-mode VM
//! request path (which doesn't run when only `cargo test` or `soli test`
//! exercise the interpreter).

use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

/// Pick a free local port by binding to :0 and reading what the OS assigned.
/// We immediately drop the listener so the port can be reused; there's a
/// small race window before the server picks it back up.
fn pick_port() -> u16 {
    static FALLBACK: AtomicU16 = AtomicU16::new(28100);
    if let Ok(listener) = TcpListener::bind("127.0.0.1:0") {
        if let Ok(addr) = listener.local_addr() {
            return addr.port();
        }
    }
    FALLBACK.fetch_add(1, Ordering::SeqCst)
}

struct ServerProcess {
    child: Child,
    port: u16,
}

impl ServerProcess {
    /// A server of our own, with one worker and an explicit default locale.
    ///
    /// One worker so both requests in the isolation test land on the same
    /// thread — which is the only arrangement where the leak this guards
    /// against was observable.
    fn start_single_worker(default_locale: &str) -> Self {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_soli"));
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/_e2e_app");
        let port = pick_port();
        let child = Command::new(&binary)
            .arg("serve")
            .arg(&fixture)
            .arg("--port")
            .arg(port.to_string())
            .arg("--workers")
            .arg("1")
            .env("SOLI_SESSION_SECRET", "e2e-test-secret-0123456789abcdef")
            // Strict engines: a handler the VM refuses, or one that fails on
            // the VM and succeeds on the interpreter, stops the server — the
            // fallback would otherwise hide the VM gap, as it hid the model
            // scope lookup until 2.6.3.
            .env("SOLI_FAIL_ON_VM_DEMOTION", "1")
            .env("SOLI_DEFAULT_LOCALE", default_locale)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn soli serve");
        let server = ServerProcess { child, port };
        server.wait_ready();
        server
    }

    /// A server collecting line coverage, as `soli test --coverage` starts
    /// one, with the token its `/__coverage__` dump requires.
    fn start_with_coverage(token: &str) -> Self {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_soli"));
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/_e2e_app");
        let port = pick_port();
        let child = Command::new(&binary)
            .arg("serve")
            .arg(&fixture)
            .arg("--port")
            .arg(port.to_string())
            .arg("--workers")
            .arg("1")
            .env("SOLI_SESSION_SECRET", "e2e-test-secret-0123456789abcdef")
            .env("SOLI_FAIL_ON_VM_DEMOTION", "1")
            .env("SOLI_COVERAGE_ENABLED", "1")
            .env("SOLI_COVERAGE_TOKEN", token)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn soli serve");
        let server = ServerProcess { child, port };
        server.wait_ready();
        server
    }

    /// A server the way `soli test` starts one: production mode, marked as a
    /// test-runner child, and here with the database on a closed port.
    fn start_as_test_runner_child() -> Self {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_soli"));
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/_e2e_app");
        let port = pick_port();
        let child = Command::new(&binary)
            .arg("serve")
            .arg(&fixture)
            .arg("--port")
            .arg(port.to_string())
            .arg("--workers")
            .arg("1")
            .env("SOLI_SESSION_SECRET", "e2e-test-secret-0123456789abcdef")
            .env("SOLI_FAIL_ON_VM_DEMOTION", "1")
            // Only a UUID v4 marks a test-runner child (SEC-084).
            .env(
                "SOLI_INTERNAL_TEST_RUNNER",
                "3f2b8c1e-7d4a-4b6e-9c2d-1a5e8f0b7c3d",
            )
            .env("SOLIDB_HOST", "http://127.0.0.1:1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn soli serve");
        let server = ServerProcess { child, port };
        server.wait_ready();
        server
    }

    fn start() -> Self {
        // CARGO_BIN_EXE_<name> is set by cargo at compile time for integration
        // tests, so use env! (compile-time) — std::env::var (runtime) returns
        // empty under cargo-llvm-cov. The path resolves to the same target/
        // dir cargo built the binary into, instrumented when llvm-cov drives
        // the build.
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_soli"));
        assert!(binary.exists(), "soli binary not found at {:?}", binary);

        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/_e2e_app");
        assert!(fixture.exists(), "missing fixture: {:?}", fixture);

        let port = pick_port();
        let child = Command::new(&binary)
            .arg("serve")
            .arg(&fixture)
            .arg("--port")
            .arg(port.to_string())
            .arg("--workers")
            .arg("2")
            // For the signed/encrypted cookie-jar tests. Harmless to the
            // rest: the session driver stays in_memory.
            .env("SOLI_SESSION_SECRET", "e2e-test-secret-0123456789abcdef")
            // Strict engines: a handler the VM refuses, or one that fails on
            // the VM and succeeds on the interpreter, stops the server — the
            // fallback would otherwise hide the VM gap, as it hid the model
            // scope lookup until 2.6.3.
            .env("SOLI_FAIL_ON_VM_DEMOTION", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn soli serve");

        let server = ServerProcess { child, port };
        server.wait_ready();
        server
    }

    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if ureq::get(&format!("http://127.0.0.1:{}/ping", self.port))
                .timeout(Duration::from_millis(500))
                .call()
                .is_ok()
            {
                return;
            }
            if Instant::now() >= deadline {
                panic!("server on port {} never became ready", self.port);
            }
            thread::sleep(Duration::from_millis(200));
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        // Use SIGTERM (not SIGKILL) so the spawned binary's atexit handlers
        // run — including the LLVM coverage profile flush. Rust's
        // `Child::kill()` sends SIGKILL on Unix, which loses the profile.
        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;
            let _ = kill(Pid::from_raw(self.child.id() as i32), Signal::SIGTERM);
        }
        #[cfg(not(unix))]
        {
            let _ = self.child.kill();
        }
        // Wait briefly for graceful shutdown, then escalate.
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() >= deadline => break,
                _ => thread::sleep(Duration::from_millis(50)),
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One `soli serve` subprocess shared across every `#[test]` in this binary.
///
/// `ServerProcess::start()` is a `solang` boot: parse every controller, warm
/// the VM (pre-compile every handler to bytecode), load templates/locales,
/// bind a port. Per test that is 80-120ms on the small fixture and several
/// seconds for a fatter app — and the cost *regresses* with `cargo test
/// --jobs N` because every parallel boot pays its own cold-start tax
/// (separate processes, separate VM warmup, separate page-cache fill) while
/// fighting for cores. Sharing one server across all tests collapses 20
/// boots into 1 and removes the per-bind port race (`pick_port` had a
/// `drop(listener)` window between picking the port and the child binding
/// it; with one server there is one bind and no window).
///
/// `Drop` still runs at process end when the `OnceLock` is destroyed,
/// preserving the SIGTERM / coverage-flush behavior above.
static SHARED_SERVER: OnceLock<ServerProcess> = OnceLock::new();

fn shared_server() -> &'static ServerProcess {
    SHARED_SERVER.get_or_init(ServerProcess::start)
}

fn body_string(resp: ureq::Response) -> String {
    let mut buf = String::new();
    resp.into_reader().read_to_string(&mut buf).unwrap();
    buf
}

#[test]
fn ping_returns_json() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/ping"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("ping request");
    assert_eq!(resp.status(), 200);
    assert_eq!(body_string(resp), r#"{"pong":true}"#);
}

#[test]
fn render_json_evaluates_its_argument_once() {
    // `render_json(expr)` once evaluated `expr` twice: the interceptor for the
    // `as_json` override evaluated the first argument to test whether it was an
    // instance, then threw the value away and returned None for everything
    // else, so normal dispatch evaluated it again. Harmless for a literal;
    // for `render_json(Post.pluck(...).all)` it issued the database query
    // twice on every request and cost ~42% of the throughput on a 50-row JSON
    // route. The handler counts evaluations of its own argument.
    let server = shared_server();
    let resp = ureq::get(&server.url("/render_json_arg_evals"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("render_json_arg_evals request");
    assert_eq!(resp.status(), 200);
    // The handler's argument increments a counter and returns it, so the body
    // reports how many times it was evaluated: {"n":1} once, {"n":2} twice.
    assert_eq!(
        body_string(resp),
        r#"{"n":1}"#,
        "render_json evaluated its argument more than once"
    );
}

#[test]
fn render_json_sends_what_as_json_returns_on_the_vm() {
    // SEC-013b: `render_json(record)` serializes what the class's `as_json`
    // returns. The tree-walker did; the VM called the native directly and sent
    // every field — the very fields an `as_json` exists to leave out. The
    // fixture's `as_json` drops `token`.
    let server = shared_server();
    let resp = ureq::get(&server.url("/render_json_as_json"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("render_json_as_json request");
    assert_eq!(resp.status(), 200);
    assert_eq!(body_string(resp), r#"{"label":"ok"}"#);
}

/// Model scopes resolve in an action run on the VM: bare, with empty parens,
/// and with an argument, each applied to the query (`to_query`). Before 2.6.3
/// the VM could not look a model scope up; the server re-ran the action on the
/// interpreter, so this answered 200 anyway — the strict flag set on every
/// fixture server makes that re-run a hard failure.
/// A `soli test` server reports the queries each request made. Test servers
/// run in production mode (so suites cover the VM), and the query log was
/// only kept under `--dev`: every response said `query_count: 0`, so
/// `assert_no_n_plus_one` and `--fail-on-n1` could never fail. The count is
/// also per request — a second request does not add the first one's.
#[test]
fn a_test_runner_server_counts_each_requests_queries() {
    let server = ServerProcess::start_as_test_runner_child();
    let count = |path: &str| {
        let resp = ureq::get(&server.url(path))
            .timeout(Duration::from_secs(10))
            .call()
            .expect("request to the test-runner server");
        resp.header("x-soli-test-query-count")
            .map(str::to_string)
            .expect("a test-runner server tags every response with its query count")
    };
    assert_eq!(count("/queries"), "2");
    assert_eq!(count("/queries"), "2", "the log is emptied per request");
    assert_eq!(count("/ping"), "0");

    // The log is on for the runner, but `dev_queries()` keeps its contract —
    // `[]` outside `--dev` — or an app that reads it as "am I under --dev?"
    // turns its dev-only pages on in every spec.
    let body = ureq::get(&server.url("/queries"))
        .timeout(Duration::from_secs(10))
        .call()
        .expect("request to the test-runner server")
        .into_string()
        .expect("body");
    assert_eq!(body, "dev_queries=0");
}

#[test]
fn model_scopes_resolve_in_a_vm_action() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/scopes"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("scopes request (a strict server exits on an engine divergence)");
    assert_eq!(resp.status(), 200);
    let body = body_string(resp);
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(
        lines.first(),
        Some(&"gadget"),
        "static next to scopes: {body}"
    );
    assert!(
        lines[1].contains("doc.owner") && lines[1].contains("doc.size"),
        "bare: {body}"
    );
    assert!(lines[2].contains("doc.owner"), "empty parens: {body}");
    assert!(
        lines[3].contains("doc.kind") && lines[3].contains("lamp"),
        "argument: {body}"
    );
    // A scope body that calls an app helper and reads a constant: it runs in
    // its own closure, not in a copy of the VM's globals.
    assert!(
        lines[4].contains("acme-eu") && lines[4].contains("rust"),
        "scope calling helpers: {body}"
    );
}

/// Lines run on the VM are counted. The VM had no line coverage: once every
/// action ran there (2.6.2), `soli test --coverage` stopped seeing any code an
/// action executed — an app suite covering 97% reported 63%. A coverage
/// server now compiles `CoverLine` markers; the action's statements must show
/// up as hit in the `/__coverage__` dump, as they do on the interpreter.
#[test]
fn vm_actions_record_line_coverage() {
    let token = "e2e-coverage-token-0123456789";
    let server = ServerProcess::start_with_coverage(token);
    let resp = ureq::get(&server.url("/scopes"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("scopes request");
    assert_eq!(resp.status(), 200);

    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/_e2e_app/app/controllers/scopes_controller.sl"),
    )
    .expect("fixture controller");
    let statement_line = source
        .lines()
        .position(|l| l.trim_start().starts_with("args = Gadget.of_kind"))
        .expect("the action's third statement")
        + 1;

    let dump = ureq::get(&server.url("/__coverage__"))
        .set("X-Coverage-Token", token)
        .timeout(Duration::from_secs(3))
        .call()
        .expect("coverage dump");
    let json: serde_json::Value = serde_json::from_str(&body_string(dump)).expect("json");
    let files = json["files"].as_array().expect("files");
    let controller = files
        .iter()
        .find(|f| {
            f["path"]
                .as_str()
                .is_some_and(|p| p.ends_with("scopes_controller.sl"))
        })
        .unwrap_or_else(|| panic!("scopes_controller.sl not in the coverage dump: {json}"));
    let hit_lines: Vec<u64> = controller["hits"]
        .as_array()
        .expect("hits")
        .iter()
        .filter_map(|h| h[0].as_u64())
        .collect();
    assert!(
        hit_lines.contains(&(statement_line as u64)),
        "line {statement_line} not counted; hits: {hit_lines:?}"
    );

    // A static method the action calls (`Gadget.label()`) is compiled from
    // another file: its lines count too. They did not — a service is made of
    // static methods, and grc's services read 0% on the VM.
    let model_source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/_e2e_app/app/models/gadget.sl"),
    )
    .expect("fixture model");
    let static_line = model_source
        .lines()
        .position(|l| l.trim() == "return \"gadget\"")
        .expect("the static method's body")
        + 1;
    let model = files
        .iter()
        .find(|f| f["path"].as_str().is_some_and(|p| p.ends_with("gadget.sl")))
        .unwrap_or_else(|| panic!("gadget.sl not in the coverage dump: {json}"));
    let model_hits: Vec<u64> = model["hits"]
        .as_array()
        .expect("hits")
        .iter()
        .filter_map(|h| h[0].as_u64())
        .collect();
    assert!(
        model_hits.contains(&(static_line as u64)),
        "static method line {static_line} not counted; hits: {model_hits:?}"
    );
}

#[test]
fn add_handles_query_params() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/add?a=12&b=30"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("add request");
    assert_eq!(resp.status(), 200);
    assert_eq!(body_string(resp), "42");
}

#[test]
fn unknown_route_returns_404() {
    let server = shared_server();
    let result = ureq::get(&server.url("/nothing-here"))
        .timeout(Duration::from_secs(3))
        .call();
    match result {
        Err(ureq::Error::Status(code, _)) => assert_eq!(code, 404),
        Ok(resp) => panic!("expected 404, got {}", resp.status()),
        Err(e) => panic!("transport error: {:?}", e),
    }
}

#[test]
fn echo_path_returns_request_path() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/echo"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("echo request");
    assert_eq!(resp.status(), 200);
    assert_eq!(body_string(resp), "/echo");
}

#[test]
fn echo_method_handles_get_post_put_delete() {
    let server = shared_server();
    let url = server.url("/method");

    let resp = ureq::get(&url)
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    assert_eq!(body_string(resp), "GET");

    let resp = ureq::post(&url)
        .timeout(Duration::from_secs(3))
        .send_string("")
        .unwrap();
    assert_eq!(body_string(resp), "POST");

    let resp = ureq::put(&url)
        .timeout(Duration::from_secs(3))
        .send_string("")
        .unwrap();
    assert_eq!(body_string(resp), "PUT");

    let resp = ureq::delete(&url)
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    assert_eq!(body_string(resp), "DELETE");
}

#[test]
fn echo_header_returns_request_header() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/header?name=x-test"))
        .timeout(Duration::from_secs(3))
        .set("X-Test", "soli-rocks")
        .call()
        .expect("header request");
    assert_eq!(resp.status(), 200);
    assert_eq!(body_string(resp), "soli-rocks");
}

#[test]
fn json_body_round_trip() {
    let server = shared_server();
    let resp = ureq::post(&server.url("/json"))
        .timeout(Duration::from_secs(3))
        .set("Content-Type", "application/json")
        .send_string(r#"{"name":"alice","age":30}"#)
        .expect("json request");
    assert_eq!(resp.status(), 200);
    let body = body_string(resp);
    assert!(body.contains("\"got_name\":\"alice\""), "body: {}", body);
    assert!(body.contains("\"got_age\":30"), "body: {}", body);
}

#[test]
fn redirect_returns_3xx_with_location() {
    let server = shared_server();
    // ureq follows redirects by default; disable so we can inspect.
    let agent = ureq::AgentBuilder::new().redirects(0).build();
    let resp = agent
        .get(&server.url("/redirect"))
        .timeout(Duration::from_secs(3))
        .call();
    match resp {
        Ok(r) => {
            assert!(
                (300..400).contains(&r.status()),
                "expected 3xx, got {}",
                r.status()
            );
            assert_eq!(r.header("location").unwrap_or(""), "/ping");
        }
        Err(ureq::Error::Status(code, r)) => {
            assert!((300..400).contains(&code));
            assert_eq!(r.header("location").unwrap_or(""), "/ping");
        }
        Err(e) => panic!("transport error: {:?}", e),
    }
}

#[test]
fn explicit_500_propagates() {
    let server = shared_server();
    let result = ureq::get(&server.url("/oops"))
        .timeout(Duration::from_secs(3))
        .call();
    match result {
        Err(ureq::Error::Status(code, r)) => {
            assert_eq!(code, 500);
            assert_eq!(body_string(r), "boom");
        }
        Ok(r) => panic!("expected 500, got {}", r.status()),
        Err(e) => panic!("transport error: {:?}", e),
    }
}

#[test]
fn array_ops_in_handler_exercise_vm() {
    // Exercises array.map + array.reduce in the production-mode VM, not
    // just the interpreter — soli serve compiles handlers to bytecode.
    let server = shared_server();
    let resp = ureq::get(&server.url("/array"))
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    assert_eq!(resp.status(), 200);
    // [1,2,3,4,5].map(*2).reduce(+) = 2+4+6+8+10 = 30
    assert_eq!(body_string(resp), "30");
}

#[test]
fn string_ops_in_handler_exercise_vm() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/string?name=alice"))
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(body_string(resp), "HELLO, ALICE!");
}

#[test]
fn pipeline_in_handler() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/pipeline"))
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    // [1,2,3,4,5].filter(>1).map(*n).reduce(+) = 4 + 9 + 16 + 25 = 54
    assert_eq!(body_string(resp), "54");
}

#[test]
fn hash_methods_in_handler() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/hash"))
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    assert_eq!(body_string(resp), "a,b,c");
}

#[test]
fn for_loop_in_handler() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/for"))
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    assert_eq!(body_string(resp), "60");
}

#[test]
fn while_loop_in_handler() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/while"))
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    // 0+1+2+3+4 = 10
    assert_eq!(body_string(resp), "10");
}

#[test]
fn closure_in_handler() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/closure"))
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    assert_eq!(body_string(resp), "12");
}

#[test]
fn named_route_helpers_resolve_through_running_server() {
    // End-to-end verification of `*_path` / `*_url` registered through
    // `resources("posts")` and a `name: "about"` one-off in routes.sl. We hit
    // a probe action that calls each helper and returns the resolved strings;
    // failing the assertion means the registration path
    // (router_resource_enter → register_route_with_name → rebuild_named_routes
    // → register_named_route_helpers) is broken end-to-end.
    let server = shared_server();
    let resp = ureq::get(&server.url("/named_routes"))
        .timeout(Duration::from_secs(3))
        .set("Host", "test.example.com")
        .call()
        .expect("named-routes probe request");
    assert_eq!(resp.status(), 200);
    let body = body_string(resp);
    let parsed: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("invalid JSON {:?}: {}", body, e));

    // resources("posts") — collection + member + edit/new variants.
    assert_eq!(parsed["posts_path"], "/posts");
    assert_eq!(parsed["new_post_path"], "/posts/new");
    assert_eq!(parsed["post_path"], "/posts/1");
    assert_eq!(parsed["edit_post_path"], "/posts/1/edit");

    // `name:` one-off route.
    assert_eq!(parsed["about_path"], "/about");

    // *_url variants pull the scheme + host from the live request — Host
    // header we sent above plus http (no TLS / no X-Forwarded-Proto).
    assert_eq!(parsed["posts_url"], "http://test.example.com/posts");
    assert_eq!(parsed["post_url"], "http://test.example.com/posts/1");
    assert_eq!(parsed["about_url"], "http://test.example.com/about");
}

#[test]
fn server_handles_concurrent_requests() {
    let server = shared_server();
    let url = server.url("/ping");
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let url = url.clone();
            thread::spawn(move || {
                let resp = ureq::get(&url)
                    .timeout(Duration::from_secs(3))
                    .call()
                    .expect("concurrent ping");
                assert_eq!(resp.status(), 200);
                body_string(resp)
            })
        })
        .collect();
    for h in handles {
        let body = h.join().expect("thread join");
        assert_eq!(body, r#"{"pong":true}"#);
    }
}

#[test]
fn cookies_global_returns_empty_hash_when_no_cookie_header() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/cookies"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("cookies request");
    assert_eq!(resp.status(), 200);
    let body = body_string(resp);
    assert_eq!(body, "{}", "expected empty cookies hash, got: {}", body);
}

#[test]
fn cookies_global_parses_cookie_header() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/cookies"))
        .timeout(Duration::from_secs(3))
        .set("Cookie", "foo=bar; session_id=abc123")
        .call()
        .expect("cookies request");
    assert_eq!(resp.status(), 200);
    let body = body_string(resp);
    let parsed: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("invalid JSON {:?}: {}", body, e));
    assert_eq!(parsed["foo"], "bar");
    assert_eq!(parsed["session_id"], "abc123");
}

#[test]
fn set_cookie_emits_set_cookie_header() {
    let server = shared_server();
    let resp = ureq::get(&server.url("/set_cookie?name=my_cookie&value=my_value"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("set_cookie request");
    assert_eq!(resp.status(), 200);
    let set_cookie = resp.header("set-cookie").unwrap_or("");
    assert!(
        set_cookie.contains("my_cookie=my_value"),
        "expected Set-Cookie with my_cookie=my_value, got: {}",
        set_cookie
    );
}

/// GET /jar/write and return its two sealed Set-Cookie header values
/// (jar_enc, jar_sig) plus the response body.
fn jar_write(server: &ServerProcess) -> (String, String, String) {
    let resp = ureq::get(&server.url("/jar/write"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("jar write request");
    assert_eq!(resp.status(), 200);
    let cookies: Vec<String> = resp
        .all("set-cookie")
        .iter()
        .map(|s| s.to_string())
        .collect();
    let find = |name: &str| {
        cookies
            .iter()
            .find(|c| c.starts_with(&format!("{}=", name)))
            .unwrap_or_else(|| panic!("no {} in Set-Cookie headers: {:?}", name, cookies))
            .clone()
    };
    let enc = find("jar_enc");
    let sig = find("jar_sig");
    (enc, sig, body_string(resp))
}

#[test]
fn cookie_jar_seals_values_and_reads_back_same_request() {
    let server = shared_server();
    let (enc, sig, body) = jar_write(server);

    // Sealed formats on the wire; the encrypted value leaks no plaintext and
    // the cookie attributes survive alongside the sealing options.
    assert!(
        enc.starts_with("jar_enc=enc.v1."),
        "expected sealed encrypted value, got: {}",
        enc
    );
    assert!(
        !enc.contains("dark"),
        "encrypted cookie must not leak plaintext: {}",
        enc
    );
    assert!(
        enc.contains("Max-Age=3600") && enc.contains("HttpOnly"),
        "cookie attributes must survive sealing: {}",
        enc
    );
    assert!(
        sig.starts_with("jar_sig=sig.v1."),
        "expected signed value, got: {}",
        sig
    );

    // Same-request read-your-write returned the decoded values.
    let parsed: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("invalid JSON {:?}: {}", body, e));
    assert_eq!(parsed["enc"]["theme"], "dark");
    assert_eq!(parsed["enc"]["count"], 42);
    assert_eq!(parsed["sig"], 42);
}

#[test]
fn cookie_jar_round_trips_across_requests() {
    let server = shared_server();
    let (enc, sig, _) = jar_write(server);
    let pair = |header: &str| header.split(';').next().unwrap().to_string();
    let cookie_header = format!("{}; {}", pair(&enc), pair(&sig));

    let resp = ureq::get(&server.url("/jar/read"))
        .timeout(Duration::from_secs(3))
        .set("Cookie", &cookie_header)
        .call()
        .expect("jar read request");
    let body = body_string(resp);
    let parsed: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("invalid JSON {:?}: {}", body, e));
    assert_eq!(parsed["enc"]["theme"], "dark");
    assert_eq!(parsed["enc"]["count"], 42);
    assert_eq!(parsed["sig"], 42);

    // The signed cookie's payload segment is plain base64url JSON — readable
    // without the key (that's the signed/encrypted distinction).
    let sig_value = pair(&sig);
    let payload_b64 = sig_value
        .trim_start_matches("jar_sig=sig.v1.")
        .split('.')
        .next()
        .unwrap();
    use base64::Engine as _;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .expect("signed payload decodes");
    let payload: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(payload["val"], 42);
}

#[test]
fn cookie_jar_rejects_tampered_and_forged_values() {
    let server = shared_server();
    let (enc, _, _) = jar_write(server);
    let enc_value = enc
        .split(';')
        .next()
        .unwrap()
        .trim_start_matches("jar_enc=");

    // Flip a character in the ciphertext body.
    let mut bytes = enc_value.as_bytes().to_vec();
    let mid = bytes.len() / 2;
    bytes[mid] = if bytes[mid] == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(bytes).unwrap();

    // A tampered sealed value and a bare attacker-set value (the forgery
    // case: plain "42" where a *verified* 42 is expected) both read as null.
    let resp = ureq::get(&server.url("/jar/read"))
        .timeout(Duration::from_secs(3))
        .set("Cookie", &format!("jar_enc={}; jar_sig=42", tampered))
        .call()
        .expect("jar read request");
    let body = body_string(resp);
    let parsed: serde_json::Value =
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("invalid JSON {:?}: {}", body, e));
    assert_eq!(parsed["enc"], serde_json::Value::Null, "body: {}", body);
    assert_eq!(parsed["sig"], serde_json::Value::Null, "body: {}", body);
}

#[test]
fn websocket_upgrade_completes_and_round_trips() {
    let server = shared_server();
    let url = format!("ws://127.0.0.1:{}/ws/echo", server.port);
    let (mut socket, response) = tungstenite::connect(&url)
        .expect("WebSocket handshake must complete (101 + h1 protocol upgrade)");
    assert_eq!(response.status().as_u16(), 101);

    // Regression guard for the h1/h2c auto-detect change: plain
    // `serve_connection` still emits the 101 (so `connect` above succeeds)
    // but never performs the protocol upgrade — frames sent afterwards go
    // nowhere, the server logs "Handshake not finished", and no echo ever
    // comes back. Only `serve_connection_with_upgrades` arms the h1 upgrade
    // path. Bound the read so the broken case fails in seconds instead of
    // hanging the suite.
    if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref() {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
    }

    socket
        .send(tungstenite::Message::Text("hello".into()))
        .expect("send text frame");
    let reply = socket
        .read()
        .expect("server must deliver the echo frame after the upgrade");
    assert_eq!(reply.to_text().unwrap(), "echo:hello");
}

/// `--port 0` must report the port the OS actually assigned.
///
/// The bind loop used to echo back the *requested* port, which is fine for a
/// fixed port but reports `0` for an ephemeral one — leaving the bound port
/// undiscoverable both in the startup banner and to any embedding caller (a
/// desktop shell has to know where to point a browser). Regression test for
/// reading `listener.local_addr()` instead.
#[test]
fn ephemeral_port_zero_reports_the_real_bound_port() {
    use std::io::{BufRead, BufReader};
    use std::sync::mpsc;

    let binary = PathBuf::from(env!("CARGO_BIN_EXE_soli"));
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/_e2e_app");

    let mut child = Command::new(&binary)
        .arg("serve")
        .arg(&fixture)
        .arg("--port")
        .arg("0")
        .arg("--workers")
        .arg("1")
        .env("SOLI_HOST", "127.0.0.1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn soli serve --port 0");

    // Read the banner on a worker thread — the child keeps running, so a
    // blocking read on the main thread would never return.
    let stdout = child.stdout.take().expect("piped stdout");
    let (tx, rx) = mpsc::channel::<u16>();
    // Keep draining after the port is found: returning early would drop the
    // reader, close the pipe, and hand the server a SIGPIPE on its next
    // stdout write — killing the very process we're about to probe.
    thread::spawn(move || {
        let mut sent = false;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if sent {
                continue;
            }
            if let Some(rest) = line.split("listening on http://").nth(1) {
                if let Some(port) = rest.rsplit(':').next() {
                    if let Ok(port) = port.trim().parse::<u16>() {
                        let _ = tx.send(port);
                        sent = true;
                    }
                }
            }
        }
    });

    let port = rx.recv_timeout(Duration::from_secs(20));

    // Tear the child down before asserting, so a failure doesn't leak a server.
    // The banner prints once the listener is bound, but workers finish booting
    // slightly later — poll rather than firing a single request, same as
    // `ServerProcess::wait_ready`.
    let reported = port.map(|p| {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut ok = false;
        while Instant::now() < deadline {
            if ureq::get(&format!("http://127.0.0.1:{}/ping", p))
                .timeout(Duration::from_millis(500))
                .call()
                .is_ok()
            {
                ok = true;
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
        (p, ok)
    });
    let _ = child.kill();
    let _ = child.wait();

    let (port, reachable) = reported.expect("server never announced a port within 20s");
    assert_ne!(port, 0, "--port 0 reported 0 instead of the assigned port");
    assert!(
        reachable,
        "announced port {} did not accept a request — the reported port is wrong",
        port
    );
}

/// A request that never calls `set_locale` must not inherit the locale of
/// the request before it on that worker.
///
/// `CURRENT_LOCALE` is a thread-local and workers are reused, and nothing in
/// the request path reset it: one visitor asking for French left the next
/// visitor's page in French — and `Model#save` writing its translated fields
/// into the French slot. One worker, so both requests are served by the same
/// thread and the leak, if it were back, would be certain rather than likely.
#[test]
fn locale_does_not_leak_between_requests() {
    let server = ServerProcess::start_single_worker("de");

    // The default is what the operator configured, not a hard-coded "en".
    let first = body_string(ureq::get(&server.url("/locale/read")).call().unwrap());
    assert!(
        first.contains("\"de\""),
        "a fresh request starts at the configured default, got {first}"
    );

    // A request that does set it sees its own choice.
    let set = body_string(ureq::get(&server.url("/locale/set")).call().unwrap());
    assert!(set.contains("\"fr\""), "set_locale must apply, got {set}");

    // The next request on that same worker must not see French.
    for _ in 0..5 {
        let after = body_string(ureq::get(&server.url("/locale/read")).call().unwrap());
        assert!(
            after.contains("\"de\""),
            "locale leaked from the previous request, got {after}"
        );
    }
}

/// `Accept-Language` picks among the locales the application actually ships.
/// The fixture app has no locale files, so there is nothing to negotiate to
/// and the default stands — which is the safe half of the behaviour and the
/// half a regression would break first.
#[test]
fn accept_language_never_negotiates_to_a_locale_we_lack() {
    let server = ServerProcess::start_single_worker("de");
    let body = body_string(
        ureq::get(&server.url("/locale/read"))
            .set("Accept-Language", "fr-CA,fr;q=0.9,ja;q=0.5")
            .call()
            .unwrap(),
    );
    assert!(
        body.contains("\"de\""),
        "no locale files, so nothing to negotiate to, got {body}"
    );
}

/// Round-trip arbitrary binary through a multipart upload and back out.
///
/// This covers both halves of the upload-memory work:
///
/// * ingest — the body is now carried as `Bytes` and moved into the multipart
///   parser instead of being copied three times, and the raw body is no longer
///   retained in `RequestData`. `file["data"]` must still be the same base64
///   string it always was; that contract was deliberately left alone.
/// * egress — `AttachmentsController#show` now answers with `body_base64`
///   rather than `Base64.decode(...)`, which built a Soli array of one 16-byte
///   `Value::Int` per byte. The fixture handler uses the same shape, so a
///   byte-identical round trip is what catches a mistake in that swap.
///
/// The payload is deliberately **not** valid UTF-8: that is the case where the
/// old decode path produced the `Value::Int` array rather than a string.
#[test]
fn a_multipart_upload_round_trips_byte_for_byte() {
    let server = shared_server();

    // Invalid UTF-8 (lone surrogates, NULs, 0xFF) plus a CRLF and a run that
    // looks like a boundary prefix, so a naive parser would truncate.
    let mut payload: Vec<u8> = vec![0xff, 0xfe, 0x00, 0x42, 0x00, 0x0d, 0x0a, 0x2d, 0x2d];
    payload.extend((0u16..512).map(|i| (i % 256) as u8));

    let boundary = "----soli-e2e-upload";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"evidence\"; filename=\"proof.bin\"\r\n",
    );
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(&payload);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let resp = ureq::post(&server.url("/upload_echo"))
        .set(
            "Content-Type",
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .timeout(Duration::from_secs(5))
        .send_bytes(&body)
        .expect("upload request");

    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.header("X-Upload-Size"),
        Some(payload.len().to_string().as_str()),
        "`size` must stay the raw byte count, not the base64 length"
    );
    assert_eq!(resp.header("X-Upload-Name"), Some("proof.bin"));

    let mut echoed = Vec::new();
    resp.into_reader()
        .read_to_end(&mut echoed)
        .expect("read echoed body");
    assert_eq!(
        echoed, payload,
        "the uploaded bytes must survive the multipart parse and the \
         base64 response path unchanged"
    );
}

/// A stand-in for SoliDB's blob endpoint, honouring one `Range` the way
/// `GET /_api/blob/{db}/{collection}/{key}` does: `206` + `Content-Range`,
/// `416` when unsatisfiable, the whole blob otherwise. Records each request
/// head so a test can see what the server forwarded.
struct StandInSolidb {
    port: u16,
    heads: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

fn stand_in_solidb(data: std::sync::Arc<Vec<u8>>) -> StandInSolidb {
    use std::io::Write;
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stand-in");
    let port = listener.local_addr().unwrap().port();
    let heads = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = heads.clone();
    thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let (data, seen) = (data.clone(), seen.clone());
            thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if conn.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head).to_string();
                seen.lock().unwrap().push(head.clone());
                let range = head.lines().find_map(|l| {
                    let (name, value) = l.split_once(':')?;
                    name.eq_ignore_ascii_case("range")
                        .then(|| value.trim().to_string())
                });
                let len = data.len();
                let parsed = range.as_deref().and_then(|r| {
                    solilang::serve::server_constants::parse_range_header(r, len as u64)
                });
                let (status, extra, body): (&str, String, &[u8]) = match (parsed, &range) {
                    (Some((a, b)), _) => (
                        "206 Partial Content",
                        format!("Content-Range: bytes {a}-{b}/{len}\r\n"),
                        &data[a as usize..=b as usize],
                    ),
                    (None, Some(_)) => (
                        "416 Range Not Satisfiable",
                        format!("Content-Range: bytes */{len}\r\n"),
                        b"",
                    ),
                    (None, None) => ("200 OK", String::new(), &data[..]),
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\n\
                     Accept-Ranges: bytes\r\n{extra}Content-Length: {}\r\n\
                     Connection: close\r\n\r\n",
                    body.len()
                );
                if conn.write_all(reply.as_bytes()).is_err() {
                    return;
                }
                for chunk in body.chunks(64 * 1024) {
                    if conn.write_all(chunk).is_err() {
                        return;
                    }
                }
            });
        }
    });
    StandInSolidb { port, heads }
}

fn body_bytes(resp: ureq::Response) -> Vec<u8> {
    let mut buf = Vec::new();
    resp.into_reader().read_to_end(&mut buf).unwrap();
    buf
}

/// `solidb_blob_response` streams a multi-megabyte blob from SoliDB to the
/// client: whole, by `Range` (forwarded as is), headers-only for `HEAD`, and
/// `416` / `304` where they belong — with the application's representation
/// headers on every answer and its internal marker on none.
#[test]
fn a_solidb_blob_streams_whole_and_by_range() {
    let server = shared_server();
    let data: std::sync::Arc<Vec<u8>> = std::sync::Arc::new(
        (0..5 * 1024 * 1024 + 321)
            .map(|i| (i % 251) as u8)
            .collect(),
    );
    let solidb = stand_in_solidb(data.clone());
    let url = server.url(&format!(
        "/blob_stream?upstream=127.0.0.1:{}&key=ep1",
        solidb.port
    ));
    let total = data.len();

    let whole = ureq::get(&url)
        .timeout(Duration::from_secs(20))
        .call()
        .expect("whole blob");
    assert_eq!(whole.status(), 200);
    assert_eq!(
        whole.header("Content-Length"),
        Some(total.to_string().as_str())
    );
    assert_eq!(whole.header("Accept-Ranges"), Some("bytes"));
    assert_eq!(whole.header("Content-Type"), Some("audio/mpeg"));
    assert_eq!(whole.header("Content-Disposition"), Some("inline"));
    assert_eq!(whole.header("X-Content-Type-Options"), Some("nosniff"));
    assert_eq!(whole.header("ETag"), Some("\"ep1\""));
    assert_eq!(
        whole.header("X-Soli-Blob-Stream"),
        None,
        "internal marker leaked"
    );
    assert!(body_bytes(whole) == *data, "the whole blob, byte for byte");

    let (start, end) = (1024 * 1024 + 5, 3 * 1024 * 1024);
    let part = ureq::get(&url)
        .set("Range", &format!("bytes={start}-{end}"))
        .timeout(Duration::from_secs(20))
        .call()
        .expect("ranged blob");
    assert_eq!(part.status(), 206);
    assert_eq!(
        part.header("Content-Range"),
        Some(format!("bytes {start}-{end}/{total}").as_str())
    );
    assert_eq!(
        part.header("Content-Length"),
        Some((end - start + 1).to_string().as_str())
    );
    assert!(
        body_bytes(part) == data[start..=end],
        "exactly the requested span"
    );

    let head = ureq::head(&url)
        .timeout(Duration::from_secs(10))
        .call()
        .expect("HEAD");
    assert_eq!(head.status(), 200);
    assert_eq!(
        head.header("Content-Length"),
        Some(total.to_string().as_str())
    );
    assert_eq!(head.header("Accept-Ranges"), Some("bytes"));

    match ureq::get(&url)
        .set("Range", &format!("bytes={}-", total + 10))
        .timeout(Duration::from_secs(10))
        .call()
    {
        Err(ureq::Error::Status(416, resp)) => {
            assert_eq!(
                resp.header("Content-Range"),
                Some(format!("bytes */{total}").as_str())
            );
        }
        other => panic!("expected a 416, got {other:?}"),
    }

    let cached = ureq::get(&url)
        .set("If-None-Match", "\"ep1\"")
        .timeout(Duration::from_secs(10))
        .call()
        .expect("conditional GET");
    assert_eq!(cached.status(), 304);

    // Several ranges are not SoliDB's to serve: the client gets it all.
    let multi = ureq::get(&url)
        .set("Range", "bytes=0-1,5-6")
        .timeout(Duration::from_secs(20))
        .call()
        .expect("multi-range");
    assert_eq!(multi.status(), 200);
    assert_eq!(body_bytes(multi).len(), total);

    // What reached SoliDB: the instance's credentials on every request, the
    // client's Range when it was one SoliDB serves, `bytes=0-0` for the HEAD
    // probe, and nothing for the 304, which never left the server.
    let heads = solidb.heads.lock().unwrap().clone();
    assert_eq!(heads.len(), 5, "{heads:#?}");
    let basic = "Basic c3RyZWFtZXI6czNjcmV0"; // streamer:s3cret
    for head in &heads {
        assert!(head.contains(basic), "{head}");
    }
    let lower: Vec<String> = heads.iter().map(|h| h.to_ascii_lowercase()).collect();
    assert!(!lower[0].contains("range:"));
    assert!(lower[1].contains(&format!("range: bytes={start}-{end}")));
    assert!(lower[2].contains("range: bytes=0-0"));
    assert!(lower[4].contains("/_api/blob/e2e/media/ep1"));
    assert!(!lower[4].contains("range:"));
}

/// `/_metrics` answers a loopback scraper with the Prometheus text format, and
/// counts the requests served. It replaces `tests/metrics_spec.sl`, which
/// needed a server already running on :3000 and so only ever skipped.
#[test]
fn metrics_endpoint_serves_prometheus_text_to_loopback() {
    let server = shared_server();
    ureq::get(&server.url("/ping"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("ping request");

    let resp = ureq::get(&server.url("/_metrics"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("metrics request");
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.header("Content-Type"),
        Some("text/plain; charset=utf-8")
    );
    let body = body_string(resp);
    assert!(
        body.contains("# TYPE soli_http_requests_total counter"),
        "body was: {body}"
    );
    let requests = body
        .lines()
        .find_map(|line| line.strip_prefix("soli_http_requests_total "))
        .and_then(|count| count.trim().parse::<u64>().ok())
        .unwrap_or_else(|| panic!("no soli_http_requests_total sample in: {body}"));
    assert!(requests >= 1, "the /ping above was not counted: {body}");
}
