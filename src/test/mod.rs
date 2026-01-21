//! Test framework core module.

pub mod runner;
pub mod reporter;
pub mod discovery;
pub mod coverage;

use std::sync::atomic::{AtomicU64, Ordering};

/// Global test counter for unique test IDs.
static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Get next unique test ID.
pub fn next_test_id() -> u64 {
    TEST_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Test result types.
#[derive(Debug, Clone, PartialEq)]
pub enum TestResult {
    Passed,
    Failed(String),
    Pending(Option<String>),
    Error(String),
}

/// Test case representation.
#[derive(Debug, Clone)]
pub struct TestCase {
    pub id: u64,
    pub name: String,
    pub description: String,
    pub file: String,
    pub line: usize,
    pub code: String,
    pub before_each: Vec<String>,
    pub after_each: Vec<String>,
    pub tags: Vec<String>,
}

/// Test suite representation.
#[derive(Debug, Clone)]
pub struct TestSuite {
    pub name: String,
    pub tests: Vec<TestCase>,
    pub before_all: Option<String>,
    pub after_all: Option<String>,
}

/// Test configuration.
#[derive(Debug, Clone)]
pub struct TestConfig {
    pub verbose: bool,
    pub color: bool,
    pub randomize: bool,
    pub seed: Option<u64>,
    pub workers: usize,
    pub fail_fast: bool,
    pub coverage: bool,
    pub coverage_threshold: Option<f64>,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            verbose: false,
            color: true,
            randomize: false,
            seed: None,
            workers: num_cpus::get(),
            fail_fast: false,
            coverage: false,
            coverage_threshold: None,
        }
    }
}

impl TestConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    pub fn with_color(mut self, color: bool) -> Self {
        self.color = color;
        self
    }

    pub fn with_randomize(mut self, randomize: bool) -> Self {
        self.randomize = randomize;
        self
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self.randomize = true;
        self
    }

    pub fn with_workers(mut self, workers: usize) -> Self {
        self.workers = workers;
        self
    }

    pub fn with_fail_fast(mut self, fail_fast: bool) -> Self {
        self.fail_fast = fail_fast;
        self
    }

    pub fn with_coverage(mut self, enabled: bool) -> Self {
        self.coverage = enabled;
        self
    }

    pub fn with_coverage_threshold(mut self, threshold: f64) -> Self {
        self.coverage_threshold = Some(threshold);
        self.coverage = true;
        self
    }
}

/// Global test state.
#[derive(Debug, Default)]
pub struct TestState {
    pub current_test: Option<String>,
    pub test_count: AtomicU64,
    pub pass_count: AtomicU64,
    pub fail_count: AtomicU64,
    pub pending_count: AtomicU64,
    pub error_count: AtomicU64,
    pub start_time: Option<std::time::Instant>,
}

impl TestState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&self) {
        self.test_count.store(0, Ordering::SeqCst);
        self.pass_count.store(0, Ordering::SeqCst);
        self.fail_count.store(0, Ordering::SeqCst);
        self.pending_count.store(0, Ordering::SeqCst);
        self.error_count.store(0, Ordering::SeqCst);
    }

    pub fn record_pass(&self) {
        self.test_count.fetch_add(1, Ordering::SeqCst);
        self.pass_count.fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_fail(&self) {
        self.test_count.fetch_add(1, Ordering::SeqCst);
        self.fail_count.fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_pending(&self) {
        self.test_count.fetch_add(1, Ordering::SeqCst);
        self.pending_count.fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_error(&self) {
        self.test_count.fetch_add(1, Ordering::SeqCst);
        self.error_count.fetch_add(1, Ordering::SeqCst);
    }
}

/// Shared test state instance.
pub static TEST_STATE: TestState = TestState::new();
