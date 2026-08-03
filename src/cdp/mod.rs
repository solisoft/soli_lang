//! A headless-browser driver, spoken over the Chrome DevTools Protocol.
//!
//! This exists so `soli test` can drive a real browser without dragging Node,
//! npm and a 300MB Playwright download into the toolchain. Soli ships as one
//! binary; a browser test that needs a package manager would not.
//!
//! Deliberately blocking. The codebase has been bitten repeatedly by running a
//! second async runtime inside a request or a test worker (see the comments in
//! `serve::mod`'s test-server spawn and `solidb_http::block_on`), and a browser
//! session is owned by exactly one test-worker thread anyway. A blocking socket
//! on that thread sidesteps the whole class of problem.
//!
//! Every socket read is bounded by a timeout: a hung browser must fail a test,
//! not wedge the worker that was driving it.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value as Json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

/// How long to wait for the DevTools endpoint after spawning.
const READY_TIMEOUT: Duration = Duration::from_secs(20);
/// How long any single protocol command may take.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
/// Grace period for a polite shutdown before the hard kill.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);
/// Spawn attempts per binary. The reserved port can be taken between probe and
/// bind, and that race is the whole reason for retrying at all.
const SPAWN_ATTEMPTS: usize = 5;
/// How long one binary may soak up before the next candidate gets a turn.
///
/// Sized just above `READY_TIMEOUT` so the two failure shapes get what they
/// deserve: a browser that exits at once is cheap and gets every attempt, while
/// one that never publishes an endpoint gets a single 20-second shot instead of
/// five of them.
const PER_BINARY_BUDGET: Duration = Duration::from_secs(25);
/// Lines of browser stderr kept to explain a launch that failed.
const STDERR_TAIL_LINES: usize = 12;
/// Cap on one kept stderr line, since a browser can log a whole stack trace.
const STDERR_LINE_LIMIT: usize = 300;

/// Distinguishes concurrent profile directories within one process.
static PROFILE_SEQ: AtomicU32 = AtomicU32::new(0);

/// A running browser and an open protocol connection to one page.
pub struct Browser {
    child: Child,
    socket: WebSocket<MaybeTlsStream<std::net::TcpStream>>,
    profile_dir: PathBuf,
    next_id: u64,
    /// Uncaught page exceptions and `console.error` calls, in arrival order.
    /// Accumulated as a side effect of every command, since the protocol
    /// interleaves events with responses.
    page_errors: Vec<String>,
}

/// Why one launch attempt failed, and whether trying the same binary again could
/// plausibly do better.
struct LaunchError {
    message: String,
    /// The port race is real, so most failures are worth another attempt. A
    /// binary that cannot be executed at all is not: retrying it just delays the
    /// candidate that would have worked.
    retryable: bool,
}

impl LaunchError {
    fn retryable(message: String) -> LaunchError {
        LaunchError {
            message,
            retryable: true,
        }
    }

    fn permanent(message: String) -> LaunchError {
        LaunchError {
            message,
            retryable: false,
        }
    }
}

impl Browser {
    /// Launch a browser and attach to its first real page.
    ///
    /// Tries every browser on the machine, retrying each: the port is reserved by
    /// binding and releasing, so another process can take it before the browser
    /// binds. Moving on to the next candidate matters as much as the retry —
    /// a browser can be installed and still be unable to run here (a snap whose
    /// launcher refuses a session without D-Bus exits before Chromium starts),
    /// and retrying that five times only buries the browser that would have
    /// worked.
    pub fn launch(headed: bool) -> Result<Browser, String> {
        let candidates = crate::platform::browser::find_chromes();
        if candidates.is_empty() {
            return Err(no_browser_message());
        }

        let mut failures: Vec<String> = Vec::new();
        for binary in &candidates {
            let deadline = Instant::now() + PER_BINARY_BUDGET;
            let mut failure = None;
            for attempt in 1..=SPAWN_ATTEMPTS {
                match Self::try_launch_once(binary, headed) {
                    Ok(browser) => return Ok(browser),
                    Err(e) => {
                        let give_up =
                            !e.retryable || attempt == SPAWN_ATTEMPTS || Instant::now() >= deadline;
                        failure = Some(e);
                        if give_up {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(150));
                    }
                }
            }
            if let Some(e) = failure {
                failures.push(format!("  {}: {}", binary.display(), e.message));
            }
        }

        Err(format!(
            "the browser failed to start.\n{}\n{}",
            failures.join("\n"),
            launch_hint()
        ))
    }

