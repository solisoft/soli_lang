//! `soli <script> --profile` (#697): the sampling profiler names the hot
//! function on both engines, writes folded stacks or speedscope JSON, and
//! prints the hottest functions on stderr.

use std::path::Path;
use std::process::Command;

const HOT_SCRIPT: &str = r#"
def burn(n)
  total = 0
  i = 0
  while i < n
    total = total + i % 7
    i = i + 1
  end
  total
end

def outer(n)
  burn(n)
end

print(outer(ITERATIONS))
"#;

fn script(dir: &Path, iterations: u32) {
    std::fs::write(
        dir.join("hot.sl"),
        HOT_SCRIPT.replace("ITERATIONS", &iterations.to_string()),
    )
    .unwrap();
}

fn soli(dir: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run soli");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn folded_profile_names_the_hot_function_on_both_engines() {
    for (engine, iterations) in [("--vm", 3_000_000), ("--tree", 300_000)] {
        let dir = tempfile::tempdir().unwrap();
        script(dir.path(), iterations);
        let (ok, stdout, stderr) = soli(
            dir.path(),
            &[engine, "hot.sl", "--profile", "--profile-out", "hot.folded"],
        );
        assert!(ok, "{engine}: {stderr}");
        // The script ran: its output is untouched by the profiler.
        assert!(stdout.trim().parse::<i64>().is_ok(), "{engine}: {stdout}");
        assert!(stderr.contains("Profile:"), "{engine}: {stderr}");
        assert!(stderr.contains("burn (hot.sl:2)"), "{engine}: {stderr}");
        assert!(
            stderr.contains("Profile written to hot.folded"),
            "{engine}: {stderr}"
        );
        let folded = std::fs::read_to_string(dir.path().join("hot.folded")).unwrap();
        assert!(
            folded
                .lines()
                .any(|line| line.starts_with("<main>;outer (hot.sl:12);burn (hot.sl:2) ")),
            "{engine}: {folded}"
        );
    }
}

#[test]
fn json_output_is_a_speedscope_profile() {
    let dir = tempfile::tempdir().unwrap();
    script(dir.path(), 1_000_000);
    let (ok, _, stderr) = soli(
        dir.path(),
        &["--profile-hz", "500", "--profile-out=hot.json", "hot.sl"],
    );
    assert!(ok, "{stderr}");
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("hot.json")).unwrap())
            .unwrap();
    assert_eq!(doc["profiles"][0]["type"], "sampled");
    assert_eq!(doc["profiles"][0]["unit"], "milliseconds");
    assert_eq!(doc["profiles"][0]["name"], "hot.sl");
    let frames = doc["shared"]["frames"].as_array().unwrap();
    assert!(
        frames
            .iter()
            .any(|frame| frame["name"] == "burn" && frame["file"] == "hot.sl"),
        "{doc}"
    );
    assert!(stderr.contains("every 2000 µs"), "{stderr}");
}

#[test]
fn flags_after_the_script_stay_the_runners() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("args.sl"),
        "print(System.argv.join(\",\"))\n",
    )
    .unwrap();
    let (ok, stdout, stderr) = soli(dir.path(), &["args.sl", "--profile", "one", "--profile"]);
    assert!(ok, "{stderr}");
    // `--profile` is the runner's until the first script argument; after
    // that, everything is the script's.
    assert!(stdout.contains("one,--profile"), "{stdout}");
    assert!(
        stderr.contains("Profile written to soli-profile.folded"),
        "{stderr}"
    );
    assert!(dir.path().join("soli-profile.folded").exists());
}

#[test]
fn unsupported_profile_kinds_and_rates_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    script(dir.path(), 10);
    for args in [
        &["--profile=heap", "hot.sl"][..],
        &["--profile-hz", "0", "hot.sl"][..],
        &["--profile-hz", "fast", "hot.sl"][..],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_soli"))
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(64), "{args:?}");
    }
}

const ALLOC_SCRIPT: &str = r#"
def make_rows(n)
  (0..n).map { |i| {"id": i, "title": "Row number #{i}", "tags": ["a", "b", "c"]} }
end

def small(n)
  total = 0
  (0..n).each do |i|
    total = total + i
  end
  total
end

print(make_rows(20000).length)
print(small(20000))
"#;

/// `--profile=alloc` weighs bytes: the function that builds 20 000 hashes
/// carries far more than the one that only adds integers, on both engines.
#[test]
fn an_allocation_profile_names_the_allocating_function() {
    for engine in ["--vm", "--tree"] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alloc.sl"), ALLOC_SCRIPT).unwrap();
        let (ok, stdout, stderr) = soli(
            dir.path(),
            &[
                engine,
                "alloc.sl",
                "--profile=alloc",
                "--profile-out",
                "alloc.folded",
            ],
        );
        assert!(ok, "{engine}: {stderr}");
        assert!(stdout.starts_with("20000\n"), "{engine}: {stdout}");
        assert!(
            stderr.contains("MB allocated by Soli code"),
            "{engine}: {stderr}"
        );
        assert!(stderr.contains("self KB"), "{engine}: {stderr}");
        let folded = std::fs::read_to_string(dir.path().join("alloc.folded")).unwrap();
        let bytes_under = |name: &str| -> u64 {
            folded
                .lines()
                .filter(|line| line.contains(name))
                .filter_map(|line| line.rsplit(' ').next()?.parse::<u64>().ok())
                .sum()
        };
        let rows = bytes_under("make_rows (alloc.sl:2)");
        let small = bytes_under("small (alloc.sl:6)");
        assert!(rows > 1_000_000, "{engine}: make_rows {rows}\n{folded}");
        assert!(
            rows > 10 * small,
            "{engine}: make_rows {rows}, small {small}"
        );
    }
}
