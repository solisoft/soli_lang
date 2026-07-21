//! Exercises the DevTools driver against a real browser.
//!
//! Skipped when no browser is installed rather than failed: a machine without
//! Chrome is a normal place to build Soli, and browser tests are opt-in.
//! CI installs one, so the coverage is real where it counts.

use std::time::Duration;

use solilang::cdp::Browser;

/// A page with something to find, something to click, and something that only
/// appears once script has run.
const FIXTURE: &str = "data:text/html,\
<html><body>\
<h1 id='title'>Hello Soli</h1>\
<button id='go' onclick=\"document.getElementById('out').textContent='clicked'\">Go</button>\
<p id='out'>idle</p>\
<input id='field'>\
<script>window.__booted = true;</script>\
</body></html>";

fn browser() -> Option<Browser> {
    if solilang::platform::browser::find_chrome().is_none() {
        eprintln!("skipping: no Chrome or Chromium on this machine");
        return None;
    }
    Some(Browser::launch(false).expect("the browser must launch"))
}

#[test]
fn evaluates_javascript_in_a_real_page() {
    let Some(mut browser) = browser() else { return };
    browser.navigate(FIXTURE).expect("the fixture must load");

    let sum = browser.evaluate("1 + 1").expect("evaluation must succeed");
    assert_eq!(sum.as_i64(), Some(2));

    let title = browser
        .evaluate("document.getElementById('title').textContent")
        .expect("the DOM must be reachable");
    assert_eq!(title.as_str(), Some("Hello Soli"));
}

#[test]
fn inline_script_has_run_by_the_time_navigate_returns() {
    // The whole point of waiting on readyState rather than the load event: a
    // test must not race the page's own boot code.
    let Some(mut browser) = browser() else { return };
    browser.navigate(FIXTURE).expect("the fixture must load");

    let booted = browser.evaluate("window.__booted === true").unwrap();
    assert_eq!(booted.as_bool(), Some(true));
}

#[test]
fn a_real_click_triggers_the_page_handler() {
    let Some(mut browser) = browser() else { return };
    browser.navigate(FIXTURE).expect("the fixture must load");

    let point = browser
        .evaluate(
            "(() => { const r = document.getElementById('go').getBoundingClientRect(); \
             return [r.left + r.width / 2, r.top + r.height / 2]; })()",
        )
        .expect("the button must be measurable");
    let coordinates = point.as_array().expect("a coordinate pair");
    let x = coordinates[0].as_f64().unwrap();
    let y = coordinates[1].as_f64().unwrap();

    browser.click_at(x, y).expect("the click must dispatch");
    browser
        .wait_until(
            "document.getElementById('out').textContent === 'clicked'",
            Duration::from_secs(5),
        )
        .expect("the handler must have run");
}

#[test]
fn typing_reaches_the_focused_field() {
    let Some(mut browser) = browser() else { return };
    browser.navigate(FIXTURE).expect("the fixture must load");

    browser
        .evaluate("document.getElementById('field').focus()")
        .expect("the field must focus");
    browser.insert_text("typed").expect("text must insert");

    let value = browser
        .evaluate("document.getElementById('field').value")
        .unwrap();
    assert_eq!(value.as_str(), Some("typed"));
}

#[test]
fn page_exceptions_are_captured_rather_than_swallowed() {
    let Some(mut browser) = browser() else { return };
    browser.navigate(FIXTURE).expect("the fixture must load");
    assert!(
        browser.page_errors().is_empty(),
        "a clean page has no errors"
    );

    // Thrown asynchronously so it reaches the protocol as an uncaught
    // exception rather than as this evaluation's own error.
    browser
        .evaluate("setTimeout(() => { throw new Error('boom'); }, 0)")
        .expect("scheduling must succeed");
    browser.wait_until("false", Duration::from_millis(300)).ok();

    assert!(
        browser.page_errors().iter().any(|e| e.contains("boom")),
        "the uncaught exception must be recorded, got {:?}",
        browser.page_errors()
    );
}

