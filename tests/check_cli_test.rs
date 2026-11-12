//! `soli check` on part of an application (#678).
//!
//! `soli check app` read `app/` as the project root, looked for
//! `app/app/models`, and reported every auto-loaded class as undefined, while
//! `soli check .` was clean. A target inside an application now widens to the
//! application's root (`soli.toml` or `config/routes.sl`) for the names in
//! scope, and still checks only the files it was given.

use std::path::Path;
use std::process::Command;

fn write(root: &Path, rel: &str, contents: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn app() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "config/routes.sl", "get(\"/\", \"posts#index\")\n");
    write(
        root,
        "app/models/audit_entry.sl",
        "class AuditEntry < Model\n  static def recent\n    AuditEntry.all\n  end\nend\n",
    );
    write(
        root,
        "app/services/report_service.sl",
        "def report_title(name)\n  \"Report: #{name}\"\nend\n",
    );
    write(
        root,
        "app/controllers/posts_controller.sl",
        "class PostsController < Controller\n  # GET /\n  def index(req)\n    @entries = AuditEntry.recent\n    @title = report_title(\"x\")\n  end\nend\n",
    );
    dir
}

fn check(dir: &Path, target: &str) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("check")
        .arg(target)
        .current_dir(dir)
        .output()
        .expect("run soli check");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

#[test]
fn checking_the_app_directory_sees_the_auto_loaded_names() {
    let dir = app();
    let (ok, text) = check(dir.path(), "app");
    assert!(ok, "{text}");
    assert!(!text.contains("Undefined"), "{text}");
    assert!(text.contains("Checked 3 file(s)"), "{text}");
}

#[test]
fn checking_one_subdirectory_or_file_checks_only_it() {
    let dir = app();
    let (ok, text) = check(dir.path(), "app/controllers");
    assert!(ok, "{text}");
    assert!(text.contains("Checked 1 file(s)"), "{text}");

    let (ok, text) = check(dir.path(), "app/controllers/posts_controller.sl");
    assert!(ok, "{text}");
    assert!(text.contains("Checked 1 file(s)"), "{text}");
}

#[test]
fn soli_toml_also_marks_the_root() {
    let dir = app();
    std::fs::remove_file(dir.path().join("config/routes.sl")).unwrap();
    std::fs::remove_dir(dir.path().join("config")).unwrap();
    write(dir.path(), "soli.toml", "");
    let (ok, text) = check(dir.path(), "app");
    assert!(ok, "{text}");
}

#[test]
fn a_loose_directory_keeps_the_narrow_view() {
    // No application around it: a name it does not declare is still undefined.
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "scripts/run.sl", "print(report_title(\"x\"))\n");
    let (ok, text) = check(dir.path(), "scripts");
    assert!(!ok, "{text}");
    assert!(text.contains("report_title"), "{text}");
}

#[test]
fn jobs_and_subscribers_are_auto_loaded_too() {
    // `soli check`'s list had neither (#691); the server loads both into the
    // application's environment.
    let dir = app();
    write(
        dir.path(),
        "app/jobs/cleanup_job.sl",
        "def schedule_cleanup(days)\n  days\nend\n",
    );
    write(
        dir.path(),
        "app/subscribers/audit_subscriber.sl",
        "def audit_topic\n  \"audit\"\nend\n",
    );
    write(
        dir.path(),
        "app/services/maintenance.sl",
        "def run_maintenance\n  schedule_cleanup(7)\n  audit_topic()\nend\n",
    );
    let (ok, text) = check(dir.path(), "app/services/maintenance.sl");
    assert!(ok, "{text}");
    assert!(!text.contains("Undefined"), "{text}");
}

#[test]
fn a_loose_script_still_cannot_render() {
    // `render` is request-scope only: the project view must not swallow it.
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "run.sl", "render(\"x\")\n");
    let (ok, text) = check(dir.path(), "run.sl");
    assert!(!ok, "{text}");
    assert!(text.contains("Undefined variable 'render'"), "{text}");
}

#[test]
fn checking_the_app_directory_or_the_root_agrees() {
    let dir = app();
    let (ok_app, text_app) = check(dir.path(), "app");
    let (ok_root, text_root) = check(dir.path(), ".");
    assert!(ok_app && ok_root, "{text_app}\n{text_root}");
    assert!(!text_app.contains("Undefined") && !text_root.contains("Undefined"));
}

#[test]
fn a_module_imported_by_a_window_script_sees_the_widget_catalogue() {
    // `examples/pocket-garden/`: the screens never mention `eui_window`, but
    // they run in the window's workers, which load the catalogue (#693).
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "game/garden.sl",
        "import \"./screen.sl\"\n\ndef handler(event_data)\n  {}\nend\n\neui_window(\"g\", \"handler\", \"screen_view\", {\"title\": \"g\"})\n",
    );
    write(
        dir.path(),
        "game/screen.sl",
        "export def screen_view(state)\n  button(\"x\", \"go\")\nend\n",
    );
    write(
        dir.path(),
        "alone/screen.sl",
        "export def screen_view(state)\n  button(\"x\", \"go\")\nend\n",
    );
    let (ok, text) = check(dir.path(), "game");
    assert!(ok, "{text}");
    let (ok, text) = check(dir.path(), "alone");
    assert!(
        !ok,
        "a module nobody opens a window with has no catalogue: {text}"
    );
    assert!(text.contains("button"), "{text}");
}

