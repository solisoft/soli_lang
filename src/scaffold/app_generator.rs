//! Application scaffolding generator

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;
use web_time::{SystemTime, UNIX_EPOCH};

use crate::scaffold::templates::{self, agents, app, bundled_docs};
use crate::scaffold::ui::{ProgressDisplay, Spinner};

/// Create directories for a new application
pub fn create_directories(app_path: &Path) -> Result<(), String> {
    let dirs = [
        "",
        "app",
        "app/controllers",
        "app/helpers",
        // Background job handlers (`*_job.sl`). Created up front: the job
        // engine only starts when this directory exists, so a generated app can
        // add a job and have it run without also having to create the folder.
        "app/jobs",
        "app/middleware",
        "app/models",
        // Shared mixin modules (`include` / concern hooks). Auto-loaded
        // with the rest of `app/models/**`; keep reusable behavior here.
        "app/models/concerns",
        "app/views",
        "app/views/home",
        "app/views/layouts",
        "config",
        "db",
        "db/migrations",
        "db/seeds",
        "app/assets",
        "app/assets/css",
        "public",
        "public/css",
        "public/js",
        "public/images",
        "stdlib",
        "tests",
        ".claude",
        ".claude/commands",
    ];

    for dir in dirs {
        let path = app_path.join(dir);
        fs::create_dir_all(&path)
            .map_err(|e| format!("Failed to create directory '{}': {}", path.display(), e))?;
    }

    Ok(())
}

/// Write content to a file. Soli sources go through the formatter on the way
/// out, so a generated app is `soli fmt --check` clean from its first commit
/// and the code an agent reads back matches the style the generated
/// `CLAUDE.md` prescribes. Doing it here rather than in each generator's string
/// literals means every generator — including ones added later — inherits it.
pub fn write_file(path: &Path, content: &str) -> Result<(), String> {
    let owned = format_soli_source(path, content);
    let bytes = owned.as_deref().unwrap_or(content).as_bytes();
    let mut file =
        File::create(path).map_err(|e| format!("Failed to create '{}': {}", path.display(), e))?;
    file.write_all(bytes)
        .map_err(|e| format!("Failed to write to '{}': {}", path.display(), e))?;
    Ok(())
}

/// Create `app_path/rel` with `contents` unless it exists, printing the
/// scaffold's `create` / `skip` line. Generators re-run on an existing app.
pub(crate) fn write_if_absent(app_path: &Path, rel: &str, contents: &str) -> Result<(), String> {
    let path = app_path.join(rel);
    if path.exists() {
        println!("  \x1b[33mskip\x1b[0m   {} (already exists)", rel);
        return Ok(());
    }
    write_file(&path, contents)?;
    println!("  \x1b[32mcreate\x1b[0m {}", rel);
    Ok(())
}

/// Write `db/migrations/<now + offset>_<name>.sl` unless a migration whose
/// file name contains `name` is already there. `offset` orders migrations a
/// generator writes within the same second.
pub(crate) fn write_migration_once(
    app_path: &Path,
    name: &str,
    offset: u64,
    contents: &str,
) -> Result<(), String> {
    let migrations_dir = app_path.join("db/migrations");
    if let Ok(entries) = fs::read_dir(&migrations_dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().contains(name) {
                println!(
                    "  \x1b[33mskip\x1b[0m   db/migrations ({} migration already exists)",
                    name
                );
                return Ok(());
            }
        }
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("Failed to get timestamp: {}", e))?
        .as_secs()
        + offset;
    let filename = format!("{}_{}.sl", timestamp, name);
    write_file(&migrations_dir.join(&filename), contents)?;
    println!("  \x1b[32mcreate\x1b[0m db/migrations/{}", filename);
    Ok(())
}

/// Append `snippet` to `config/routes.sl` unless the file already contains
/// `marker`; `label` names the feature in the `skip` line.
pub(crate) fn append_routes_once(
    app_path: &Path,
    marker: &str,
    snippet: &str,
    label: &str,
) -> Result<(), String> {
    let routes_file = app_path.join("config/routes.sl");
    let mut content = if routes_file.exists() {
        fs::read_to_string(&routes_file)
            .map_err(|e| format!("Failed to read routes file: {}", e))?
    } else {
        String::new()
    };

    if content.contains(marker) {
        println!(
            "  \x1b[33mskip\x1b[0m   config/routes.sl ({} routes already present)",
            label
        );
        return Ok(());
    }

    if !content.ends_with('\n') && !content.is_empty() {
        content.push('\n');
    }
    content.push_str(snippet);
    write_file(&routes_file, &content)?;
    println!("  \x1b[32mupdate\x1b[0m config/routes.sl");
    Ok(())
}

/// The canonical formatting of `content` when `path` is a `.sl` file and the
/// content parses, else `None` — the caller then writes the original bytes.
/// A generator emitting something the parser rejects is a bug worth seeing as
/// a broken generated app, not one worth hiding behind a failed write.
pub(crate) fn format_soli_source(path: &Path, content: &str) -> Option<String> {
    if path.extension().and_then(|e| e.to_str()) != Some("sl") {
        return None;
    }
    crate::fmt::format_source(content).ok()
}

/// Create the routes configuration file.
///
/// With `eui`, the `router_eui` line for the generated component is part
/// of the same write: the file goes through the formatter once, with
/// everything it will hold.
pub fn create_routes_file(app_path: &Path, eui: bool) -> Result<(), String> {
    let mut routes = app::ROUTES_TEMPLATE.to_string();
    #[cfg(feature = "eui")]
    if eui {
        routes.push_str(templates::eui::EUI_ROUTES);
    }
    let _ = eui;
    write_file(&app_path.join("config/routes.sl"), &routes)
}

