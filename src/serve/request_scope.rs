//! What every request resets on arrival, and what it installs for itself.
//!
//! Lifted out of the head of `handle_request`. Worker threads are pooled and
//! reused, so this is the module about *not inheriting the previous visitor*:
//! eight log buffers, the test-runner's captured render, the taint marks, the
//! cookie-driver state, the response cookies and the response cache all live in
//! thread-locals that outlive the request that filled them. Every clear here
//! carries the comment that says what leaked without it.
//!
//! It is a function rather than a stage because it answers nothing and decides
//! nothing: it takes the request in and hands back the three values the rest of
//! the pipeline reads from it.
//!
//! **`LocaleGuard` is deliberately not here.** `install` sets the request's
//! starting locale, but the guard that restores the default has to be *bound in
//! the caller* — one constructed and dropped inside this module would restore
//! the default before the handler ever ran, silently reverting the fix its own
//! doc comment describes.
//!
//! - [`reset_worker_thread_locals`] — the clears that must happen before
//!   anything is recorded.
//! - [`install`] — the session, the cookies and the locale for this request.

use crate::interpreter::builtins::session::{
    clear_response_cookies, parse_cookie_pairs, session_id_from_cookie_pairs,
    set_current_session_id,
};
use crate::interpreter::value::HashPairs;

use super::{
    header_str, middleware_log, otel, phase_log, prod_log, resolve_request_locale, route_log,
    span_log, template_warnings, view_log, RequestData,
};

/// What [`install`] resolved out of the incoming request, for the stages that
/// run after it.
pub(super) struct Scope {
    /// The `Cookie` header, parsed exactly once. Becomes `req["cookies"]` and
    /// the `cookies` global further down; the header used to be scanned twice.
    pub(super) cookie_pairs: HashPairs,
    /// The session id the client *claimed*, or `None`. Kept apart from
    /// `session_id` because the response only re-emits `Set-Cookie` when the
    /// two differ.
    pub(super) cookie_session_id: Option<String>,
    /// The session id that actually exists, or `None`. A cookie naming an
    /// unknown session resolves to `None` — see [`install`].
    pub(super) session_id: Option<String>,
}

/// Drop what the previous request on this worker thread left behind.
///
/// Runs before anything else, including the file-mode short-circuit: a stale
/// buffer is stale whatever this request turns out to be.
pub(super) fn reset_worker_thread_locals(dev_mode: bool) {
    // Reset the per-request AQL log so `dev_queries()` only returns this
    // request's queries. Cheap when dev mode is off (early-out on the flag).
    // Also clear when production logging is on, otherwise the thread-local
    // buffers would accumulate across requests on the same worker thread.
    // OpenTelemetry reuses span_log, so clear that tree whenever OTEL is on.
    forget_request_logs(dev_mode);

    // E2E test client: clear any render captured by a prior request on this
    // pooled worker thread, so assigns()/view_path()/render_template() reflect
    // only the current request. A single atomic load in non-test processes.
    if crate::interpreter::builtins::test_server::is_test_runner_process() {
        crate::interpreter::builtins::test_server::clear_captured_render();
    }

    forget_unfinished_request();
}

/// Drop what a request that ended early — raised, halted, panicked — left
/// in flight on this worker.
///
/// Each of these is set while a handler runs and only consumed on the path
/// where it finishes normally, so an early end handed it to the next visitor:
///
/// - the `render_json` / `render_text` response: a handler that rendered JSON
///   and then raised had its body, status and headers sent to the next request;
/// - the request and headers views read: a view rendered from the next
///   request's middleware saw the previous visitor's cookies and path, and
///   `redirect(:back)` sent them to the previous visitor's referer;
/// - the action name: `authorize(record)` in middleware checked the policy of
///   the previous request's action;
/// - the `req`/`params`/`session`/`cookies`/`headers` view helpers read: a
///   custom 404 page or a view rendered by middleware showed the previous
///   visitor's `req["current_user"]`;
/// - a transaction or `grouped` batch a panic interrupted: the next request
///   wrote into the dead request's uncommitted transaction, or deferred its
///   reads into its batch.
///
/// Also run by a socket event, which shares the worker threads.
pub(super) fn forget_unfinished_request() {
    use crate::interpreter::builtins::{model, server, set_current_action, template};

    server::take_fast_path_response();
    template::clear_current_request();
    template::clear_current_headers();
    template::clear_helper_request_context();
    set_current_action("");

    if model::crud::has_active_tx() {
        // Rolled back rather than dropped, so a SQL connection goes back to its
        // pool clean; cleared either way, as a failed rollback leaves it set.
        let _ = model::crud::rollback_transaction();
        model::crud::clear_current_tx();
    }
    model::batch::discard_stale();
}

/// What a request begins by forgetting.
///
/// The per-request logs — queries, HTTP calls, KV commands, phases,
/// middleware, views, spans, routes, template warnings — are thread-local
/// buffers a worker fills as it serves and a dev bar reads at the end.
/// They are only ever emptied here, so anything that serves without
/// passing through here accumulates for the life of the worker. Cheap when
/// nothing is recording: every `clear` is a `Vec::clear` on an empty
/// vector. Cleared for production detail logging and OpenTelemetry too,
/// which reuse the same buffers.
pub(super) fn forget_request_logs(dev_mode: bool) {
    // A `soli test` server keeps a query log in any mode (see `serve`), so
    // it is emptied per request too, or each count would include the
    // requests before it.
    if dev_mode
        || prod_log::channels().has_detail()
        || otel::enabled()
        || crate::interpreter::builtins::test_server::is_test_runner_process()
    {
        crate::interpreter::builtins::model::query_log::clear();
        crate::interpreter::builtins::http_log::clear();
        crate::interpreter::builtins::kv_log::clear();
        phase_log::clear();
        middleware_log::clear();
        view_log::clear();
        span_log::clear();
        route_log::clear();
        template_warnings::clear();
    }
}