// Phase 2 of #693: the project's declarations are known by their signatures,
// not only by their names.

#[test]
fn a_neighbours_function_is_checked_for_arity() {
    let dir = app();
    write(
        dir.path(),
        "app/controllers/reports_controller.sl",
        "class ReportsController < Controller\n  # GET /reports\n  def index(req)\n    @title = report_title(\"x\", \"y\")\n  end\nend\n",
    );
    let (ok, text) = check(dir.path(), "app/controllers/reports_controller.sl");
    assert!(!ok, "{text}");
    assert!(text.contains("expected 1"), "{text}");
}

#[test]
fn a_neighbours_class_is_checked_for_its_members() {
    let dir = app();
    write(
        dir.path(),
        "app/services/pricing.sl",
        "class Pricing\n  def total(cents: Int) -> Int\n    cents * 2\n  end\nend\n",
    );
    write(
        dir.path(),
        "app/services/checkout.sl",
        "def checkout_total\n  pricing = Pricing.new()\n  pricing.totl(3)\nend\n",
    );
    let (ok, text) = check(dir.path(), "app/services/checkout.sl");
    assert!(!ok, "{text}");
    assert!(text.contains("totl"), "{text}");

    write(
        dir.path(),
        "app/services/checkout.sl",
        "def checkout_total -> Int\n  pricing = Pricing.new()\n  pricing.total(3)\nend\n",
    );
    let (ok, text) = check(dir.path(), "app/services/checkout.sl");
    assert!(ok, "{text}");
}

#[test]
fn a_parent_in_another_file_is_resolved() {
    let dir = app();
    write(
        dir.path(),
        "app/policies/application_policy.sl",
        "class ApplicationPolicy\n  def allowed? -> Bool\n    true\n  end\nend\n",
    );
    write(
        dir.path(),
        "app/policies/post_policy.sl",
        "class PostPolicy < ApplicationPolicy\n  def show? -> Bool\n    @allowed?\n  end\n\n  def edit? -> Bool\n    @alowed?\n  end\nend\n",
    );
    let (ok, text) = check(dir.path(), "app/policies/post_policy.sl");
    assert!(!ok, "{text}");
    assert!(text.contains("alowed?"), "{text}");
    assert!(text.contains("1 error(s)"), "{text}");
}

#[test]
fn a_name_two_files_declare_stays_unchecked() {
    let dir = app();
    write(
        dir.path(),
        "app/services/other_report.sl",
        "def report_title(name, suffix)\n  name + suffix\nend\n",
    );
    write(
        dir.path(),
        "app/services/uses_title.sl",
        "def heading\n  report_title(\"a\", \"b\")\nend\n",
    );
    let (ok, text) = check(dir.path(), "app/services/uses_title.sl");
    assert!(ok, "{text}");
}

#[test]
fn model_fields_stay_open_across_files() {
    let dir = app();
    write(
        dir.path(),
        "app/services/audit.sl",
        "def audit_names\n  AuditEntry.recent.map { |entry| entry.whatever }\nend\n",
    );
    let (ok, text) = check(dir.path(), "app/services/audit.sl");
    assert!(ok, "{text}");
}

// Phase 3 of #693: strictness levels.

fn check_with(dir: &Path, args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("check")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run soli check");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

const MISMATCH: &str = "def double(n: Int) -> Int\n  n * 2\nend\ndouble(\"two\")\n";
const UNTYPED: &str = "def double(n)\n  n * 2\nend\nprint(double(2))\n";

#[test]
fn a_file_can_turn_itself_off_or_loose() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.sl", MISMATCH);
    assert!(!check_with(dir.path(), &["a.sl"]).0);

    write(dir.path(), "a.sl", &format!("# soli: off\n{MISMATCH}"));
    let (ok, text) = check_with(dir.path(), &["a.sl"]);
    assert!(ok, "{text}");

    // Loose keeps what certainly fails at run time, not a type mismatch.
    write(dir.path(), "a.sl", &format!("# soli: loose\n{MISMATCH}"));
    let (ok, text) = check_with(dir.path(), &["a.sl"]);
    assert!(ok, "{text}");
    write(
        dir.path(),
        "a.sl",
        &format!("# soli: loose\n{MISMATCH}dobule(2)\n"),
    );
    let (ok, text) = check_with(dir.path(), &["a.sl"]);
    assert!(!ok && text.contains("dobule"), "{text}");
}

