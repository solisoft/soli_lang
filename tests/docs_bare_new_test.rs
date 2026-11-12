//! The docs and the agent guides `soli new` ships state one rule for `()` on
//! a zero-argument call — and, since #655, that a bare `Klass.new` builds an
//! instance.
//!
//! Before #655 a bare `Post.new` was the constructor itself, uncalled, and
//! this file scanned every code sample for one. Both engines now construct
//! on a bare `Klass.new`, so the scan is gone; what stays is that the
//! surfaces agree, and that none of them still teaches the old trap.

use std::fs;
use std::path::Path;

/// The house rule on `()` for a zero-argument call, as one surface states
/// it: the bullet from its bold title to the next bullet, joined to one line.
fn zero_argument_rule(text: &str) -> Option<String> {
    let start = text.find("- **No `()` on a zero-argument method call**")?;
    let rest = &text[start + 2..];
    let end = rest.find("\n- ").unwrap_or(rest.len());
    Some(rest[..end].split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The root `CLAUDE.md`, the guide `soli new` ships and the language docs
/// give one rule, and it says `Post.new` builds an instance: no `Klass.new()`
/// exception left.
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
        first.contains("`Post.new` builds an instance like `Post.new()`"),
        "CLAUDE.md says a bare `Post.new` builds an instance: {first}"
    );
    assert!(!first.contains("`Klass.new()`"), "{first}");
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
    assert!(line.contains("current_user()"), "{line}");
    assert!(!line.contains("Klass.new()"), "{line}");
}

/// No docs page or shipped guide still warns that a bare `.new` is not a
/// call.
#[test]
fn no_surface_teaches_the_old_bare_new_trap() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut stack = vec![
        repo.join("www/docs"),
        repo.join("www/app/views/docs"),
        repo.join("src/scaffold/templates"),
    ];
    let mut offenders = Vec::new();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            // The changelog records the old behaviour on purpose.
            if path.ends_with("changelog.html.slv") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap_or_default();
            let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
            for trap in [
                "Counter.new` is not a call",
                "Counter.new</code> is not a call",
                "bare `.new` returns the function",
                "write `Model.new()`",
                "<code class=\"text-amber-400\">Model.new()</code>",
            ] {
                if flat.contains(trap) {
                    offenders.push(format!("{}: {trap}", path.display()));
                }
            }
        }
    }
    assert!(offenders.is_empty(), "{offenders:#?}");
}
