//! Regression test: `new()` constructor bodies must be counted in code
//! coverage, exactly like `def` method bodies are.
//!
//! Before the fix, the four constructor-execution sites ran the body via a
//! bare `execute_block` with no call-stack frame, so coverage hits attributed
//! to the *caller* (the test file, under `tests/`) instead of the class's own
//! source file. The reporter then dropped those hits, leaving constructor body
//! lines permanently at 0 hits ("uncovered") in the denominator. Methods were
//! unaffected because `call_function_with_this` pushes a frame carrying the
//! method's `source_path`. The fix pushes the same frame for constructors.

use solilang::coverage::{CoverageConfig, CoverageTracker};
use std::sync::{Arc, Mutex};

#[test]
fn constructor_bodies_are_line_counted() {
    let dir = tempfile::tempdir().unwrap();
    let model_path = dir.path().join("widget.sl");
    let test_path = dir.path().join("widget_run.sl");

    // Line 4 (1-indexed) is the constructor body `this.name = name;` — the
    // line that was never counted before the fix.
    let model_src = "\
class Widget {
    name: String;
    new(name: String) {
        this.name = name;
    }
}
";
    std::fs::write(&model_path, model_src).unwrap();

    // Exercise both construction paths: the bare `Widget(...)` call form
    // (calls/function.rs) and the `new Widget(...)` form (objects/classes.rs).
    let test_src = "let a = Widget(\"hi\");\nlet b = new Widget(\"yo\");\n";
    std::fs::write(&test_path, test_src).unwrap();

    let mut tracker = CoverageTracker::new(CoverageConfig::new());
    tracker.register_executable_lines_from_source(&model_path, model_src);
    let tracker = Arc::new(Mutex::new(tracker));

    // Mirror the real test runner: the model is a preamble file (loaded before
    // the test, no imports → resolver skipped, so constructor statements are
    // NOT source-path-stamped), and the test file path is passed as
    // `source_file_path` so `current_source_path` is the test file during the
    // run — the exact conditions under which the bug manifests.
    let (_assertions, result) = solilang::run_with_path_and_coverage(
        test_src,
        Some(&test_path),
        false,
        Some(&tracker),
        Some(&test_path),
        &[(model_path.clone(), model_src.to_string())],
    );
    assert!(result.is_ok(), "run failed: {:?}", result.err());

    let aggregated = tracker.lock().unwrap().get_aggregated_coverage();

    // The tracker keys files by their canonicalized path; match on file name.
    let file_cov = aggregated
        .file_coverages
        .iter()
        .find(|(path, _)| path.file_name().is_some_and(|n| n == "widget.sl"))
        .map(|(_, cov)| cov)
        .expect("model file should appear in coverage");

    let ctor_line = file_cov
        .lines
        .get(&4)
        .expect("constructor body line 4 should be registered as executable");

    assert!(
        ctor_line.hits > 0,
        "constructor body line must be counted as covered, got {} hits (source: {:?})",
        ctor_line.hits,
        ctor_line.source_code,
    );
}