#[test]
fn strict_requires_parameter_and_return_types() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.sl", UNTYPED);
    assert!(check_with(dir.path(), &["a.sl"]).0);

    let (ok, text) = check_with(dir.path(), &["--level", "strict", "a.sl"]);
    assert!(!ok, "{text}");
    assert!(text.contains("parameter 'n' of double()"), "{text}");
    assert!(text.contains("return type of double()"), "{text}");
    assert!(text.contains("E0404"), "{text}");

    write(
        dir.path(),
        "a.sl",
        "# soli: strict\ndef double(n: Int) -> Int\n  n * 2\nend\n\nclass Box\n  new(item: Any)\n    @item = item\n  end\n\n  def each(&block) -> Void\n    block(@item)\n  end\nend\n",
    );
    let (ok, text) = check_with(dir.path(), &["a.sl"]);
    assert!(ok, "{text}");

    // The file's own line wins over the flag.
    write(dir.path(), "a.sl", &format!("# soli: standard\n{UNTYPED}"));
    let (ok, text) = check_with(dir.path(), &["--level=strict", "a.sl"]);
    assert!(ok, "{text}");
}

#[test]
fn soli_toml_sets_the_project_level() {
    let dir = app();
    write(
        dir.path(),
        "soli.toml",
        "[package]\nname = \"x\"\n\n[check]\nlevel = \"strict\"\n",
    );
    let (ok, text) = check_with(dir.path(), &["app/services"]);
    assert!(!ok, "{text}");
    assert!(
        text.contains("parameter 'name' of report_title()"),
        "{text}"
    );

    let (ok, text) = check_with(dir.path(), &["--level", "standard", "app/services"]);
    assert!(ok, "{text}");
}

#[test]
fn an_unknown_level_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.sl", UNTYPED);
    let (ok, text) = check_with(dir.path(), &["--level", "pedantic", "a.sl"]);
    assert!(!ok);
    assert!(text.contains("--level takes"), "{text}");
}

#[test]
fn a_script_honours_its_own_line() {
    let dir = tempfile::tempdir().unwrap();
    // `shout(5)` runs (parameter types are not enforced at run time), but the
    // checker refuses it unless the file turns it off.
    let source = "def shout(s: String)\n  s\nend\nprint(shout(5))\nprint(\"ran\")\n";
    let run = |contents: &str| {
        write(dir.path(), "a.sl", contents);
        let output = Command::new(env!("CARGO_BIN_EXE_soli"))
            .arg("a.sl")
            .current_dir(dir.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).contains("ran")
    };
    assert!(!run(source));
    assert!(run(&format!("# soli: off\n{source}")));
}

// The views (#693): their tags' code is checked as one program, against the
// view helpers, `app/helpers/` and the project.

fn app_with_view(view: &str) -> tempfile::TempDir {
    let dir = app();
    write(
        dir.path(),
        "app/models/invoice.sl",
        "class Invoice\n  def total -> Int\n    1\n  end\nend\n",
    );
    write(
        dir.path(),
        "app/helpers/money_helper.sl",
        "def money(cents)\n  \"$#{cents}\"\nend\n",
    );
    write(dir.path(), "app/views/posts/index.html.slv", view);
    dir
}

#[test]
fn a_view_is_checked_with_its_helpers_and_locals() {
    let dir = app_with_view(
        "<h1><%= @title %></h1>\n<% for post in @posts %>\n<a href=\"<%= post_path(post) %>\"><%= h(post.title) %></a>\n<%= money(post.cents) %>\n<%= item %>\n<% if post.draft %>\n<% else %>\n<% end %>\n<% end %>\n<% page_url = \"/x\" %>\n<%= public_path(\"a.js\") %>\n<%= partial(\"posts/row\", {\"post\": 1}) %>\n<%= Invoice.new.total %>\n",
    );
    let (ok, text) = check(dir.path(), ".");
    assert!(ok, "{text}");
    assert!(text.contains("Checked 7 file(s)"), "{text}");
}

#[test]
fn a_view_reports_a_misspelt_helper_a_wrong_arity_and_an_unknown_member() {
    let dir = app_with_view("<%= monye(1) %>\n<%= money(1, 2) %>\n<%= Invoice.new.totl %>\n");
    let (ok, text) = check(dir.path(), ".");
    assert!(!ok, "{text}");
    assert!(text.contains("Undefined variable 'monye'"), "{text}");
    assert!(text.contains("expected 1, got 2"), "{text}");
    assert!(text.contains("'totl' on type 'Invoice'"), "{text}");
    // A view named outright is checked too.
    let (ok, _) = check(dir.path(), "app/views/posts/index.html.slv");
    assert!(!ok);
    // `--no-views` leaves them out.
    let (ok, text) = check_with(dir.path(), &["--no-views", "."]);
    assert!(ok, "{text}");
}

#[test]
fn a_route_helper_is_known_in_an_application() {
    let dir = app();
    write(
        dir.path(),
        "app/controllers/links_controller.sl",
        "class LinksController < Controller\n  # GET /links\n  def index(req)\n    @back = posts_url()\n    redirect(post_path(1))\n  end\nend\n",
    );
    let (ok, text) = check(dir.path(), "app");
    assert!(ok, "{text}");
}