/// Write the EUI half of a new application: the reference catalogue and a
/// first component that uses it. `config/routes.sl` carries the route,
/// written by `create_routes_file`.
///
/// The catalogue is written verbatim, around `write_file` and so around
/// the formatter. It is not generated code: it is a vendored copy of
/// files maintained in the EUI repository, hand-laid-out and not `soli
/// fmt` clean there. Reformatting it here would put 3 000 lines of
/// difference between a new application and the upstream it will want to
/// take fixes from.
///
/// It arrives as nine files that load into one namespace, in no
/// particular order; `templates::eui::EUI_BUILDERS` names them.
#[cfg(feature = "eui")]
pub fn create_eui(app_path: &Path) -> Result<(), String> {
    for (name, source) in templates::eui::EUI_BUILDERS {
        let catalogue = app_path.join("app/controllers").join(name);
        // The catalogue is tested in the framework, not by the app: keep its
        // 17 000 lines out of the app's coverage report.
        let source = format!(
            "{} — the EUI widget catalogue, tested by the framework.\n{source}",
            crate::coverage::COVERAGE_OFF_DIRECTIVE
        );
        fs::write(&catalogue, source)
            .map_err(|e| format!("Failed to write to '{}': {}", catalogue.display(), e))?;
    }
    write_file(
        &app_path.join("app/controllers/eui_controller.sl"),
        templates::eui::EUI_CONTROLLER,
    )?;
    let spec = app_path.join("tests/controllers/eui_controller_spec.sl");
    if let Some(dir) = spec.parent() {
        fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create '{}': {}", dir.display(), e))?;
    }
    write_file(&spec, templates::eui::EUI_CONTROLLER_SPEC)
}

/// Create the `db/seeds.sl` starter file. Run with `soli db:seed`; additional
/// seed files can live in `db/seeds/` (generated via `soli db:seed generate`).
pub fn create_seeds_file(app_path: &Path) -> Result<(), String> {
    write_file(&app_path.join("db/seeds.sl"), app::SEEDS_TEMPLATE)
}

/// Create the application boot config file. Loaded by `soli serve` before
/// routes. Ships with `enable_trust_proxy` active and CSRF same-origin
/// checks left at their default-on setting — the typical scaffolded app
/// runs behind a reverse proxy (nginx / Caddy / ALB / fly-proxy). The
/// template's banner comment calls out when to flip `enable_trust_proxy`
/// off (direct-to-internet deploys) so spoofed `X-Forwarded-*` headers
/// can't be trusted.
pub fn create_application_config(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("config/application.sl"),
        app::APPLICATION_CONFIG_TEMPLATE,
    )
}

/// Create the home controller
pub fn create_home_controller(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("app/controllers/home_controller.sl"),
        app::HOME_CONTROLLER_TEMPLATE,
    )
}

/// Create the application layout
pub fn create_layout(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("app/views/layouts/application.html.slv"),
        app::LAYOUT_TEMPLATE,
    )
}

/// Write the default share image the layout's Open Graph tags point at.
pub fn create_og_image(app_path: &Path) -> Result<(), String> {
    let path = app_path.join("public/images/og.png");
    fs::write(&path, app::OG_IMAGE)
        .map_err(|e| format!("Failed to write to '{}': {}", path.display(), e))
}

/// Create the home index view
pub fn create_index_view(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("app/views/home/index.html.slv"),
        app::INDEX_VIEW_TEMPLATE,
    )
}

/// Create the CSS file (Tailwind source in app/assets/css/)
pub fn create_css_file(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("app/assets/css/application.css"),
        app::CSS_TEMPLATE,
    )
}

/// Vendor the default client-side stack (HTMx + Alpine.js) into `public/js/`.
/// Wired into the base layout by `create_layout`; see
/// `docs/core-concepts/client-interactivity` for usage.
pub fn create_client_js(app_path: &Path) -> Result<(), String> {
    write_file(&app_path.join("public/js/htmx.min.js"), app::HTMX_JS)?;
    write_file(&app_path.join("public/js/alpine.min.js"), app::ALPINE_JS)?;
    Ok(())
}

/// Turn an app name into a SoliDB-safe database identifier: lowercase, with
/// every run of non-alphanumeric characters collapsed to a single `_` and
/// leading/trailing separators trimmed. Falls back to `default` if the result
/// would be empty (e.g. a name made entirely of punctuation).
pub fn slugify_db_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_sep = true; // suppress leading separators
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_sep = false;
        } else if !prev_sep {
            out.push('_');
            prev_sep = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "default".to_string()
    } else {
        out
    }
}

/// Create the .env file, seeding `SOLIDB_DATABASE` with the slugified app name
/// instead of the generic `default` so each new project gets its own database.
pub fn create_env_file(app_path: &Path, name: &str) -> Result<(), String> {
    let content = app::ENV_TEMPLATE.replace(
        "SOLIDB_DATABASE=default",
        &format!("SOLIDB_DATABASE={}", slugify_db_name(name)),
    );
    write_file(&app_path.join(".env"), &content)
}

/// Create `.env.test`, which `soli test` refuses to start without. It names
/// its own `<app>_test` database so a spec run never truncates the dev data,
/// and it is committed (the `.gitignore` only drops `.env` and `*.local`).
pub fn create_env_test_file(app_path: &Path, name: &str) -> Result<(), String> {
    let content = format!(
        "# Loaded by `soli test`, which refuses to start without it.\n\
         # A database of its own: the runner creates, truncates and drops it.\n\
         \n\
         SOLIDB_HOST=http://localhost:6745\n\
         SOLIDB_DATABASE={}_test\n\
         SOLIDB_USERNAME=admin\n\
         SOLIDB_PASSWORD=admin\n",
        slugify_db_name(name)
    );
    write_file(&app_path.join(".env.test"), &content)
}

/// Create the .gitignore file
pub fn create_gitignore(app_path: &Path) -> Result<(), String> {
    write_file(&app_path.join(".gitignore"), app::GITIGNORE_TEMPLATE)
}

/// Create the CLAUDE.md file
pub fn create_claude_md(app_path: &Path) -> Result<(), String> {
    write_file(&app_path.join("CLAUDE.md"), app::CLAUDE_MD_TEMPLATE)
}

/// Create the AGENTS.md tool-agnostic stub at the project root.
pub fn create_agents_md(app_path: &Path) -> Result<(), String> {
    write_file(&app_path.join("AGENTS.md"), agents::AGENTS_MD_TEMPLATE)
}

/// Drop a short, directory-scoped CLAUDE.md into each app/ subdir, tests/, and db/migrations/.
/// Claude Code auto-loads CLAUDE.md from the cwd up to the project root, so these scope
/// rules to where the agent is actually working.
pub fn create_nested_claude_mds(app_path: &Path) -> Result<(), String> {
    let nested = [
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
        ("public/js/CLAUDE.md", agents::CLAUDE_PUBLIC_JS_TEMPLATE),
        ("tests/CLAUDE.md", agents::CLAUDE_TESTS_TEMPLATE),
        (
            "db/migrations/CLAUDE.md",
            agents::CLAUDE_MIGRATIONS_TEMPLATE,
        ),
    ];
    for (rel, content) in nested {
        write_file(&app_path.join(rel), content)?;
    }
    Ok(())
}

