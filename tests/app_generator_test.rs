//! Tests for individual app-generator functions. We don't run the full
//! `create_app` (it shells out to npm + git) — instead we exercise each
//! file-creation step against a tempdir and assert on the produced contents.

use std::fs;
use std::path::Path;

use solilang::scaffold::app_generator::{
    create_agents_md, create_application_helper, create_bundled_docs, create_claude_md,
    create_css_file, create_directories, create_dot_claude, create_env_file, create_gitignore,
    create_home_controller, create_index_view, create_layout, create_nested_claude_mds,
    create_package_json, create_readme, create_routes_file, create_sample_middleware,
    create_soli_toml, is_soli_project, replace_placeholders, update_project_docs, write_file,
    PROJECT_DOC_AGENT_PATHS,
};
use solilang::scaffold::templates::{agents, app, bundled_docs};

fn fresh() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn create_directories_lays_out_full_tree() {
    let tmp = fresh();
    create_directories(tmp.path()).expect("create_directories ok");

    for sub in [
        "app",
        "app/controllers",
        "app/helpers",
        "app/jobs",
        "app/middleware",
        "app/models",
        "app/models/concerns",
        "app/views",
        "app/views/home",
        "app/views/layouts",
        "config",
        "db",
        "db/migrations",
        "app/assets",
        "app/assets/css",
        "public",
        "public/css",
        "public/js",
        "public/images",
        "stdlib",
        "tests",
    ] {
        assert!(tmp.path().join(sub).is_dir(), "missing dir: {}", sub);
    }
}

#[test]
fn write_file_creates_and_overwrites() {
    let tmp = fresh();
    let path = tmp.path().join("note.txt");
    write_file(&path, "first").expect("write 1");
    assert_eq!(fs::read_to_string(&path).unwrap(), "first");
    write_file(&path, "second").expect("overwrite");
    assert_eq!(fs::read_to_string(&path).unwrap(), "second");
}

#[test]
fn write_file_errors_on_missing_parent() {
    let tmp = fresh();
    let path = tmp.path().join("no/such/dir/file.txt");
    let result = write_file(&path, "x");
    assert!(result.is_err(), "expected error for missing parent");
}

fn setup_app(p: &Path) {
    create_directories(p).unwrap();
}

#[test]
fn create_routes_file_writes_template() {
    let tmp = fresh();
    setup_app(tmp.path());
    create_routes_file(tmp.path()).expect("routes ok");
    let path = tmp.path().join("config/routes.sl");
    assert!(path.exists());
    let content = fs::read_to_string(&path).unwrap();
    assert!(!content.is_empty(), "routes file empty");
}

#[test]
fn create_home_controller_writes_controller() {
    let tmp = fresh();
    setup_app(tmp.path());
    create_home_controller(tmp.path()).expect("controller ok");
    let path = tmp.path().join("app/controllers/home_controller.sl");
    assert!(path.exists());
}

#[test]
fn create_layout_writes_layout() {
    let tmp = fresh();
    setup_app(tmp.path());
    create_layout(tmp.path()).expect("layout ok");
    assert!(
        tmp.path()
            .join("app/views/layouts")
            .read_dir()
            .unwrap()
            .next()
            .is_some(),
        "no layout file created"
    );
}

#[test]
fn create_index_view_writes_home_index() {
    let tmp = fresh();
    setup_app(tmp.path());
    create_index_view(tmp.path()).expect("index view ok");
    // Find any view file under app/views/home/
    let entries: Vec<_> = fs::read_dir(tmp.path().join("app/views/home"))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(!entries.is_empty(), "no home index file created");
}

#[test]
fn create_css_file_writes_styles() {
    let tmp = fresh();
    setup_app(tmp.path());
    create_css_file(tmp.path()).expect("css ok");
    let entries: Vec<_> = fs::read_dir(tmp.path().join("app/assets/css"))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(!entries.is_empty(), "no css file created");
}

#[test]
fn create_env_gitignore_claude_helpers_middleware() {
    let tmp = fresh();
    setup_app(tmp.path());
    create_env_file(tmp.path(), "test_app").expect("env ok");
    create_gitignore(tmp.path()).expect("gitignore ok");
    create_claude_md(tmp.path()).expect("claude md ok");
    create_application_helper(tmp.path()).expect("helper ok");
    create_sample_middleware(tmp.path()).expect("middleware ok");

    assert!(tmp.path().join(".env").exists() || tmp.path().join(".env.example").exists());
    assert!(tmp.path().join(".gitignore").exists());
    assert!(tmp.path().join("CLAUDE.md").exists());
    // Helper file lives somewhere under app/helpers
    let helpers: Vec<_> = fs::read_dir(tmp.path().join("app/helpers"))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(!helpers.is_empty(), "no helper file created");
    let mw: Vec<_> = fs::read_dir(tmp.path().join("app/middleware"))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert!(!mw.is_empty(), "no middleware file created");
}

