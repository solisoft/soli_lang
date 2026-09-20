//! The three endpoints an orchestrator and a scraper ask for: liveness,
//! readiness, and Prometheus metrics.
//!
//! Lifted out of `handle_hyper_request`. They answer from the path, the request
//! headers and the peer address, and they run before any routing — so they are
//! a function, not a stage of the request pipeline.

use std::net::IpAddr;

use bytes::Bytes;
use hyper::{HeaderMap, Response, StatusCode};

use super::{full, metrics_request_allowed, shutdown, ResponseBody};

fn plain(status: StatusCode, body: &'static [u8]) -> Response<ResponseBody> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("Cache-Control", "no-store")
        .body(full(Bytes::from_static(body)))
        .expect("static plain-text response is always valid")
}

/// `/_health`, `/_ready` and `/_metrics`, or `None` for anything else.
pub(super) fn handle(
    path: &str,
    method: &str,
    headers: &HeaderMap,
    peer_ip: IpAddr,
) -> Option<Response<ResponseBody>> {
    let head_or_get = method == "GET" || method == "HEAD";

    match path {
        // Liveness: is this process up at all? 200 for as long as the server is
        // running, **including mid-drain** — a draining process is healthy, it
        // just does not want new traffic. An orchestrator that gets a non-200
        // here restarts the container, so it must not fail during a normal
        // shutdown.
        "/_health" if head_or_get => Some(plain(StatusCode::OK, b"ok")),

        // Readiness: should this instance be in the load balancer right now?
        // 503 while the workers are still booting, and again for the whole
        // drain, so a rolling deploy stops routing here before connections
        // start being refused.
        "/_ready" if head_or_get => {
            let (status, body) = if shutdown::is_ready() {
                (StatusCode::OK, &b"ready"[..])
            } else if shutdown::is_draining() {
                (StatusCode::SERVICE_UNAVAILABLE, &b"draining"[..])
            } else {
                (StatusCode::SERVICE_UNAVAILABLE, &b"starting"[..])
            };
            Some(plain(status, body))
        }

        // Prometheus. No CSRF check — it is meant to be scraped.
        //
        // The text tells whoever reads it the request volume, timing
        // distribution and error rate of the whole application, and it was
        // served to anybody who asked on the same public port.
        // `SOLI_METRICS_TOKEN` requires a bearer token; unset, access is
        // limited to loopback and private-range peers, where a scraper normally
        // lives — so an in-cluster Prometheus keeps working with no
        // configuration while the public internet stops seeing it. A refusal is
        // a 404, not a 403: the endpoint does not advertise that it is there.
        "/_metrics" if method == "GET" => {
            if !metrics_request_allowed(headers, peer_ip) {
                return Some(
                    Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .header("Content-Type", "text/plain; charset=utf-8")
                        .body(full(Bytes::from_static(b"Not Found")))
                        .expect("static 404 is always valid"),
                );
            }
            Some(
                Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/plain; charset=utf-8")
                    .body(full(Bytes::from(
                        crate::metrics::Metrics::global().render_prometheus(),
                    )))
                    .expect("metrics response is always valid"),
            )
        }

        _ => None,
    }
}