    fn try_launch_once(binary: &Path, headed: bool) -> Result<Browser, LaunchError> {
        let port = pick_loopback_port().map_err(LaunchError::retryable)?;
        let profile_dir = new_profile_dir().map_err(LaunchError::permanent)?;

        let mut command = Command::new(binary);
        if !headed {
            command.arg("--headless=new");
        }
        command
            .arg(format!("--remote-debugging-port={}", port))
            // Without a dedicated profile a browser the user already has open
            // adopts the launch, and the debugging port is silently ignored —
            // the failure looks like "the endpoint never appeared".
            .arg(format!("--user-data-dir={}", profile_dir.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            // CI containers usually cannot use the sandbox, and this browser
            // only ever loads pages from the test server on loopback.
            .arg("--no-sandbox")
            .arg("--disable-gpu")
            // Background work that would otherwise make timings erratic.
            .arg("--disable-background-timer-throttling")
            .arg("--disable-renderer-backgrounding")
            .arg("--disable-backgrounding-occluded-windows")
            .arg("--disable-dev-shm-usage")
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            // Piped rather than discarded: when a launch fails, the browser has
            // usually already said why on stderr, and throwing that away leaves
            // nothing but an exit status to report.
            .stderr(Stdio::piped());
        arm_parent_death(&mut command);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&profile_dir);
                // A binary that will not exec will not exec on the second try.
                return Err(LaunchError::permanent(format!("cannot start it: {}", e)));
            }
        };
        let stderr = child.stderr.take().map(drain_stderr);

        let target = match wait_for_page_target(&mut child, port) {
            Ok(target) => target,
            Err(e) => return Err(abandon(&mut child, &profile_dir, stderr.as_ref(), e)),
        };

        let socket = match connect(&target) {
            Ok(socket) => socket,
            Err(e) => return Err(abandon(&mut child, &profile_dir, stderr.as_ref(), e)),
        };

        let mut browser = Browser {
            child,
            socket,
            profile_dir,
            next_id: 0,
            page_errors: Vec::new(),
        };

