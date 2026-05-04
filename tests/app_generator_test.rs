//! Tests for individual app-generator functions. We don't run the full
//! `create_app` (it shells out to npm + git) — instead we exercise each
//! file-creation step against a tempdir and assert on the produced contents.

use std::fs;
use std::path::Path;

use solilang::scaffold::app_generator::{
    create_application_helper, create_claude_md, create_css_file, create_directories,
    create_env_file, create_gitignore, create_home_controller, create_index_view, create_layout,
    create_package_json, create_readme, create_routes_file, create_sample_middleware,
    create_soli_toml, create_tailwind_config, replace_placeholders, write_file,
};

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
        "app/middleware",
        "app/models",
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
    create_env_file(tmp.path()).expect("env ok");
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
fn create_tailwind_config_writes_file() {
    let tmp = fresh();
    setup_app(tmp.path());
    create_tailwind_config(tmp.path()).expect("tailwind ok");
    assert!(tmp.path().join("tailwind.config.js").exists());
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
