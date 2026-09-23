//! The listening half of the server: one OS thread owning a tokio runtime, a
//! listener, and a task per connection.
//!
//! Lifted out of `run_hyper_server_worker_pool`, where it was 261 lines of
//! four nested layers — thread, runtime, accept loop, connection task — and
//! where every value it needed had already been cloned into a
//! `*_for_tokio` local just above it, which is what made it a closure rather
//! than a function. Those clones are [`Server`]'s fields now.
//!
//! The layers are four functions: [`spawn`] owns the thread and the runtime,
//! [`bind`] finds a port, [`accept_loop`] takes sockets, [`serve_connection`]
//! holds one for as long as it lives, and [`dispatch`] answers one request on
//! it.

use std::net::SocketAddr;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use bytes::Bytes;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::conn::auto;
use tokio::net::TcpListener;

use super::{
    add_header_checked, cors, full, handle_hyper_request, server_constants, shutdown,
    spawn_drain_on_signal, strict_port_requested, tenant, vhost, ResponseBody, TenantRuntime,
};

/// Everything the listening thread needs, gathered rather than captured.
pub(super) struct Server {
    /// Which interface to listen on (`SOLI_HOST`, or a default that depends
    /// on whether this is an application or a served folder).
    pub bind_host: std::net::IpAddr,
    /// The port asked for. `0` means "any", and a taken one is scanned past
    /// unless `--strict-port`.
    pub port: u16,
    /// Sized for I/O concurrency, not for the interpreter-worker count: the
    /// workers reach their outbound SoliDB and HTTP calls by `block_on` on
    /// this runtime, so it needs room for every worker's outbound call *plus*
    /// inbound serving headroom. Sizing it to the worker count starved it —
    /// with `--workers 1` the single I/O thread had to serve the inbound
    /// request and drive the outbound DB call at once, and stalled to the 30s
    /// client timeout on every request.
    pub tokio_worker_threads: usize,
    /// The one application this process serves. A host serving several would
    /// build the router with one entry per mounted application; nothing in
    /// the request path would differ.
    pub runtime: TenantRuntime,
    /// Answered as soon as the runtime exists, because the interpreter
    /// workers cannot start without a handle to `block_on`.
    pub runtime_handle_tx: std::sync::mpsc::Sender<tokio::runtime::Handle>,
    /// Answered with the port the kernel actually gave us, which is not
    /// `port` when that was `0` or already taken.
    pub bound_port_tx: std::sync::mpsc::Sender<u16>,
}

/// Start listening, on a thread of its own.
///
/// Returns immediately; the caller waits on `runtime_handle_tx` and
/// `bound_port_tx` for the two facts it needs back.
pub(super) fn spawn(server: Server) {
    let Server {
        bind_host,
        port,
        tokio_worker_threads,
        runtime: tenant_runtime,
        runtime_handle_tx,
        bound_port_tx,
    } = server;

    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(tokio_worker_threads)
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");

        runtime.block_on(async move {
            // Send runtime handle to main thread for workers to use
            let handle = tokio::runtime::Handle::current();
            crate::serve::websocket::set_runtime_handle(handle.clone());
            let _ = runtime_handle_tx.send(handle);

            // Bounds how many connections can be open at once (see
            // `server_constants::max_connections`). A semaphore rather than a
            // counter so the permit is released by `Drop` on every exit path.
            let connection_limiter = Arc::new(tokio::sync::Semaphore::new(
                match server_constants::max_connections() {
                    0 => tokio::sync::Semaphore::MAX_PERMITS,
                    n => n,
                },
            ));

            let listener = bind(bind_host, port).await;
            // Report the port the OS actually gave us, not the one we asked
            // for. They differ when `port` is 0: the caller is asking for an
            // ephemeral port, the kernel picks one — so without this the bound
            // port is undiscoverable and every consumer (the startup banner, a
            // desktop shell that has to open a browser at the right URL) sees
            // `0`.
            let bound_port = listener
                .local_addr()
                .map(|addr| addr.port())
                .unwrap_or(port);
            let _ = bound_port_tx.send(bound_port);

            // Own SIGTERM/SIGINT for the lifetime of the server so shutdown
            // drains instead of truncating.
            spawn_drain_on_signal();

            // One application, so the router is a single fallback entry that
            // answers every `Host`. A host serving several would build this
            // with `Router::new()` and one `insert` per mounted application;
            // nothing else in the request path would differ.
            let router = Arc::new(vhost::Router::single(tenant_runtime));

            accept_loop(listener, router, connection_limiter).await;
        });
    });
}