/// Install this request's peer, cookies, session and locale on the worker
/// thread, and hand back what the later stages need to read.
///
/// The caller binds `LocaleGuard` immediately after — see the module header.
pub(super) fn install(data: &RequestData) -> Scope {
    // Record the TCP peer for the trust-proxy gate: with SOLI_TRUSTED_PROXIES
    // set, `X-Forwarded-*` is only honoured for requests that actually arrived
    // from a listed hop.
    crate::interpreter::builtins::trust_proxy::set_current_peer_ip(data.peer_ip.parse().ok());

    // Drop the previous request's taint marks before this one records its own.
    crate::interpreter::taint::clear_request_values();

    // No tenant until this request chooses one (`switch_tenant`): the worker
    // may last have served another site, whose database must not answer here.
    crate::db::tenancy::clear();
    // Nor events or `after_commit` blocks a previous request left waiting on a
    // commit that never came.
    crate::interpreter::events::discard_pending();

    // Parse the Cookie header ONCE: the same parse feeds both the session-ID
    // resolution here and `req["cookies"]` in the request hash below (the
    // header used to be scanned twice per request).
    let cookie_pairs = parse_cookie_pairs(header_str(&data.headers, "cookie"));

    // Hand the raw header to the cookie jar so `read_cookie` can verify/open
    // sealed values on demand. Installing `None` doubles as the per-request
    // clear, alongside the session-state clears below.
    crate::interpreter::builtins::cookie_jar::install_request_cookie_header(header_str(
        &data.headers,
        "cookie",
    ));

    // Drop any cookie-driver session state a previous request left on this
    // worker thread. Must happen before `ensure_session` installs this
    // request's state — a no-cookie request would otherwise silently inherit
    // (and re-emit) the previous visitor's session.
    crate::interpreter::builtins::session_cookie::clear_request_state();

    // Resolve the session ID from the parsed cookies (if any). When no cookie
    // is sent, we leave the thread-local unset — session_set / session_regenerate
    // will create one lazily on first use, and `finalize::finish` emits
    // Set-Cookie whenever the post-handler session ID differs from the cookie's.
    // SEC-077 precedence (`__Host-session_id` over `session_id`) is preserved
    // inside session_id_from_cookie_pairs.
    let cookie_session_id = session_id_from_cookie_pairs(&cookie_pairs);
    // Resolve without creating. A cookie naming a session we do not have used to
    // mint and persist an empty one on every request, so any client could grow
    // the store without limit just by sending a fresh UUID each time. An unknown
    // cookie now takes the same lazy path a cookie-less request always did: no
    // session exists until the app stores something.
    let session_id = match cookie_session_id
        .as_deref()
        .and_then(|id| crate::interpreter::builtins::session::resolve_existing_session(Some(id)))
    {
        Some(resolved) => {
            set_current_session_id(Some(resolved.clone()));
            Some(resolved)
        }
        None => {
            set_current_session_id(None);
            None
        }
    };
    // The locale this request starts from, now that the session is resolved
    // (a stored choice is consulted first). `LocaleGuard` puts it back at the
    // end of the request; see there for what it cost not to.
    crate::interpreter::builtins::i18n::helpers::set_locale(
        resolve_request_locale(&data.headers, &cookie_pairs)
            .unwrap_or_default()
            .as_str(),
    );

    // Clear response cookies from any previous request on this thread.
    clear_response_cookies();
    // And its flashes: the next visitor would otherwise read the messages the
    // previous one took out of their session.
    crate::interpreter::builtins::flash::clear_request_flashes();
    // Reset the static-page response cacheability flags so this request
    // starts clean. set_cookie / session_set trip `mark_response_dirty`
    // and clock / random trip `mark_data_dirty` while the controller
    // runs; the cache lookup in TemplateCache::render consults both
    // and short-circuits to a cache hit only when neither is set.
    crate::template::response_cache::reset_for_new_request();

    Scope {
        cookie_pairs,
        cookie_session_id,
        session_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::builtins::model::batch;
    use crate::interpreter::builtins::{current_action_name, server, set_current_action, template};
    use crate::interpreter::value::Value;

    /// What a request that rendered JSON and then raised left behind used to
    /// be served to the next visitor on the worker.
    #[test]
    fn a_request_that_ended_early_leaves_nothing_for_the_next() {
        server::set_fast_path_response(server::FastPathResponse {
            status: 200,
            headers: Vec::new(),
            body: r#"{"secret":"alice"}"#.to_string(),
        });
        template::set_current_request(Value::String("alice's request".into()));
        let mut headers = HashPairs::default();
        headers.insert(
            crate::interpreter::value::HashKey::String("cookie".into()),
            Value::String("sid=ALICE".into()),
        );
        template::set_current_headers(Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(
            headers,
        ))));
        set_current_action("destroy");
        assert!(batch::begin());
        batch::enter_block();

        forget_unfinished_request();

        assert!(server::take_fast_path_response().is_none());
        assert!(template::get_current_request().is_none());
        assert!(template::current_header("cookie").is_none());
        assert_eq!(current_action_name(), "");
        assert!(!batch::is_active());
        assert!(!batch::in_block());
    }
}