/// Copy the bundled Soli language docs (www/docs/*.md) into the user's app
/// under `docs/`. Per-directory CLAUDE.md files reference these by relative
/// path, so a coding agent can resolve them locally without an internet
/// round-trip.
pub fn create_bundled_docs(app_path: &Path) -> Result<(), String> {
    let target_root = app_path.join("docs");
    fs::create_dir_all(&target_root)
        .map_err(|e| format!("Failed to create docs/ directory: {e}"))?;

    write_bundled_dir(&bundled_docs::DOCS, &target_root)
}

/// Recursively write every file from an embedded `include_dir!` tree to disk,
/// honoring the skip list defined in `bundled_docs`.
fn write_bundled_dir(dir: &include_dir::Dir, target_root: &Path) -> Result<(), String> {
    for file in dir.files() {
        let rel = file.path().to_string_lossy();
        if !bundled_docs::should_copy(&rel) {
            continue;
        }
        let target = target_root.join(file.path());
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create '{}': {e}", parent.display()))?;
        }
        fs::write(&target, file.contents())
            .map_err(|e| format!("Failed to write '{}': {e}", target.display()))?;
    }
    for subdir in dir.dirs() {
        write_bundled_dir(subdir, target_root)?;
    }
    Ok(())
}

/// Create the .claude/ directory: permissions allowlist + project slash commands.
/// Pre-allowing safe `soli` subcommands removes the per-prompt permission tax for agents.
pub fn create_dot_claude(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join(".claude/settings.json"),
        agents::CLAUDE_SETTINGS_TEMPLATE,
    )?;
    write_file(
        &app_path.join(".claude/commands/soli-verify.md"),
        agents::CMD_SOLI_VERIFY_TEMPLATE,
    )?;
    write_file(
        &app_path.join(".claude/commands/soli-test.md"),
        agents::CMD_SOLI_TEST_TEMPLATE,
    )?;
    write_file(
        &app_path.join(".claude/commands/soli-resource.md"),
        agents::CMD_SOLI_RESOURCE_TEMPLATE,
    )
}

/// Relative paths rewritten by [`update_project_docs`] (agent guides + slash
/// commands + settings). Language reference under `docs/` is refreshed via
/// [`create_bundled_docs`] and is not listed here (it is a tree, not a fixed set).
pub const PROJECT_DOC_AGENT_PATHS: &[&str] = &[
    "CLAUDE.md",
    "AGENTS.md",
    "app/controllers/CLAUDE.md",
    "app/models/CLAUDE.md",
    "app/models/concerns/CLAUDE.md",
    "app/views/CLAUDE.md",
    "app/middleware/CLAUDE.md",
    "public/js/CLAUDE.md",
    "tests/CLAUDE.md",
    "db/migrations/CLAUDE.md",
    ".claude/settings.json",
    ".claude/commands/soli-verify.md",
    ".claude/commands/soli-test.md",
    ".claude/commands/soli-resource.md",
];

/// Whether `app_path` looks like a Soli MVC project (or at least a place
/// `soli new` would have laid out docs).
pub fn is_soli_project(app_path: &Path) -> bool {
    app_path.join("soli.toml").is_file()
        || app_path.join("config/routes.sl").is_file()
        || app_path.join("app/controllers").is_dir()
        || app_path.join("CLAUDE.md").is_file()
}

/// Refresh every markdown / agent file that `soli new` embeds, from the
/// templates baked into this `soli` binary (sourced from the soli_lang git
/// tree at compile time). Overwrites existing content — project-local
/// customizations in those files will be replaced.
///
/// Returns the relative agent-guide paths that were written. Language docs
/// under `docs/` are also rewritten (same skip list as `soli new`).
pub fn update_project_docs(app_path: &Path) -> Result<Vec<String>, String> {
    if !app_path.exists() {
        return Err(format!("Directory '{}' does not exist", app_path.display()));
    }
    if !app_path.is_dir() {
        return Err(format!("'{}' is not a directory", app_path.display()));
    }
    if !is_soli_project(app_path) {
        return Err(format!(
            "'{}' does not look like a Soli project (expected soli.toml, config/routes.sl, or app/controllers/)",
            app_path.display()
        ));
    }

    // Host dirs for nested agent files — older projects may lack some of them.
    for dir in [
        "app/controllers",
        "app/models",
        "app/models/concerns",
        "app/views",
        "app/middleware",
        "public/js",
        "tests",
        "db/migrations",
        ".claude/commands",
        "docs",
    ] {
        fs::create_dir_all(app_path.join(dir))
            .map_err(|e| format!("Failed to create '{}': {e}", app_path.join(dir).display()))?;
    }

    create_claude_md(app_path)?;
    create_agents_md(app_path)?;
    create_nested_claude_mds(app_path)?;
    create_dot_claude(app_path)?;
    create_bundled_docs(app_path)?;

    Ok(PROJECT_DOC_AGENT_PATHS
        .iter()
        .map(|s| (*s).to_string())
        .collect())
}

/// Create the application helper
pub fn create_application_helper(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("app/helpers/application_helper.sl"),
        app::APPLICATION_HELPER_TEMPLATE,
    )
}

/// Create sample middleware files
pub fn create_sample_middleware(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("app/middleware/request_stamp.sl"),
        app::REQUEST_STAMP_MIDDLEWARE_TEMPLATE,
    )?;

    write_file(
        &app_path.join("app/middleware/auth.sl"),
        app::AUTH_MIDDLEWARE_TEMPLATE,
    )
}

/// Specs for the home controller, the application helper and the sample
/// auth middleware: a fresh app's first `soli test` has something to run, and
/// its coverage starts above the runner's threshold rather than at zero.
pub fn create_starter_specs(app_path: &Path) -> Result<(), String> {
    for (path, content) in [
        (
            "tests/controllers/home_controller_spec.sl",
            app::HOME_CONTROLLER_SPEC_TEMPLATE,
        ),
        (
            "tests/helpers/application_helper_spec.sl",
            app::APPLICATION_HELPER_SPEC_TEMPLATE,
        ),
        (
            "tests/middleware/auth_spec.sl",
            app::AUTH_MIDDLEWARE_SPEC_TEMPLATE,
        ),
    ] {
        let path = app_path.join(path);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Failed to create '{}': {}", dir.display(), e))?;
        }
        write_file(&path, content)?;
    }
    Ok(())
}

