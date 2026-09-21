//! `/__coverage__`: the line-hit dump the test runner scrapes out of a server
//! subprocess, and the token that keeps anyone else from reading it.
//!
//! Lifted out of `handle_hyper_request`. It answers from the path, the method
//! and one header, before any routing — so it is a function, not a stage of
//! the request pipeline. `register_app_source_lines` comes with it: it runs at
//! boot rather than per request, but it exists only so that this endpoint's
//! answer counts the lines that were never hit as well as the ones that were.

use std::path::Path;

use bytes::Bytes;
use hyper::{HeaderMap, Response, StatusCode};

use super::{full, ResponseBody};

/// The coverage dump, or `None` for anything else.
pub(super) fn handle(
    path: &str,
    method: &str,
    headers: &HeaderMap,
) -> Option<Response<ResponseBody>> {
    // Only active when the parent process asked us to collect coverage (via
    // SOLI_COVERAGE_ENABLED). Returns a JSON blob the test runner merges into
    // its own aggregated report.
    if path != "/__coverage__" || method != "GET" || std::env::var("SOLI_COVERAGE_ENABLED").is_err()
    {
        return None;
    }

    // SEC-080: gate the dump on a per-process `SOLI_COVERAGE_TOKEN`. The
    // test runner mints a fresh random token, hands it to each child via
    // env, and presents it as `X-Coverage-Token` when scraping. Coverage
    // accidentally enabled in production would otherwise let any remote
    // client read source paths and line-hit counts; with the token gate
    // an unauthenticated GET returns 403, even if `SOLI_COVERAGE_ENABLED`
    // is set. The token is required — running without it (legacy callers,
    // misconfiguration) is rejected too, so the endpoint is never open.
    let expected = std::env::var("SOLI_COVERAGE_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    let provided = headers
        .get("x-coverage-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !coverage_request_authorized(expected.as_deref(), provided) {
        return Some(
            Response::builder()
                .status(StatusCode::FORBIDDEN)
                .header("Content-Type", "text/plain; charset=utf-8")
                .body(full(Bytes::from(
                    "coverage endpoint requires X-Coverage-Token matching SOLI_COVERAGE_TOKEN",
                )))
                .unwrap(),
        );
    }

    Some(
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(full(Bytes::from(coverage_dump_json())))
            .unwrap(),
    )
}

/// SEC-080: decide whether a `/__coverage__` GET is authorised. The
/// endpoint is only reachable when `SOLI_COVERAGE_ENABLED` is set; the
/// test runner additionally mints a random `SOLI_COVERAGE_TOKEN` per
/// run and presents it as `X-Coverage-Token`. `expected` is the env
/// value (None = not configured = reject), `provided` is the request
/// header value (empty = no header = reject). Constant-time compare so
/// the negative result doesn't leak token shape.
fn coverage_request_authorized(expected: Option<&str>, provided: &str) -> bool {
    let Some(tok) = expected else { return false };
    if tok.is_empty() || provided.is_empty() {
        return false;
    }
    crate::interpreter::builtins::crypto::do_secure_compare(tok, provided)
}

/// Build a JSON response that enumerates every recorded line hit on the
/// server-side global coverage tracker. Consumed by the test runner right
/// before it kills the subprocess so the parent process can merge the data
/// into its own aggregated report.
fn coverage_dump_json() -> String {
    let Some(tracker) = crate::coverage::tracker::get_global_coverage_tracker() else {
        return "{}".to_string();
    };
    let Ok(tracker) = tracker.lock() else {
        return "{}".to_string();
    };
    let coverage = tracker.get_aggregated_coverage();
    let mut out = String::from("{\"files\":[");
    let mut first = true;
    for (path, file_cov) in &coverage.file_coverages {
        if !first {
            out.push(',');
        }
        first = false;
        let path_str = path
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        out.push_str(&format!("{{\"path\":\"{}\",\"hits\":[", path_str));
        let mut line_first = true;
        for (line_num, line_cov) in &file_cov.lines {
            if line_cov.hits == 0 {
                continue;
            }
            if !line_first {
                out.push(',');
            }
            line_first = false;
            out.push_str(&format!("[{},{}]", line_num, line_cov.hits));
        }
        out.push_str("]}");
    }
    out.push_str("]}");
    out
}

/// Walk the MVC app directories that the test runner also walks for coverage
/// (`app/`, `config/`, `lib/`) and pre-register every `.sl` file's executable
/// lines on the server-side coverage tracker. Without this, lines that are
/// never hit would be absent from the report (the aggregator only knows about
/// lines it has seen hit).
pub(super) fn register_app_source_lines(
    tracker: &mut crate::coverage::CoverageTracker,
    app_dir: &Path,
) {
    let source_dirs = [
        app_dir.join("app"),
        app_dir.join("config"),
        app_dir.join("lib"),
    ];
    for source_dir in &source_dirs {
        if source_dir.is_dir() {
            collect_and_register_server_sources(tracker, source_dir);
        }
    }
}

fn collect_and_register_server_sources(tracker: &mut crate::coverage::CoverageTracker, dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_and_register_server_sources(tracker, &path);
            } else if path.extension().is_some_and(|e| e == "sl") {
                if let Ok(source) = std::fs::read_to_string(&path) {
                    tracker.register_executable_lines_from_source(&path, &source);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_rejected_without_env_token() {
        // Test runner forgot to set SOLI_COVERAGE_TOKEN — endpoint must
        // refuse rather than fall back to "any caller wins".
        assert!(!coverage_request_authorized(None, "anything"));
        assert!(!coverage_request_authorized(Some(""), "anything"));
    }

    #[test]
    fn coverage_rejected_without_request_header() {
        assert!(!coverage_request_authorized(Some("secret-token"), ""));
    }

    #[test]
    fn coverage_rejected_on_token_mismatch() {
        assert!(!coverage_request_authorized(
            Some("secret-token"),
            "wrong-token"
        ));
    }

    #[test]
    fn coverage_accepted_on_token_match() {
        assert!(coverage_request_authorized(
            Some("secret-token"),
            "secret-token"
        ));
    }
}
