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

    /// A one-worker server for another fixture app under `tests/fixtures/`.
    fn start_fixture(name: &str) -> Self {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_soli"));
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let port = pick_port();
        let child = Command::new(&binary)
            .arg("serve")
            .arg(&fixture)
            .arg("--port")
            .arg(port.to_string())
            .arg("--workers")
            .arg("1")
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

/// An app written into `root` and served in production with one worker and
/// `env`; its stderr goes to `root/stderr.log`. The app answers `GET /ping`.
fn serve_app_in(
    root: &std::path::Path,
    files: &[(&str, &str)],
    env: &[(&str, &str)],
) -> ServerProcess {
    serve_app_with_args(root, files, env, &[])
}

/// [`serve_app_in`], with more `soli serve` arguments (`--dev`).
fn serve_app_with_args(
    root: &std::path::Path,
    files: &[(&str, &str)],
    env: &[(&str, &str)],
    args: &[&str],
) -> ServerProcess {
    for (path, body) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent dir")).expect("mkdir");
        std::fs::write(path, body).expect("write app file");
    }
    let port = pick_port();
    let mut command = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_soli")));
    command
        .arg("serve")
        .arg(root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        .args(args)
        // `/_metrics` answers loopback without a token; the engine flags are
        // the test's to set.
        .env_remove("SOLI_METRICS_TOKEN")
        .env_remove("SOLI_FAIL_ON_VM_DEMOTION")
        .env_remove("SOLI_ENGINE_LOG")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(root.join("stderr.log")).expect("stderr file"));
    for (name, value) in env {
        command.env(name, value);
    }
    let server = ServerProcess {
        child: command.spawn().expect("spawn soli serve"),
        port,
    };
    server.wait_ready();
    server
}

/// The status of `GET path`, with one optional request header.
fn status_of(server: &ServerProcess, path: &str, header: Option<(&str, &str)>) -> u16 {
    let mut request = ureq::get(&server.url(path)).timeout(Duration::from_secs(10));
    if let Some((name, value)) = header {
        request = request.set(name, value);
    }
    match request.call() {
        Ok(resp) => resp.status(),
        Err(ureq::Error::Status(status, _)) => status,
        Err(e) => panic!("GET {path}: {e}"),
    }
}

/// One counter from `/_metrics`.
fn metric(server: &ServerProcess, name: &str) -> u64 {
    let body = body_string(
        ureq::get(&server.url("/_metrics"))
            .timeout(Duration::from_secs(3))
            .call()
            .expect("metrics request"),
    );
    body.lines()
        .find_map(|line| line.strip_prefix(&format!("{name} ")))
        .and_then(|count| count.trim().parse().ok())
        .unwrap_or_else(|| panic!("no {name} sample in: {body}"))
}