        // Page events drive navigation waits; Runtime events are what make
        // `assert_no_page_errors` possible at all.
        browser
            .send("Page.enable", json!({}))
            .map_err(LaunchError::retryable)?;
        browser
            .send("Runtime.enable", json!({}))
            .map_err(LaunchError::retryable)?;
        browser
            .send("Network.enable", json!({}))
            .map_err(LaunchError::retryable)?;
        Ok(browser)
    }

    /// Send one protocol command and wait for its response.
    ///
    /// Events arriving before the response are drained into `page_errors` rather
    /// than discarded, because that is the only chance to see them.
    pub fn send(&mut self, method: &str, params: Json) -> Result<Json, String> {
        self.next_id += 1;
        let id = self.next_id;
        let payload = json!({ "id": id, "method": method, "params": params }).to_string();

        self.socket
            .send(Message::Text(payload))
            .map_err(|e| format!("cannot send {} to the browser: {}", method, e))?;

        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            if Instant::now() >= deadline {
                return Err(format!("the browser did not answer {} in time", method));
            }
            let message = match self.socket.read() {
                Ok(Message::Text(text)) => text.to_string(),
                Ok(Message::Close(_)) => {
                    return Err("the browser closed the connection".to_string())
                }
                // Ping/pong and binary frames are not part of this protocol.
                Ok(_) => continue,
                Err(e) => return Err(format!("lost the browser connection: {}", e)),
            };

            let parsed: Json = match serde_json::from_str(&message) {
                Ok(parsed) => parsed,
                Err(_) => continue,
            };

            if parsed.get("id").and_then(|v| v.as_u64()) == Some(id) {
                if let Some(error) = parsed.get("error") {
                    let text = error
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown protocol error");
                    return Err(format!("{} failed: {}", method, text));
                }
                return Ok(parsed.get("result").cloned().unwrap_or(Json::Null));
            }

            self.record_event(&parsed);
        }
    }

    /// Evaluate a JavaScript expression in the page and return its value.
    ///
    /// Promises are awaited, so a caller can hand over an `async` expression and
    /// get the settled value.
    pub fn evaluate(&mut self, expression: &str) -> Result<Json, String> {
        let result = self.send(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": true,
            }),
        )?;

        if let Some(details) = result.get("exceptionDetails") {
            let text = details
                .get("exception")
                .and_then(|e| e.get("description"))
                .and_then(|d| d.as_str())
                .or_else(|| details.get("text").and_then(|t| t.as_str()))
                .unwrap_or("evaluation failed");
            return Err(text.to_string());
        }

        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Json::Null))
    }

    /// Navigate and wait until the document has finished parsing.
    ///
    /// Waits on `document.readyState` rather than the protocol's load event: the
    /// load event can fire before an instant-nav or LiveView client has booted,
    /// and a test that races the framework it is testing is worthless.
    pub fn navigate(&mut self, url: &str) -> Result<(), String> {
        let result = self.send("Page.navigate", json!({ "url": url }))?;
        if let Some(error) = result.get("errorText").and_then(|v| v.as_str()) {
            return Err(format!("cannot open {}: {}", url, error));
        }
        self.wait_until("document.readyState === 'complete'", READY_TIMEOUT)
            .map_err(|_| format!("{} did not finish loading", url))
    }

    /// Poll a JavaScript expression until it is truthy.
    ///
    /// Polling rather than an event subscription because the condition is
    /// arbitrary page state, and because an expression that throws while the
    /// page is mid-render must be treated as "not yet", not as a failure.
    pub fn wait_until(&mut self, expression: &str, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(value) = self.evaluate(expression) {
                if is_truthy(&value) {
                    return Ok(());
                }
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "timed out after {}s waiting for: {}",
                    timeout.as_secs(),
                    expression
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Dispatch a real mouse click at a viewport coordinate.
    ///
    /// Real input events, not `element.click()`: the point of driving a browser
    /// is to exercise the same path a user takes, including whether the element
    /// is actually hittable at that position.
    pub fn click_at(&mut self, x: f64, y: f64) -> Result<(), String> {
        for (kind, buttons) in [("mousePressed", 1), ("mouseReleased", 0)] {
            self.send(
                "Input.dispatchMouseEvent",
                json!({
                    "type": kind,
                    "x": x,
                    "y": y,
                    "button": "left",
                    "buttons": buttons,
                    "clickCount": 1,
                }),
            )?;
        }
        Ok(())
    }

    /// Type text into whatever currently has focus.
    pub fn insert_text(&mut self, text: &str) -> Result<(), String> {
        self.send("Input.insertText", json!({ "text": text }))
            .map(|_| ())
    }

    /// Press a named key (`Enter`, `Tab`, `Escape`, …), with modifiers.
    ///
    /// `modifiers` is the protocol's bitmask: Alt 1, Ctrl 2, Meta 4, Shift 8.
    pub fn press_key(&mut self, key: &str, modifiers: u32) -> Result<(), String> {
        let (code, text) = key_details(key);
        for kind in ["keyDown", "keyUp"] {
            let mut params = json!({
                "type": kind,
                "key": key,
                "code": code,
                "modifiers": modifiers,
                "windowsVirtualKeyCode": virtual_key_code(key),
            });
            // Printable keys need `text` on the keydown or the character never
            // reaches the input. A modified chord is a shortcut rather than
            // typing, so it carries no text.
            if kind == "keyDown" && modifiers == 0 {
                if let Some(text) = text {
                    params["text"] = json!(text);
                }
            }
            self.send("Input.dispatchKeyEvent", params)?;
        }
        Ok(())
    }

    /// Emulate a viewport of a given size, pixel density and device class.
    ///
    /// Overrides the metrics rather than resizing the window: the window is
    /// the wrong lever even in headed mode, because the frame, scrollbars and
    /// window manager all take a cut, so the page would end up with a layout
    /// viewport a spec never asked for and cannot predict.
    ///
    /// `mobile` is what makes the difference between "a narrow desktop" and a
    /// phone: it turns on the mobile meta-viewport, and touch is enabled with
    /// it so a page that only binds touch handlers is reachable.
    pub fn set_viewport(
        &mut self,
        width: u32,
        height: u32,
        scale: f64,
        mobile: bool,
    ) -> Result<(), String> {
        self.send(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": width,
                "height": height,
                "deviceScaleFactor": scale,
                "mobile": mobile,
                // Without these, `screen.width` keeps reporting the host's
                // monitor — so a page that sizes itself off `screen` rather
                // than the viewport would see a device the spec never asked
                // for, and headlessly at that.
                "screenWidth": width,
                "screenHeight": height,
            }),
        )?;
        self.send(
            "Emulation.setTouchEmulationEnabled",
            json!({
                "enabled": mobile,
                "maxTouchPoints": if mobile { 5 } else { 1 },
            }),
        )
        .map(|_| ())
    }

    /// Capture the visible page as PNG bytes.
    pub fn screenshot(&mut self) -> Result<Vec<u8>, String> {
        use base64::Engine;
        let result = self.send("Page.captureScreenshot", json!({ "format": "png" }))?;
        let data = result
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "the browser returned no image data".to_string())?;
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| format!("the browser returned an unreadable image: {}", e))
    }

    /// Install a cookie, so a session established over HTTP carries into the page.
    pub fn set_cookie(&mut self, name: &str, value: &str, url: &str) -> Result<(), String> {
        self.send(
            "Network.setCookie",
            json!({
                "name": name,
                "value": value,
                "url": url,
                "path": "/",
                // Chrome rejects some cookies that declare no policy at all, and
                // Lax is what a server-set session cookie would default to.
                "sameSite": "Lax",
            }),
        )
        .map(|_| ())
    }

    /// Read cookies for an origin, as `(name, value)` pairs.
    ///
    /// The origin is explicit because the protocol otherwise answers for the
    /// current page's frames — which is empty before the first navigation, and
    /// silently wrong after a navigation to somewhere else.
    pub fn cookies(&mut self, url: &str) -> Result<Vec<(String, String)>, String> {
        let result = self.send("Network.getCookies", json!({ "urls": [url] }))?;
        let Some(entries) = result.get("cookies").and_then(|v| v.as_array()) else {
            return Ok(Vec::new());
        };
        Ok(entries
            .iter()
            .filter_map(|cookie| {
                let name = cookie.get("name")?.as_str()?.to_string();
                let value = cookie.get("value")?.as_str()?.to_string();
                Some((name, value))
            })
            .collect())
    }

    /// Uncaught exceptions and `console.error` output seen so far.
    pub fn page_errors(&self) -> &[String] {
        &self.page_errors
    }

    /// Forget accumulated page errors, so each test starts clean.
    pub fn clear_page_errors(&mut self) {
        self.page_errors.clear();
    }

    /// Drain any events the browser has already queued.
    ///
    /// Errors are reported by whatever command runs next, so a spec that ends
    /// with an assertion would otherwise never see an exception raised after it.
    pub fn pump_events(&mut self) {
        // A no-op round trip: its response is the barrier that guarantees every
        // event emitted before it has been read.
        let _ = self.send("Runtime.evaluate", json!({ "expression": "0" }));
    }

    fn record_event(&mut self, message: &Json) {
        const MAX_ERRORS: usize = 50;
        let Some(method) = message.get("method").and_then(|v| v.as_str()) else {
            return;
        };
        let params = message.get("params");

        let text = match method {
            "Runtime.exceptionThrown" => params
                .and_then(|p| p.get("exceptionDetails"))
                .and_then(|d| {
                    d.get("exception")
                        .and_then(|e| e.get("description"))
                        .and_then(|v| v.as_str())
                        .or_else(|| d.get("text").and_then(|v| v.as_str()))
                })
                .map(|s| s.to_string()),
            "Runtime.consoleAPICalled" => {
                if params.and_then(|p| p.get("type")).and_then(|v| v.as_str()) != Some("error") {
                    return;
                }
                params
                    .and_then(|p| p.get("args"))
                    .and_then(|v| v.as_array())
                    .map(|args| {
                        args.iter()
                            .map(|arg| {
                                arg.get("value")
                                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                                    .or_else(|| {
                                        arg.get("description")
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string())
                                    })
                                    .unwrap_or_default()
                            })
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
            }
            _ => return,
        };

        if let Some(text) = text {
            // Bounded: a page erroring in a render loop would otherwise grow
            // this without limit for as long as the test runs.
            if self.page_errors.len() < MAX_ERRORS {
                self.page_errors.push(text);
            }
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.socket.close(None);
        shutdown_child(&mut self.child, SHUTDOWN_GRACE);
        // Profiles are per-session and can reach tens of megabytes; leaving them
        // behind would fill the temp directory over a suite's worth of runs.
        let _ = std::fs::remove_dir_all(&self.profile_dir);
    }
}

/// The message shown when no browser could be found.
///
/// Names the override explicitly: "install Chrome" is not actionable for someone
/// who has one in a place we did not look.
fn no_browser_message() -> String {
    if let Some(path) = crate::platform::browser::chrome_path_override() {
        return format!(
            "{} points at '{}', which is not an executable file.",
            crate::platform::browser::CHROME_PATH_ENV,
            path
        );
    }
    format!(
        "no Chrome or Chromium found — browser tests need one.\n\
         Looked for: {}\n\
         Set {} to use a browser from somewhere else.",
        crate::platform::browser::CHROMIUM_BINARIES.join(", "),
        crate::platform::browser::CHROME_PATH_ENV,
    )
}

/// What to try next when every browser on the machine refused to start.
fn launch_hint() -> String {
    match crate::platform::browser::chrome_path_override() {
        Some(path) => format!(
            "{} pins the browser to '{}' — unset it to try the others on this machine.",
            crate::platform::browser::CHROME_PATH_ENV,
            path
        ),
        None => format!(
            "Set {} to a browser that can run here.",
            crate::platform::browser::CHROME_PATH_ENV
        ),
    }
}

/// Tear down a half-started browser and turn its failure into a report.
///
/// Whatever the browser managed to say before dying goes into the message: on a
/// snap-confined Chromium in a session without D-Bus, that one line is the whole
/// diagnosis, and the exit status on its own says nothing at all.
fn abandon(
    child: &mut Child,
    profile_dir: &Path,
    stderr: Option<&Arc<Mutex<VecDeque<String>>>>,
    message: String,
) -> LaunchError {
    kill_child(child);
    let _ = std::fs::remove_dir_all(profile_dir);

    let said = stderr.map(recent_stderr).unwrap_or_default();
    if said.is_empty() {
        return LaunchError::retryable(message);
    }
    let quoted = said
        .iter()
        .map(|line| format!("    {}", line))
        .collect::<Vec<_>>()
        .join("\n");
    LaunchError::retryable(format!("{}\n{}", message, quoted))
}

/// Drain a child's stderr into a bounded tail, on a thread.
///
/// Both halves of that matter. The pipe has to be read or it fills and stops the
/// browser dead — Edge logs a D-Bus error every few frames on a host without a
/// session bus — and the last lines have to be kept somewhere a failing launch
/// can reach them.
///
/// Detached rather than joined: Chromium's forked children inherit the pipe, so
/// EOF can lag well behind the process we killed, and a test reporting a failure
/// must not wait on that.
fn drain_stderr(stderr: ChildStderr) -> Arc<Mutex<VecDeque<String>>> {
    let tail = Arc::new(Mutex::new(VecDeque::new()));
    let sink = Arc::clone(&tail);
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut sink = sink.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if sink.len() == STDERR_TAIL_LINES {
                sink.pop_front();
            }
            sink.push_back(shorten(line));
        }
    });
    tail
}

