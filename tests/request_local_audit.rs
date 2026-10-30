//! Every `thread_local!` static under `src/` is either request state —
//! declared with `crate::request_local!`, so a worker can switch between
//! requests — or named in `tests/thread_locals_shared.txt` as shared by
//! the whole thread. A new plain thread-local fails here until it is put in
//! one of the two: holding per-request state in one would leak it between
//! requests served by the same worker.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// `(file, NAME)` of every static declared inside a `thread_local!` block.
fn thread_local_statics(dir: &Path, out: &mut BTreeSet<(String, String)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            thread_local_statics(&path, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap();
        let file = path.to_string_lossy().replace('\\', "/");
        let mut rest = source.as_str();
        while let Some(at) = rest.find("thread_local!") {
            let line_start = rest[..at].rfind('\n').map_or(0, |i| i + 1);
            let is_comment = rest[line_start..at].trim_start().starts_with("//");
            let after = &rest[at..];
            let Some(open) = after.find('{') else { break };
            let mut depth = 0usize;
            let mut end = open;
            for (i, c) in after[open..].char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if !is_comment {
                for line in after[open..end].lines() {
                    let line = line.trim_start();
                    let line = line
                        .strip_prefix("pub(crate) ")
                        .or_else(|| line.strip_prefix("pub(super) "))
                        .or_else(|| line.strip_prefix("pub "))
                        .unwrap_or(line);
                    if let Some(decl) = line.strip_prefix("static ") {
                        let name: String = decl
                            .chars()
                            .take_while(|c| {
                                c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_'
                            })
                            .collect();
                        if !name.is_empty() {
                            out.insert((file.clone(), name));
                        }
                    }
                }
            }
            rest = &after[end.max(1)..];
        }
    }
}

#[test]
fn every_thread_local_is_request_state_or_known_to_be_shared() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = BTreeSet::new();
    thread_local_statics(&root.join("src"), &mut found);
    let prefix = format!("{}/", root.to_string_lossy().replace('\\', "/"));
    let found: BTreeSet<(String, String)> = found
        .into_iter()
        .map(|(file, name)| {
            let file = file.strip_prefix(&prefix).unwrap_or(&file).to_string();
            (file, name)
        })
        .collect();

    let listed = fs::read_to_string(root.join("tests/thread_locals_shared.txt")).unwrap();
    let shared: BTreeSet<(String, String)> = listed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (file, name) = l.split_once(' ')?;
            Some((file.to_string(), name.trim().to_string()))
        })
        .collect();

    let unknown: Vec<_> = found.difference(&shared).collect();
    assert!(
        unknown.is_empty(),
        "thread_local! statics neither request state nor listed as shared — declare \
         request state with crate::request_local!, or add a line to \
         tests/thread_locals_shared.txt: {unknown:#?}"
    );
    let stale: Vec<_> = shared.difference(&found).collect();
    assert!(
        stale.is_empty(),
        "tests/thread_locals_shared.txt names statics that no longer exist: {stale:#?}"
    );
}
