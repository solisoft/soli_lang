//! Browser-driving test helpers — Soli's answer to Rails system tests.
//!
//! These sit on top of `crate::cdp` and give specs a vocabulary close to the
//! one they already use for HTTP: bare verbs for actions, `assert_*` for
//! assertions, and the same test server the request helpers talk to.
//!
//! One browser per test-worker thread, launched on first use. Lazy because most
//! suites have no browser specs at all and should not pay a browser launch to
//! find that out; per-thread because `Value` is `Rc`-based and cannot cross
//! threads, and because each worker already owns its own server subprocess.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::json;

use crate::cdp::Browser;
use crate::interpreter::builtins::assertions::increment_assertion_count;
use crate::interpreter::builtins::request_helpers::{current_cookies, set_cookie_inner};
use crate::interpreter::builtins::test_server::get_test_server_port;
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

/// Whether browser specs may run. Armed by `soli test --browser`.
static BROWSER_ENABLED: AtomicBool = AtomicBool::new(false);
/// Whether to show the browser window. Armed by `soli test --headed`.
static HEADED: AtomicBool = AtomicBool::new(false);

/// Default patience for anything that waits on the page.
const DEFAULT_WAIT: Duration = Duration::from_secs(10);

thread_local! {
    /// This worker's browser. `None` until the first helper needs it.
    static BROWSER: RefCell<Option<Browser>> = const { RefCell::new(None) };
}

/// Allow browser helpers to launch a browser.
pub fn enable_browser_tests() {
    BROWSER_ENABLED.store(true, Ordering::SeqCst);
}

/// Show the browser window instead of running headless.
pub fn enable_headed() {
    HEADED.store(true, Ordering::SeqCst);
}

/// Whether `--browser` was passed.
pub fn browser_tests_enabled() -> bool {
    BROWSER_ENABLED.load(Ordering::SeqCst)
}

/// Close this thread's browser, if it has one.
///
/// Called between test files and at worker shutdown. The `Drop` impl would
/// eventually do this anyway, but "eventually" means "when the thread ends",
/// and a suite should not accumulate one browser per file.
pub fn close_browser() {
    BROWSER.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

/// Return the browser to a clean slate between tests.
///
/// The browser deliberately outlives a single test — relaunching one per test
/// would dominate the runtime — so anything it accumulated has to be cleared
/// explicitly, or tests stop being independent. Two things accumulate:
///
/// - Page errors, which would otherwise fail every test after the one that
///   caused them.
/// - `sessionStorage` and `localStorage`, which survive navigation by design.
///   A test that hides a panel would leave it hidden for the rest of the suite,
///   making results depend on test order.
///
/// Cookies are left alone: they are shared with the HTTP jar, where the
/// established convention is that specs manage sign-in themselves (`logout()`
/// in a `before_each`). Clearing them here would silently break that.
///
/// Costs nothing when the thread has no browser, which is every worker in a
/// suite that has no browser specs.
pub fn reset_browser_state() {
    BROWSER.with(|cell| {
        if let Some(browser) = cell.borrow_mut().as_mut() {
            browser.clear_page_errors();
            // Fails harmlessly before the first navigation, when there is no
            // document to have storage on.
            let _ = browser.evaluate(
                "(function () { try { sessionStorage.clear(); localStorage.clear(); } \
                 catch (e) {} return true; })()",
            );
        }
    });
}

/// The origin this worker's test server is listening on.
fn base_url() -> Result<String, String> {
    get_test_server_port()
        .map(|port| format!("http://127.0.0.1:{}", port))
        .ok_or_else(|| {
            "No test server is running, so there is nothing for the browser to visit.".to_string()
        })
}

/// Run `body` against this thread's browser, launching one if needed.
fn with_browser<T>(body: impl FnOnce(&mut Browser) -> Result<T, String>) -> Result<T, String> {
    if !browser_tests_enabled() {
        return Err(
            "Browser helpers need a browser: run this spec with `soli test --browser`.".to_string(),
        );
    }

    BROWSER.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(Browser::launch(HEADED.load(Ordering::SeqCst))?);
        }
        let browser = slot.as_mut().expect("just launched");
        body(browser)
    })
}

