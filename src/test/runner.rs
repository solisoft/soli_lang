//! Test runner with parallel execution support.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam::channel;
use rand::seq::SliceRandom;
use rand::SeedableRng;

use crate::test::{next_test_id, TestCase, TestConfig, TestResult, TestState, TestSuite};

/// Run tests in parallel with configurable workers.
pub fn run_tests(suites: Vec<TestSuite>, config: &TestConfig) -> HashMap<u64, TestResult> {
    let mut results = HashMap::new();

    // Collect all tests
    let mut all_tests: Vec<TestCase> = suites.into_iter().flat_map(|suite| suite.tests).collect();

    // Randomize if configured
    if config.randomize {
        let mut rng = match config.seed {
            Some(seed) => rand::rngs::StdRng::seed_from_u64(seed),
            None => rand::rngs::StdRng::from_entropy(),
        };
        all_tests.shuffle(&mut rng);
    }

    // Setup channels for worker communication
    let (test_tx, test_rx) = channel::bounded(all_tests.len());
    let (result_tx, result_rx) = channel::bounded(all_tests.len());

    // Send all tests to workers
    for test in all_tests {
        let _ = test_tx.send(test);
    }
    drop(test_tx);

    // Create worker threads
    let mut workers = Vec::new();
    let config = Arc::new(config.clone());
    let test_state = Arc::new(TestState::new());

    for worker_id in 0..config.workers {
        let test_rx = test_rx.clone();
        let result_tx = result_tx.clone();
        let config = config.clone();
        let test_state = test_state.clone();

        let worker = thread::Builder::new()
            .name(format!("test-worker-{}", worker_id))
            .spawn(move || {
                worker_loop(worker_id, test_rx, result_tx, &config, &test_state);
            });

        match worker {
            Ok(h) => workers.push(h),
            Err(e) => eprintln!("Failed to spawn worker {}: {}", worker_id, e),
        }
    }

    // Collect results
    for _ in 0..all_tests.len() {
        if let Ok((test_id, result)) = result_rx.recv_timeout(Duration::from_secs(300)) {
            results.insert(test_id, result);
        }
    }

    // Wait for workers to finish
    for worker in workers {
        let _ = worker.join();
    }

    results
}

/// Individual worker loop.
fn worker_loop(
    worker_id: usize,
    test_rx: channel::Receiver<TestCase>,
    result_tx: channel::Sender<(u64, TestResult)>,
    config: &Arc<TestConfig>,
    _test_state: &Arc<TestState>,
) {
    loop {
        match test_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(test) => {
                let result = execute_test(&test, config);
                let _ = result_tx.send((test.id, result));
            }
            Err(channel::RecvTimeoutError::Disconnected) => break,
            Err(channel::RecvTimeoutError::Timeout) => {
                // No more tests, exit
                break;
            }
        }
    }
}

/// Execute a single test.
fn execute_test(test: &TestCase, config: &Arc<TestConfig>) -> TestResult {
    if config.verbose {
        println!("  Running: {}", test.name);
    }

    // For now, just return passed since we don't have actual execution
    // This will be connected to the interpreter later
    TestResult::Passed
}

/// Run tests sequentially (single-threaded).
pub fn run_tests_sequential(
    suites: Vec<TestSuite>,
    config: &TestConfig,
) -> HashMap<u64, TestResult> {
    let mut results = HashMap::new();
    let mut all_tests: Vec<TestCase> = suites.into_iter().flat_map(|suite| suite.tests).collect();

    // Randomize if configured
    if config.randomize {
        let mut rng = match config.seed {
            Some(seed) => rand::rngs::StdRng::seed_from_u64(seed),
            None => rand::rngs::StdRng::from_entropy(),
        };
        all_tests.shuffle(&mut rng);
    }

    for test in all_tests {
        if config.verbose {
            println!("  Running: {}", test.name);
        }

        let result = execute_test(&test, config);
        results.insert(test.id, result);

        if config.fail_fast {
            if let TestResult::Failed(_) = result {
                break;
            }
        }
    }

    results
}

/// Shuffle tests with optional seed.
pub fn shuffle_tests(tests: &mut Vec<TestCase>, seed: Option<u64>) {
    let mut rng = match seed {
        Some(s) => rand::rngs::StdRng::seed_from_u64(s),
        None => rand::rngs::StdRng::from_entropy(),
    };
    tests.shuffle(&mut rng);
}

/// Split tests for parallel execution.
pub fn split_tests_for_workers(tests: Vec<TestCase>, num_workers: usize) -> Vec<Vec<TestCase>> {
    let mut chunks: Vec<Vec<TestCase>> = Vec::with_capacity(num_workers);
    for _ in 0..num_workers {
        chunks.push(Vec::new());
    }

    for (i, test) in tests.into_iter().enumerate() {
        chunks[i % num_workers].push(test);
    }

    chunks
}
