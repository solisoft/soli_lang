//! `soli check` on the in-repo corpus: zero false positives, held there (#693).
//!
//! An agent acts on what the checker says. A false error sends it down a
//! wrong path, and a gate that cries wolf is one people learn to skip. So the
//! code this repository ships — the docs site, the examples, and what every
//! generator writes — is checked here, the same way `soli check` checks it
//! (`solilang::check`), and each file's error count is compared with
//! `tests/check_corpus_baseline.txt`, a ratchet that may only shrink.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const BASELINE: &str = include_str!("check_corpus_baseline.txt");

fn baseline() -> BTreeMap<String, usize> {
    BASELINE
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (file, count) = line
                .rsplit_once(' ')
                .expect("baseline: `<corpus>/<path> <count>`");
            (
                file.to_string(),
                count.parse().expect("baseline: a count after the path"),
            )
        })
        .collect()
}

/// Per file of `root`, the errors `soli check <root>` reports and their text,
/// keyed `<label>/<path relative to root>`.
fn check_corpus(label: &str, root: &Path, out: &mut BTreeMap<String, Vec<String>>) {
    let targets = vec![root.to_path_buf()];
    let project = solilang::check::project(&targets);
    let files = solilang::check::source_files(root);
    assert!(!files.is_empty(), "{label}: no .sl file under {root:?}");
    for file in files {
        let report = solilang::check::check_file(&file, &project).expect("read corpus file");
        let rel = file.strip_prefix(root).unwrap_or(&file);
        let key = format!("{label}/{}", rel.display());
        out.insert(key, report.errors.iter().map(|e| e.to_string()).collect());
    }
}

fn write(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// What `soli new` writes, minus the Tailwind build and `git init` (which
/// write no Soli).
fn new_app(root: &Path, eui: bool) {
    use solilang::scaffold::app_generator::*;

    create_directories(root).unwrap();
    create_routes_file(root, eui).unwrap();
    create_seeds_file(root).unwrap();
    create_application_config(root).unwrap();
    create_soli_toml(root, "corpus").unwrap();
    create_home_controller(root).unwrap();
    create_layout(root).unwrap();
    create_index_view(root).unwrap();
    create_application_helper(root).unwrap();
    create_sample_middleware(root).unwrap();
    create_stdlib(root).unwrap();
    #[cfg(feature = "eui")]
    if eui {
        create_eui(root).unwrap();
    }
}

fn scaffolds(out: &mut BTreeMap<String, Vec<String>>) -> Vec<tempfile::TempDir> {
    let mut dirs = Vec::new();

    let plain = tempfile::tempdir().unwrap();
    new_app(plain.path(), false);
    check_corpus("new", plain.path(), out);
    dirs.push(plain);

    #[cfg(feature = "eui")]
    {
        let eui = tempfile::tempdir().unwrap();
        new_app(eui.path(), true);
        check_corpus("new-eui", eui.path(), out);
        dirs.push(eui);
    }

    for (label, passkeys) in [("auth", false), ("auth-passkeys", true)] {
        let app = tempfile::tempdir().unwrap();
        new_app(app.path(), false);
        solilang::scaffold::create_auth_with(app.path().to_str().unwrap(), passkeys)
            .expect("generate auth");
        check_corpus(label, app.path(), out);
        dirs.push(app);
    }

    let app = tempfile::tempdir().unwrap();
    new_app(app.path(), false);
    solilang::scaffold::create_scaffold_with_fields(
        app.path().to_str().unwrap(),
        "post",
        &[
            "title:string".to_string(),
            "body:text".to_string(),
            "views:integer".to_string(),
            "published:boolean".to_string(),
        ],
    )
    .expect("generate scaffold");
    check_corpus("scaffold", app.path(), out);
    dirs.push(app);

    dirs
}

#[test]
fn the_corpus_reports_no_more_errors_than_its_baseline() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    check_corpus("www", &repo.join("www"), &mut found);
    check_corpus("examples", &repo.join("examples"), &mut found);
    let _dirs = scaffolds(&mut found);

    // www alone has 40 files and every generated app a dozen or more: a
    // corpus this small means a walk stopped finding them.
    assert!(found.len() > 150, "only {} corpus files found", found.len());

    let allowed = baseline();
    let mut over = Vec::new();
    let mut under = Vec::new();
    for (file, errors) in &found {
        let limit = allowed.get(file).copied().unwrap_or(0);
        if errors.len() > limit {
            over.push(format!(
                "{file}: {} error(s), baseline {limit}\n    {}",
                errors.len(),
                errors.join("\n    ")
            ));
        } else if errors.len() < limit {
            under.push(format!("{file} {} (baseline says {limit})", errors.len()));
        }
    }
    for file in allowed.keys() {
        if !found.contains_key(file) {
            under.push(format!("{file}: no longer in the corpus"));
        }
    }

    assert!(
        over.is_empty(),
        "\n`soli check` reports errors the corpus baseline does not allow. Each is a \
         real bug (fix the code) or a false positive (fix the checker):\n  {}\n",
        over.join("\n  ")
    );
    assert!(
        under.is_empty(),
        "\nFewer errors than tests/check_corpus_baseline.txt allows. Good — lower or \
         delete these lines so the ratchet stays tight:\n  {}\n",
        under.join("\n  ")
    );
}

/// The library view the corpus is read through is the project view: a
/// controller calling a service's `def` with no import is not an error.
#[test]
fn a_scaffold_written_in_an_app_is_checked_against_the_app() {
    let dir = tempfile::tempdir().unwrap();
    new_app(dir.path(), false);
    write(
        dir.path(),
        "app/services/greeting.sl",
        "def greeting(name)\n  \"Hello, #{name}\"\nend\n",
    );
    write(
        dir.path(),
        "app/controllers/greet_controller.sl",
        "class GreetController < Controller\n  # GET /greet\n  def index(req)\n    @text = greeting(\"x\")\n  end\nend\n",
    );
    let mut found = BTreeMap::new();
    check_corpus("app", dir.path(), &mut found);
    for (file, errors) in &found {
        assert!(errors.is_empty(), "{file}: {errors:?}");
    }
}