/// The stderr lines kept so far, oldest first.
fn recent_stderr(tail: &Arc<Mutex<VecDeque<String>>>) -> Vec<String> {
    let lines = tail.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    lines.iter().cloned().collect()
}

/// Clip one log line, on a character boundary.
fn shorten(line: &str) -> String {
    if line.chars().count() <= STDERR_LINE_LIMIT {
        return line.to_string();
    }
    let kept: String = line.chars().take(STDERR_LINE_LIMIT).collect();
    format!("{}…", kept)
}

/// Reserve a free loopback port by binding and immediately releasing it.
fn pick_loopback_port() -> Result<u16, String> {
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .map_err(|e| format!("cannot reserve a loopback port: {}", e))?;
    listener
        .local_addr()
        .map(|addr| addr.port())
        .map_err(|e| format!("cannot read the reserved port: {}", e))
}

/// Create a private profile directory for one browser session.
///
/// Under the temp directory rather than the home directory: snap-confined
/// browsers cannot write to hidden directories in `$HOME` (a `~/.cache/...`
/// profile fails with a bare "Permission denied" on the singleton lock), and
/// the temp directory is writable under every packaging we have met.
fn new_profile_dir() -> Result<PathBuf, String> {
    let sequence = PROFILE_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("soli-browser-{}-{}", std::process::id(), sequence));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| {
        format!(
            "cannot create a browser profile at {}: {}",
            dir.display(),
            e
        )
    })?;
    Ok(dir)
}