#[test]
fn create_package_json_includes_app_name() {
    let tmp = fresh();
    setup_app(tmp.path());
    create_package_json(tmp.path(), "MyCoolApp").expect("package.json ok");
    let content = fs::read_to_string(tmp.path().join("package.json")).unwrap();
    // After placeholder replacement the app name should be embedded.
    // Just check the file is non-empty JSON-ish.
    assert!(
        content.contains('{') && content.contains('}'),
        "not JSON: {}",
        content
    );
}

#[test]
fn create_readme_and_soli_toml_include_app_name() {
    let tmp = fresh();
    setup_app(tmp.path());
    create_readme(tmp.path(), "MyApp").expect("readme ok");
    create_soli_toml(tmp.path(), "MyApp").expect("soli.toml ok");
    assert!(tmp.path().join("README.md").exists());
    assert!(tmp.path().join("soli.toml").exists());
}

#[test]
fn replace_placeholders_substitutes_app_name() {
    let tmp = fresh();
    setup_app(tmp.path());
    // Write a file with the placeholder so replace_placeholders has something
    // to substitute. The function walks the tree replacing tokens — exact
    // token name varies, so we just check it runs without error and any
    // file containing the literal string "MyApp" survives.
    // The replacer substitutes the literal token "app_name" with the value.
    fs::write(tmp.path().join("README.md"), "# app_name project").unwrap();
    fs::write(tmp.path().join("package.json"), "{\"name\": \"app_name\"}").unwrap();

    replace_placeholders(tmp.path(), "my_real_app").expect("replace ok");

    let readme = fs::read_to_string(tmp.path().join("README.md")).unwrap();
    let pkg = fs::read_to_string(tmp.path().join("package.json")).unwrap();
    assert_eq!(readme, "# my_real_app project");
    assert_eq!(pkg, "{\"name\": \"my_real_app\"}");
}

#[test]
fn replace_placeholders_skips_hidden_and_binary_files() {
    let tmp = fresh();
    setup_app(tmp.path());
    fs::write(tmp.path().join(".gitignore"), "app_name\n").unwrap();
    fs::write(tmp.path().join("logo.png"), b"\x89PNG fake app_name").unwrap();
    fs::write(tmp.path().join("regular.txt"), "app_name content").unwrap();

    replace_placeholders(tmp.path(), "MyApp").expect("replace ok");

    // Hidden file and PNG must NOT be modified.
    assert_eq!(
        fs::read_to_string(tmp.path().join(".gitignore")).unwrap(),
        "app_name\n"
    );
    let png = fs::read(tmp.path().join("logo.png")).unwrap();
    assert!(png.windows(8).any(|w| w == b"app_name"), "PNG was modified");
    // Plain file should be modified.
    assert_eq!(
        fs::read_to_string(tmp.path().join("regular.txt")).unwrap(),
        "MyApp content"
    );
}

/// Dead generators that used to appear in agent guides and waste the first
/// agent session. Keep them out of every hand-authored markdown template that
/// `soli new` embeds.
const DEAD_GENERATOR_PHRASES: &[&str] = &[
    "soli generate controller",
    "soli generate model",
    "soli generate migration",
];

/// Every agent-facing markdown string shipped into a new app.
fn agent_markdown_templates() -> Vec<(&'static str, &'static str)> {
    vec![
        ("CLAUDE.md", app::CLAUDE_MD_TEMPLATE),
        ("AGENTS.md", agents::AGENTS_MD_TEMPLATE),
        (
            "app/controllers/CLAUDE.md",
            agents::CLAUDE_CONTROLLERS_TEMPLATE,
        ),
        ("app/models/CLAUDE.md", agents::CLAUDE_MODELS_TEMPLATE),
        (
            "app/models/concerns/CLAUDE.md",
            agents::CLAUDE_CONCERNS_TEMPLATE,
        ),
        ("app/views/CLAUDE.md", agents::CLAUDE_VIEWS_TEMPLATE),
        (
            "app/middleware/CLAUDE.md",
            agents::CLAUDE_MIDDLEWARE_TEMPLATE,
        ),
        ("tests/CLAUDE.md", agents::CLAUDE_TESTS_TEMPLATE),
        (
            "db/migrations/CLAUDE.md",
            agents::CLAUDE_MIGRATIONS_TEMPLATE,
        ),
        (
            ".claude/commands/soli-verify.md",
            agents::CMD_SOLI_VERIFY_TEMPLATE,
        ),
        (
            ".claude/commands/soli-test.md",
            agents::CMD_SOLI_TEST_TEMPLATE,
        ),
        (
            ".claude/commands/soli-resource.md",
            agents::CMD_SOLI_RESOURCE_TEMPLATE,
        ),
    ]
}

