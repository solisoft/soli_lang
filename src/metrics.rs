use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct Metrics {
    pub http_requests_total: AtomicU64,
    pub lexing_duration_ns_total: AtomicU64,
    pub lexing_count: AtomicU64,
    pub parsing_duration_ns_total: AtomicU64,
    pub parsing_count: AtomicU64,
    pub vm_execution_ns_total: AtomicU64,
    pub vm_execution_count: AtomicU64,
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

        out
    }

    pub fn record_lexing(&self, elapsed: Duration) {
        self.lexing_duration_ns_total
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
        self.lexing_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_parsing(&self, elapsed: Duration) {
        self.parsing_duration_ns_total
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
        self.parsing_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_vm_execution(&self, elapsed: Duration) {
        self.vm_execution_ns_total
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
        self.vm_execution_count.fetch_add(1, Ordering::Relaxed);
    }
}

pub struct VmTimingGuard {
    start: Instant,
}

impl VmTimingGuard {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for VmTimingGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for VmTimingGuard {
    fn drop(&mut self) {
        Metrics::global().record_vm_execution(self.start.elapsed());
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}
