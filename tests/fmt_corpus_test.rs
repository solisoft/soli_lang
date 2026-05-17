//! Corpus-level safety check for `soli fmt`.
//!
//! Walks every `.sl` file under the project root, formats it, and verifies
//! the formatter's output re-lexes and re-parses cleanly. Catches any
//! lossy/incorrect rewrite the formatter performs across real Soli idioms
//! present in the codebase (postfix conditionals, ternaries, interfaces,
//! `#{...}` interpolation, `&{...}` block args, static blocks, etc.).
//!
//! Files whose original source already fails to lex or parse are skipped:
//! the formatter can't process them, and they don't gate this test.
//!
//! Also asserts idempotency on each successfully-formatted file:
//! `fmt(fmt(src)) == fmt(src)`. This catches subtle non-converging rewrites
//! that would manifest as spurious "would reformat" output on later runs.

use std::path::{Path, PathBuf};

fn collect_sl_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(s) => s,
            None => continue,
        };
        // Skip vendored/build dirs to keep the test fast and deterministic.
        if matches!(
            name,
            "target" | "node_modules" | ".git" | "dist" | "build" | ".cargo"
        ) {
            continue;
        }
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("sl") {
            out.push(path);
        }
    }
}

fn parses(source: &str) -> bool {
    let Ok(tokens) = solilang::lexer::Scanner::new(source).scan_tokens() else {
        return false;
    };
    solilang::parser::Parser::new(tokens).parse().is_ok()
}

#[test]
fn formatter_output_re_parses_on_every_repo_file() {
    let root = env!("CARGO_MANIFEST_DIR");
    let files = collect_sl_files(Path::new(root));
    assert!(!files.is_empty(), "no .sl files discovered under {}", root);

    let mut skipped: Vec<PathBuf> = Vec::new();
    let mut broken: Vec<(PathBuf, String)> = Vec::new();

    for file in &files {
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(_) => continue,
        };
        // Skip files the parser already can't handle — fmt won't touch them.
        if !parses(&source) {
            skipped.push(file.clone());
            continue;
        }
        let formatted = match solilang::fmt::format_source(&source) {
            Ok(s) => s,
            Err(e) => {
                broken.push((file.clone(), format!("format_source: {}", e)));
                continue;
            }
        };
        if !parses(&formatted) {
            // Surface a snippet of the broken output to make CI logs useful.
            let preview: String = formatted.lines().take(40).collect::<Vec<_>>().join("\n");
            broken.push((
                file.clone(),
                format!(
                    "re-parse FAILED. First 40 lines of fmt output:\n{}",
                    preview
                ),
            ));
            continue;
        }
        // Idempotency: fmt(fmt(x)) == fmt(x).
        match solilang::fmt::format_source(&formatted) {
            Ok(twice) if twice == formatted => {}
            Ok(twice) => {
                broken.push((
                    file.clone(),
                    format!(
                        "not idempotent: fmt(fmt(src)) != fmt(src) (delta {} bytes)",
                        (twice.len() as i64 - formatted.len() as i64).abs()
                    ),
                ));
            }
            Err(e) => {
                broken.push((file.clone(), format!("second format_source failed: {}", e)));
            }
        }
    }

    if !broken.is_empty() {
        let mut msg = format!(
            "{} files broken by soli fmt (out of {} scanned, {} pre-skipped):\n",
            broken.len(),
            files.len(),
            skipped.len()
        );
        for (path, reason) in &broken {
            msg.push_str(&format!("  - {}: {}\n", path.display(), reason));
        }
        panic!("{}", msg);
    }
}