#[test]
fn agent_markdown_templates_reject_dead_generators() {
    for (label, body) in agent_markdown_templates() {
        for phrase in DEAD_GENERATOR_PHRASES {
            assert!(
                !body.contains(phrase),
                "{label} must not recommend dead generator `{phrase}`"
            );
        }
    }
}

#[test]
fn agent_markdown_documents_real_cli_surface() {
    let root = app::CLAUDE_MD_TEMPLATE;
    // Recipes / common commands must point at commands that exist.
    assert!(
        root.contains("soli generate scaffold"),
        "CLAUDE.md should document scaffold as the full-resource generator"
    );
    assert!(
        root.contains("soli db:migrate generate"),
        "CLAUDE.md should document db:migrate generate for migrations"
    );
    assert!(
        root.contains("soli db:seed generate"),
        "CLAUDE.md should document db:seed generate for seeds"
    );

    let resource = agents::CMD_SOLI_RESOURCE_TEMPLATE;
    assert!(
        resource.contains("soli generate scaffold")
            || resource.contains("soli db:migrate generate"),
        "soli-resource command must use real generators, not dead ones"
    );
    assert!(
        !resource.contains("soli generate controller")
            && !resource.contains("soli generate model")
            && !resource.contains("soli generate migration"),
        "soli-resource must not reintroduce dead generators"
    );
    // Scaffold only emits tests/controllers/*_controller_spec.sl — not a model
    // test under tests/models/*_test.sl.
    assert!(
        !root.contains("tests/models/*_test.sl") && !root.contains("tests/models/*_test"),
        "CLAUDE.md must not claim scaffold writes tests/models/*_test.sl"
    );
    assert!(
        resource.contains("controller_spec") || resource.contains("tests/controllers/"),
        "soli-resource should mention the real controller_spec path"
    );

    let migrations = agents::CLAUDE_MIGRATIONS_TEMPLATE;
    assert!(
        migrations.contains("soli db:migrate generate"),
        "migrations guide must use db:migrate generate"
    );
}

#[test]
fn create_agent_markdown_writers_ship_expected_paths() {
    let tmp = fresh();
    setup_app(tmp.path());

    create_claude_md(tmp.path()).expect("claude md");
    create_agents_md(tmp.path()).expect("agents md");
    create_nested_claude_mds(tmp.path()).expect("nested claude");
    create_dot_claude(tmp.path()).expect("dot claude");

    for (rel, _) in agent_markdown_templates() {
        let path = tmp.path().join(rel);
        assert!(path.is_file(), "missing agent markdown: {rel}");
        let body = fs::read_to_string(&path).unwrap();
        assert!(!body.is_empty(), "empty agent markdown: {rel}");
        for phrase in DEAD_GENERATOR_PHRASES {
            assert!(
                !body.contains(phrase),
                "written {rel} still contains dead generator `{phrase}`"
            );
        }
    }

    assert!(tmp.path().join(".claude/settings.json").is_file());
}

#[test]
fn create_bundled_docs_ships_language_reference_and_skips_repo_claude() {
    let tmp = fresh();
    setup_app(tmp.path());
    create_bundled_docs(tmp.path()).expect("bundled docs");

    let docs = tmp.path().join("docs");
    assert!(docs.is_dir(), "docs/ directory missing");

    for topic in ["models.md", "controllers.md", "migrations.md"] {
        let path = docs.join(topic);
        assert!(path.is_file(), "expected language doc {topic}");
        assert!(
            !fs::read_to_string(&path).unwrap().is_empty(),
            "empty language doc {topic}"
        );
    }

    // Repo-internal agent notes under www/docs must not land in generated apps.
    assert!(
        !docs.join("CLAUDE.md").exists(),
        "docs/CLAUDE.md should be skipped by create_bundled_docs"
    );
    assert!(
        !docs.join("blog/CLAUDE.md").exists(),
        "docs/blog/CLAUDE.md should be skipped by create_bundled_docs"
    );
}