/// Create the bundled stdlib modules (FeatureFlags, …)
pub fn create_stdlib(app_path: &Path) -> Result<(), String> {
    write_file(
        &app_path.join("stdlib/feature_flags.sl"),
        app::FEATURE_FLAGS_TEMPLATE,
    )
}

/// Create README.md
pub fn create_readme(app_path: &Path, name: &str) -> Result<(), String> {
    write_file(&app_path.join("README.md"), &app::readme(name))
}

/// Create soli.toml
pub fn create_soli_toml(app_path: &Path, name: &str) -> Result<(), String> {
    write_file(&app_path.join("soli.toml"), &app::soli_toml(name))
}

/// Compile `app/assets/css/*.css` into `public/css/` with the same
/// standalone Tailwind binary `soli serve --dev` uses — the one cached in
/// `~/.soli/bin/` and verified against a pinned SHA-256 (SEC-081).
///
/// No npm, no `node_modules`: a new app is styled on a machine that has
/// never seen Node, and the compiler that runs is the one we checksummed
/// rather than whatever a floating `^4` range resolved to today. The
/// binary is downloaded once per machine and shared by every project.
///
/// Prints its own progress (including the first-run download), so it is
/// deliberately not wrapped in a spinner.
pub fn build_tailwind_css(app_path: &Path) {
    crate::serve::tailwind::compile_tailwind_css_once(app_path);
}