/// Copy the HTTP cookie jar into the browser.
///
/// This is what makes `login()` / `as_user(id, opts)` carry into `visit()`. The
/// jar holds only `name=value` — every attribute was dropped when the response
/// was parsed — so path and same-site policy are synthesized here.
fn push_cookies(browser: &mut Browser, origin: &str) -> Result<(), String> {
    for pair in current_cookies().split(';') {
        let pair = pair.trim();
        if pair.is_empty() {
            continue;
        }
        if let Some((name, value)) = pair.split_once('=') {
            browser.set_cookie(name.trim(), value.trim(), origin)?;
        }
    }
    Ok(())
}

/// Copy the browser's cookies back into the HTTP jar.
///
/// Without this a login performed by clicking a form would be invisible to a
/// later `get()` or `signed_in()`, and the two halves of a spec would quietly
/// disagree about who is signed in.
fn pull_cookies(browser: &mut Browser, origin: &str) -> Result<(), String> {
    for (name, value) in browser.cookies(origin)? {
        set_cookie_inner(name, value);
    }
    Ok(())
}

/// A JavaScript function that resolves a Soli selector to an element.
///
/// Specs should not have to know whether a field is addressed by CSS, by its
/// label, or by its name — Capybara set that expectation and it is the right
/// one. Order matters: CSS first so an explicit selector is never second-guessed.
const FIND_JS: &str = r#"function (s) {
  var el = null;
  try { el = document.querySelector(s); } catch (e) { el = null; }
  if (el) return el;
  var labels = Array.prototype.slice.call(document.querySelectorAll('label'));
  for (var i = 0; i < labels.length; i++) {
    if (labels[i].textContent.trim() === s) {
      var forId = labels[i].getAttribute('for');
      if (forId) {
        var target = document.getElementById(forId);
        if (target) return target;
      }
      var nested = labels[i].querySelector('input, textarea, select');
      if (nested) return nested;
    }
  }
  var fields = Array.prototype.slice.call(
    document.querySelectorAll('input, textarea, select, button')
  );
  for (var j = 0; j < fields.length; j++) {
    if (fields[j].name === s || fields[j].placeholder === s) return fields[j];
  }
  return null;
}"#;

/// A JavaScript function that resolves clickable text to an element.
const FIND_CLICKABLE_JS: &str = r#"function (s) {
  var el = null;
  try { el = document.querySelector(s); } catch (e) { el = null; }
  if (el) return el;
  var candidates = Array.prototype.slice.call(
    document.querySelectorAll('a, button, input[type=submit], input[type=button], [role=button]')
  );
  for (var i = 0; i < candidates.length; i++) {
    var node = candidates[i];
    var label = (node.value || node.textContent || '').trim();
    if (label === s) return node;
  }
  for (var j = 0; j < candidates.length; j++) {
    var alt = candidates[j];
    var altLabel = (alt.value || alt.textContent || '').trim();
    if (altLabel.indexOf(s) !== -1) return alt;
  }
  return null;
}"#;

/// Build an expression yielding the centre point of `selector`, or null.
fn center_expr(finder: &str, selector: &str) -> String {
    format!(
        "(function () {{ var el = ({})({}); if (!el) return null; \
         el.scrollIntoView({{block: 'center', inline: 'center'}}); \
         var r = el.getBoundingClientRect(); \
         if (r.width === 0 && r.height === 0) return null; \
         return [r.left + r.width / 2, r.top + r.height / 2]; }})()",
        finder,
        json!(selector)
    )
}

/// Build an expression that is true when `selector` matches something.
fn exists_expr(selector: &str) -> String {
    format!("(({})({}) !== null)", FIND_JS, json!(selector))
}

/// Build an expression that is true when the page shows `text`.
fn text_expr(text: &str) -> String {
    format!(
        "((document.body ? document.body.innerText : '').indexOf({}) !== -1)",
        json!(text)
    )
}