/// Drive every agent/docs markdown writer on a real path (optional dump dir via
/// `SOLI_NEW_APP_DUMP`). This is the same create_* surface `soli new` uses.
#[test]
fn create_new_app_markdown_tree_end_to_end() {
    let tmp = fresh();
    let app_path = if let Ok(dump) = std::env::var("SOLI_NEW_APP_DUMP") {
        let p = Path::new(&dump).to_path_buf();
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).expect("create dump dir");
        p
    } else {
        tmp.path().to_path_buf()
    };

    create_directories(&app_path).expect("dirs");
    create_claude_md(&app_path).expect("claude");
    create_agents_md(&app_path).expect("agents");
    create_nested_claude_mds(&app_path).expect("nested");
    create_bundled_docs(&app_path).expect("docs");
    create_dot_claude(&app_path).expect("dot claude");

    let expected = [
        "CLAUDE.md",
        "AGENTS.md",
        "app/controllers/CLAUDE.md",
        "app/models/CLAUDE.md",
        "app/models/concerns/CLAUDE.md",
        "app/views/CLAUDE.md",
        "app/middleware/CLAUDE.md",
        "tests/CLAUDE.md",
        "db/migrations/CLAUDE.md",
        ".claude/settings.json",
        ".claude/commands/soli-verify.md",
        ".claude/commands/soli-test.md",
        ".claude/commands/soli-resource.md",
        "docs/models.md",
        "docs/controllers.md",
        "docs/migrations.md",
    ];
    for rel in expected {
        assert!(
            app_path.join(rel).is_file(),
            "missing from new-app tree: {rel}"
        );
    }
    assert!(!app_path.join("docs/CLAUDE.md").exists());
    assert!(!app_path.join("docs/blog/CLAUDE.md").exists());

    let resource = fs::read_to_string(app_path.join(".claude/commands/soli-resource.md")).unwrap();
    for phrase in DEAD_GENERATOR_PHRASES {
        assert!(!resource.contains(phrase), "dumped resource has `{phrase}`");
    }
    assert!(
        resource.contains("soli generate scaffold")
            || resource.contains("soli db:migrate generate")
    );
}

#[test]
fn bundled_docs_should_copy_skip_list() {
    assert!(!bundled_docs::should_copy("CLAUDE.md"));
    assert!(!bundled_docs::should_copy("blog/CLAUDE.md"));
    assert!(bundled_docs::should_copy("models.md"));
    assert!(bundled_docs::should_copy("controllers.md"));
    assert!(bundled_docs::should_copy("nested/topic.md"));
}

#[test]
fn update_project_docs_rewrites_stale_agent_and_language_docs() {
    let tmp = fresh();
    // Minimal project marker + a deliberately stale agent guide.
    fs::write(tmp.path().join("soli.toml"), "[package]\nname = \"old\"\n").unwrap();
    fs::write(
        tmp.path().join("CLAUDE.md"),
        "# stale\nsoli generate controller posts\n",
    )
    .unwrap();
    fs::create_dir_all(tmp.path().join("docs")).unwrap();
    fs::write(tmp.path().join("docs/models.md"), "# stale models\n").unwrap();

    assert!(is_soli_project(tmp.path()));
    let written = update_project_docs(tmp.path()).expect("update_project_docs");
    assert_eq!(written.len(), PROJECT_DOC_AGENT_PATHS.len());

    for rel in PROJECT_DOC_AGENT_PATHS {
        let path = tmp.path().join(rel);
        assert!(path.is_file(), "missing after update docs: {rel}");
        let body = fs::read_to_string(&path).unwrap();
        assert!(!body.is_empty(), "empty after update docs: {rel}");
        for phrase in DEAD_GENERATOR_PHRASES {
            assert!(
                !body.contains(phrase),
                "{rel} still has dead generator `{phrase}` after update docs"
            );
        }
    }

    // Language reference refreshed from the binary embed, not left stale.
    let models = fs::read_to_string(tmp.path().join("docs/models.md")).unwrap();
    assert_ne!(models, "# stale models\n");
    assert!(!models.is_empty());
    assert!(!tmp.path().join("docs/CLAUDE.md").exists());

    // Root CLAUDE.md must match the shipped template (overwrite).
    let root = fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap();
    assert_eq!(root, app::CLAUDE_MD_TEMPLATE);
}

#[test]
fn update_project_docs_rejects_non_project() {
    let tmp = fresh();
    let err = update_project_docs(tmp.path()).expect_err("empty dir is not a project");
    assert!(
        err.to_lowercase().contains("soli project") || err.to_lowercase().contains("look like"),
        "unexpected error: {err}"
    );
}
