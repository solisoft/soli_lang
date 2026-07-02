use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Whether metrics collection is enabled for this process.
///
/// Collection is opt-in: it is only worth the per-operation `Instant::now()`
/// and atomic bookkeeping when someone actually scrapes the `/_metrics`
/// Prometheus endpoint. Enable by setting `SOLI_METRICS=1` (or `true`).
/// The value is read from the environment exactly once and cached.
#[inline]
pub fn metrics_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("SOLI_METRICS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    })
}

pub struct Metrics {
    pub http_requests_total: AtomicU64,
    pub lexing_duration_ns_total: AtomicU64,
    pub lexing_count: AtomicU64,
    pub parsing_duration_ns_total: AtomicU64,
    pub parsing_count: AtomicU64,
    pub vm_execution_ns_total: AtomicU64,
    pub vm_execution_count: AtomicU64,
    /// Total wall time spent rendering templates (views + layouts + partials) in production.
    pub template_render_duration_ns_total: AtomicU64,
    pub template_render_count: AtomicU64,
    /// Coarse total time spent inside all middleware for the request (populated from middleware_log).
    pub middleware_duration_ns_total: AtomicU64,
    pub middleware_count: AtomicU64,
    /// Coarse total time spent in DB / SolidB queries (from query_log).
    pub db_query_duration_ns_total: AtomicU64,
    pub db_query_count: AtomicU64,
    /// Number of handlers demoted from the VM to the tree-walking interpreter
    /// (per worker, first failing request only — the demotion is then cached).
    /// A non-zero value means some production handlers run on the slower
    /// engine; set `SOLI_ENGINE_LOG=1` to log which handler and why.
    pub vm_handler_demotions_total: AtomicU64,
    start_time: std::sync::OnceLock<Instant>,
}

impl Metrics {
    pub const fn new() -> Self {
        Self {
            http_requests_total: AtomicU64::new(0),
            lexing_duration_ns_total: AtomicU64::new(0),
            lexing_count: AtomicU64::new(0),
            parsing_duration_ns_total: AtomicU64::new(0),
            parsing_count: AtomicU64::new(0),
            vm_execution_ns_total: AtomicU64::new(0),
            vm_execution_count: AtomicU64::new(0),
            template_render_duration_ns_total: AtomicU64::new(0),
            template_render_count: AtomicU64::new(0),
            middleware_duration_ns_total: AtomicU64::new(0),
            middleware_count: AtomicU64::new(0),
            db_query_duration_ns_total: AtomicU64::new(0),
            db_query_count: AtomicU64::new(0),
            vm_handler_demotions_total: AtomicU64::new(0),
            start_time: std::sync::OnceLock::new(),
        }
    }