/// Shorten a value for a failure message.
///
/// A page's text runs to kilobytes; an assertion that dumps all of it buries
/// the failure it was supposed to report.
fn brief(text: &str) -> String {
    const LIMIT: usize = 200;
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= LIMIT {
        return collapsed;
    }
    let head: String = collapsed.chars().take(LIMIT).collect();
    format!("{}… ({} chars)", head, collapsed.chars().count())
}

/// Read the page's visible text.
fn page_text(browser: &mut Browser) -> Result<String, String> {
    Ok(browser
        .evaluate("document.body ? document.body.innerText : ''")?
        .as_str()
        .unwrap_or_default()
        .to_string())
}

/// The first argument as a string, or a typed error naming the helper.
fn string_arg(args: &[Value], index: usize, helper: &str) -> Result<String, String> {
    match args.get(index) {
        Some(Value::String(s)) => Ok(s.to_string()),
        Some(other) => Err(format!(
            "{}() expects a string, got {}",
            helper,
            other.type_name()
        )),
        None => Err(format!("{}() is missing an argument", helper)),
    }
}

/// Optional timeout from a trailing options hash, in seconds.
fn wait_timeout(args: &[Value]) -> Duration {
    for arg in args {
        if let Value::Hash(hash) = arg {
            let borrowed = hash.borrow();
            let key = crate::interpreter::value::HashKey::String("timeout".into());
            if let Some(value) = borrowed.get(&key) {
                let seconds = match value {
                    Value::Int(n) => *n as f64,
                    Value::Float(f) => *f,
                    _ => continue,
                };
                if seconds > 0.0 {
                    return Duration::from_millis((seconds * 1000.0) as u64);
                }
            }
        }
    }
    DEFAULT_WAIT
}

