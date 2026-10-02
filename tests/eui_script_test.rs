//! `eui_window(name, handler, view)`: a script serves its own EUI component.
//!
//! The window needs a display and the `eui-desktop` feature, neither of which
//! a test has, so these run the script with `SOLI_EUI_NO_WINDOW=1`: it serves
//! the session and prints its URL and the cookie the window would present.

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

const COUNTER: &str = r#"
def counter(event_data)
  state = event_data["state"] ?? {}
  count = state["count"] ?? 0
  count += 1 if event_data["event"] == "increment"
  {"count": count}
end

def counter_view(state)
  {"k": "text", "t": str(state["count"] ?? 0), "s": {}}
end

print("before")
eui_window("counter", "counter", "counter_view", {"title": "Counter"})
"#;

struct Served {
    child: Child,
    port: u16,
    cookie: String,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn serve(dir: &std::path::Path) -> Served {
    let mut child = Command::new(env!("CARGO_BIN_EXE_soli"))
        .current_dir(dir)
        .arg("app.sl")
        .env("SOLI_EUI_NO_WINDOW", "1")
        .env("SOLI_EUI_KEY", dir.join("key.pkcs8"))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the script");
    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    let mut next = || lines.next().expect("a line").expect("utf-8");
    assert_eq!(next(), "before", "the script runs before the window opens");
    let url = next();
    let cookie = next();
    let port = url
        .strip_prefix("ws://127.0.0.1:")
        .and_then(|rest| rest.split('/').next())
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(|| panic!("a loopback session URL, got {url:?}"));
    assert!(url.ends_with("/_eui/session/counter"), "{url}");
    assert!(cookie.starts_with("soli_desktop="), "{cookie}");
    Served {
        child,
        port,
        cookie,
    }
}

fn status(port: u16, cookie: Option<&str>) -> u16 {
    let mut request = ureq::get(&format!("http://127.0.0.1:{port}/up"));
    if let Some(cookie) = cookie {
        request = request.set("Cookie", cookie);
    }
    match request.call() {
        Ok(response) => response.status(),
        Err(ureq::Error::Status(code, _)) => code,
        Err(e) => panic!("no answer from the script's server: {e}"),
    }
}

#[test]
fn a_script_serves_its_component_on_loopback_behind_the_gate() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("app.sl"), COUNTER).expect("write");
    let served = serve(dir.path());

    // Only the window, which holds the cookie, gets in.
    let without = status(served.port, None);
    assert!(
        (400..500).contains(&without),
        "without the cookie the server answered {without}"
    );
    let with = status(served.port, Some(&served.cookie));
    assert!(
        !(400..500).contains(&with),
        "with the cookie the server answered {with}"
    );
}

#[test]
fn a_handler_that_is_not_a_def_of_the_script_is_named() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("app.sl"),
        "def view(state)\n  {\"k\": \"text\", \"t\": \"x\", \"s\": {}}\nend\neui_window(\"x\", \"missing\", \"view\")\n",
    )
    .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_soli"))
        .current_dir(dir.path())
        .arg("app.sl")
        .env("SOLI_EUI_NO_WINDOW", "1")
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(70));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("the handler `missing` is not a top-level `def` of the script"),
        "{err}"
    );
}

#[test]
#[cfg_attr(feature = "eui-desktop", ignore = "this build has the window")]
fn without_the_window_the_error_says_how_to_get_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("app.sl"), COUNTER).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_soli"))
        .current_dir(dir.path())
        .arg("app.sl")
        .env_remove("SOLI_EUI_NO_WINDOW")
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(70));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("built without the `eui-desktop` feature"),
        "{err}"
    );
    assert!(err.contains("SOLI_EUI_NO_WINDOW=1"), "{err}");
}

/// With the window built in but no display (an SSH login, a container), the
/// script stops before serving, saying so, instead of failing inside winit
/// with a message that names winit's source file.
#[test]
#[cfg(target_os = "linux")]
#[cfg_attr(not(feature = "eui-desktop"), ignore = "this build has no window")]
fn without_a_display_the_error_says_so() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("app.sl"), COUNTER).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_soli"))
        .current_dir(dir.path())
        .arg("app.sl")
        .env_remove("SOLI_EUI_NO_WINDOW")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("DISPLAY")
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(70));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("there is no display to open the window on"),
        "{err}"
    );
    assert!(err.contains("SOLI_EUI_NO_WINDOW=1"), "{err}");
    assert!(!err.contains("winit"), "{err}");
}