/// Bind `port`, or the first free port above it.
///
/// `--strict-port` turns "already in use" into an exit instead of a scan,
/// because a caller that pinned a port is usually a script that will talk to
/// exactly that one. Every failure here exits the process: there is no server
/// without a listener, and half a server is worse than none.
async fn bind(bind_host: std::net::IpAddr, port: u16) -> TcpListener {
    let mut try_port = port;
    loop {
        let addr = SocketAddr::from((bind_host, try_port));
        match TcpListener::bind(addr).await {
            Ok(l) => break l,
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                if strict_port_requested() {
                    eprintln!(
                        "Port {} is already in use and --strict-port was given, so no \
                         other port will be tried.",
                        port
                    );
                    std::process::exit(1);
                }
                if try_port == port {
                    eprintln!(
                        "Port {} is already in use, looking for a free port...",
                        port
                    );
                }
                try_port = try_port.checked_add(1).unwrap_or_else(|| {
                    eprintln!("No free port found");
                    std::process::exit(1);
                });
            }
            Err(e) => {
                eprintln!("Failed to bind: {}", e);
                std::process::exit(1);
            }
        }
    }
}

/// Accept for as long as the process lives.
///
/// Deliberately still accepting during a drain: breaking out of this loop
/// would return from the `block_on` that owns the runtime, dropping it and
/// killing the in-flight connections the drain exists to protect.
async fn accept_loop(
    listener: TcpListener,
    router: Arc<vhost::Router<TenantRuntime>>,
    connection_limiter: Arc<tokio::sync::Semaphore>,
) {
    loop {
        // The accept loop deliberately keeps running during a drain.
        // Breaking out would return from the enclosing `block_on`,
        // dropping the tokio runtime and killing the in-flight
        // connections this drain exists to protect. A load balancer also
        // needs to *reach* `/_ready` to learn this instance is going
        // away; a closed listener gives it a TCP refusal instead.
        let (stream, peer_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(_) => continue,
        };
        // Disable Nagle's algorithm: without this, small responses
        // written across multiple TCP segments stall ~40ms waiting on
        // the peer's delayed-ACK, even when the CPU is idle. Matches the
        // proxy/db convention which already set this on their sockets.
        let _ = stream.set_nodelay(true);

        // Global connection cap. Without one, a client that opens
        // sockets and trickles bodies exhausts file descriptors and
        // memory long before any per-request limit applies. Closing
        // immediately past the cap sheds the flood while keeping the
        // listener responsive for everyone else.
        let connection_permit = match connection_limiter.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                // Drop the stream: a RST is the fastest possible signal
                // to a well-behaved client to back off.
                drop(stream);
                continue;
            }
        };

        let io = TokioIo::new(stream);
        let router = router.clone(); // Arc clone is cheap

        tokio::spawn(serve_connection(io, router, peer_addr, connection_permit));
    }
}

