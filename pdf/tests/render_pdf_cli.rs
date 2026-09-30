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