#[test]
fn a_timed_out_wait_reports_the_condition() {
    let Some(mut browser) = browser() else { return };
    browser.navigate(FIXTURE).expect("the fixture must load");

    let error = browser
        .wait_until("window.__never_set", Duration::from_millis(200))
        .expect_err("the condition never becomes true");
    assert!(
        error.contains("__never_set"),
        "the message must name the condition, got: {}",
        error
    );
}

#[test]
fn screenshots_are_png_bytes() {
    let Some(mut browser) = browser() else { return };
    browser.navigate(FIXTURE).expect("the fixture must load");

    let image = browser.screenshot().expect("a screenshot must be produced");
    assert!(image.len() > 100, "an image should not be nearly empty");
    assert_eq!(&image[..8], b"\x89PNG\r\n\x1a\n", "must be a real PNG");
}

/// A page that opts into the device width, the way every layout that means to
/// be usable on a phone does. Without the meta tag a mobile viewport lays out
/// at 980 CSS pixels — real phone behavior, and not what a spec is testing.
const RESPONSIVE_FIXTURE: &str = "data:text/html,\
<html><head>\
<meta name='viewport' content='width=device-width, initial-scale=1'>\
<style>.panel { display: none } \
@media (max-width: 600px) { .panel { display: block } }</style>\
</head><body><div class='panel'>menu</div></body></html>";

#[test]
fn the_page_lays_out_at_the_emulated_viewport() {
    let Some(mut browser) = browser() else { return };
    browser
        .set_viewport(390, 844, 3.0, true)
        .expect("the viewport must apply");
    browser
        .navigate(RESPONSIVE_FIXTURE)
        .expect("the fixture must load");

    let width = browser.evaluate("window.innerWidth").unwrap();
    let height = browser.evaluate("window.innerHeight").unwrap();
    assert_eq!(width.as_i64(), Some(390));
    assert_eq!(height.as_i64(), Some(844));

    // The pixel ratio and the coarse pointer are what a responsive layout and
    // a touch-only handler actually branch on — a narrow window alone is not
    // a phone.
    let ratio = browser.evaluate("window.devicePixelRatio").unwrap();
    assert_eq!(ratio.as_f64(), Some(3.0));
    let coarse = browser
        .evaluate("matchMedia('(pointer: coarse)').matches")
        .unwrap();
    assert_eq!(coarse.as_bool(), Some(true));

    // And back: a spec that resizes mid-test must really get the desktop
    // layout afterwards, not a phone with more pixels.
    browser
        .set_viewport(1280, 800, 1.0, false)
        .expect("the viewport must change");
    let width = browser.evaluate("window.innerWidth").unwrap();
    assert_eq!(width.as_i64(), Some(1280));
    let coarse = browser
        .evaluate("matchMedia('(pointer: coarse)').matches")
        .unwrap();
    assert_eq!(coarse.as_bool(), Some(false));
}

#[test]
fn a_media_query_sees_the_emulated_width() {
    // The point of the feature: `@media (max-width: 600px)` rules must apply,
    // and stop applying when the spec asks for a desktop.
    let Some(mut browser) = browser() else { return };
    browser
        .set_viewport(390, 844, 3.0, true)
        .expect("the viewport must apply");
    browser
        .navigate(RESPONSIVE_FIXTURE)
        .expect("the fixture must load");

    let display = "getComputedStyle(document.querySelector('.panel')).display";
    assert_eq!(browser.evaluate(display).unwrap().as_str(), Some("block"));

    browser
        .set_viewport(1280, 800, 1.0, false)
        .expect("the viewport must change");
    assert_eq!(browser.evaluate(display).unwrap().as_str(), Some("none"));
}

#[test]
fn cookies_round_trip_through_the_page() {
    let Some(mut browser) = browser() else { return };
    // Cookies belong to an origin, not to whatever page happens to be open —
    // which is why both calls name one rather than relying on the current page.
    browser
        .set_cookie("soli_session", "abc123", "http://127.0.0.1/")
        .expect("the cookie must install");

    let cookies = browser
        .cookies("http://127.0.0.1/")
        .expect("cookies must be readable");
    assert!(
        cookies
            .iter()
            .any(|(name, value)| name == "soli_session" && value == "abc123"),
        "the installed cookie must come back, got {:?}",
        cookies
    );
}