/// Initialize git repository with spinner
pub fn init_git_repo(app_path: &Path) {
    // Check if git is available
    if Command::new("git")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_err()
    {
        let spinner = Spinner::start("Initializing git repository...");
        spinner.stop_with_warning("git not found - run 'git init' manually");
        return;
    }

    let spinner = Spinner::start("Initializing git repository...");

    // Run git init
    let init_result = Command::new("git")
        .args(["init"])
        .current_dir(app_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    if init_result.is_err() || !init_result.unwrap().success() {
        spinner.stop_with_warning("git init failed");
        return;
    }

    // Stage all files
    let add_result = Command::new("git")
        .args(["add", "."])
        .current_dir(app_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    if add_result.is_err() || !add_result.unwrap().success() {
        spinner.stop_with_warning("git init succeeded but staging failed");
        return;
    }

    // Create initial commit
    let commit_result = Command::new("git")
        .args(["commit", "-m", "Initial commit from soli new"])
        .current_dir(app_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match commit_result {
        Ok(status) if status.success() => {
            spinner.stop_with_success("Git repository initialized");
        }
        _ => {
            spinner.stop_with_warning("git init succeeded but commit failed");
        }
    }
}

/// Replace app_name placeholder in all files within a directory
pub fn replace_placeholders(app_path: &Path, name: &str) -> Result<(), String> {
    use walkdir::WalkDir;

    let walker = WalkDir::new(app_path).follow_links(false);

    for entry in walker {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();

        if path.is_file() {
            // Skip hidden files and git directories
            if let Some(file_name) = path.file_name() {
                if file_name.to_string_lossy().starts_with('.') {
                    continue;
                }
            }

            // Skip binary files (based on extension)
            if let Some(ext) = path.extension() {
                let ext_str = ext.to_string_lossy();
                if ext_str == "png"
                    || ext_str == "jpg"
                    || ext_str == "jpeg"
                    || ext_str == "gif"
                    || ext_str == "ico"
                    || ext_str == "woff"
                    || ext_str == "woff2"
                    || ext_str == "ttf"
                    || ext_str == "eot"
                    || ext_str == "svg"
                {
                    continue;
                }
            }

            // Read, replace, and write back
            let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
            let new_content = content.replace("app_name", name);
            if content != new_content {
                fs::write(path, new_content).map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(())
}

/// Create from template archive
#[cfg(not(target_arch = "wasm32"))]
pub fn create_from_template(name: &str, app_path: &Path, template_url: &str) -> Result<(), String> {
    use flate2::read::GzDecoder;
    use tar;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().map_err(|e| e.to_string())?;
    let archive_path = temp_dir.path().join("template.tar.gz");

    println!();
    println!("  Downloading template from {}...", template_url);

    // Download the archive
    let response = ureq::get(template_url)
        .set("Accept", "application/vnd.github.v3.raw")
        .call()
        .map_err(|e| format!("Failed to download template: {}", e))?;

    let mut file = File::create(&archive_path).map_err(|e| e.to_string())?;
    let mut reader = response.into_reader();
    io::copy(&mut reader, &mut file).map_err(|e| e.to_string())?;

    // Extract the archive
    let file = File::open(&archive_path).map_err(|e| e.to_string())?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);

    // Create app directory
    fs::create_dir_all(app_path).map_err(|e| e.to_string())?;

    // Extract files. SEC-011: a malicious tarball can carry entries with
    // absolute paths (`/etc/passwd`) or `..` segments (`../../../.bashrc`)
    // that the bare `out_path.push(component)` loop would happily honour,
    // letting `soli new --template <url>` overwrite arbitrary files in
    // the operator's home directory. Run every entry path through
    // `validate_template_entry_path` first; refuse the whole archive on
    // anything other than plain `Normal(_)` components.
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();

        // Skip root-level files that shouldn't be in the extracted content
        if path.iter().count() == 0 {
            continue;
        }

        validate_template_entry_path(&path)?;

        let mut out_path = app_path.to_path_buf();
        for component in path.iter() {
            out_path.push(component);
        }

        // Defence in depth: even after rejecting `..`/root components above,
        // re-canonicalise (where possible) and confirm the result is still
        // under `app_path`. The loop ran inside the Normal-only filter so
        // the only way to fail this check is a symlink in the existing
        // ancestor chain pointing outside — also worth refusing.
        if let Some(parent_for_check) = out_path.parent() {
            if let (Ok(canon_parent), Ok(canon_root)) = (
                fs::canonicalize(parent_for_check),
                fs::canonicalize(app_path),
            ) {
                if !canon_parent.starts_with(&canon_root) {
                    return Err(format!(
                        "Refusing to extract template entry that resolves outside the app directory: {}",
                        path.display()
                    ));
                }
            }
        }

        if entry.header().entry_type() == tar::EntryType::Directory {
            fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out_file = File::create(&out_path).map_err(|e| e.to_string())?;
            io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;
        }
    }

    // Replace placeholders in all files
    replace_placeholders(app_path, name)?;

    // Initialize git repository
    println!("  Initializing git repository...");
    if Command::new("git")
        .args(["init"])
        .current_dir(app_path)
        .output()
        .is_ok()
        && Command::new("git")
            .args(["add", "."])
            .current_dir(app_path)
            .output()
            .is_ok()
        && Command::new("git")
            .args(["commit", "-m", "Initial commit from Soli template"])
            .current_dir(app_path)
            .output()
            .is_ok()
    {
        println!("  ✓ Git repository initialized");
    }

    println!("  ✓ Template extracted");
    println!();

    print_success_message(name);
    Ok(())
}
#[cfg(target_arch = "wasm32")]
pub fn create_from_template(
    _name: &str,
    _app_path: &Path,
    _template_url: &str,
) -> Result<(), String> {
    Err(crate::platform::unsupported_on_edge("create_from_template"))
}

/// Print success message after creating an app
pub fn print_success_message(name: &str) {
    println!();
    println!(
        "  \x1b[32m\x1b[1m✓ Success!\x1b[0m Created \x1b[1m{}\x1b[0m",
        name
    );
    println!();
    for line in get_started_box(name) {
        println!("{line}");
    }
    println!();
}

/// The "Get started" box, one string per line. Each row is padded on its
/// visible text (colour codes take no column), and the box widens for a long
/// app name. The rows were padded by hand and the right border wandered.
pub(crate) fn get_started_box(name: &str) -> Vec<String> {
    // (visible text, rendered text) for each row inside the borders.
    let cd = format!("    cd {name}");
    let rows: Vec<(String, String)> = vec![
        (
            "  Get started:".into(),
            "  \x1b[1mGet started:\x1b[0m".into(),
        ),
        (String::new(), String::new()),
        (cd.clone(), format!("    \x1b[36mcd {name}\x1b[0m")),
        (
            "    soli serve . --dev".into(),
            "    \x1b[36msoli serve . --dev\x1b[0m".into(),
        ),
        (String::new(), String::new()),
        (
            "  Then open http://localhost:5011".into(),
            "  Then open \x1b[4mhttp://localhost:5011\x1b[0m".into(),
        ),
    ];
    let inner = rows
        .iter()
        .map(|(visible, _)| visible.chars().count() + 2)
        .max()
        .unwrap_or(0)
        .max(41);
    let mut lines = vec![format!("  \x1b[2m┌{}┐\x1b[0m", "─".repeat(inner))];
    for (visible, rendered) in &rows {
        let pad = inner - visible.chars().count();
        lines.push(format!(
            "  \x1b[2m│\x1b[0m{rendered}{}\x1b[2m│\x1b[0m",
            " ".repeat(pad)
        ));
    }
    lines.push(format!("  \x1b[2m└{}┘\x1b[0m", "─".repeat(inner)));
    lines
}

/// What `--eui` added, and how to see it. Printed after the box rather
/// than inside it: the box is a fixed width, and this is two paths and a
/// command that are not.
pub fn print_eui_message() {
    println!("  \x1b[1mThe window:\x1b[0m");
    println!();
    println!("    \x1b[2mapp/controllers/\x1b[0meui_controller.sl  \x1b[2ma component: a handler and a view\x1b[0m");
    println!("    \x1b[2mapp/controllers/\x1b[0meui_builders*.sl   \x1b[2m670 definitions in nine files, all of them plain Soli\x1b[0m");
    println!();
    println!("  With the server up, open the component in a native window:");
    println!();
    println!("    \x1b[36mEUI_ALLOW_INSECURE_LOOPBACK=1 eui-client ws://127.0.0.1:5011/_eui/session/counter\x1b[0m");
    println!();
}

/// `--edge`: the files that make the application a Cloudflare Worker app —
/// its own `wrangler.toml` (D1, cookie sessions, the other bindings
/// commented), a `.env` for SQLite and the cookie driver under `soli serve`,
/// local secrets in `.dev.vars`, and a README and CLAUDE.md that say so.
pub fn create_edge(app_path: &Path, name: &str) -> Result<(), String> {
    let worker: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let worker = worker.trim_matches('-');
    let worker = if worker.is_empty() {
        "soli-app"
    } else {
        worker
    };
    let db_name = format!("{worker}-db");
    // 64 hex characters, well past the cookie driver's 32.
    let secret = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    write_file(
        &app_path.join("wrangler.toml"),
        &templates::edge::wrangler_toml(worker, &db_name, &today),
    )?;
    write_file(
        &app_path.join(".env"),
        &templates::edge::env_file(&db_name, &secret),
    )?;
    write_file(
        &app_path.join(".dev.vars"),
        &templates::edge::dev_vars(&secret),
    )?;
    write_file(
        &app_path.join("README.md"),
        &templates::edge::readme(name, &db_name),
    )?;
    for (file, extra) in [
        (".gitignore", templates::edge::GITIGNORE_EXTRA),
        ("CLAUDE.md", templates::edge::CLAUDE_MD_EXTRA),
    ] {
        let path = app_path.join(file);
        let mut text = fs::read_to_string(&path).map_err(|e| format!("read {file}: {e}"))?;
        text.push_str(extra);
        write_file(&path, &text)?;
    }
    Ok(())
}

/// What `--edge` added, and how to run it.
pub fn print_edge_message() {
    println!();
    println!("  \x1b[1mCloudflare Workers\x1b[0m");
    println!("    \x1b[2mwrangler.toml\x1b[0m   \x1b[2mthe Worker: D1, cookie sessions, R2/Images/Queues/Hyperdrive ready to uncomment\x1b[0m");
    println!("    \x1b[2m.dev.vars\x1b[0m       \x1b[2mlocal secrets for soli edge dev (not committed)\x1b[0m");
    println!();
    println!("    \x1b[36msoli edge dev\x1b[0m   \x1b[2m# on workerd, http://localhost:8787 — the README has the deploy steps\x1b[0m");
}

/// Create a new Soli MVC application.
///
/// With `eui`, it also gets the reference widget catalogue and a first
/// EUI component: a native window this application serves over a
/// WebSocket, beside the HTML it serves over HTTP. With `edge`, it is set up
/// to deploy to Cloudflare Workers (see [`create_edge`]).
pub fn create_app(name: &str, template: Option<&str>, eui: bool, edge: bool) -> Result<(), String> {
    let app_path = Path::new(name);

    if app_path.exists() {
        return Err(format!("Directory '{}' already exists", name));
    }

    // Display header
    ProgressDisplay::header(name);

    if let Some(template_url) = template {
        // Use custom template from git archive. The template decides what
        // the application holds, `--eui` included; args.rs refuses the
        // two together rather than writing over what was downloaded.
        return create_from_template(name, app_path, template_url);
    }

    // Use default template generation
    let mut progress = ProgressDisplay::new(6);

    // Step 1: Create directory structure
    progress.step("Creating directory structure...");
    create_directories(app_path)?;
    ProgressDisplay::done();

    // Step 2: Generate configuration files
    progress.step("Generating configuration files...");
    create_routes_file(app_path, eui)?;
    create_seeds_file(app_path)?;
    create_application_config(app_path)?;
    create_env_file(app_path, name)?;
    create_env_test_file(app_path, name)?;
    create_gitignore(app_path)?;
    create_claude_md(app_path)?;
    create_agents_md(app_path)?;
    create_nested_claude_mds(app_path)?;
    create_bundled_docs(app_path)?;
    create_dot_claude(app_path)?;
    create_soli_toml(app_path, name)?;
    ProgressDisplay::done();

    // Step 3: Create MVC components
    progress.step("Creating MVC components...");
    create_home_controller(app_path)?;
    create_layout(app_path)?;
    create_og_image(app_path)?;
    create_index_view(app_path)?;
    create_application_helper(app_path)?;
    create_sample_middleware(app_path)?;
    create_starter_specs(app_path)?;
    create_stdlib(app_path)?;
    #[cfg(feature = "eui")]
    if eui {
        create_eui(app_path)?;
    }
    ProgressDisplay::done();

    // Step 4: Create assets
    progress.step("Setting up assets...");
    create_css_file(app_path)?;
    create_client_js(app_path)?;
    create_readme(app_path, name)?;
    if edge {
        create_edge(app_path, name)?;
    }
    ProgressDisplay::done();

    // Step 5: Build CSS with the standalone Tailwind binary (no npm)
    progress.step("Building Tailwind CSS...");
    io::stdout().flush().unwrap();
    println!(); // Move to next line for the compiler's own output
    build_tailwind_css(app_path);

    // Step 6: Initialize git repository
    progress.step("Initializing git repository...");
    io::stdout().flush().unwrap();
    println!(); // Move to next line for spinner
    init_git_repo(app_path);

    // Print created files summary
    println!();
    println!("  \x1b[2m─────────────────────────────────────────\x1b[0m");
    println!();
    ProgressDisplay::info("\x1b[1mProject structure:\x1b[0m");
    println!("  \x1b[2m│\x1b[0m");
    println!("  \x1b[2m│\x1b[0m  \x1b[36m{}/\x1b[0m", name);
    println!("  \x1b[2m│\x1b[0m  \x1b[2m├──\x1b[0m CLAUDE.md        \x1b[2m# AI agent guide (root + per-dir)\x1b[0m");
    println!("  \x1b[2m│\x1b[0m  \x1b[2m├──\x1b[0m AGENTS.md        \x1b[2m# tool-agnostic agent stub\x1b[0m");
    println!("  \x1b[2m│\x1b[0m  \x1b[2m├──\x1b[0m .claude/         \x1b[2m# permissions + /soli-verify, /soli-test, /soli-resource\x1b[0m");
    println!("  \x1b[2m│\x1b[0m  \x1b[2m├──\x1b[0m app/");
    println!(
        "  \x1b[2m│\x1b[0m  \x1b[2m│   ├──\x1b[0m controllers/    \x1b[2m# Request handlers{}\x1b[0m",
        if eui { ", + the EUI widget catalogue" } else { "" }
    );
    println!(
        "  \x1b[2m│\x1b[0m  \x1b[2m│   ├──\x1b[0m helpers/        \x1b[2m# View helpers\x1b[0m"
    );
    println!(
        "  \x1b[2m│\x1b[0m  \x1b[2m│   ├──\x1b[0m middleware/     \x1b[2m# Request filters\x1b[0m"
    );
    println!(
        "  \x1b[2m│\x1b[0m  \x1b[2m│   ├──\x1b[0m models/         \x1b[2m# Data models + concerns/\x1b[0m"
    );
    println!("  \x1b[2m│\x1b[0m  \x1b[2m│   └──\x1b[0m views/          \x1b[2m# Templates\x1b[0m");
    println!("  \x1b[2m│\x1b[0m  \x1b[2m├──\x1b[0m config/");
    println!(
        "  \x1b[2m│\x1b[0m  \x1b[2m│   ├──\x1b[0m routes.sl       \x1b[2m# URL routing\x1b[0m"
    );
    println!("  \x1b[2m│\x1b[0m  \x1b[2m│   └──\x1b[0m application.sl  \x1b[2m# boot config (trust_proxy, etc.)\x1b[0m");
    println!("  \x1b[2m│\x1b[0m  \x1b[2m├──\x1b[0m db/migrations/      \x1b[2m# Database migrations\x1b[0m");
    println!(
        "  \x1b[2m│\x1b[0m  \x1b[2m├──\x1b[0m public/             \x1b[2m# Static assets\x1b[0m"
    );
    println!("  \x1b[2m│\x1b[0m  \x1b[2m└──\x1b[0m tests/              \x1b[2m# Test files\x1b[0m");
    println!("  \x1b[2m│\x1b[0m");

    Ok(())
}

/// Reject tarball entry paths that contain anything other than plain
/// `Normal(_)` segments. This is the SEC-011 Zip-Slip guard: a template
/// archive that smuggles `..` or absolute path components could otherwise
/// overwrite arbitrary files when `soli new --template <url>` extracts
/// it under the operator's home directory.
fn validate_template_entry_path(path: &Path) -> Result<(), String> {
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => {
                return Err(format!(
                    "Refusing to extract template entry with unsafe path: {}",
                    path.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_directories_includes_model_concerns() {
        let dir = tempfile::tempdir().unwrap();
        create_directories(dir.path()).unwrap();
        assert!(dir.path().join("app/models/concerns").is_dir());
        assert!(dir.path().join("app/jobs").is_dir());
    }

    /// A new app's links preview from day one: the layout carries the share
    /// tags, with absolute URLs from the helper, and the image they name is
    /// a 1200×630 PNG.
    #[test]
    fn a_new_app_ships_share_tags_and_their_image() {
        let dir = tempfile::tempdir().unwrap();
        create_directories(dir.path()).unwrap();
        create_layout(dir.path()).unwrap();
        create_og_image(dir.path()).unwrap();
        create_application_helper(dir.path()).unwrap();

        let layout =
            std::fs::read_to_string(dir.path().join("app/views/layouts/application.html.slv"))
                .unwrap();
        for tag in [
            "og:title",
            "og:description",
            "og:url",
            "og:image",
            "twitter:card",
        ] {
            assert!(layout.contains(tag), "layout lacks {tag}");
        }
        assert!(layout.contains("absolute_url(og_image ?? \"/images/og.png\")"));
        assert!(
            !layout.contains("req[\"headers\"][\"host\"]"),
            "built from the Host header"
        );

        let png = std::fs::read(dir.path().join("public/images/og.png")).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
        let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
        assert_eq!((width, height), (1200, 630));

        let helper =
            std::fs::read_to_string(dir.path().join("app/helpers/application_helper.sl")).unwrap();
        assert!(helper.contains("def absolute_url(path)"));
    }

    /// Every starter helper renders in a view, on the tree-walker and on the
    /// view VM, from the environment `soli serve` loads helpers into.
    /// `truncate_text` and `capitalize` called `substring(...)`, a function
    /// only a view has, and `upcase(...)`, which exists nowhere: a helper sees
    /// neither, so every page that truncated or capitalized answered 500.
    #[test]
    fn the_starter_helpers_render_in_a_view() -> Result<(), Box<dyn std::error::Error>> {
        use crate::interpreter::builtins::template::{
            clear_view_helpers, load_view_helpers, set_view_helper_env_for_test,
        };
        use crate::interpreter::value::{HashKey, HashPairs, Value};
        use crate::template::core_eval::{reset_builtins_rc, template_env_bindings};
        use crate::template::parser::parse_template;
        use crate::template::renderer::render_nodes;
        use crate::template::vm_template::VmTemplate;
        use std::cell::RefCell;
        use std::rc::Rc;

        let dir = tempfile::tempdir()?;
        create_directories(dir.path())?;
        create_application_helper(dir.path())?;
        load_view_helpers(&dir.path().join("app/helpers"))?;
        // A thread builds the views' scope once: rebuild it with these helpers.
        reset_builtins_rc();

        let mut data = HashPairs::default();
        data.insert(
            HashKey::String("long".into()),
            Value::String("x".repeat(200).into()),
        );
        data.insert(
            HashKey::String("titles".into()),
            Value::Array(Rc::new(RefCell::new(vec![
                Value::String("hello WORLD".into()),
                Value::String("élan".into()),
                Value::String(String::new().into()),
            ]))),
        );
        let data = Value::Hash(Rc::new(RefCell::new(data)));

        // The loop makes it a view serve compiles for the VM.
        let source = concat!(
            "<%= truncate_text(long, 10, \"...\") %>|",
            "<%= truncate_text(\"short\", 10, \"...\") %>|",
            "<%= truncate_text(\"héllo wörld\", 8, \"…\") %>|",
            "<%= truncate_text(\"hello world\", 2, \"...\") %>|",
            "<% for title in titles %>[<%= capitalize(title) %>]<% end %>|",
            "<%= pluralize(2, \"post\", \"posts\") %>|",
            "<%= pluralize_simple(1, \"post\") %>|",
            "<%- link_to(\"Home\", \"/\") %>|",
            "<%- link_to_class(\"A\", \"javascript:alert(1)\", \"btn\") %>|",
            "<%= absolute_url(\"/og.png\").ends_with?(\"/og.png\") %>",
        );
        let expected = concat!(
            "xxxxxxx...|short|héllo w…|...|",
            "[Hello WORLD][Élan][]|",
            "2 posts|1 post|",
            "<a href=\"/\">Home</a>|<a href=\"#\" class=\"btn\">A</a>|",
            "true",
        );

        // What serve's renderer enters: a name nothing defines reads as nil.
        let _lenient = crate::interpreter::executor::enter_template_lenient_vars();
        let nodes = parse_template(source)?;
        assert_eq!(render_nodes(&nodes, &data, None)?, expected);

        let compiled = VmTemplate::compile_for_serve(&nodes)?;
        let mut vm = crate::vm::Vm::new();
        vm.tree_walk_functions = true;
        vm.globals = template_env_bindings().into_iter().collect();
        assert_eq!(compiled.render(&mut vm, &data)?.html, expected);

        clear_view_helpers();
        set_view_helper_env_for_test(None);
        reset_builtins_rc();
        Ok(())
    }

    /// Every bundled doc must be reachable from the generated root CLAUDE.md,
    /// or an agent in a new app never learns the feature exists (TOTP was
    /// documented in `docs/builtins.md` and still reimplemented by hand).
    /// Tutorials (`blog/`) and contributor internals are exempt; a whole
    /// directory can be covered by one `docs/<dir>/*.md` glob.
    #[test]
    fn root_claude_md_indexes_every_bundled_doc() {
        fn collect(dir: &include_dir::Dir, out: &mut Vec<String>) {
            for file in dir.files() {
                out.push(file.path().to_string_lossy().into_owned());
            }
            for sub in dir.dirs() {
                collect(sub, out);
            }
        }
        let mut paths = Vec::new();
        collect(&bundled_docs::DOCS, &mut paths);

        let index = app::CLAUDE_MD_TEMPLATE;
        let missing: Vec<_> = paths
            .iter()
            .filter(|p| p.ends_with(".md") && bundled_docs::should_copy(p))
            .filter(|p| !p.starts_with("blog/") && !p.starts_with("internals/"))
            .filter(|p| {
                let glob = p
                    .rsplit_once('/')
                    .map(|(dir, _)| format!("docs/{dir}/*.md"));
                !index.contains(&format!("docs/{p}")) && !glob.is_some_and(|g| index.contains(&g))
            })
            .collect();
        assert!(
            missing.is_empty(),
            "add these to the \"Built in\" index in src/scaffold/templates/CLAUDE.md: {missing:?}"
        );
    }

    #[test]
    fn accepts_normal_relative_paths() {
        assert!(validate_template_entry_path(Path::new("app/models/user.sl")).is_ok());
        assert!(validate_template_entry_path(Path::new("README.md")).is_ok());
        assert!(validate_template_entry_path(Path::new("a/b/c/d/e")).is_ok());
    }

    #[test]
    fn rejects_parent_dir_segment() {
        let err = validate_template_entry_path(Path::new("../bashrc")).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    #[test]
    fn rejects_buried_parent_dir_segment() {
        let err = validate_template_entry_path(Path::new("a/b/../../../etc/passwd")).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    #[test]
    fn rejects_absolute_path() {
        let err = validate_template_entry_path(Path::new("/etc/passwd")).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    #[test]
    fn rejects_current_dir_only() {
        // `./foo` parses as CurDir + Normal("foo"); the CurDir component
        // alone is harmless but we still refuse the entry rather than
        // strip it silently — predictable failure beats silent rewrite.
        let err = validate_template_entry_path(Path::new("./foo")).unwrap_err();
        assert!(err.contains("unsafe path"), "{}", err);
    }

    /// `--edge` writes the Worker config beside app/, local-only secrets git
    /// ignores, and says where the app runs in README and CLAUDE.md.
    #[test]
    fn edge_apps_get_their_worker_config() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("My Shop");
        fs::create_dir_all(&app).unwrap();
        fs::write(app.join(".gitignore"), "node_modules/\n").unwrap();
        fs::write(app.join("CLAUDE.md"), "# My Shop\n").unwrap();
        create_edge(&app, "My Shop").unwrap();

        let toml = fs::read_to_string(app.join("wrangler.toml")).unwrap();
        assert!(toml.contains("name = \"my-shop\""), "{toml}");
        assert!(toml.contains("database_name = \"my-shop-db\""), "{toml}");
        assert!(toml.contains("SOLI_DB_ADAPTER = \"d1\""), "{toml}");
        assert!(toml.contains("SOLI_SESSION_DRIVER = \"cookie\""), "{toml}");
        // A top-level key after a table would belong to that table.
        assert!(toml.find("compatibility_date").unwrap() < toml.find("[assets]").unwrap());

        let env = fs::read_to_string(app.join(".env")).unwrap();
        let dev_vars = fs::read_to_string(app.join(".dev.vars")).unwrap();
        let secret = env
            .lines()
            .find_map(|l| l.strip_prefix("SOLI_SESSION_SECRET="))
            .unwrap();
        assert_eq!(secret.len(), 64);
        assert!(
            dev_vars.contains(secret),
            "local Worker and soli serve share it"
        );
        assert!(env.contains("SOLI_DB_ADAPTER=sqlite"), "{env}");
        assert!(
            !toml.contains(secret),
            "the secret never goes in wrangler.toml"
        );

        let gitignore = fs::read_to_string(app.join(".gitignore")).unwrap();
        assert!(gitignore.starts_with("node_modules/\n"));
        for entry in ["/dist/edge/", ".wrangler/", ".dev.vars"] {
            assert!(
                gitignore.lines().any(|l| l == entry),
                "{entry}: {gitignore}"
            );
        }
        assert!(fs::read_to_string(app.join("CLAUDE.md"))
            .unwrap()
            .contains("runs on Cloudflare Workers"));
        assert!(fs::read_to_string(app.join("README.md"))
            .unwrap()
            .contains("soli edge dev"));
    }

    /// The generated .gitignore ignores the EUI publisher key: its entry was
    /// written with literal `\n`s and matched nothing.
    #[test]
    fn the_gitignore_ignores_the_eui_publisher_key() {
        assert!(app::GITIGNORE_TEMPLATE
            .lines()
            .any(|l| l == "config/eui_publisher.pkcs8"));
        assert!(!app::GITIGNORE_TEMPLATE.contains("\\n"));
    }

    #[test]
    fn slugify_db_name_normalizes() {
        assert_eq!(slugify_db_name("blog"), "blog");
        assert_eq!(slugify_db_name("My App"), "my_app");
        assert_eq!(slugify_db_name("my-cool-app"), "my_cool_app");
        assert_eq!(slugify_db_name("Foo.Bar/Baz"), "foo_bar_baz");
        assert_eq!(slugify_db_name("  spaced  out  "), "spaced_out");
        assert_eq!(slugify_db_name("__weird--name__"), "weird_name");
    }

    #[test]
    fn slugify_db_name_falls_back_to_default() {
        assert_eq!(slugify_db_name(""), "default");
        assert_eq!(slugify_db_name("---"), "default");
    }

    #[test]
    fn starter_specs_cover_what_new_ships() {
        let dir = tempfile::tempdir().unwrap();
        create_starter_specs(dir.path()).unwrap();
        for (spec, calls) in [
            (
                "tests/controllers/home_controller_spec.sl",
                "get(\"/health\")",
            ),
            ("tests/helpers/application_helper_spec.sl", "truncate_text("),
            ("tests/middleware/auth_spec.sl", "authenticate("),
        ] {
            let content = std::fs::read_to_string(dir.path().join(spec)).unwrap();
            assert!(content.contains(calls), "{spec} should call {calls}");
        }
    }

    #[test]
    fn get_started_box_lines_up() {
        let strip = |line: &str| {
            let mut out = String::new();
            let mut chars = line.chars();
            while let Some(c) = chars.next() {
                if c == '\x1b' {
                    for e in chars.by_ref() {
                        if e == 'm' {
                            break;
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            out
        };
        for name in ["demo", "a-much-longer-application-name-than-usual"] {
            let widths: Vec<usize> = get_started_box(name)
                .iter()
                .map(|line| strip(line).chars().count())
                .collect();
            assert!(widths.iter().all(|w| *w == widths[0]), "{name}: {widths:?}");
        }
    }

    #[test]
    fn env_test_file_names_a_test_database() {
        let dir = tempfile::tempdir().unwrap();
        create_env_test_file(dir.path(), "My Cool App").unwrap();
        let content = std::fs::read_to_string(dir.path().join(".env.test")).unwrap();
        assert!(
            content.contains("SOLIDB_DATABASE=my_cool_app_test"),
            "{content}"
        );
        assert!(content.contains("SOLIDB_HOST="), "{content}");
    }

    #[test]
    fn env_file_uses_slugified_app_name() {
        let dir = std::env::temp_dir().join(format!("soli_env_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        create_env_file(&dir, "My Cool App").unwrap();
        let content = std::fs::read_to_string(dir.join(".env")).unwrap();
        assert!(
            content.contains("SOLIDB_DATABASE=my_cool_app"),
            "expected slugified db name, got:\n{}",
            content
        );
        assert!(!content.contains("SOLIDB_DATABASE=default"));
        assert!(
            content.contains("SOLI_CSRF_TOKENS=require"),
            "new apps must require CSRF tokens, got:\n{}",
            content
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
