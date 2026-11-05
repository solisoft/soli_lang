//! The docs and the agent guides `soli new` ships must not teach a bare
//! `Klass.new`.
//!
//! Without parentheses `Klass.new` is a member read, not a call, on both
//! engines: a class with a constructor answers with the class itself, any
//! other class with an unbound native `new`. `@post = Post.new` therefore
//! stores a function, and `Checkout.new.pay(1200)` raises "Cannot access
//! property 'pay' on Function". The agent guide in
//! `src/scaffold/templates/CLAUDE_controllers.md` (copied into every new
//! app's `app/controllers/CLAUDE.md`) and three docs pages taught exactly
//! that, contradicting `soli-language.md`, which says a bare `Counter.new` is
//! not a call.
//!
//! This scans the Soli code in those surfaces — fenced blocks in Markdown,
//! `<code class="language-soli">` blocks in the docs views, and whole `.sl` /
//! `.slv` scaffold templates — for `Name.new` not followed by `(`. A few
//! builtin classes do answer a bare `.new` with an instance; they are
//! allowed. Prose that explains the trap sits outside code blocks and is not
//! scanned.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

/// Builtin classes whose bare `.new` reads as an instance on both engines
/// (checked with `type(X.new)` under `--vm` and `--tree`: `Hash`, `Set` and a
/// ULID string). `Range.new` and `NanoID.new` read bare are functions, so they
/// are not listed and the scan flags them.
const BARE_NEW_INSTANTIATES: &[&str] = &["Hash", "Set", "ULID"];

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// The lines of `text` inside ``` fences, with their 1-based line numbers.
fn markdown_code(text: &str) -> Vec<(usize, String)> {
    let mut in_fence = false;
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            out.push((i + 1, line.to_string()));
        }
    }
    out
}

/// The lines inside `<code class="language-soli…">…</code>` blocks.
fn view_code(text: &str, block: &Regex) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for caps in block.captures_iter(text) {
        let body = caps.get(1).unwrap();
        let first_line = text[..body.start()].matches('\n').count() + 1;
        for (j, line) in body.as_str().lines().enumerate() {
            out.push((first_line + j, line.to_string()));
        }
    }
    out
}

fn whole_file(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.to_string()))
        .collect()
}

/// Every bare `Klass.new` in the code of the scanned files, as
/// `path:line: source`.
fn bare_new_offenders(repo: &Path) -> Vec<String> {
    let bare_new = Regex::new(r"\b([A-Z][A-Za-z0-9_]*)\.new(?:[^(A-Za-z0-9_?!]|$)").unwrap();
    // A `#` comment (not a `#{…}` interpolation) to the end of the line.
    let comment = Regex::new(r"(?:^|\s)#(?:[^{].*)?$").unwrap();
    let soli_block = Regex::new(r#"(?s)<code class="language-soli[^"]*">(.*?)</code>"#).unwrap();

    let mut offenders = Vec::new();
    for root in ["www/docs", "www/app/views/docs", "src/scaffold/templates"] {
        let mut files = Vec::new();
        files_under(&repo.join(root), &mut files);
        files.sort();
        for path in files {
            let name = path.to_string_lossy().to_string();
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let lines = if name.ends_with(".md") {
                markdown_code(&text)
            } else if name.ends_with(".slv") && root != "src/scaffold/templates" {
                view_code(&text, &soli_block)
            } else if name.ends_with(".sl") || name.ends_with(".slv") {
                whole_file(&text)
            } else {
                continue;
            };
            for (line_no, line) in lines {
                let code = comment.replace(&line, "");
                for caps in bare_new.captures_iter(&code) {
                    let class = &caps[1];
                    if BARE_NEW_INSTANTIATES.contains(&class) {
                        continue;
                    }
                    let relative = path.strip_prefix(repo).unwrap_or(&path).display();
                    offenders.push(format!("{relative}:{line_no}: {}", line.trim()));
                }
            }
        }
    }
    offenders
}

#[test]
fn docs_and_agent_guides_call_new_with_parentheses() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let offenders = bare_new_offenders(repo);
    assert!(
        offenders.is_empty(),
        "bare `Klass.new` is a member read, not a call — write `Klass.new()`:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_scan_sees_a_bare_new() {
    let bare_new = Regex::new(r"\b([A-Z][A-Za-z0-9_]*)\.new(?:[^(A-Za-z0-9_?!]|$)").unwrap();
    for offending in [
        "@post = Post.new",
        "Checkout.new.pay(1200)",
        "x = [User.new]",
    ] {
        assert!(bare_new.is_match(offending), "{offending}");
    }
    for fine in [
        "@post = Post.new()",
        "Checkout.new().pay(1200)",
        "Post.new_record",
        "Post.new?",
        "post.new",
    ] {
        assert!(!bare_new.is_match(fine), "{fine}");
    }
    // Only builtins whose bare `.new` really instantiates are exempt.
    for flagged in ["Range", "NanoID", "Post"] {
        assert!(!BARE_NEW_INSTANTIATES.contains(&flagged), "{flagged}");
    }
    for exempt in ["Hash", "Set", "ULID"] {
        assert!(BARE_NEW_INSTANTIATES.contains(&exempt), "{exempt}");
    }
    let md = "text Post.new\n```soli\n@post = Post.new\n```\n";
    assert_eq!(markdown_code(md), vec![(3, "@post = Post.new".to_string())]);
}

/// The house rule on `()` for a zero-argument call, as one surface states
/// it: the bullet from its bold title to the next bullet, joined to one line.
fn zero_argument_rule(text: &str) -> Option<String> {
    let start = text.find("- **No `()` on a zero-argument method call**")?;
    let rest = &text[start + 2..];
    let end = rest.find("\n- ").unwrap_or(rest.len());
    Some(rest[..end].split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The root `CLAUDE.md`, the guide `soli new` ships and the language docs
/// give one rule for `()` on a zero-argument call, and it keeps them on
/// `Klass.new()`. The two guides said every zero-argument call drops them,
/// while the docs (and this file's scan) require `Klass.new()`, and the
/// docs alone kept them on model writes.
#[test]
fn the_guides_and_the_docs_state_one_rule_for_zero_argument_calls() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |path: &str| fs::read_to_string(repo.join(path)).unwrap_or_default();
    let rules: Vec<(&str, Option<String>)> = [
        "CLAUDE.md",
        "src/scaffold/templates/CLAUDE.md",
        "www/docs/soli-language.md",
    ]
    .into_iter()
    .map(|path| (path, zero_argument_rule(&read(path))))
    .collect();
    let first = rules[0].1.clone().unwrap_or_default();
    assert!(
        first.contains("`Klass.new()`"),
        "CLAUDE.md keeps `()` on `Klass.new()`: {first}"
    );
    for (path, rule) in &rules {
        assert_eq!(
            rule.as_deref(),
            Some(first.as_str()),
            "{path} states the rule as CLAUDE.md does"
        );
    }
    let index = read("www/app/views/docs/language/index.html.slv");
    let line = index
        .lines()
        .find(|line| line.contains("on a zero-argument method call</strong>"))
        .unwrap_or_default();
    for keeps in ["Klass.new()", "current_user()", "list.sort()[0]"] {
        assert!(
            line.contains(keeps),
            "the language index keeps `()` on {keeps}: {line}"
        );
    }
}
