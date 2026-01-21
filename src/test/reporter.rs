//! Test result reporter with formatted output.

use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;

use crate::test::coverage::CoverageReport;
use crate::test::{TestConfig, TestResult, TestState};

/// Output format options.
#[derive(Debug, Clone, PartialEq)]
pub enum OutputFormat {
    Progress,
    Dots,
    Json,
    Tap,
}

/// Test reporter.
pub struct TestReporter {
    config: TestConfig,
    format: OutputFormat,
    start_time: Instant,
    pub total_duration: Duration,
}

impl TestReporter {
    pub fn new(config: TestConfig, format: OutputFormat) -> Self {
        Self {
            config,
            format,
            start_time: Instant::now(),
            total_duration: Duration::new(0, 0),
        }
    }

    /// Report test run start.
    pub fn report_start(&self, test_count: usize) {
        match self.format {
            OutputFormat::Progress => {
                println!("Running {} tests...\n", test_count);
            }
            OutputFormat::Dots => {
                print!("Running {} tests... ", test_count);
            }
            OutputFormat::Json | OutputFormat::Tap => {}
        }
    }

    /// Report individual test result.
    pub fn report_test(&mut self, name: &str, result: &TestResult, duration: Duration) {
        match self.format {
            OutputFormat::Progress => match result {
                TestResult::Passed => {
                    println!("  ✓ {} ({:.3}s)", name, as_seconds(duration));
                }
                TestResult::Failed(msg) => {
                    println!("  ✗ {} ({:.3}s)", name, as_seconds(duration));
                    println!("    Failure: {}", msg);
                }
                TestResult::Pending(reason) => {
                    println!("  ○ {} (pending)", name);
                    if let Some(r) = reason {
                        println!("    Reason: {}", r);
                    }
                }
                TestResult::Error(msg) => {
                    println!("  ✗ {} ({:.3}s) - ERROR", name, as_seconds(duration));
                    println!("    Error: {}", msg);
                }
            },
            OutputFormat::Dots => {
                match result {
                    TestResult::Passed => print!("."),
                    TestResult::Failed(_) | TestResult::Error(_) => print!("F"),
                    TestResult::Pending(_) => print!("."),
                }
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            OutputFormat::Json => {
                // JSON output handled in final report
            }
            OutputFormat::Tap => {
                // TAP output handled in final report
            }
        }
    }

    /// Report test run completion.
    pub fn report_end(
        &mut self,
        results: &HashMap<u64, TestResult>,
        test_state: &TestState,
        coverage: Option<&CoverageReport>,
    ) {
        self.total_duration = self.start_time.elapsed();

        match self.format {
            OutputFormat::Progress => {
                self.format_progress_report(results, test_state, coverage);
            }
            OutputFormat::Dots => {
                println!("\n");
                self.format_progress_report(results, test_state, coverage);
            }
            OutputFormat::Json => {
                self.format_json_report(results, test_state, coverage);
            }
            OutputFormat::Tap => {
                self.format_tap_report(results, test_state);
            }
        }
    }

    /// Format progress-style report.
    fn format_progress_report(
        &self,
        results: &HashMap<u64, TestResult>,
        test_state: &TestState,
        coverage: Option<&CoverageReport>,
    ) {
        let pass_count = results
            .values()
            .filter(|r| matches!(r, TestResult::Passed))
            .count();
        let fail_count = results
            .values()
            .filter(|r| matches!(r, TestResult::Failed(_)))
            .count();
        let error_count = results
            .values()
            .filter(|r| matches!(r, TestResult::Error(_)))
            .count();
        let pending_count = results
            .values()
            .filter(|r| matches!(r, TestResult::Pending(_)))
            .count();

        println!("{}", if self.config.color { "\x1b[0m" } else { "" });

        if fail_count == 0 && error_count == 0 {
            println!("\x1b[32m{} tests, 0 failures\x1b[0m", results.len());
        } else {
            println!(
                "\x1b[31m{} tests, {} failures\x1b[0m",
                results.len(),
                fail_count + error_count
            );
        }

        if pending_count > 0 {
            println!("{} pending", pending_count);
        }

        println!("Duration: {:.3}s", as_seconds(self.total_duration));
        println!("Workers: {}", self.config.workers);

        if let Some(coverage_report) = coverage {
            self.format_coverage_report(coverage_report);
        }

        // List failed tests
        if fail_count > 0 || error_count > 0 {
            println!("\nFailures:");
            for (id, result) in results {
                if let TestResult::Failed(msg) = result {
                    println!("  {}) {}", id, msg);
                }
            }
        }
    }

    /// Format coverage report.
    fn format_coverage_report(&self, coverage: &CoverageReport) {
        println!("\nCoverage: {:.1}%", coverage.total_percentage);

        if let Some(threshold) = coverage.threshold {
            if coverage.total_percentage < threshold {
                println!(
                    "\x1b[31mCoverage {:.1}% below threshold {:.1}%\x1b[0m",
                    coverage.total_percentage, threshold
                );
            } else {
                println!(
                    "\x1b[32mCoverage {:.1}% meets threshold {:.1}%\x1b[0m",
                    coverage.total_percentage, threshold
                );
            }
        }

        if self.config.verbose {
            println!("\nCoverage by file:");
            for (file, percentage) in &coverage.by_file {
                let status = if *percentage >= 70.0 {
                    "✓"
                } else if *percentage >= 50.0 {
                    "⚠"
                } else {
                    "✗"
                };
                println!("  {} {}: {:.1}%", status, file, percentage);
            }
        }
    }

    /// Format JSON report.
    fn format_json_report(
        &self,
        results: &HashMap<u64, TestResult>,
        test_state: &TestState,
        coverage: Option<&CoverageReport>,
    ) {
        use serde::Serialize;

        #[derive(Serialize)]
        struct JsonReport {
            summary: JsonSummary,
            tests: Vec<JsonTest>,
            coverage: Option<JsonCoverage>,
        }

        #[derive(Serialize)]
        struct JsonSummary {
            total: usize,
            passed: usize,
            failed: usize,
            pending: usize,
            duration_seconds: f64,
        }

        #[derive(Serialize)]
        struct JsonTest {
            id: u64,
            name: String,
            status: String,
            duration_seconds: f64,
            message: Option<String>,
        }

        #[derive(Serialize)]
        struct JsonCoverage {
            total_percentage: f64,
            by_file: HashMap<String, f64>,
        }

        let pass_count = results
            .values()
            .filter(|r| matches!(r, TestResult::Passed))
            .count();
        let fail_count = results
            .values()
            .filter(|r| matches!(r, TestResult::Failed(_)))
            .count();
        let pending_count = results
            .values()
            .filter(|r| matches!(r, TestResult::Pending(_)))
            .count();

        let report = JsonReport {
            summary: JsonSummary {
                total: results.len(),
                passed: pass_count,
                failed: fail_count,
                pending: pending_count,
                duration_seconds: as_seconds(self.total_duration),
            },
            tests: results
                .iter()
                .map(|(id, result)| JsonTest {
                    id: *id,
                    name: format!("Test {}", id),
                    status: match result {
                        TestResult::Passed => "passed".to_string(),
                        TestResult::Failed(_) => "failed".to_string(),
                        TestResult::Pending(_) => "pending".to_string(),
                        TestResult::Error(_) => "error".to_string(),
                    },
                    duration_seconds: 0.0,
                    message: match result {
                        TestResult::Passed => None,
                        TestResult::Failed(msg) => Some(msg.clone()),
                        TestResult::Pending(reason) => reason.clone(),
                        TestResult::Error(msg) => Some(msg.clone()),
                    },
                })
                .collect(),
            coverage: coverage.map(|c| JsonCoverage {
                total_percentage: c.total_percentage,
                by_file: c.by_file.clone(),
            }),
        };

        let json = serde_json::to_string_pretty(&report).unwrap();
        println!("{}", json);
    }

    /// Format TAP report.
    fn format_tap_report(&self, results: &HashMap<u64, TestResult>, _test_state: &TestState) {
        println!("TAP version 14");
        println!("1..{}", results.len());

        for (id, result) in results {
            let status = match result {
                TestResult::Passed => "ok".to_string(),
                TestResult::Failed(msg) => format!("not ok - {}", msg),
                TestResult::Pending(reason) => {
                    format!("ok # skip: {}", reason.as_deref().unwrap_or("pending"))
                }
                TestResult::Error(msg) => format!("not ok - {}", msg),
            };
            println!("{} {} - Test {}", id, status, id);
        }
    }
}

/// Convert duration to seconds as f64.
fn as_seconds(duration: Duration) -> f64 {
    duration.as_secs() as f64 + duration.subsec_nanos() as f64 / 1_000_000_000.0
}