const HOME_CONTROLLER_WITH_PING: &str = "class HomeController < Controller
  # GET /ping
  def ping(req)
    {\"status\": 200, \"body\": \"pong\"}
  end

  # GET /work
  def work(req)
    throw \"bad input\" if params[\"fail\"] == \"1\"

    {\"status\": 200, \"body\": \"work ok\"}
  end

  # GET /nilerr
  def nilerr(req)
    {\"status\": 200, \"body\": \"#{params[\"q\"].length}\"}
  end
end
";

/// An error both engines raise is the app's own — a `throw`, a nil read from
/// a missing parameter — and leaves the handler on the VM. One such request
/// used to move the action, hook or middleware to the tree-walker for the rest
/// of the worker's life (242 → 767 µs a request on a compute-heavy action), and
/// any client could send it. Strict engines: a refusal or a divergence would
/// stop this server.
#[test]
fn app_errors_leave_handlers_on_the_vm() {
    let app = tempfile::tempdir().expect("temp app dir");
    let root = app.path();
    let server = serve_app_in(
        root,
        &[
            (
                "config/routes.sl",
                "get(\"/ping\", \"home#ping\")
get(\"/work\", \"home#work\")
get(\"/nilerr\", \"home#nilerr\")
get(\"/hooked\", \"hooked#index\")
get(\"/fnwork\", \"fnstyle#work\")
middleware(\"check\", -> {
  get(\"/guarded\", \"home#ping\")
})
",
            ),
            (
                "app/controllers/home_controller.sl",
                HOME_CONTROLLER_WITH_PING,
            ),
            (
                "app/controllers/hooked_controller.sl",
                "class HookedController < Controller
  static {
    this.before_action = fn(req) {
      throw \"hook refused\" if params[\"fail\"] == \"1\"
      req
    }
  }

  # GET /hooked
  def index(req)
    {\"status\": 200, \"body\": \"hooked\"}
  end
end
",
            ),
            (
                "app/controllers/fnstyle_controller.sl",
                "# GET /fnwork
def work(req)
  throw \"bad input\" if params[\"fail\"] == \"1\"

  {\"status\": 200, \"body\": \"fn ok\"}
end
",
            ),
            (
                "app/middleware/check.sl",
                "# order: 50
# scope_only: true
def check(req)
  throw \"refused\" if req[\"headers\"][\"x-fail\"] == \"1\"

  req
end
",
            ),
        ],
        &[("SOLI_FAIL_ON_VM_DEMOTION", "1"), ("SOLI_ENGINE_LOG", "1")],
    );

    // Each fails on both engines, then passes; twice for the action.
    for _ in 0..2 {
        assert_eq!(status_of(&server, "/work?fail=1", None), 500);
    }
    assert_eq!(status_of(&server, "/work", None), 200);
    assert_eq!(status_of(&server, "/nilerr", None), 500);
    assert_eq!(status_of(&server, "/nilerr?q=abc", None), 200);
    assert_eq!(status_of(&server, "/hooked?fail=1", None), 500);
    assert_eq!(status_of(&server, "/hooked", None), 200);
    assert_eq!(status_of(&server, "/guarded", Some(("x-fail", "1"))), 500);
    assert_eq!(status_of(&server, "/guarded", None), 200);
    assert_eq!(status_of(&server, "/fnwork?fail=1", None), 500);
    assert_eq!(status_of(&server, "/fnwork", None), 200);

    assert_eq!(metric(&server, "soli_vm_handler_demotions_total"), 0);
    assert_eq!(metric(&server, "soli_vm_engine_divergences_total"), 0);
    drop(server);
    let log = std::fs::read_to_string(root.join("stderr.log")).expect("server stderr");
    assert!(!log.contains("demoted to the interpreter"), "{log}");
}

/// A failure the tree-walker does not reproduce is the VM's — a gap, or a
/// transient error the re-run did not hit — and still moves the handler off
/// the VM. Here an action and its `before_action` fail on their first run
/// only (both engines share the hash that counts the runs): the re-run answers,
/// each is demoted once, and the next request runs on the tree-walker alone.
#[test]
fn a_handler_only_the_vm_fails_is_demoted_once() {
    let app = tempfile::tempdir().expect("temp app dir");
    let root = app.path();
    let server = serve_app_in(
        root,
        &[
            (
                "config/routes.sl",
                "get(\"/ping\", \"home#ping\")\nget(\"/flaky\", \"flaky#show\")\n",
            ),
            (
                "app/controllers/home_controller.sl",
                HOME_CONTROLLER_WITH_PING,
            ),
            (
                "app/controllers/flaky_controller.sl",
                "flaky_runs = {\"action\": 0, \"hook\": 0}

class FlakyController < Controller
  static {
    this.before_action = fn(req) {
      flaky_runs[\"hook\"] = flaky_runs[\"hook\"] + 1
      throw \"the hook's first run fails\" if flaky_runs[\"hook\"] == 1
      req
    }
  }

  # GET /flaky
  def show(req)
    flaky_runs[\"action\"] = flaky_runs[\"action\"] + 1
    throw \"the action's first run fails\" if flaky_runs[\"action\"] == 1

    {\"status\": 200, \"body\": \"runs #{flaky_runs[\"action\"]}\"}
  end
end
",
            ),
        ],
        &[("SOLI_ENGINE_LOG", "1")],
    );

    let get = |path: &str| {
        body_string(
            ureq::get(&server.url(path))
                .timeout(Duration::from_secs(10))
                .call()
                .expect("the re-run answers"),
        )
    };
    // The VM's run fails; the re-run is the action's second.
    assert_eq!(get("/flaky"), "runs 2");
    // On the tree-walker alone now: one run.
    assert_eq!(get("/flaky"), "runs 3");
    assert_eq!(metric(&server, "soli_vm_handler_demotions_total"), 2);
    assert_eq!(metric(&server, "soli_vm_engine_divergences_total"), 2);
    drop(server);
    let log = std::fs::read_to_string(root.join("stderr.log")).expect("server stderr");
    for handler in ["FlakyController#before_action:5", "FlakyController#show"] {
        assert_eq!(
            log.matches(&format!("handler '{handler}' demoted to the interpreter"))
                .count(),
            1,
            "{log}"
        );
        assert!(
            log.contains(&format!(
                "handler '{handler}' failed on the VM and succeeded on the interpreter"
            )),
            "{log}"
        );
    }
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

/// A page rendered from objects is rendered again for each request. The
/// response cache keyed a render on a signature that hashed an object by the
/// length of its class name, so `@cards = [Card.draw]` (a new card each time,
/// as `Post.all` is after an edit) got the first request's page back (2.29.3).
/// One worker, so both requests meet the same cache.
#[test]
fn a_page_rendered_from_objects_is_not_answered_from_the_cache() {
    let server = ServerProcess::start_single_worker("en");
    let first = body_string(ureq::get(&server.url("/cards")).call().unwrap());
    let second = body_string(ureq::get(&server.url("/cards")).call().unwrap());
    assert!(first.contains("<p>card 1</p>"), "first page: {first}");
    assert!(
        second.contains("<p>card 2</p>"),
        "the second request got the first page back: {second}"
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

/// A page whose footer reads `clock` bare renders on the VM in production,
/// and `SOLI_ENGINE_LOG=1` names, once, each template the interpreter renders
/// and why. In a compiled view the bare `clock` used to be the function
/// itself: `clock - started` raised, every request rendered the page a second
/// time on the tree-walker, and the log said nothing about any of it.
#[test]
fn a_bare_clock_view_renders_on_the_vm_and_the_engine_log_names_the_rest() {
    let app = tempfile::tempdir().expect("temp app dir");
    let root = app.path();
    let write = |path: &str, body: &str| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent dir")).expect("mkdir");
        std::fs::write(path, body).expect("write app file");
    };
    write(
        "config/routes.sl",
        "get(\"/ping\", \"home#ping\")\nget(\"/timing\", \"home#timing\")\n",
    );
    write(
        "app/controllers/home_controller.sl",
        "class HomeController < Controller\n  \
           # GET /ping\n  \
           def ping(req)\n    {\"status\": 200, \"body\": \"pong\"}\n  end\n\n  \
           # GET /timing\n  \
           def timing(req)\n    @rows = [1, 2, 3]\n    @started = clock()\n    render(\"home/timing\")\n  end\n\
         end\n",
    );
    write(
        "app/views/layouts/application.html.slv",
        "<main><%= yield %></main>",
    );
    write(
        "app/views/home/timing.html.slv",
        "<% for row in rows %><%= render \"home/row\", {\"row\": row} %><% end %>|<%= (clock - started) >= 0 %>",
    );
    write("app/views/home/_row.html.slv", "[<%= row %>]");
    let check_path = root.join("vm_views_check.log");
    let stderr_path = root.join("stderr.log");

    let port = pick_port();
    let child = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_soli")))
        .arg("serve")
        .arg(root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        .env("SOLI_ENGINE_LOG", "1")
        // Each compiled render is checked against the tree-walker's.
        .env("SOLI_VM_VIEWS_CHECK", &check_path)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&stderr_path).expect("stderr file"))
        .spawn()
        .expect("spawn soli serve");
    let server = ServerProcess { child, port };
    server.wait_ready();
    for _ in 0..2 {
        let body = body_string(
            ureq::get(&server.url("/timing"))
                .call()
                .expect("GET /timing"),
        );
        assert!(body.contains("<main>[1][2][3]|true</main>"), "{body}");
    }
    drop(server);

    let check = std::fs::read_to_string(&check_path).unwrap_or_default();
    assert!(check.contains("home/timing.html.slv: same"), "{check}");
    assert!(!check.contains("render failed"), "{check}");
    let log = std::fs::read_to_string(&stderr_path).expect("server stderr");
    assert!(
        log.contains(
            "[soli engine] view 'home/_row.html.slv' renders on the interpreter: \
             too small to gain from the VM"
        ),
        "{log}"
    );
    assert!(
        log.contains(
            "[soli engine] layout 'layouts/application.html.slv' renders on the interpreter: \
             layouts are not compiled"
        ),
        "{log}"
    );
    // Once, for two requests; and nothing about the view the VM renders.
    assert_eq!(
        log.matches("layouts/application.html.slv").count(),
        1,
        "{log}"
    );
    assert!(!log.contains("home/timing.html.slv"), "{log}");
}

/// A list page whose rows are includes, served in production. Each include
/// reads the controller's `@ivars` and the app's helpers — one named like the
/// form builder's `button_to`, as once-campfire's is — on the interpreter and
/// on the VM alike (`SOLI_VM_VIEWS_CHECK` renders every compiled view and
/// partial on both and compares), and the hash a view passes is left as it
/// was. `partial()` used to copy every helper and every controller field into
/// that hash, and the render tag called the framework's `button_to`.
#[test]
fn includes_read_ivars_and_helpers_through_their_scope_on_both_engines() {
    let app = tempfile::tempdir().expect("temp app dir");
    let root = app.path();
    let write = |path: &str, body: &str| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent dir")).expect("mkdir");
        std::fs::write(path, body).expect("write app file");
    };
    write(
        "config/routes.sl",
        "get(\"/ping\", \"rows#ping\")\nget(\"/list\", \"rows#list\")\n",
    );
    write(
        "app/controllers/rows_controller.sl",
        "class RowsController < Controller\n  \
           # GET /ping\n  \
           def ping(req)\n    {\"status\": 200, \"body\": \"pong\"}\n  end\n\n  \
           # GET /list\n  \
           def list(req)\n    \
             @rows = [{\"id\": 1, \"title\": \"a\"}, {\"id\": 2, \"title\": \"b\"}]\n    \
             @heading = \"Rows\"\n    \
             @nonce = clock()\n    \
             render(\"rows/list\")\n  \
           end\n\
         end\n",
    );
    write(
        "app/helpers/rows_helper.sl",
        "def button_to(url, options, inner_html)\n  \"USER-button_to\"\nend\n\n\
         def shout(text)\n  text.upcase + \"!\"\nend\n",
    );
    write(
        "app/views/layouts/application.html.slv",
        "<main><%= yield %></main>",
    );
    write(
        "app/views/rows/list.html.slv",
        "<% opts = {\"x\": 1} %>\
         <% for row in rows %>\
         <%- partial(\"rows/row\", {\"row\": row}) %>\
         <%= render \"rows/row\", {\"row\": row} %>\
         <%- component(\"badge\", {\"label\": row.title}) %>\
         <% end %>|<%- partial(\"rows/wide\", opts) %>|<%- partial(\"rows/whole\", opts) %>\
         |<%= opts.length %>",
    );
    // Small: the interpreter renders it.
    write(
        "app/views/rows/_row.html.slv",
        "[<%= row.id %> <%= @heading %> <%= shout(row.title) %> <%= button_to(\"/r\", {}, \"r\") %>]",
    );
    // A loop: compiled, so the VM binds the `@ivar` and the helpers.
    write(
        "app/views/rows/_wide.html.slv",
        "<% for i in [1] %>(<%= @heading %> <%= x %> <%= shout(\"w\") %> \
         <%= button_to(\"/w\", {}, \"w\") %>)<% end %>",
    );
    // Reads its data whole: the `@ivars` are among `locals`, the helpers not.
    write(
        "app/views/rows/_whole.html.slv",
        "{<%= locals[\"heading\"] %> <%= locals[\"shout\"].nil? %> <%= locals.has_key(\"rows\") %>}",
    );
    write(
        "app/views/components/badge.html.slv",
        "(<%= label %>:<%= @heading %>)",
    );
    let check_path = root.join("vm_views_check.log");

    let port = pick_port();
    let child = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_soli")))
        .arg("serve")
        .arg(root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        // Each compiled render is checked against the tree-walker's.
        .env("SOLI_VM_VIEWS_CHECK", &check_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn soli serve");
    let server = ServerProcess { child, port };
    server.wait_ready();
    let expected = "<main>\
        [1 Rows A! USER-button_to][1 Rows A! USER-button_to](a:Rows)\
        [2 Rows B! USER-button_to][2 Rows B! USER-button_to](b:Rows)\
        |(Rows 1 W! USER-button_to)|{Rows true true}|1</main>";
    for _ in 0..2 {
        let body = body_string(ureq::get(&server.url("/list")).call().expect("GET /list"));
        assert!(body.contains(expected), "{body}");
    }
    drop(server);

    let check = std::fs::read_to_string(&check_path).unwrap_or_default();
    assert!(check.contains("rows/list.html.slv: same"), "{check}");
    assert!(check.contains("rows/_wide.html.slv: same"), "{check}");
    assert!(!check.contains("differs"), "{check}");
    assert!(!check.contains("render failed"), "{check}");
}

/// Nothing a view or an action renders writes into the hash it is handed. A
/// view's `opts` keeps its one key through `partial()`, `component()`, the
/// render tag and an app helper's `partial()`, compiled or not. A `const`
/// passed to `render()` or `render_partial()` keeps its keys too, so each
/// request renders its own `req`, cookies and `@ivars`. The first request on
/// the worker used to add its own to the const, and they won over every later
/// request's: bob was shown alice's page.
#[test]
fn views_and_actions_leave_the_callers_hash_alone_across_requests() {
    let app = tempfile::tempdir().expect("temp app dir");
    let root = app.path();
    let write = |path: &str, body: &str| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent dir")).expect("mkdir");
        std::fs::write(path, body).expect("write app file");
    };
    write(
        "config/routes.sl",
        "get(\"/ping\", \"leak#ping\")\nget(\"/compiled\", \"leak#compiled\")\n\
         get(\"/plain\", \"leak#plain\")\nget(\"/shared\", \"leak#shared\")\n\
         get(\"/fragment\", \"leak#fragment\")\nget(\"/kept\", \"leak#kept\")\n",
    );
    write(
        "app/controllers/leak_controller.sl",
        "const SHARED = {\"title\": \"Home\"}\nconst FRAGMENT = {\"title\": \"Fragment\"}\n\n\
         class LeakController < Controller\n  \
           # GET /ping\n  \
           def ping(req)\n    {\"status\": 200, \"body\": \"pong\"}\n  end\n\n  \
           # GET /compiled\n  \
           def compiled(req)\n    @heading = \"H\"\n    render(\"leak/compiled\")\n  end\n\n  \
           # GET /plain\n  \
           def plain(req)\n    @heading = \"H\"\n    render(\"leak/plain\")\n  end\n\n  \
           # GET /shared\n  \
           def shared(req)\n    \
             @who = req[\"query\"][\"who\"]\n    \
             render(\"leak/shared\", SHARED)\n  \
           end\n\n  \
           # GET /fragment\n  \
           def fragment(req)\n    \
             {\"status\": 200, \"body\": render_partial(\"leak/fragment\", FRAGMENT)}\n  \
           end\n\n  \
           # GET /kept\n  \
           def kept(req)\n    \
             {\"status\": 200, \"body\": SHARED.keys.join(\",\") + \"|\" + FRAGMENT.keys.join(\",\")}\n  \
           end\n\
         end\n",
    );
    // An app helper closes over the global `partial()`, not the view's.
    write(
        "app/helpers/leak_helper.sl",
        "def card(opts)\n  partial(\"leak/card\", opts)\nend\n",
    );
    write(
        "app/views/layouts/application.html.slv",
        "<main><%= yield %></main>",
    );
    let includes = "<%- partial(\"leak/row\", opts) %><%- component(\"chip\", opts) %>\
                    <%= render \"leak/row\", opts %><%- card(opts) %>";
    // A loop: compiled, so the VM renders it.
    write(
        "app/views/leak/compiled.html.slv",
        &format!(
            "<% opts = {{\"x\": 1}} %><% for i in [1] %>{includes}<% end %>\
             |<%= opts.keys.join(\",\") %>"
        ),
    );
    // Small: the interpreter renders it.
    write(
        "app/views/leak/plain.html.slv",
        &format!("<% opts = {{\"x\": 1}} %>{includes}|<%= opts.keys.join(\",\") %>"),
    );
    write("app/views/leak/_row.html.slv", "[<%= x %> <%= @heading %>]");
    write("app/views/components/chip.html.slv", "(<%= x %>)");
    write(
        "app/views/leak/_card.html.slv",
        "{<%= x %> <%= req[\"query\"][\"q\"] %>}",
    );
    write(
        "app/views/leak/shared.html.slv",
        "who=<%= who %> q=<%= req[\"query\"][\"q\"] %> sid=<%= cookies[\"sid\"] %>",
    );
    write(
        "app/views/leak/_fragment.html.slv",
        "q=<%= req[\"query\"][\"q\"] %> sid=<%= cookies[\"sid\"] %>",
    );
    let check_path = root.join("vm_views_check.log");

    let port = pick_port();
    let child = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_soli")))
        .arg("serve")
        .arg(root)
        .arg("--port")
        .arg(port.to_string())
        // One worker: every request meets the consts the first one saw.
        .arg("--workers")
        .arg("1")
        .env("SOLI_VM_VIEWS_CHECK", &check_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn soli serve");
    let server = ServerProcess { child, port };
    server.wait_ready();
    let get = |path: &str, sid: &str| {
        body_string(
            ureq::get(&server.url(path))
                .set("Cookie", &format!("sid={sid}"))
                .call()
                .unwrap_or_else(|e| panic!("GET {path}: {e}")),
        )
    };

    for page in ["/compiled", "/plain"] {
        let body = get(&format!("{page}?q=a"), "alice");
        assert!(
            body.contains("<main>[1 H](1)[1 H]{1 a}|x</main>"),
            "{page}: {body}"
        );
    }
    for (who, q) in [("alice", "a"), ("bob", "b")] {
        let page = get(&format!("/shared?who={who}&q={q}"), who);
        assert!(
            page.contains(&format!("who={who} q={q} sid={who}")),
            "{who}: {page}"
        );
        let fragment = get(&format!("/fragment?q={q}"), who);
        assert!(
            fragment.contains(&format!("q={q} sid={who}")),
            "{who}: {fragment}"
        );
    }
    assert_eq!(get("/kept", "carol"), "title|title");
    drop(server);

    let check = std::fs::read_to_string(&check_path).unwrap_or_default();
    assert!(check.contains("leak/compiled.html.slv: same"), "{check}");
    assert!(!check.contains("differs"), "{check}");
    assert!(!check.contains("render failed"), "{check}");
}

#[test]
fn render_json_and_render_text_skip_the_actions_default_view() {
    // `render_json` / `render_text` answer through the fast-path slot and
    // return nil, which the controller took for "render the default view": a
    // JSON action whose name matched a view rendered that view for nothing,
    // and answered 500 when it raised. An action that returns nothing still
    // gets its view.
    let app = tempfile::tempdir().expect("temp app dir");
    let root = app.path();
    let write = |path: &str, body: &str| {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().expect("a parent dir")).expect("mkdir");
        std::fs::write(path, body).expect("write app file");
    };
    write(
        "config/routes.sl",
        "get(\"/ping\", \"api#ping\")\nget(\"/json\", \"api#json\")\n\
         get(\"/text\", \"api#text\")\nget(\"/page\", \"api#page\")\n",
    );
    write(
        "app/controllers/api_controller.sl",
        "class ApiController < Controller\n  \
           # GET /ping\n  \
           def ping(req)\n    render_text(\"pong\")\n  end\n\n  \
           # GET /json\n  \
           def json(req)\n    @who = \"json\"\n    render_json({\"ok\": true}, 201)\n  end\n\n  \
           # GET /text\n  \
           def text(req)\n    render_text(\"plain\")\n  end\n\n  \
           # GET /page\n  \
           def page(req)\n    @who = \"view\"\n  end\n\
         end\n",
    );
    write(
        "app/views/layouts/application.html.slv",
        "<main><%= yield %></main>",
    );
    write("app/views/api/json.html.slv", "<%= 3 / 0 %>");
    write("app/views/api/text.html.slv", "<%= 3 / 0 %>");
    write("app/views/api/page.html.slv", "hello <%= who %>");

    let port = pick_port();
    let child = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_soli")))
        .arg("serve")
        .arg(root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn soli serve");
    let server = ServerProcess { child, port };
    server.wait_ready();
    let get = |path: &str| match ureq::get(&server.url(path)).call() {
        Ok(resp) => resp,
        Err(ureq::Error::Status(_, resp)) => resp,
        Err(e) => panic!("GET {path}: {e}"),
    };

    let json = get("/json");
    assert_eq!(json.status(), 201, "/json answered with its view");
    assert!(
        json.content_type().starts_with("application/json"),
        "{}",
        json.content_type()
    );
    assert_eq!(body_string(json), "{\"ok\":true}");

    let text = get("/text");
    assert_eq!(text.status(), 200, "/text answered with its view");
    assert!(text.content_type().starts_with("text/plain"));
    assert_eq!(body_string(text), "plain");

    let page = get("/page");
    assert_eq!(page.status(), 200);
    assert!(body_string(page).contains("<main>hello view</main>"));
}

/// GET `path` with optional `X-User`, whatever the status.
fn get_page(server: &ServerProcess, path: &str, user: Option<&str>) -> String {
    let mut request = ureq::get(&server.url(path)).timeout(Duration::from_secs(5));
    if let Some(user) = user {
        request = request.set("X-User", user);
    }
    match request.call() {
        Ok(resp) => body_string(resp),
        Err(ureq::Error::Status(_, resp)) => body_string(resp),
        Err(e) => panic!("GET {path}: {e}"),
    }
}

/// The view helpers read `req` from an env their worker thread keeps, and it
/// was only rebound in the action. A page rendered without one — a route
/// miss's custom `errors/404`, a view a middleware renders — showed the
/// previous visitor's `req["current_user"]`. One worker, so every request
/// lands on the thread alice's request used.
#[test]
fn view_helpers_never_see_the_previous_visitors_request() {
    let server = ServerProcess::start_fixture("_helper_context_app");
    let alice = Some("alice@example.com");

    assert!(get_page(&server, "/me", alice).contains("me: alice@example.com"));
    let not_found = get_page(&server, "/nope", None);
    assert!(
        not_found.contains("404 page, signed in as: guest"),
        "a route miss must not render with alice's request: {not_found}"
    );

    assert!(get_page(&server, "/me", alice).contains("me: alice@example.com"));
    let blocked = get_page(&server, "/blocked", None);
    assert!(
        blocked.contains("blocked for: guest"),
        "a middleware-rendered view must see this request: {blocked}"
    );
    let bob = get_page(&server, "/blocked", Some("bob@example.com"));
    assert!(bob.contains("blocked for: bob@example.com"), "{bob}");
    assert!(get_page(&server, "/me", None).contains("me: guest"));
}

/// App metrics and the per-route request histogram, end to end: a controller
/// calls `Metrics.*`, `/_metrics` shows them next to
/// `soli_http_request_duration_seconds{method,route,status_class}`, and a
/// DogStatsD agent on a loopback UDP socket receives the same samples with
/// the `DD_*` global tags.
#[test]
fn app_metrics_reach_prometheus_and_dogstatsd() {
    let agent = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind agent");
    agent
        .set_read_timeout(Some(Duration::from_millis(500)))
        .expect("timeout");
    let agent_addr = agent.local_addr().expect("agent addr").to_string();
    let app = tempfile::tempdir().expect("temp app dir");
    let server = serve_app_in(
        app.path(),
        &[
            (
                "config/routes.sl",
                "get(\"/ping\", \"home#ping\")\nget(\"/order\", \"home#order\")\n",
            ),
            (
                "app/controllers/home_controller.sl",
                "class HomeController < Controller
  # GET /ping
  def ping(req)
    {\"status\": 200, \"body\": \"pong\"}
  end

  # GET /order
  def order(req)
    Metrics.increment(\"orders.created\", tags: {\"plan\": \"pro\"})
    Metrics.gauge(\"queue.depth\", 4)
    total = Metrics.time(\"checkout.duration\", {\"step\": \"pay\"}) do
      21 * 2
    end
    {\"status\": 201, \"body\": \"total #{total}\"}
  end
end
",
            ),
        ],
        &[
            ("SOLI_METRICS", "1"),
            // The action must run on the VM, not quietly on the interpreter.
            ("SOLI_FAIL_ON_VM_DEMOTION", "1"),
            ("SOLI_STATSD", &agent_addr),
            ("DD_SERVICE", "shop"),
            ("DD_ENV", "e2e"),
        ],
    );
    for _ in 0..2 {
        let resp = ureq::get(&server.url("/order"))
            .timeout(Duration::from_secs(5))
            .call()
            .expect("order request");
        assert_eq!(resp.status(), 201);
        assert_eq!(body_string(resp), "total 42");
    }
    assert_eq!(status_of(&server, "/nowhere", None), 404);

    let body = body_string(
        ureq::get(&server.url("/_metrics"))
            .timeout(Duration::from_secs(3))
            .call()
            .expect("metrics request"),
    );
    for expected in [
        "# TYPE orders_created counter\norders_created{plan=\"pro\"} 2\n",
        "# TYPE queue_depth gauge\nqueue_depth 4\n",
        "checkout_duration_count{step=\"pay\"} 2\n",
        "soli_http_request_duration_seconds_count{method=\"GET\",route=\"home#order\",status_class=\"2xx\"} 2\n",
        "soli_http_request_duration_seconds_count{method=\"GET\",route=\"unmatched\",status_class=\"4xx\"} 1\n",
        "soli_http_request_duration_seconds_bucket{method=\"GET\",route=\"home#order\",status_class=\"2xx\",le=\"+Inf\"} 2\n",
    ] {
        assert!(body.contains(expected), "missing {expected:?} in:\n{body}");
    }

    let mut lines = Vec::new();
    let mut buf = [0u8; 65536];
    while let Ok(n) = agent.recv(&mut buf) {
        let text = String::from_utf8_lossy(&buf[..n]).into_owned();
        lines.extend(text.lines().map(str::to_string));
    }
    let has = |prefix: &str| lines.iter().filter(|l| l.starts_with(prefix)).count();
    assert_eq!(
        has("orders.created:1|c|#plan:pro,env:e2e,service:shop"),
        2,
        "{lines:#?}"
    );
    assert_eq!(
        has("queue.depth:4|g|#env:e2e,service:shop"),
        2,
        "{lines:#?}"
    );
    assert_eq!(has("checkout.duration:"), 2, "{lines:#?}");
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("soli.http.request.duration:")
                && l.ends_with(
                    "|d|#method:GET,route:home#order,status_class:2xx,env:e2e,service:shop"
                )),
        "{lines:#?}"
    );
}

/// Four view bugs met building an app (#675), served in production, where
/// the view and its helper run on the VM, and under `--dev`, where they run
/// on the tree-walker: a helper reading its file's `const` saw nil; a block
/// variable named `component` opened a component block; `partial()` did not
/// find `components/_badge.html.slv`; `render`'s `headers` option was dropped.
#[test]
fn view_helpers_constants_components_and_render_headers_on_both_engines() {
    const FILES: &[(&str, &str)] = &[
        (
            "config/routes.sl",
            "get(\"/ping\", \"home#ping\")\nget(\"/vigie\", \"home#vigie\")\n",
        ),
        (
            "app/controllers/home_controller.sl",
            "class HomeController < Controller
  # GET /ping
  def ping(req)
    {\"status\": 200, \"body\": \"pong\"}
  end

  # GET /vigie
  def vigie(req)
    @items = [{\"list\": [\"a\", \"b\"]}, {\"list\": [\"c\"]}]
    cache_headers = {\"Cache-Control\": \"public, max-age=60\", \"X-Vigie\": \"1\"}
    render(\"home/vigie\", {}, {\"layout\": false, \"status\": 203, \"headers\": cache_headers})
  end
end
",
        ),
        (
            "app/helpers/status_helper.sl",
            "const STATUSES = [\"up\", \"down\"]\n\ndef status_name(i)\n  STATUSES[i]\nend\n",
        ),
        (
            "app/views/home/vigie.html.slv",
            "<%= status_name(1) %>|<% @items.each do |component| %>\
             <% component[\"list\"].each do |x| %><%= x %><% end %><% end %>|\
             <%- partial(\"components/badge\", {\"label\": \"ok\"}) %>",
        ),
        ("app/views/components/_badge.html.slv", "[<%= label %>]"),
    ];
    for args in [&[][..], &["--dev"][..]] {
        let app = tempfile::tempdir().expect("temp app dir");
        let server = serve_app_with_args(app.path(), FILES, &[], args);
        let resp = ureq::get(&server.url("/vigie"))
            .timeout(Duration::from_secs(10))
            .call()
            .unwrap_or_else(|e| panic!("{args:?}: GET /vigie: {e}"));
        assert_eq!(resp.status(), 203, "{args:?}");
        assert_eq!(
            resp.header("Cache-Control"),
            Some("public, max-age=60"),
            "{args:?}"
        );
        assert_eq!(resp.header("X-Vigie"), Some("1"), "{args:?}");
        // `--dev` marks each view and component with an HTML comment.
        let mut body = body_string(resp);
        while let Some(start) = body.find("<!--solidev:") {
            let end = body[start..]
                .find("-->")
                .map_or(body.len(), |at| start + at + 3);
            body.replace_range(start..end, "");
        }
        assert!(body.starts_with("down|abc|[ok]"), "{args:?}: {body}");
    }
}

/// The single-node WebSocket API answers for real (#695): `ws_join` /
/// `ws_leave` act on the connection whose handler runs, `ws_count`,
/// `ws_clients` and `ws_clients_in` count it, and a `broadcast` from an HTTP
/// action — on the VM — reaches the sockets a handler — on the tree-walker —
/// joined to the channel.
#[test]
fn websocket_rooms_join_count_and_receive_broadcasts() {
    let app = tempfile::tempdir().expect("temp app dir");
    let server = serve_app_in(
        app.path(),
        &[
            (
                "config/routes.sl",
                "get(\"/ping\", \"home#ping\")
get(\"/stats\", \"home#stats\")
get(\"/announce\", \"home#announce\")
router_websocket(\"/ws/room\", \"room#handle\")
",
            ),
            (
                "app/controllers/home_controller.sl",
                "class HomeController < Controller
  # GET /ping
  def ping(req)
    {\"status\": 200, \"body\": \"pong\"}
  end

  # GET /stats
  def stats(req)
    backend = realtime_bus()[\"backend\"]
    joined = ws_clients_in(\"lobby\").length
    refused = \"no\"
    try
      ws_join(\"lobby\")
    catch error
      refused = \"yes\"
    end
    {\"status\": 200, \"body\": \"#{ws_count()}|#{joined}|#{ws_clients().length}|#{backend}|#{refused}\"}
  end

  # GET /announce
  def announce(req)
    delivered = broadcast(\"lobby\", {\"text\": params[\"text\"]})
    {\"status\": 200, \"body\": \"sse #{delivered}\"}
  end
end
",
            ),
            (
                "app/controllers/room_controller.sl",
                "def handle(event)
  if event[\"type\"] == \"message\"
    if event[\"message\"] == \"join\"
      ws_join(\"lobby\")
      members = ws_clients_in(\"lobby\").length
      return {\"send\": \"joined #{members}\"}
    end
    if event[\"message\"] == \"leave\"
      ws_leave(\"lobby\")
      members = ws_clients_in(\"lobby\").length
      return {\"send\": \"left #{members}\"}
    end
  end
  {}
end
",
            ),
        ],
        &[("SOLI_FAIL_ON_VM_DEMOTION", "1")],
    );

    let connect = || {
        let url = format!("ws://127.0.0.1:{}/ws/room", server.port);
        let (socket, _) = tungstenite::connect(&url).expect("WebSocket handshake");
        if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref() {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
        }
        socket
    };
    let ask = |socket: &mut tungstenite::WebSocket<_>, text: &str| -> String {
        socket
            .send(tungstenite::Message::Text(text.into()))
            .expect("send frame");
        socket
            .read()
            .expect("a reply")
            .to_text()
            .unwrap()
            .to_string()
    };
    let get = |path: &str| {
        body_string(
            ureq::get(&server.url(path))
                .timeout(Duration::from_secs(10))
                .call()
                .expect("GET"),
        )
    };

    let mut alice = connect();
    let mut bob = connect();
    assert_eq!(ask(&mut alice, "join"), "joined 1");
    assert_eq!(ask(&mut bob, "join"), "joined 2");
    assert_eq!(get("/stats"), "2|2|2|local|yes");

    assert_eq!(get("/announce?text=hi"), "sse 0");
    for socket in [&mut alice, &mut bob] {
        let frame = socket.read().expect("the broadcast");
        assert_eq!(frame.to_text().unwrap(), r#"{"text":"hi"}"#);
    }

    assert_eq!(ask(&mut alice, "leave"), "left 1");
    assert_eq!(get("/stats"), "2|1|2|local|yes");
}

/// A SoliDB for the realtime-bus test: `SOLIDB_DRIVER_TEST_HOST=127.0.0.1:16745`
/// (admin/admin unless `SOLIDB_USERNAME` / `SOLIDB_PASSWORD` say otherwise).
/// Skipped without one, and failed instead under `SOLI_REQUIRE_DB=1`, which CI
/// sets.
fn solidb_test_host() -> Option<String> {
    match std::env::var("SOLIDB_DRIVER_TEST_HOST") {
        Ok(host) if !host.is_empty() => Some(host),
        _ => {
            if std::env::var("SOLI_REQUIRE_DB").is_ok_and(|flag| flag == "1") {
                panic!("SOLI_REQUIRE_DB=1 but SOLIDB_DRIVER_TEST_HOST is unset");
            }
            eprintln!("skip: SOLIDB_DRIVER_TEST_HOST unset (no SoliDB for the realtime bus)");
            None
        }
    }
}

/// With `SOLI_REALTIME_BUS=solidb`, what one `soli serve` process broadcasts —
/// `broadcast`, `ws_broadcast_room`, a handler's `broadcast_room` — reaches a
/// socket held by another process, and `{"local": true}` keeps a message home.
#[test]
fn a_realtime_bus_carries_broadcasts_between_two_servers() {
    use base64::Engine as _;
    let Some(host) = solidb_test_host() else {
        return;
    };
    let user = std::env::var("SOLIDB_USERNAME").unwrap_or_else(|_| "admin".into());
    let password = std::env::var("SOLIDB_PASSWORD").unwrap_or_else(|_| "admin".into());
    let auth = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    );
    let database = format!("soli_rt_bus_test_{}", std::process::id());
    let _ = ureq::post(&format!("http://{host}/_api/database"))
        .set("Authorization", &auth)
        .send_json(serde_json::json!({ "name": database }));
    two_servers_share_a_realtime_bus(&[
        ("SOLIDB_HOST", format!("http://{host}")),
        ("SOLIDB_DATABASE", database.clone()),
        ("SOLIDB_USERNAME", user.clone()),
        ("SOLIDB_PASSWORD", password.clone()),
        ("SOLI_REALTIME_BUS", "solidb".to_string()),
    ]);
    let _ = ureq::delete(&format!("http://{host}/_api/database/{database}"))
        .set("Authorization", &auth)
        .call();
}

/// `SOLIKV_TEST_HOST` (`host:port`) names a SoliKV, Redis or Valkey for the
/// `solikv` realtime bus test; without it the test skips.
fn solikv_test_host() -> Option<(String, String)> {
    match std::env::var("SOLIKV_TEST_HOST") {
        Ok(host) if !host.is_empty() => {
            let (name, port) = host.rsplit_once(':').unwrap_or((host.as_str(), "6380"));
            Some((name.to_string(), port.to_string()))
        }
        _ => {
            eprintln!("skip: SOLIKV_TEST_HOST unset (no SoliKV for the realtime bus)");
            None
        }
    }
}

/// The same two-server exchange over `SOLI_REALTIME_BUS=solikv`: `PUBLISH` /
/// `SUBSCRIBE`, no database at all.
#[test]
fn a_solikv_realtime_bus_carries_broadcasts_between_two_servers() {
    let Some((host, port)) = solikv_test_host() else {
        return;
    };
    let mut env = vec![
        ("SOLIKV_RESP_HOST", host),
        ("SOLIKV_RESP_PORT", port),
        ("SOLI_REALTIME_BUS", "solikv".to_string()),
        (
            "SOLI_REALTIME_CHANNEL",
            format!("soli:rt:test:{}", std::process::id()),
        ),
    ];
    if let Ok(token) = std::env::var("SOLIKV_TEST_TOKEN") {
        env.push(("SOLIKV_TOKEN", token));
    }
    two_servers_share_a_realtime_bus(&env);
}

/// Boot two copies of one app on `bus_env`, wait for both to subscribe, and
/// check that broadcasts cross between them.
fn two_servers_share_a_realtime_bus(bus_env: &[(&str, String)]) {
    let files: &[(&str, &str)] = &[
        (
            "config/routes.sl",
            "get(\"/ping\", \"home#ping\")
get(\"/bus\", \"home#bus\")
get(\"/announce\", \"home#announce\")
get(\"/room\", \"home#room\")
get(\"/who\", \"home#who\")
get(\"/dm\", \"home#dm\")
get(\"/count\", \"home#count\")
router_websocket(\"/ws/room\", \"room#handle\")
",
        ),
        (
            "app/controllers/home_controller.sl",
            "class HomeController < Controller
  # GET /ping
  def ping(req)
    {\"status\": 200, \"body\": \"pong\"}
  end

  # GET /bus
  def bus(req)
    bus = realtime_bus()
    connected = bus[\"connected\"]
    published = bus[\"published\"]
    received = bus[\"received\"]
    {\"status\": 200, \"body\": \"#{connected}|#{published}|#{received}\"}
  end

  # GET /announce
  def announce(req)
    broadcast(\"lobby\", {\"text\": params[\"text\"]}, {\"local\": params[\"local\"] == \"1\"})
    {\"status\": 200, \"body\": \"ok\"}
  end

  # GET /room
  def room(req)
    ws_broadcast_room(\"lobby\", params[\"text\"])
    {\"status\": 200, \"body\": \"ok\"}
  end

  # GET /who
  def who(req)
    users = ws_list_presence(\"lobby\").map { |p| p[\"user_id\"] }.sort.join(\",\")
    {\"status\": 200, \"body\": \"#{ws_presence_count(\"lobby\")}|#{users}\"}
  end

  # GET /dm
  def dm(req)
    ws_send(params[\"to\"], params[\"text\"])
    {\"status\": 200, \"body\": \"ok\"}
  end

  # GET /count
  def count(req)
    {\"status\": 200, \"body\": \"#{ws_count()}|#{ws_count({\"cluster\": true})}\"}
  end
end
",
        ),
        (
            "app/controllers/room_controller.sl",
            "def handle(event)
  if event[\"type\"] == \"message\"
    if event[\"message\"] == \"join\"
      ws_join(\"lobby\")
      return {\"send\": \"joined\"}
    end
    return {\"broadcast_room\": event[\"message\"] + \"!\"} if event[\"message\"] == \"shout\"
    return {\"send\": event[\"connection_id\"]} if event[\"message\"] == \"whoami\"
    if event[\"message\"].starts_with?(\"track:\")
      return {\"track\": {\"channel\": \"lobby\", \"user_id\": event[\"message\"].replace(\"track:\", \"\")}}
    end
  end
  {}
end
",
        ),
    ];
    let mut env: Vec<(&str, &str)> = bus_env.iter().map(|(k, v)| (*k, v.as_str())).collect();
    env.push(("SOLI_FAIL_ON_VM_DEMOTION", "1"));
    let app_a = tempfile::tempdir().expect("temp app dir");
    let app_b = tempfile::tempdir().expect("temp app dir");
    let server_a = serve_app_in(app_a.path(), files, &env);
    let server_b = serve_app_in(app_b.path(), files, &env);

    let get = |server: &ServerProcess, path: &str| {
        body_string(
            ureq::get(&server.url(path))
                .timeout(Duration::from_secs(10))
                .call()
                .expect("GET"),
        )
    };
    let connect = |server: &ServerProcess| {
        let url = format!("ws://127.0.0.1:{}/ws/room", server.port);
        let (mut socket, _) = tungstenite::connect(&url).expect("WebSocket handshake");
        if let tungstenite::stream::MaybeTlsStream::Plain(stream) = socket.get_ref() {
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
        }
        socket
            .send(tungstenite::Message::Text("join".into()))
            .expect("send join");
        assert_eq!(
            socket.read().expect("join reply").to_text().unwrap(),
            "joined"
        );
        socket
    };

    // Both subscribed before anything is published: the changefeed carries
    // only what is written after a subscription.
    let deadline = Instant::now() + Duration::from_secs(30);
    for server in [&server_a, &server_b] {
        while !get(server, "/bus").starts_with("true|") {
            assert!(
                Instant::now() < deadline,
                "the bus never connected; stderr: {}",
                std::fs::read_to_string(app_b.path().join("stderr.log")).unwrap_or_default()
            );
            thread::sleep(Duration::from_millis(200));
        }
    }

    let mut on_b = connect(&server_b);
    get(&server_a, "/announce?text=hi");
    let frame = on_b.read().expect("the broadcast from A");
    assert_eq!(frame.to_text().unwrap(), r#"{"text":"hi"}"#);

    get(&server_a, "/announce?text=quiet&local=1");
    get(&server_a, "/room?text=two");
    let frame = on_b.read().expect("ws_broadcast_room from A");
    assert_eq!(
        frame.to_text().unwrap(),
        "two",
        "a {{local: true}} broadcast stays on A"
    );

    let mut on_a = connect(&server_a);
    on_a.send(tungstenite::Message::Text("shout".into()))
        .expect("send shout");
    assert_eq!(
        on_a.read().expect("own room echo").to_text().unwrap(),
        "shout!"
    );
    assert_eq!(
        on_b.read().expect("A's room action").to_text().unwrap(),
        "shout!"
    );

    let counts = get(&server_b, "/bus");
    let received: i64 = counts.rsplit('|').next().unwrap().parse().unwrap();
    assert!(received >= 3, "B received {counts}");

    // ws_send from A reaches a connection held by B.
    on_b.send(tungstenite::Message::Text("whoami".into()))
        .expect("send whoami");
    let b_id = on_b.read().expect("B's id").to_text().unwrap().to_string();
    get(&server_a, &format!("/dm?to={b_id}&text=psst"));
    assert_eq!(on_b.read().expect("A's ws_send").to_text().unwrap(), "psst");

    // Presence spans both: bob tracked on B, alice on A.
    on_b.send(tungstenite::Message::Text("track:bob".into()))
        .expect("track bob");
    let state = on_b.read().expect("presence_state on B");
    assert!(
        state.to_text().unwrap().contains("presence_state"),
        "{state}"
    );
    on_a.send(tungstenite::Message::Text("track:alice".into()))
        .expect("track alice");
    // A may hear bob's join before its own presence_state, or after it: on
    // the changefeed bus it can still be in flight.
    let _state = (0..3)
        .map(|_| {
            on_a.read()
                .expect("a frame on A")
                .to_text()
                .unwrap()
                .to_string()
        })
        .find(|frame| frame.contains("presence_state"))
        .expect("presence_state on A");
    let join = on_b.read().expect("alice's join, on B");
    let join = join.to_text().unwrap();
    assert!(
        join.contains("presence_diff") && join.contains("alice"),
        "{join}"
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    for server in [&server_a, &server_b] {
        loop {
            let who = get(server, "/who");
            if who == "2|alice,bob" {
                break;
            }
            assert!(Instant::now() < deadline, "presence never merged: {who}");
            thread::sleep(Duration::from_millis(200));
        }
    }
    // Each server holds one socket; the cluster count adds the other's
    // (reported at its next heartbeat).
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let count = get(&server_a, "/count");
        if count == "1|2" {
            break;
        }
        assert!(Instant::now() < deadline, "cluster count: {count}");
        thread::sleep(Duration::from_millis(500));
    }

    // A closes: bob's side sees alice leave.
    drop(on_a);
    let leave = on_b.read().expect("alice's leave, on B");
    let leave = leave.to_text().unwrap();
    assert!(
        leave.contains("leaves") && leave.contains("alice"),
        "{leave}"
    );

    drop((on_b, server_a, server_b));
}

/// A DAP client over TCP, for `soli serve --dev --inspect`.
struct InspectClient {
    stream: std::net::TcpStream,
    reader: std::io::BufReader<std::net::TcpStream>,
    seq: i64,
}

impl InspectClient {
    fn connect(port: u16) -> InspectClient {
        let deadline = Instant::now() + Duration::from_secs(10);
        let stream = loop {
            match std::net::TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(e) if Instant::now() < deadline => {
                    let _ = e;
                    thread::sleep(Duration::from_millis(100));
                }
                Err(e) => panic!("inspector port: {e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        let reader = std::io::BufReader::new(stream.try_clone().unwrap());
        InspectClient {
            stream,
            reader,
            seq: 0,
        }
    }

    fn read(&mut self) -> Option<serde_json::Value> {
        use std::io::{BufRead, Read};
        let mut length = None;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line).ok()? == 0 {
                return None;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length:") {
                length = value.trim().parse::<usize>().ok();
            }
        }
        let mut body = vec![0u8; length?];
        self.reader.read_exact(&mut body).ok()?;
        serde_json::from_slice(&body).ok()
    }

    /// Send a request and return its response (success or not).
    fn request(&mut self, command: &str, arguments: serde_json::Value) -> serde_json::Value {
        use std::io::Write;
        self.seq += 1;
        let body = serde_json::json!({"seq": self.seq, "type": "request",
            "command": command, "arguments": arguments})
        .to_string();
        write!(
            self.stream,
            "Content-Length: {}\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        let seq = self.seq;
        loop {
            let message = self.read().expect("an answer from the inspector");
            if message["type"] == "response" && message["request_seq"] == seq {
                return message;
            }
        }
    }

    fn wait_stopped(&mut self) -> serde_json::Value {
        loop {
            let message = self.read().expect("a stopped event");
            if message["event"] == "stopped" {
                return message["body"].clone();
            }
        }
    }
}

/// `soli serve --dev --inspect`: an editor attaches with the dev token, a
/// breakpoint in a controller stops a real request, its locals are
/// readable, `continue` lets the response go, and once the editor leaves
/// the server answers freely again.
#[test]
fn an_attached_debugger_stops_a_request() {
    let dir = tempfile::tempdir().expect("temp app dir");
    let inspect_port = pick_port();
    let files: &[(&str, &str)] = &[
        (
            "config/routes.sl",
            "get(\"/ping\", \"home#ping\")\nget(\"/sum\", \"home#sum\")\n",
        ),
        (
            "app/controllers/home_controller.sl",
            "class HomeController < Controller
  # GET /ping
  def ping(req)
    {\"status\": 200, \"body\": \"pong\"}
  end

  # GET /sum
  def sum(req)
    a = params[\"a\"].to_i
    b = params[\"b\"].to_i
    total = a + b
    {\"status\": 200, \"body\": \"total #{total}\"}
  end
end
",
        ),
    ];
    let inspect_arg = format!("--inspect=127.0.0.1:{inspect_port}");
    let server = serve_app_with_args(
        dir.path(),
        files,
        &[("SOLI_DEV_REPL_SECRET", "inspect-spec-token")],
        &["--dev", &inspect_arg],
    );
    let controller = dir
        .path()
        .join("app/controllers/home_controller.sl")
        .to_string_lossy()
        .to_string();

    // A wrong token is refused and the connection closed.
    let mut intruder = InspectClient::connect(inspect_port);
    intruder.request("initialize", serde_json::json!({}));
    let refused = intruder.request("attach", serde_json::json!({"token": "guess"}));
    assert_eq!(refused["success"], false, "{refused}");
    assert!(intruder.read().is_none(), "the connection is closed");

    let mut client = InspectClient::connect(inspect_port);
    client.request("initialize", serde_json::json!({}));
    let early = client.request(
        "setBreakpoints",
        serde_json::json!({"source": {"path": controller}, "breakpoints": [{"line": 11}]}),
    );
    assert_eq!(early["success"], false, "nothing before attach: {early}");
    let attached = client.request("attach", serde_json::json!({"token": "inspect-spec-token"}));
    assert_eq!(attached["success"], true, "{attached}");
    client.request(
        "setBreakpoints",
        serde_json::json!({"source": {"path": controller}, "breakpoints": [{"line": 11}]}),
    );
    client.request("configurationDone", serde_json::json!({}));

    let url = server.url("/sum?a=2&b=3");
    let response = thread::spawn(move || {
        body_string(
            ureq::get(&url)
                .timeout(Duration::from_secs(60))
                .call()
                .expect("GET /sum"),
        )
    });
    let stopped = client.wait_stopped();
    assert_eq!(stopped["reason"], "breakpoint", "{stopped}");
    let trace = client.request("stackTrace", serde_json::json!({"threadId": 1}));
    let top = &trace["body"]["stackFrames"][0];
    assert_eq!(top["name"], "sum", "{trace}");
    assert_eq!(top["line"], 11);
    let scopes = client.request("scopes", serde_json::json!({"frameId": top["id"]}));
    let locals = client.request(
        "variables",
        serde_json::json!({"variablesReference": scopes["body"]["scopes"][0]["variablesReference"]}),
    );
    let names: Vec<&str> = locals["body"]["variables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"a") && names.contains(&"b"), "{locals}");
    assert!(!names.iter().any(|n| n.starts_with("__")), "{locals}");

    // Held past nothing: the request is still waiting on the breakpoint.
    thread::sleep(Duration::from_millis(500));
    assert!(!response.is_finished());
    client.request("continue", serde_json::json!({"threadId": 1}));
    assert_eq!(response.join().unwrap(), "total 5");

    client.request("disconnect", serde_json::json!({}));
    drop(client);
    // Breakpoints went with the editor.
    let free = body_string(
        ureq::get(&server.url("/sum?a=1&b=1"))
            .timeout(Duration::from_secs(10))
            .call()
            .expect("GET /sum after disconnect"),
    );
    assert_eq!(free, "total 2");
}

/// `--inspect` is a development tool: without `--dev` the server refuses to
/// start, and it binds loopback only.
#[test]
fn inspect_needs_dev_and_loopback() {
    let dir = tempfile::tempdir().expect("temp app dir");
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::write(dir.path().join("config/routes.sl"), "").unwrap();
    let run = |args: &[&str]| {
        Command::new(PathBuf::from(env!("CARGO_BIN_EXE_soli")))
            .arg("serve")
            .arg(dir.path())
            .args(args)
            .output()
            .expect("soli serve runs")
    };
    let no_dev = run(&["--inspect"]);
    assert!(!no_dev.status.success());
    assert!(String::from_utf8_lossy(&no_dev.stderr).contains("--inspect needs --dev"));
    let remote = run(&["--dev", "--inspect=0.0.0.0:9339"]);
    assert!(!remote.status.success());
    assert!(String::from_utf8_lossy(&remote.stderr).contains("loopback"));
}