pub fn register_browser_helpers(env: &mut Environment) {
    // --- navigation -------------------------------------------------------

    env.define(
        "visit".to_string(),
        Value::NativeFunction(NativeFunction::new("visit", None, |args| {
            let path = string_arg(&args, 0, "visit")?;
            let origin = base_url()?;
            // Relative paths are the common case and keep specs portable across
            // the ephemeral ports each worker gets.
            let url = if path.starts_with("http://") || path.starts_with("https://") {
                path
            } else {
                format!("{}{}", origin, path)
            };
            with_browser(|browser| {
                push_cookies(browser, &origin)?;
                browser.navigate(&url)?;
                pull_cookies(browser, &origin)?;
                Ok(Value::Null)
            })
        })),
    );

    env.define(
        "page_path".to_string(),
        Value::NativeFunction(NativeFunction::new("page_path", Some(0), |_args| {
            with_browser(|browser| {
                let path = browser.evaluate("location.pathname + location.search")?;
                Ok(Value::String(
                    path.as_str().unwrap_or_default().to_string().into(),
                ))
            })
        })),
    );

    env.define(
        "page_url".to_string(),
        Value::NativeFunction(NativeFunction::new("page_url", Some(0), |_args| {
            with_browser(|browser| {
                let url = browser.evaluate("location.href")?;
                Ok(Value::String(
                    url.as_str().unwrap_or_default().to_string().into(),
                ))
            })
        })),
    );

    env.define(
        "page_title".to_string(),
        Value::NativeFunction(NativeFunction::new("page_title", Some(0), |_args| {
            with_browser(|browser| {
                let title = browser.evaluate("document.title")?;
                Ok(Value::String(
                    title.as_str().unwrap_or_default().to_string().into(),
                ))
            })
        })),
    );

    env.define(
        "page_text".to_string(),
        Value::NativeFunction(NativeFunction::new("page_text", Some(0), |_args| {
            with_browser(|browser| Ok(Value::String(page_text(browser)?.into())))
        })),
    );

    env.define(
        "page_html".to_string(),
        Value::NativeFunction(NativeFunction::new("page_html", Some(0), |_args| {
            with_browser(|browser| {
                let html = browser.evaluate("document.documentElement.outerHTML")?;
                Ok(Value::String(
                    html.as_str().unwrap_or_default().to_string().into(),
                ))
            })
        })),
    );

    // --- interaction ------------------------------------------------------

    env.define(
        "click".to_string(),
        Value::NativeFunction(NativeFunction::new("click", None, |args| {
            let selector = string_arg(&args, 0, "click")?;
            let origin = base_url()?;
            with_browser(|browser| {
                click_selector(browser, FIND_CLICKABLE_JS, &selector, "click")?;
                pull_cookies(browser, &origin)?;
                Ok(Value::Null)
            })
        })),
    );

    env.define(
        "click_link".to_string(),
        Value::NativeFunction(NativeFunction::new("click_link", None, |args| {
            let selector = string_arg(&args, 0, "click_link")?;
            let origin = base_url()?;
            with_browser(|browser| {
                click_selector(browser, FIND_CLICKABLE_JS, &selector, "click_link")?;
                pull_cookies(browser, &origin)?;
                Ok(Value::Null)
            })
        })),
    );

    env.define(
        "click_button".to_string(),
        Value::NativeFunction(NativeFunction::new("click_button", None, |args| {
            let selector = string_arg(&args, 0, "click_button")?;
            let origin = base_url()?;
            with_browser(|browser| {
                click_selector(browser, FIND_CLICKABLE_JS, &selector, "click_button")?;
                pull_cookies(browser, &origin)?;
                Ok(Value::Null)
            })
        })),
    );

    env.define(
        "fill_in".to_string(),
        Value::NativeFunction(NativeFunction::new("fill_in", None, |args| {
            let selector = string_arg(&args, 0, "fill_in")?;
            let text = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                Some(Value::Int(n)) => n.to_string(),
                Some(Value::Float(f)) => f.to_string(),
                Some(Value::Null) | None => String::new(),
                Some(other) => {
                    return Err(format!(
                        "fill_in() expects a string value, got {}",
                        other.type_name()
                    ))
                }
            };
            with_browser(|browser| {
                // Focus and clear through the DOM, then type through the input
                // pipeline: clearing by keystroke would need a key per existing
                // character, while typing by assignment skips the events that
                // frameworks listen for.
                let focused = browser.evaluate(&format!(
                    "(function () {{ var el = ({})({}); if (!el) return false; \
                     el.focus(); el.value = ''; return true; }})()",
                    FIND_JS,
                    json!(selector)
                ))?;
                if focused.as_bool() != Some(true) {
                    return Err(format!("fill_in() found no field matching {:?}", selector));
                }
                browser.insert_text(&text)?;
                // `input` fires from the key pipeline, but `change` only fires
                // on blur — which a spec that never leaves the field would miss.
                browser.evaluate(&format!(
                    "(function () {{ var el = ({})({}); if (el) \
                     el.dispatchEvent(new Event('change', {{bubbles: true}})); }})()",
                    FIND_JS,
                    json!(selector)
                ))?;
                Ok(Value::Null)
            })
        })),
    );

    env.define(
        "select_option".to_string(),
        Value::NativeFunction(NativeFunction::new("select_option", None, |args| {
            let selector = string_arg(&args, 0, "select_option")?;
            let option = string_arg(&args, 1, "select_option")?;
            with_browser(|browser| {
                let ok = browser.evaluate(&format!(
                    "(function () {{ var el = ({})({}); if (!el) return false; \
                     var wanted = {}; var found = false; \
                     for (var i = 0; i < el.options.length; i++) {{ \
                       var opt = el.options[i]; \
                       if (opt.value === wanted || opt.textContent.trim() === wanted) {{ \
                         el.selectedIndex = i; found = true; break; }} }} \
                     if (!found) return false; \
                     el.dispatchEvent(new Event('input', {{bubbles: true}})); \
                     el.dispatchEvent(new Event('change', {{bubbles: true}})); \
                     return true; }})()",
                    FIND_JS,
                    json!(selector),
                    json!(option)
                ))?;
                if ok.as_bool() != Some(true) {
                    return Err(format!(
                        "select_option() found no option {:?} in {:?}",
                        option, selector
                    ));
                }
                Ok(Value::Null)
            })
        })),
    );

    for (name, should_check) in [("check", true), ("uncheck", false)] {
        env.define(
            name.to_string(),
            Value::NativeFunction(NativeFunction::new(name, None, move |args| {
                let selector = string_arg(&args, 0, name)?;
                with_browser(|browser| {
                    let ok = browser.evaluate(&format!(
                        "(function () {{ var el = ({})({}); if (!el) return false; \
                         if (el.checked !== {}) el.click(); return true; }})()",
                        FIND_JS,
                        json!(selector),
                        should_check
                    ))?;
                    if ok.as_bool() != Some(true) {
                        return Err(format!("{}() found no box matching {:?}", name, selector));
                    }
                    Ok(Value::Null)
                })
            })),
        );
    }

    env.define(
        "choose".to_string(),
        Value::NativeFunction(NativeFunction::new("choose", None, |args| {
            let selector = string_arg(&args, 0, "choose")?;
            with_browser(|browser| {
                click_selector(browser, FIND_JS, &selector, "choose")?;
                Ok(Value::Null)
            })
        })),
    );

    env.define(
        "press".to_string(),
        Value::NativeFunction(NativeFunction::new("press", None, |args| {
            let chord = string_arg(&args, 0, "press")?;
            let (key, modifiers) = parse_chord(&chord);
            with_browser(|browser| {
                browser.press_key(&key, modifiers)?;
                Ok(Value::Null)
            })
        })),
    );

    env.define(
        "evaluate".to_string(),
        Value::NativeFunction(NativeFunction::new("evaluate", None, |args| {
            let expression = string_arg(&args, 0, "evaluate")?;
            with_browser(|browser| {
                let result = browser.evaluate(&expression)?;
                Ok(json_to_soli(result))
            })
        })),
    );

    env.define(
        "screenshot".to_string(),
        Value::NativeFunction(NativeFunction::new("screenshot", None, |args| {
            let path = string_arg(&args, 0, "screenshot")?;
            with_browser(|browser| {
                let image = browser.screenshot()?;
                std::fs::write(&path, image)
                    .map_err(|e| format!("cannot write {}: {}", path, e))?;
                Ok(Value::String(path.clone().into()))
            })
        })),
    );

    // --- waiting ----------------------------------------------------------

    env.define(
        "wait_for".to_string(),
        Value::NativeFunction(NativeFunction::new("wait_for", None, |args| {
            let selector = string_arg(&args, 0, "wait_for")?;
            let timeout = wait_timeout(&args);
            with_browser(|browser| {
                browser.wait_until(&exists_expr(&selector), timeout)?;
                Ok(Value::Null)
            })
        })),
    );

    env.define(
        "wait_for_text".to_string(),
        Value::NativeFunction(NativeFunction::new("wait_for_text", None, |args| {
            let text = string_arg(&args, 0, "wait_for_text")?;
            let timeout = wait_timeout(&args);
            with_browser(|browser| {
                browser.wait_until(&text_expr(&text), timeout)?;
                Ok(Value::Null)
            })
        })),
    );

    // --- assertions -------------------------------------------------------
    //
    // Each waits before failing. A browser test that asserts the instant after
    // an action is a race, and making every spec write its own wait would make
    // the fast path (already true) indistinguishable from the flaky one.

    env.define(
        "assert_text".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_text", None, |args| {
            let expected = string_arg(&args, 0, "assert_text")?;
            let timeout = wait_timeout(&args);
            with_browser(|browser| {
                if browser.wait_until(&text_expr(&expected), timeout).is_ok() {
                    increment_assertion_count();
                    return Ok(Value::Int(1));
                }
                let actual = page_text(browser).unwrap_or_default();
                Err(format!(
                    "expected the page to show {:?}, but it shows: {}",
                    expected,
                    brief(&actual)
                ))
            })
        })),
    );

    env.define(
        "assert_no_text".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_no_text", None, |args| {
            let unexpected = string_arg(&args, 0, "assert_no_text")?;
            with_browser(|browser| {
                let present = browser.evaluate(&text_expr(&unexpected))?;
                if present.as_bool() == Some(true) {
                    return Err(format!(
                        "expected the page not to show {:?}, but it does",
                        unexpected
                    ));
                }
                increment_assertion_count();
                Ok(Value::Int(1))
            })
        })),
    );

    env.define(
        "assert_selector".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_selector", None, |args| {
            let selector = string_arg(&args, 0, "assert_selector")?;
            let timeout = wait_timeout(&args);
            with_browser(|browser| {
                if browser.wait_until(&exists_expr(&selector), timeout).is_ok() {
                    increment_assertion_count();
                    return Ok(Value::Int(1));
                }
                Err(format!("expected {:?} to be on the page", selector))
            })
        })),
    );

    env.define(
        "assert_no_selector".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_no_selector", None, |args| {
            let selector = string_arg(&args, 0, "assert_no_selector")?;
            with_browser(|browser| {
                let present = browser.evaluate(&exists_expr(&selector))?;
                if present.as_bool() == Some(true) {
                    return Err(format!("expected {:?} not to be on the page", selector));
                }
                increment_assertion_count();
                Ok(Value::Int(1))
            })
        })),
    );

    env.define(
        "assert_page_path".to_string(),
        Value::NativeFunction(NativeFunction::new("assert_page_path", None, |args| {
            let expected = string_arg(&args, 0, "assert_page_path")?;
            let timeout = wait_timeout(&args);
            with_browser(|browser| {
                let condition = format!("(location.pathname === {})", json!(expected));
                if browser.wait_until(&condition, timeout).is_ok() {
                    increment_assertion_count();
                    return Ok(Value::Int(1));
                }
                let actual = browser
                    .evaluate("location.pathname")?
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                Err(format!(
                    "expected to be at {:?}, but the browser is at {:?}",
                    expected, actual
                ))
            })
        })),
    );

    env.define(
        "assert_no_page_errors".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "assert_no_page_errors",
            Some(0),
            |_args| {
                with_browser(|browser| {
                    // Events arrive asynchronously, so an error thrown by the
                    // action just performed may still be in flight.
                    browser.pump_events();
                    let errors = browser.page_errors();
                    if errors.is_empty() {
                        increment_assertion_count();
                        return Ok(Value::Int(1));
                    }
                    Err(format!(
                        "the page reported {} JavaScript error(s):\n  {}",
                        errors.len(),
                        errors
                            .iter()
                            .map(|e| brief(e))
                            .collect::<Vec<_>>()
                            .join("\n  ")
                    ))
                })
            },
        )),
    );

    env.define(
        "page_errors".to_string(),
        Value::NativeFunction(NativeFunction::new("page_errors", Some(0), |_args| {
            with_browser(|browser| {
                browser.pump_events();
                let errors: Vec<Value> = browser
                    .page_errors()
                    .iter()
                    .map(|e| Value::String(e.clone().into()))
                    .collect();
                Ok(Value::Array(std::rc::Rc::new(RefCell::new(errors))))
            })
        })),
    );

    env.define(
        "close_browser".to_string(),
        Value::NativeFunction(NativeFunction::new("close_browser", Some(0), |_args| {
            close_browser();
            Ok(Value::Null)
        })),
    );
}

