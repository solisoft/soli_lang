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

fn body_string(resp: ureq::Response) -> String {
    let mut buf = String::new();
    resp.into_reader().read_to_string(&mut buf).unwrap();
    buf
}

#[test]
fn ping_returns_json() {
    let server = ServerProcess::start();
    let resp = ureq::get(&server.url("/ping"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("ping request");
    assert_eq!(resp.status(), 200);
    assert_eq!(body_string(resp), r#"{"pong":true}"#);
}

#[test]
fn add_handles_query_params() {
    let server = ServerProcess::start();
    let resp = ureq::get(&server.url("/add?a=12&b=30"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("add request");
    assert_eq!(resp.status(), 200);
    assert_eq!(body_string(resp), "42");
}

#[test]
fn unknown_route_returns_404() {
    let server = ServerProcess::start();
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
    let server = ServerProcess::start();
    let resp = ureq::get(&server.url("/echo"))
        .timeout(Duration::from_secs(3))
        .call()
        .expect("echo request");
    assert_eq!(resp.status(), 200);
    assert_eq!(body_string(resp), "/echo");
}

#[test]
fn echo_method_handles_get_post_put_delete() {
    let server = ServerProcess::start();
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
    let server = ServerProcess::start();
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
    let server = ServerProcess::start();
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
    let server = ServerProcess::start();
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
    let server = ServerProcess::start();
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
    let server = ServerProcess::start();
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
    let server = ServerProcess::start();
    let resp = ureq::get(&server.url("/string?name=alice"))
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(body_string(resp), "HELLO, ALICE!");
}

#[test]
fn pipeline_in_handler() {
    let server = ServerProcess::start();
    let resp = ureq::get(&server.url("/pipeline"))
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    // [1,2,3,4,5].filter(>1).map(*n).reduce(+) = 4 + 9 + 16 + 25 = 54
    assert_eq!(body_string(resp), "54");
}

#[test]
fn hash_methods_in_handler() {
    let server = ServerProcess::start();
    let resp = ureq::get(&server.url("/hash"))
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    assert_eq!(body_string(resp), "a,b,c");
}

#[test]
fn for_loop_in_handler() {
    let server = ServerProcess::start();
    let resp = ureq::get(&server.url("/for"))
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    assert_eq!(body_string(resp), "60");
}

#[test]
fn while_loop_in_handler() {
    let server = ServerProcess::start();
    let resp = ureq::get(&server.url("/while"))
        .timeout(Duration::from_secs(3))
        .call()
        .unwrap();
    // 0+1+2+3+4 = 10
    assert_eq!(body_string(resp), "10");
}

#[test]
fn closure_in_handler() {
    let server = ServerProcess::start();
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
    let server = ServerProcess::start();
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
    let server = ServerProcess::start();
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
    let server = ServerProcess::start();
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
    let server = ServerProcess::start();
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
    let server = ServerProcess::start();
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