/// Poll the DevTools endpoint until a real page target appears.
///
/// Checks the child first on every pass: a browser that died on startup should
/// report that, not spend the whole timeout waiting for an endpoint that is
/// never coming.
fn wait_for_page_target(child: &mut Child, port: u16) -> Result<String, String> {
    let deadline = Instant::now() + READY_TIMEOUT;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("the browser exited during startup ({})", status));
        }
        if let Some(url) = fetch_page_target(port) {
            return Ok(url);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "the browser's DevTools endpoint never came up on port {}",
                port
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Ask the endpoint for a driveable page.
///
/// Skips extension background pages and DevTools' own targets: a fresh profile
/// still lists component-extension pages, and attaching to one of those means
/// every later command runs against the wrong document.
fn fetch_page_target(port: u16) -> Option<String> {
    let response = ureq::get(&format!("http://127.0.0.1:{}/json/list", port))
        .timeout(Duration::from_secs(2))
        .call()
        .ok()?;
    let targets: Json = response.into_json().ok()?;

    targets.as_array()?.iter().find_map(|target| {
        if target.get("type")?.as_str()? != "page" {
            return None;
        }
        let url = target.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if url.starts_with("chrome-extension://") || url.starts_with("devtools://") {
            return None;
        }
        Some(target.get("webSocketDebuggerUrl")?.as_str()?.to_string())
    })
}

/// Open the protocol socket, with reads bounded by a timeout.
fn connect(url: &str) -> Result<WebSocket<MaybeTlsStream<std::net::TcpStream>>, String> {
    let (socket, _response) =
        tungstenite::connect(url).map_err(|e| format!("cannot attach to the browser: {}", e))?;

    // An unbounded read is the difference between a failing test and a test
    // worker that never returns, because a wedged browser simply stops sending.
    if let MaybeTlsStream::Plain(stream) = socket.get_ref() {
        stream
            .set_read_timeout(Some(COMMAND_TIMEOUT))
            .map_err(|e| format!("cannot bound the browser connection: {}", e))?;
    }
    Ok(socket)
}

/// Ask a child to stop, escalating to a hard kill.
fn shutdown_child(child: &mut Child, grace: Duration) {
    if !matches!(child.try_wait(), Ok(None)) {
        // Already exited; reap it so it does not linger as a zombie.
        let _ = child.wait();
        return;
    }

    #[cfg(unix)]
    {
        // SAFETY: this is our own child's pid, and it has not been reaped.
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
        }
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if !matches!(child.try_wait(), Ok(None)) {
                let _ = child.wait();
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    #[cfg(not(unix))]
    let _ = grace;

    kill_child(child);
}

fn kill_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// On Linux, ask the kernel to kill the browser when its parent thread dies.
///
/// The signal fires when the *spawning thread* exits rather than the process,
/// which is exactly right here: a browser belongs to one test worker, so it
/// should not outlive that worker even if the runner is killed outright.
#[cfg(target_os = "linux")]
fn arm_parent_death(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: `prctl` is async-signal-safe and this closure runs in the child
    // between fork and exec, where only such calls are permitted.
    unsafe {
        command.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
}

#[cfg(not(target_os = "linux"))]
fn arm_parent_death(_command: &mut Command) {}

/// JavaScript truthiness, for values that crossed the protocol as JSON.
fn is_truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(b) => *b,
        Json::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
        Json::String(s) => !s.is_empty(),
        Json::Array(_) | Json::Object(_) => true,
    }
}

/// The `code` and printable `text` for a key name.
fn key_details(key: &str) -> (&str, Option<&str>) {
    match key {
        "Enter" => ("Enter", Some("\r")),
        "Tab" => ("Tab", Some("\t")),
        "Backspace" => ("Backspace", None),
        "Delete" => ("Delete", None),
        "Escape" => ("Escape", None),
        "ArrowUp" => ("ArrowUp", None),
        "ArrowDown" => ("ArrowDown", None),
        "ArrowLeft" => ("ArrowLeft", None),
        "ArrowRight" => ("ArrowRight", None),
        " " | "Space" => ("Space", Some(" ")),
        other => (other, None),
    }
}

/// Legacy virtual key codes, which some frameworks still read off the event.
fn virtual_key_code(key: &str) -> u32 {
    match key {
        "Enter" => 13,
        "Tab" => 9,
        "Backspace" => 8,
        "Delete" => 46,
        "Escape" => 27,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        " " | "Space" => 32,
        _ => 0,
    }
}

/// Read a whole stream, used only by tests that inspect a spawned browser.
#[allow(dead_code)]
fn drain(mut source: impl Read) -> String {
    let mut buffer = String::new();
    let _ = source.read_to_string(&mut buffer);
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthiness_matches_javascript() {
        assert!(!is_truthy(&Json::Null));
        assert!(!is_truthy(&json!(false)));
        assert!(!is_truthy(&json!(0)));
        assert!(!is_truthy(&json!("")));
        assert!(is_truthy(&json!(true)));
        assert!(is_truthy(&json!(1)));
        assert!(is_truthy(&json!("x")));
        // Empty array and object are truthy in JavaScript, unlike in most
        // languages — a selector count of `[]` must not read as "absent".
        assert!(is_truthy(&json!([])));
        assert!(is_truthy(&json!({})));
    }

    #[test]
    fn each_profile_directory_is_distinct() {
        let first = new_profile_dir().expect("temp dir must be writable");
        let second = new_profile_dir().expect("temp dir must be writable");
        assert_ne!(first, second);
        let _ = std::fs::remove_dir_all(&first);
        let _ = std::fs::remove_dir_all(&second);
    }

    #[test]
    fn printable_keys_carry_text_and_others_do_not() {
        assert_eq!(key_details("Enter"), ("Enter", Some("\r")));
        assert_eq!(key_details("Escape"), ("Escape", None));
    }

    #[test]
    fn a_reserved_port_is_usable() {
        let port = pick_loopback_port().expect("loopback must be bindable");
        assert!(port > 0);
    }

    /// A stand-in for a browser that dies on startup with something to say —
    /// which is what a snap launcher refusing the session actually looks like.
    #[cfg(unix)]
    fn dying_process(said: &str) -> Child {
        Command::new("sh")
            .arg("-c")
            .arg(format!("echo '{}' >&2; exit 1", said))
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("sh must be spawnable")
    }

    #[cfg(unix)]
    #[test]
    fn what_the_browser_said_before_dying_reaches_the_failure() {
        let mut child = dying_process("not a snap cgroup for tag snap.chromium.chromium");
        let stderr = child.stderr.take().map(drain_stderr);
        let _ = child.wait();

        // The drain runs on its own thread, so the line can land just after the
        // child is reaped.
        let deadline = Instant::now() + Duration::from_secs(5);
        while stderr
            .as_ref()
            .map(recent_stderr)
            .unwrap_or_default()
            .is_empty()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }

        let profile = new_profile_dir().expect("temp dir must be writable");
        let failure = abandon(
            &mut child,
            &profile,
            stderr.as_ref(),
            "the browser exited during startup (exit status: 1)".to_string(),
        );

        assert!(failure.message.contains("exited during startup"));
        // The point of the whole exercise: the reason, not just the status.
        assert!(
            failure.message.contains("not a snap cgroup"),
            "stderr was dropped: {}",
            failure.message
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_kept_stderr_is_bounded() {
        let lines = STDERR_TAIL_LINES * 3;
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "for i in $(seq {}); do echo line$i >&2; done",
                lines
            ))
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("sh must be spawnable");
        let tail = drain_stderr(child.stderr.take().expect("stderr was piped"));
        let _ = child.wait();

        // The drain runs on its own thread, so wait for the last line to land.
        // Waiting for the ring to merely be *full* would race it: under a loaded
        // machine the first twelve lines fill it before the rest are consumed.
        let last = format!("line{}", lines);
        let deadline = Instant::now() + Duration::from_secs(10);
        while recent_stderr(&tail).last().map(String::as_str) != Some(last.as_str()) {
            assert!(
                Instant::now() < deadline,
                "the drain never reached {}: {:?}",
                last,
                recent_stderr(&tail)
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        let kept = recent_stderr(&tail);
        assert_eq!(kept.len(), STDERR_TAIL_LINES);
        // The oldest lines are the ones dropped: a failure is explained by what
        // the browser said last, not by the first thing it logged.
        assert_eq!(
            kept.first().map(String::as_str),
            Some(&format!("line{}", lines - STDERR_TAIL_LINES + 1)[..])
        );
    }

    #[test]
    fn a_long_log_line_is_clipped_on_a_character_boundary() {
        let long = "é".repeat(STDERR_LINE_LIMIT * 2);
        let clipped = shorten(&long);
        assert_eq!(clipped.chars().count(), STDERR_LINE_LIMIT + 1);
        assert!(clipped.ends_with('…'));
        assert_eq!(shorten("short"), "short");
    }

    #[test]
    fn one_binary_cannot_soak_up_every_attempt() {
        // Otherwise a browser that hangs to the ready timeout costs
        // SPAWN_ATTEMPTS × READY_TIMEOUT before the next candidate is tried.
        assert!(PER_BINARY_BUDGET < READY_TIMEOUT * SPAWN_ATTEMPTS as u32);
        assert!(PER_BINARY_BUDGET > READY_TIMEOUT);
    }
}