/// One connection, for as long as it is open.
async fn serve_connection(
    io: TokioIo<tokio::net::TcpStream>,
    router: Arc<vhost::Router<TenantRuntime>>,
    peer_addr: SocketAddr,
    // Released when this connection ends, however it ends.
    _connection_permit: tokio::sync::OwnedSemaphorePermit,
) {
    // Held for the whole connection so the drain knows when the
    // last client has actually finished. Dropped on every exit
    // path, including error and panic.
    let _conn = shutdown::ConnectionGuard::new();
    // Request accounting for the idle watchdog below: how many requests have
    // started on this connection, and how many are still being answered.
    let activity = Arc::new(ConnectionActivity::default());
    let service_activity = activity.clone();
    let service = service_fn(move |req| {
        let router = router.clone(); // Arc clone is cheap
        let in_flight = InFlight::start(service_activity.clone());
        async move {
            let _in_flight = in_flight;
            dispatch(req, &router, peer_addr).await
        }
    });
    // `hyper_util::server::conn::auto::Builder` auto-detects
    // HTTP/1.1 vs HTTP/2 (h2c prior knowledge) from the first
    // bytes the client sends. h1 connections go through the
    // same `http1::Builder` as before; h2c connections get a
    // multiplexed stream handler with one TCP connection
    // carrying N concurrent requests. SEC-045's 10 s header
    // read timeout still applies on the h1 path.
    //
    // `Builder::new` takes an executor (used by h2 to spawn
    // stream tasks), not the IO — the IO goes into
    // `serve_connection_with_upgrades` below.
    let mut builder = auto::Builder::new(hyper_util::rt::TokioExecutor::new());
    // Configure h1: bound the header read timeout so slowloris
    // attackers don't pin accept slots. h2c has its own
    // header-equivalent timeouts inside hyper.
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(Duration::from_secs(10));
    // h2c had no liveness bound at all: a peer that vanished without a FIN
    // (or never sent another byte) held its task and a connection permit
    // forever. Ping it; no acknowledgement within the timeout closes the
    // connection. The timer is required for the pings to run.
    builder
        .http2()
        .timer(TokioTimer::new())
        .keep_alive_interval(Duration::from_secs(
            server_constants::h2_keep_alive_interval_secs(),
        ))
        .keep_alive_timeout(Duration::from_secs(20));
    // MUST be the `_with_upgrades` variant: plain
    // `serve_connection` never performs the HTTP/1.1 protocol
    // upgrade after a 101, so every WebSocket (live reload,
    // /ws/* routes, LiveView, presence) dies with
    // "Handshake not finished". h2 streams are unaffected by
    // the wrapper — it only arms the h1 upgrade path.
    let connection = builder.serve_connection_with_upgrades(io, service);
    tokio::pin!(connection);

    // Idle watchdog. Pings only prove the peer is alive, and a live client
    // can hold an h2c connection open forever without sending a request —
    // twenty thousand of those exhaust `SOLI_MAX_CONNECTIONS`. (h1 already
    // closes an idle keep-alive connection through `header_read_timeout`.)
    // A connection with nothing in flight and no new request for a whole idle
    // period is shut down gracefully: h2 sends GOAWAY and lets open streams
    // (SSE included) finish; h1 finishes the current response first.
    let idle = Duration::from_secs(server_constants::connection_idle_timeout_secs());
    let mut seen_requests = activity.started();
    let mut shutting_down = false;
    loop {
        tokio::select! {
            _result = connection.as_mut() => {
                // Connection errors are silently ignored, as before.
                break;
            }
            _ = tokio::time::sleep(idle), if !shutting_down => {
                let started = activity.started();
                if started == seen_requests && activity.in_flight() == 0 {
                    connection.as_mut().graceful_shutdown();
                    shutting_down = true;
                }
                seen_requests = started;
            }
        }
    }
}

/// Per-connection request counters for the idle watchdog in
/// [`serve_connection`]. Two relaxed atomics per request.
#[derive(Default)]
struct ConnectionActivity {
    started: std::sync::atomic::AtomicU64,
    in_flight: std::sync::atomic::AtomicUsize,
}