    pub fn global() -> &'static Self {
        static METRICS: Metrics = Metrics::new();
        METRICS.ensure_start_time();
        &METRICS
    }

    fn ensure_start_time(&self) {
        let _ = self.start_time.get_or_init(Instant::now);
    }

    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(1024);

        let requests = self.http_requests_total.load(Ordering::Relaxed);

        out.push_str("# HELP soli_http_requests_total Total number of HTTP requests handled.\n");
        out.push_str("# TYPE soli_http_requests_total counter\n");
        out.push_str(&format!("soli_http_requests_total {}\n", requests));

        out.push_str(
            "# HELP soli_lexing_duration_seconds Total time spent in the lexer, in seconds.\n",
        );
        out.push_str("# TYPE soli_lexing_duration_seconds counter\n");
        out.push_str(&format!(
            "soli_lexing_duration_seconds {:.9}\n",
            self.lexing_duration_ns_total.load(Ordering::Relaxed) as f64 / 1_000_000_000.0
        ));
        out.push_str("# HELP soli_lexing_duration_seconds_count Number of lexing operations.\n");
        out.push_str("# TYPE soli_lexing_duration_seconds_count counter\n");
        out.push_str(&format!(
            "soli_lexing_duration_seconds_count {}\n",
            self.lexing_count.load(Ordering::Relaxed)
        ));

        out.push_str(
            "# HELP soli_parsing_duration_seconds Total time spent in the parser, in seconds.\n",
        );
        out.push_str("# TYPE soli_parsing_duration_seconds counter\n");
        out.push_str(&format!(
            "soli_parsing_duration_seconds {:.9}\n",
            self.parsing_duration_ns_total.load(Ordering::Relaxed) as f64 / 1_000_000_000.0
        ));
        out.push_str("# HELP soli_parsing_duration_seconds_count Number of parsing operations.\n");
        out.push_str("# TYPE soli_parsing_duration_seconds_count counter\n");
        out.push_str(&format!(
            "soli_parsing_duration_seconds_count {}\n",
            self.parsing_count.load(Ordering::Relaxed)
        ));

        out.push_str(
            "# HELP soli_vm_execution_seconds Total time spent executing bytecode in the VM, in seconds.\n",
        );
        out.push_str("# TYPE soli_vm_execution_seconds counter\n");
        out.push_str(&format!(
            "soli_vm_execution_seconds {:.9}\n",
            self.vm_execution_ns_total.load(Ordering::Relaxed) as f64 / 1_000_000_000.0
        ));
        out.push_str("# HELP soli_vm_execution_seconds_count Number of VM executions.\n");
        out.push_str("# TYPE soli_vm_execution_seconds_count counter\n");
        out.push_str(&format!(
            "soli_vm_execution_seconds_count {}\n",
            self.vm_execution_count.load(Ordering::Relaxed)
        ));

        // Template rendering (views + layouts + partials) — always measured, cheap atomics.
        out.push_str(
            "# HELP soli_template_render_duration_seconds Total time spent rendering templates (views, layouts, partials).\n",
        );
        out.push_str("# TYPE soli_template_render_duration_seconds counter\n");
        out.push_str(&format!(
            "soli_template_render_duration_seconds {:.9}\n",
            self.template_render_duration_ns_total
                .load(Ordering::Relaxed) as f64
                / 1_000_000_000.0
        ));
        out.push_str(
            "# HELP soli_template_render_duration_seconds_count Number of template renders.\n",
        );
        out.push_str("# TYPE soli_template_render_duration_seconds_count counter\n");
        out.push_str(&format!(
            "soli_template_render_duration_seconds_count {}\n",
            self.template_render_count.load(Ordering::Relaxed)
        ));

        // Coarse middleware total (populated from middleware_log snapshot at end of request).
        out.push_str(
            "# HELP soli_middleware_duration_seconds Total time spent in middleware (all middleware combined).\n",
        );
        out.push_str("# TYPE soli_middleware_duration_seconds counter\n");
        out.push_str(&format!(
            "soli_middleware_duration_seconds {:.9}\n",
            self.middleware_duration_ns_total.load(Ordering::Relaxed) as f64 / 1_000_000_000.0
        ));
        out.push_str("# HELP soli_middleware_duration_seconds_count Number of requests that went through middleware timing.\n");
        out.push_str("# TYPE soli_middleware_duration_seconds_count counter\n");
        out.push_str(&format!(
            "soli_middleware_duration_seconds_count {}\n",
            self.middleware_count.load(Ordering::Relaxed)
        ));

        // Coarse DB / SolidB query time (from query_log snapshot).
        out.push_str(
            "# HELP soli_db_query_duration_seconds Total time spent in database / SolidB queries.\n",
        );
        out.push_str("# TYPE soli_db_query_duration_seconds counter\n");
        out.push_str(&format!(
            "soli_db_query_duration_seconds {:.9}\n",
            self.db_query_duration_ns_total.load(Ordering::Relaxed) as f64 / 1_000_000_000.0
        ));
        out.push_str("# HELP soli_db_query_duration_seconds_count Number of requests with DB query timing.\n");
        out.push_str("# TYPE soli_db_query_duration_seconds_count counter\n");
        out.push_str(&format!(
            "soli_db_query_duration_seconds_count {}\n",
            self.db_query_count.load(Ordering::Relaxed)
        ));

        out.push_str(
            "# HELP soli_vm_handler_demotions_total Handlers demoted from the bytecode VM to the tree-walking interpreter (cached per worker; non-zero means some handlers run on the slower engine).\n",
        );
        out.push_str("# TYPE soli_vm_handler_demotions_total counter\n");
        out.push_str(&format!(
            "soli_vm_handler_demotions_total {}\n",
            self.vm_handler_demotions_total.load(Ordering::Relaxed)
        ));

        out
    }

    pub fn record_lexing(&self, elapsed: Duration) {
        if !metrics_enabled() {
            return;
        }
        self.lexing_duration_ns_total
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
        self.lexing_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_parsing(&self, elapsed: Duration) {
        if !metrics_enabled() {
            return;
        }
        self.parsing_duration_ns_total
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
        self.parsing_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_vm_execution(&self, elapsed: Duration) {
        if !metrics_enabled() {
            return;
        }
        self.vm_execution_ns_total
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
        self.vm_execution_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one template render (view + layout + any partials executed during it).
    pub fn record_template_render(&self, elapsed: Duration) {
        if !metrics_enabled() {
            return;
        }
        self.template_render_duration_ns_total
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
        self.template_render_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record total time spent in middleware for one request (sum of all middleware durations).
    pub fn record_middleware(&self, elapsed: Duration) {
        if !metrics_enabled() {
            return;
        }
        self.middleware_duration_ns_total
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
        self.middleware_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Record total time spent in DB/SolidB queries for one request.
    pub fn record_db_queries(&self, elapsed: Duration) {
        if !metrics_enabled() {
            return;
        }
        self.db_query_duration_ns_total
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
        self.db_query_count.fetch_add(1, Ordering::Relaxed);
    }
}

pub struct VmTimingGuard {
    /// `None` when metrics are disabled — avoids the `Instant::now()` syscall
    /// on every `vm.run()`, which is on the hot path of every request handler.
    start: Option<Instant>,
}

impl VmTimingGuard {
    #[inline]
    pub fn new() -> Self {
        Self {
            start: metrics_enabled().then(Instant::now),
        }
    }
}

impl Default for VmTimingGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for VmTimingGuard {
    #[inline]
    fn drop(&mut self) {
        if let Some(start) = self.start {
            Metrics::global().record_vm_execution(start.elapsed());
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}
