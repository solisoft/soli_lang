//! The `render_pdf` binary: stdin in, PDF bytes out on stdout.

use std::io::Write;
use std::process::{Command, Stdio};

const TEMPLATE: &str = "tests/fixtures/template.json";
const DATA: &[u8] = include_bytes!("fixtures/data.json");

fn render_pdf() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_render_pdf"));
    command.args(["--font-dir", "fonts", "--no-images"]);
    command
}

#[test]
fn data_from_stdin_pdf_to_stdout() {
    let mut child = render_pdf()
        .args(["--template", TEMPLATE, "--data", "-", "-o", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(DATA)
        .expect("write");
    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.starts_with(b"%PDF-"));
}

#[test]
fn stdin_cannot_feed_two_inputs() {
    let output = render_pdf()
        .args(["--template", "-", "--data", "-", "-o", "-"])
        .stdin(Stdio::null())
        .output()
        .expect("run");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("only feed one"));
}

// ---- image sources -------------------------------------------------------
//
// Without a policy installed the library refuses every file and URL source,
// and `render_pdf` used to install none, so only `data:` URIs were drawn.

const SVG: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><rect width='10' height='10'/></svg>";

/// A scratch directory. Inside `CARGO_TARGET_TMPDIR` it sits under the
/// package root, the tests' working directory; under the system temp dir it
/// is outside it.
fn scratch(base: &std::path::Path, name: &str) -> std::path::PathBuf {
    let dir = base.join(format!("render_pdf_{name}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Renders a one-image template and returns the renderer's stderr.
fn render_image(dir: &std::path::Path, src: &str, extra: &[&str]) -> (bool, String) {
    let template = dir.join("template.json");
    let body = serde_json::json!({ "content": [{ "type": "image", "value": src, "width": 40 }] });
    std::fs::write(&template, body.to_string()).expect("template");
    let data = dir.join("data.json");
    std::fs::write(&data, "{}").expect("data");
    let output = Command::new(env!("CARGO_BIN_EXE_render_pdf"))
        .args(["--font-dir", "fonts", "-o", "-", "--template"])
        .arg(&template)
        .arg("--data")
        .arg(&data)
        .args(extra)
        .output()
        .expect("run");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn image_under_the_working_directory_is_drawn() {
    let dir = scratch(std::path::Path::new(env!("CARGO_TARGET_TMPDIR")), "cwd");
    let logo = dir.join("logo.svg");
    std::fs::write(&logo, SVG).expect("svg");
    let (ok, stderr) = render_image(&dir, &format!("file://{}", logo.display()), &[]);
    assert!(ok, "{stderr}");
    assert!(!stderr.contains("skipped"), "{stderr}");
}

#[test]
fn image_outside_the_allowed_directories_is_skipped() {
    let dir = scratch(&std::env::temp_dir(), "outside");
    let logo = dir.join("logo.svg");
    std::fs::write(&logo, SVG).expect("svg");
    let src = format!("file://{}", logo.display());

    let (ok, stderr) = render_image(&dir, &src, &[]);
    assert!(ok, "{stderr}");
    assert!(stderr.contains("outside the working directory"), "{stderr}");

    let (ok, stderr) = render_image(&dir, &src, &["--image-dir", dir.to_str().unwrap()]);
    assert!(ok, "{stderr}");
    assert!(!stderr.contains("skipped"), "{stderr}");
}

#[test]
fn image_on_a_private_address_is_refused() {
    let dir = scratch(std::path::Path::new(env!("CARGO_TARGET_TMPDIR")), "ssrf");
    for src in [
        "http://127.0.0.1:9/logo.png",
        "http://169.254.169.254/latest/meta-data",
    ] {
        let (ok, stderr) = render_image(&dir, src, &[]);
        assert!(ok, "{stderr}");
        assert!(stderr.contains("not a public address"), "{src}: {stderr}");
    }
}

#[test]
fn a_missing_image_dir_is_an_error() {
    let dir = scratch(std::path::Path::new(env!("CARGO_TARGET_TMPDIR")), "missing");
    let (ok, stderr) = render_image(&dir, "data:,", &["--image-dir", "/no/such/dir"]);
    assert!(!ok);
    assert!(stderr.contains("--image-dir"), "{stderr}");
}
