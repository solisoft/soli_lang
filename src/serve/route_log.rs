//! The route matched for the current request, for the dev bar's "requests"
//! panel.
//!
//! The dev bar shows the page's own route (controller#action) and, via a
//! client-side `fetch`/`XMLHttpRequest` patch, the routes of the XHR/HTMx
//! calls the page fires afterwards. Those later requests are separate and
//! don't carry a dev bar, so the server tags *every* dev-mode response with an
//! `X-Soli-Route` header the client reads. This module holds the matched
//! handler name for the current request so the finalizer can emit that header
//! and render the page's own route.
//!
//! Unlike `middleware_log` (a growing Vec), a request matches exactly one
//! route, so this stores a single `Option`.
//!
//! Cheap when dev mode is off — `record` early-outs on the flag.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static ROUTE: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

#[inline]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn clear() {
    ROUTE.with(|r| *r.borrow_mut() = None);
}

/// Record the handler name (e.g. `users#show`) matched for this request.
pub fn record(handler: &str) {
    if !is_enabled() {
        return;
    }
    ROUTE.with(|r| *r.borrow_mut() = Some(handler.to_string()));
}

pub fn snapshot() -> Option<String> {
    ROUTE.with(|r| r.borrow().clone())
}