impl ConnectionActivity {
    fn started(&self) -> u64 {
        self.started.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn in_flight(&self) -> usize {
        self.in_flight.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// One request being answered; dropped (on every path) when its handler
/// future completes or is cancelled.
struct InFlight(Arc<ConnectionActivity>);

impl InFlight {
    fn start(activity: Arc<ConnectionActivity>) -> Self {
        activity
            .started
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        activity
            .in_flight
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        InFlight(activity)
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0
            .in_flight
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// One request, from the connection task to `handle_hyper_request` and back.
///
/// Three things happen around the handler and none of them is routing: the
/// drain check, choosing which application serves this `Host`, and CORS.
async fn dispatch(
    req: Request<Incoming>,
    router: &vhost::Router<TenantRuntime>,
    peer_addr: SocketAddr,
) -> Result<Response<ResponseBody>, hyper::Error> {
    // Draining: refuse new work, but let the probes
    // through — `/_ready` answering 503 is precisely how
    // a load balancer learns to stop routing here, and it
    // cannot do that if the drain check swallows it.
    if shutdown::is_draining() && !matches!(req.uri().path(), "/_health" | "/_ready") {
        return Ok(Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header("Connection", "close")
            .body(full(Bytes::from("Server shutting down")))
            .unwrap());
    }
    // Which application serves this. `Host` first, and
    // the URI authority behind it: an HTTP/2 request
    // carries `:authority` instead, which hyper leaves
    // in the URI rather than synthesising a header.
    let host = req
        .headers()
        .get(hyper::header::HOST)
        .and_then(|value| value.to_str().ok())
        .or_else(|| req.uri().host());
    // Borrowed, not cloned: the bundle holds several channel senders whose
    // shared refcounts every request would otherwise bump and drop.
    let Some(runtime) = router.resolve(host) else {
        // No application claims this host and there is
        // no fallback. 421 is the answer that tells a
        // client it reached the wrong server, rather
        // than handing it whichever app is first.
        return Ok(Response::builder()
            .status(StatusCode::MISDIRECTED_REQUEST)
            .header("Server", "soliMVC")
            .body(full(Bytes::from("Misdirected request")))
            .unwrap());
    };
    // Everything from here runs as that tenant, on
    // every thread this future is polled on: the CORS
    // and CSRF policies, the cookie jar, the session
    // config and the dev-bar store are all keyed by it,
    // and a thread-local binding does not survive an
    // `.await`.
    let tenant_id = runtime.tenant;
    let (result, cors_decision) = tenant::task_scope(tenant_id, async move {
        // Built-in CORS (`cors("/api/*", {...})` in
        // config/routes.sl). Wraps the whole handler so
        // every response of a CORS-managed path —
        // buffered, streamed, static, or error — carries
        // the allow headers, and preflights are answered
        // before routing.
        let cors_decision = cors::evaluate(req.method().as_str(), req.uri().path(), req.headers());
        if let Some(preflight) = cors_decision.as_ref().and_then(|d| d.preflight.as_ref()) {
            let mut builder = Response::builder()
                .status(StatusCode::NO_CONTENT)
                .header("Server", "soliMVC");
            for (key, value) in preflight {
                builder = add_header_checked(builder, key, value);
            }
            let preflight_response = Ok(builder
                .body(full(Bytes::new()))
                .unwrap_or_else(|_| Response::new(full(Bytes::new()))));
            // A preflight carries no allow headers of
            // its own beyond the ones just added.
            return (preflight_response, None);
        }
        let result = handle_hyper_request(req, runtime, peer_addr).await;
        // The decision leaves the scope with the result:
        // the allow headers are stamped on the response
        // below, outside it.
        (result, cors_decision)
    })
    .await;
    match (result, cors_decision) {
        (Ok(mut response), Some(decision)) => {
            for (key, value) in decision.response_headers {
                if let (Ok(name), Ok(val)) = (
                    hyper::header::HeaderName::try_from(key.as_str()),
                    hyper::header::HeaderValue::try_from(value.as_str()),
                ) {
                    response.headers_mut().append(name, val);
                }
            }
            Ok(response)
        }
        (result, _) => result,
    }
}