/// Convert a value that came back from the page into a Soli value.
///
/// Deliberately *not* the shared `json_to_value`: that one promotes any
/// numeric-looking string to a `Decimal`, which is a reasonable guess when
/// parsing an API response of unknown provenance and plain wrong here. The
/// page already told us the type — `textContent` is a string even when it
/// reads "0" — and silently retyping it means `assert_eq(evaluate(...), "0")`
/// fails for reasons a spec author cannot see.
fn json_to_soli(json: serde_json::Value) -> Value {
    use serde_json::Value as Json;
    match json {
        Json::Null => Value::Null,
        Json::Bool(b) => Value::Bool(b),
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else {
                Value::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        Json::String(s) => Value::String(s.into()),
        Json::Array(items) => Value::Array(std::rc::Rc::new(RefCell::new(
            items.into_iter().map(json_to_soli).collect(),
        ))),
        Json::Object(fields) => {
            let mut hash = crate::interpreter::value::HashPairs::default();
            for (key, value) in fields {
                hash.insert(
                    crate::interpreter::value::HashKey::String(key.into()),
                    json_to_soli(value),
                );
            }
            Value::Hash(std::rc::Rc::new(RefCell::new(hash)))
        }
    }
}

/// Split `"Alt+d"` into its key and the protocol's modifier bitmask.
///
/// Keyboard shortcuts are how a lot of developer UI is actually reached, and
/// spelling one as a chord is how everyone already writes them down.
fn parse_chord(chord: &str) -> (String, u32) {
    let mut modifiers = 0;
    let mut parts: Vec<&str> = chord.split('+').collect();
    // The last segment is the key; everything before it is a modifier. Split on
    // '+' means a bare "+" arrives as an empty final part, so fall back to it.
    let key = parts.pop().unwrap_or("");
    for part in parts {
        match part.trim().to_ascii_lowercase().as_str() {
            "alt" | "option" => modifiers |= 1,
            "ctrl" | "control" => modifiers |= 2,
            "meta" | "cmd" | "command" => modifiers |= 4,
            "shift" => modifiers |= 8,
            _ => {}
        }
    }
    let key = if key.is_empty() { "+" } else { key };
    (key.to_string(), modifiers)
}

/// Find an element, scroll it into view and click its centre.
fn click_selector(
    browser: &mut Browser,
    finder: &str,
    selector: &str,
    helper: &str,
) -> Result<(), String> {
    // Wait first: the element may be about to appear, and failing on the very
    // first look would make every spec pad itself with sleeps.
    let expression = center_expr(finder, selector);
    browser
        .wait_until(&format!("{} !== null", expression), DEFAULT_WAIT)
        .map_err(|_| format!("{}() found nothing matching {:?}", helper, selector))?;

    let point = browser.evaluate(&expression)?;
    let coordinates = point
        .as_array()
        .ok_or_else(|| format!("{}() could not locate {:?} on screen", helper, selector))?;
    let x = coordinates
        .first()
        .and_then(|v| v.as_f64())
        .ok_or_else(|| format!("{}() got no position for {:?}", helper, selector))?;
    let y = coordinates
        .get(1)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| format!("{}() got no position for {:?}", helper, selector))?;

    browser.click_at(x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_text_is_shortened_with_a_count() {
        let long = "x".repeat(500);
        let shortened = brief(&long);
        assert!(shortened.contains("(500 chars)"));
        assert!(shortened.chars().count() < 250);
    }

    #[test]
    fn short_text_survives_intact() {
        assert_eq!(brief("Saved successfully"), "Saved successfully");
    }

    #[test]
    fn whitespace_is_collapsed_so_failures_stay_on_one_line() {
        assert_eq!(brief("a\n\n  b\tc"), "a b c");
    }

    #[test]
    fn selectors_are_escaped_into_javascript() {
        // A selector containing a quote must not be able to terminate the
        // string literal it is embedded in.
        let expression = exists_expr("a[title=\"x\"]");
        assert!(expression.contains(r#"\"x\""#), "got: {}", expression);
    }

    #[test]
    fn a_bare_key_has_no_modifiers() {
        assert_eq!(parse_chord("Enter"), ("Enter".to_string(), 0));
    }

    #[test]
    fn chords_map_to_the_protocol_bitmask() {
        assert_eq!(parse_chord("Alt+d"), ("d".to_string(), 1));
        assert_eq!(parse_chord("Ctrl+Shift+k"), ("k".to_string(), 2 | 8));
        // Spelling varies by platform and habit; accept the common ones.
        assert_eq!(parse_chord("cmd+s"), ("s".to_string(), 4));
        assert_eq!(parse_chord("Control+a"), ("a".to_string(), 2));
    }

    #[test]
    fn a_literal_plus_survives_the_split() {
        assert_eq!(parse_chord("+"), ("+".to_string(), 0));
        assert_eq!(parse_chord("Alt++"), ("+".to_string(), 1));
    }

    #[test]
    fn the_default_wait_applies_when_no_options_hash_is_given() {
        assert_eq!(wait_timeout(&[Value::String("#x".into())]), DEFAULT_WAIT);
    }
}
