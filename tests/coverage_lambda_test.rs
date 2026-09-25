//! Regression test: the body of a lambda is counted against the file that
//! *defines* it, not against the entry script that happens to be running.
//!
//! Before the fix, `evaluate_lambda` stamped each lambda with
//! `current_source_path` — the entry script (the spec, or the request's
//! controller). Every line the lambda executed was then keyed to that file and
//! dropped by the reporter, so a service whose static methods build results in
//! lambdas showed those lines as never run. Found in an app where fifteen
//! search-result shapes, each rendered by a passing test, sat at 0 hits. Inside
//! a controller the entry script *is* the controller, which is why only
//! services looked broken. The fix takes the calling frame's file, as
//! statement coverage already does.

use solilang::coverage::{CoverageConfig, CoverageTracker};
use std::sync::{Arc, Mutex};

#[test]
fn lambda_bodies_are_counted_in_their_own_file() {
    let dir = tempfile::tempdir().unwrap();
    let service_path = dir.path().join("shapes.sl");
    let test_path = dir.path().join("shapes_run.sl");

    // Lines 4-6 are the lambda built in a static method, line 12 the one built
    // in an instance method.
    let service_src = "\
class Shapes
  static def from_static
    return Shapes.apply(fn(x) {
      {
        \"a\": x
      }
    })
  end

  def from_instance
    return Shapes.apply(fn(x) {
      let row = {\"b\": x}
      row
    })
  end

  static def apply(shape)
    return shape(1)
  end
end
";
    std::fs::write(&service_path, service_src).unwrap();

    let test_src = "let a = Shapes.from_static();\nlet b = Shapes.new().from_instance();\n";
    std::fs::write(&test_path, test_src).unwrap();

    let mut tracker = CoverageTracker::new(CoverageConfig::new());
    tracker.register_executable_lines_from_source(&service_path, service_src);
    let tracker = Arc::new(Mutex::new(tracker));

    // As the test runner does it: the service is a preamble file, and the
    // entry script — `current_source_path` — is the test file.
    let (_assertions, result) = solilang::run_with_path_and_coverage(
        test_src,
        Some(&test_path),
        false,
        Some(&tracker),
        Some(&test_path),
        &[(service_path.clone(), service_src.to_string())],
    );
    assert!(result.is_ok(), "run failed: {:?}", result.err());

    let aggregated = tracker.lock().unwrap().get_aggregated_coverage();
    let file_cov = aggregated
        .file_coverages
        .iter()
        .find(|(path, _)| path.file_name().is_some_and(|n| n == "shapes.sl"))
        .map(|(_, cov)| cov)
        .expect("service file should appear in coverage");

    let unhit: Vec<usize> = file_cov
        .lines
        .iter()
        .filter(|(_, line)| line.hits == 0)
        .map(|(n, _)| *n)
        .collect();
    assert!(
        unhit.is_empty(),
        "every line of shapes.sl runs, yet these were counted as never hit: {unhit:?}"
    );
}
