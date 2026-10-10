//! `soli edge build`: package an app as a Cloudflare Worker.
//!
//! The Worker is the edge runtime (`soli_edge_bg.wasm` and its JS host, built
//! by `scripts/build-edge.sh`) plus the app's source as `src/app.json`, which
//! the host mounts in memory before booting — a Worker has no filesystem.
//! `public/` becomes the Worker's static assets. The output is a plain
//! Wrangler project: `npx wrangler dev` runs it, `npx wrangler deploy` ships it.

use std::fs;
use std::path::{Path, PathBuf};

/// The files `scripts/build-edge.sh` produces, all required. The first four are
/// copied into every Worker, the Postgres driver's glue only when the app uses
/// Postgres.
pub const RUNTIME_FILES: [&str; 5] = [
    "soli_edge_bg.wasm",
    "soli_edge.js",
    "jspi.js",
    "worker.js",
    "sql-pg.js",
];

/// The SQL dialects a Worker can carry a JavaScript driver for: the dialect's
/// name in `src/sql.js`, its driver module, and the npm package it imports.
/// Only Postgres: MySQL's `mysql2` weighs 410 KB compressed, more than the
/// room left under the free plan's 3 MB.
const SQL_DRIVERS: [(&str, &str, &str, &str); 1] = [("postgres", "sql-pg.js", "pg", "^8.13.0")];

/// Why an app on MySQL cannot run on the edge build.
const NO_MYSQL: &str = "MySQL is not available on the edge build (its driver would take the \
                        Worker past the free plan's 3 MB): use postgres through Hyperdrive, \
                        d1 or solidb";

/// The app directories the Worker needs at boot; `public/` is served as assets.
const SOURCE_DIRS: [&str; 3] = ["app", "config", "lib"];

pub struct Options<'a> {
    pub app_dir: &'a Path,
    pub out_dir: &'a Path,
    /// Where the runtime lives; `None` searches the usual places.
    pub runtime_dir: Option<&'a Path>,
    /// `--sql`: `postgres` to bundle its driver, or `none`; `None` detects it
    /// from the app's configuration.
    pub sql: Option<&'a str>,
    /// Run `npm install` when a driver is missing from `node_modules`.
    pub npm_install: bool,
}

#[derive(Debug)]
pub struct Summary {
    pub out_dir: PathBuf,
    pub source_files: usize,
    pub source_bytes: usize,
    /// Files left out of the bundle because they are not UTF-8 text.
    pub skipped: Vec<PathBuf>,
    pub asset_files: usize,
    pub wasm_bytes: u64,
    /// Whether this run wrote `wrangler.toml` (an existing one is kept).
    pub wrote_wrangler_toml: bool,
    /// The SQL dialects the Worker carries a driver for.
    pub sql_drivers: Vec<&'static str>,
    /// What `npm install` did, when it ran.
    pub npm: Option<String>,
    /// Things the user has to fix by hand.
    pub warnings: Vec<String>,
}

pub fn build(opts: &Options<'_>) -> Result<Summary, String> {
    let app_dir = opts.app_dir;
    if !app_dir.join("app").join("controllers").is_dir() {
        return Err(format!(
            "{} is not a Soli app: app/controllers is missing",
            app_dir.display()
        ));
    }
    let runtime = find_runtime(opts.runtime_dir)?;
    let out = opts.out_dir;
    let src = out.join("src");
    fs::create_dir_all(&src).map_err(|e| format!("create {}: {e}", src.display()))?;

    let mut warnings = Vec::new();
    let sql_drivers = match opts.sql {
        Some(list) => parse_sql_flag(list)?,
        None => {
            let mut found = detect_sql(app_dir, out);
            if found.contains(&"mysql") {
                warnings.push(NO_MYSQL.to_string());
                found.retain(|dialect| *dialect != "mysql");
            }
            found
        }
    };
    let mut copied: Vec<&str> = RUNTIME_FILES[..4].to_vec();
    for (dialect, module, _, _) in SQL_DRIVERS {
        let stale = src.join(module);
        if sql_drivers.contains(&dialect) {
            copied.push(module);
        } else if stale.exists() {
            fs::remove_file(&stale).map_err(|e| format!("remove {}: {e}", stale.display()))?;
        }
    }
    for name in copied {
        let from = runtime.join(name);
        fs::copy(&from, src.join(name)).map_err(|e| format!("copy {}: {e}", from.display()))?;
    }
    fs::write(src.join("sql.js"), sql_module(&sql_drivers))
        .map_err(|e| format!("write sql.js: {e}"))?;
    let wasm_bytes = fs::metadata(src.join("soli_edge_bg.wasm"))
        .map(|m| m.len())
        .unwrap_or(0);

    // The app's source, keyed by the absolute path the Worker boots from.
    let mut files: Vec<(String, String)> = Vec::new();
    let mut skipped = Vec::new();
    let mut source_bytes = 0;
    for dir in SOURCE_DIRS {
        for path in walk(&app_dir.join(dir))? {
            let relative = path
                .strip_prefix(app_dir)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            match fs::read(&path).map(String::from_utf8) {
                Ok(Ok(text)) => {
                    source_bytes += text.len();
                    files.push((format!("/app/{relative}"), text));
                }
                Ok(Err(_)) => skipped.push(PathBuf::from(relative)),
                Err(e) => return Err(format!("read {}: {e}", path.display())),
            }
        }
    }
    let json = serde_json::to_string(&files).map_err(|e| e.to_string())?;
    fs::write(src.join("app.json"), json).map_err(|e| format!("write app.json: {e}"))?;

    // Static assets: a fresh copy, so a file deleted from public/ goes away.
    let assets = out.join("public");
    if assets.exists() {
        fs::remove_dir_all(&assets).map_err(|e| format!("clear {}: {e}", assets.display()))?;
    }
    let public = app_dir.join("public");
    let mut asset_files = 0;
    if public.is_dir() {
        for path in walk(&public)? {
            let to = assets.join(path.strip_prefix(&public).map_err(|e| e.to_string())?);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("create {}: {e}", parent.display()))?;
            }
            fs::copy(&path, &to).map_err(|e| format!("copy {}: {e}", path.display()))?;
            asset_files += 1;
        }
    }

    let wrangler_toml = out.join("wrangler.toml");
    let wrote_wrangler_toml = !wrangler_toml.exists();
    if wrote_wrangler_toml {
        let config = wrangler_config(&worker_name(app_dir), asset_files > 0, &sql_drivers);
        fs::write(&wrangler_toml, config).map_err(|e| format!("write wrangler.toml: {e}"))?;
    } else if !sql_drivers.is_empty() {
        let existing = fs::read_to_string(&wrangler_toml).unwrap_or_default();
        if !existing.contains("nodejs_compat") {
            warnings.push(
                "the SQL driver needs Node.js compatibility: add \
                 `compatibility_flags = [\"nodejs_compat\"]` to wrangler.toml, above its first table"
                    .to_string(),
            );
        }
    }

    let mut npm = None;
    if !sql_drivers.is_empty() {
        write_package_json(out, &worker_name(app_dir), &sql_drivers)?;
        let missing: Vec<&str> = SQL_DRIVERS
            .iter()
            .filter(|(dialect, ..)| sql_drivers.contains(dialect))
            .map(|(_, _, package, _)| *package)
            .filter(|package| !out.join("node_modules").join(package).is_dir())
            .collect();
        if !missing.is_empty() {
            if opts.npm_install {
                match npm_install(out) {
                    Ok(()) => npm = Some(format!("npm install: {}", missing.join(", "))),
                    Err(e) => warnings.push(format!(
                        "{e}: run `npm install` in {} before `wrangler`",
                        out.display()
                    )),
                }
            } else {
                warnings.push(format!(
                    "run `npm install` in {} before `wrangler` ({} missing)",
                    out.display(),
                    missing.join(", ")
                ));
            }
        }
    }

    Ok(Summary {
        out_dir: out.to_path_buf(),
        source_files: files.len(),
        source_bytes,
        skipped,
        asset_files,
        wasm_bytes,
        wrote_wrangler_toml,
        sql_drivers,
        npm,
        warnings,
    })
}

/// `--sql postgres` / `--sql none`.
fn parse_sql_flag(list: &str) -> Result<Vec<&'static str>, String> {
    let mut out = Vec::new();
    for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        if name.eq_ignore_ascii_case("none") {
            continue;
        }
        match crate::db::parse_adapter(Some(name)) {
            Ok(crate::db::Adapter::Postgres) => out.push("postgres"),
            Ok(crate::db::Adapter::Mysql) => return Err(NO_MYSQL.to_string()),
            _ => return Err(format!("--sql takes postgres or none (got {name:?})")),
        }
    }
    out.dedup();
    Ok(out)
}

/// The dialects the app's configuration names: `adapter = "…"` in
/// `config/database.toml`, `SOLI_DB_ADAPTER` in the app's `.env` or under the
/// Worker's `[vars]`.
fn detect_sql(app_dir: &Path, out: &Path) -> Vec<&'static str> {
    let mut found = Vec::new();
    for file in [
        app_dir.join("config/database.toml"),
        app_dir.join(".env"),
        out.join("wrangler.toml"),
    ] {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or_default();
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().trim_start_matches("export ").trim();
            if !key.eq_ignore_ascii_case("adapter") && key != "SOLI_DB_ADAPTER" {
                continue;
            }
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            let dialect = match crate::db::parse_adapter(Some(value)) {
                Ok(crate::db::Adapter::Postgres) => "postgres",
                Ok(crate::db::Adapter::Mysql) => "mysql",
                _ => continue,
            };
            if !found.contains(&dialect) {
                found.push(dialect);
            }
        }
    }
    found
}

/// `src/sql.js`: the drivers this Worker imports, and `null` for the others.
fn sql_module(drivers: &[&str]) -> String {
    let mut out =
        String::from("// Written by `soli edge build`: the SQL drivers this Worker carries.\n");
    for (dialect, module, _, _) in SQL_DRIVERS {
        if drivers.contains(&dialect) {
            out.push_str(&format!(
                "export {{ connect as {dialect} }} from \"./{module}\";\n"
            ));
        } else {
            out.push_str(&format!("export const {dialect} = null;\n"));
        }
    }
    out
}

/// Add the drivers to `package.json`, creating it if needed; other entries stay.
fn write_package_json(out: &Path, name: &str, drivers: &[&str]) -> Result<(), String> {
    let path = out.join("package.json");
    let mut package: serde_json::Value = match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
        Err(_) => serde_json::json!({ "name": name, "private": true }),
    };
    let Some(object) = package.as_object_mut() else {
        return Err(format!("{}: not a JSON object", path.display()));
    };
    let dependencies = object
        .entry("dependencies")
        .or_insert_with(|| serde_json::json!({}));
    let Some(dependencies) = dependencies.as_object_mut() else {
        return Err(format!("{}: dependencies is not an object", path.display()));
    };
    let mut changed = !path.exists();
    for (dialect, _, package_name, version) in SQL_DRIVERS {
        if drivers.contains(&dialect) && !dependencies.contains_key(package_name) {
            dependencies.insert(package_name.to_string(), serde_json::json!(version));
            changed = true;
        }
    }
    if changed {
        let text = serde_json::to_string_pretty(&package).map_err(|e| e.to_string())?;
        fs::write(&path, text + "\n").map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(())
}

fn npm_install(out: &Path) -> Result<(), String> {
    let status = std::process::Command::new("npm")
        .args(["install", "--no-audit", "--no-fund"])
        .current_dir(out)
        .status()
        .map_err(|e| format!("could not run npm ({e})"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("npm install failed ({status})"))
    }
}

/// The runtime directory: the one given, else `$SOLI_EDGE_RUNTIME`, else next
/// to the `soli` binary (`edge/`, or `../share/soli/edge` for an installed one).
fn find_runtime(given: Option<&Path>) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = given {
        candidates.push(dir.to_path_buf());
    } else {
        if let Some(dir) = std::env::var_os("SOLI_EDGE_RUNTIME") {
            candidates.push(PathBuf::from(dir));
        }
        if let Some(bin_dir) = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
        {
            candidates.push(bin_dir.join("edge"));
            candidates.push(bin_dir.join("../share/soli/edge"));
        }
    }
    let complete = |dir: &Path| RUNTIME_FILES.iter().all(|f| dir.join(f).is_file());
    candidates
        .into_iter()
        .find(|dir| complete(dir))
        .ok_or_else(|| {
            let looked = given
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "$SOLI_EDGE_RUNTIME and next to the soli binary".to_string());
            format!(
                "no edge runtime found ({looked}). Build it with scripts/build-edge.sh \
             in the Soli source tree, then pass --runtime <dir> or set SOLI_EDGE_RUNTIME"
            )
        })
}

/// Every file under `dir`, sorted; nothing when `dir` does not exist.
fn walk(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for entry in walkdir::WalkDir::new(dir).sort_by_file_name() {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_type().is_file() {
            out.push(entry.into_path());
        }
    }
    Ok(out)
}

/// A Worker name from the app's folder: lowercase letters, digits and dashes.
fn worker_name(app_dir: &Path) -> String {
    let folder = app_dir
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let name: String = folder
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let name = name.trim_matches('-');
    if name.is_empty() {
        "soli-app".to_string()
    } else {
        name.to_string()
    }
}

fn wrangler_config(name: &str, assets: bool, sql_drivers: &[&str]) -> String {
    let today = chrono::Utc::now().format("%Y-%m-%d");
    // Top-level keys first: in TOML everything after a `[table]` header
    // belongs to that table, `routes` included.
    let mut config = format!(
        r#"# Written by `soli edge build`; it never overwrites this file, so edit away.
name = "{name}"
main = "src/worker.js"
compatibility_date = "{today}"

# A custom domain, once its zone is on this Cloudflare account:
# routes = [{{ pattern = "app.example.com", custom_domain = true }}]
"#
    );
    if !sql_drivers.is_empty() {
        config.push_str(
            r#"
# The Postgres driver (pg) runs on Workers' Node.js compatibility layer.
compatibility_flags = ["nodejs_compat"]
"#,
        );
    }
    if assets {
        config.push_str(
            r#"
# public/ is served before the Worker runs.
[assets]
directory = "./public"
"#,
        );
    }
    config.push_str(
        r#"
# Configuration the app reads with getenv(). Plain values here; secrets with
# `npx wrangler secret put NAME` (e.g. SOLIDB_PASSWORD), never in this file.
# [vars]
# SOLIDB_HOST = "https://db.example.com"
# SOLIDB_DATABASE = "my_app"
# SOLIDB_USERNAME = "app"

# Models on Cloudflare D1 instead (`npx wrangler d1 create my-app-db` prints
# the database_id); set SOLI_DB_ADAPTER = "d1" under [vars] as well.
# [[d1_databases]]
# binding = "DB"
# database_name = "my-app-db"
# database_id = "…"

# Attachments on R2 (`service: "r2"`; `npx wrangler r2 bucket create my-app-files`).
# [[r2_buckets]]
# binding = "ATTACHMENTS"
# bucket_name = "my-app-files"

# Image transforms on R2 attachment URLs (`?w=300&fmt=webp`) — without it they
# answer the stored bytes.
# [images]
# binding = "IMAGES"

# Background jobs (`perform_later`, `Job.enqueue`) through Cloudflare Queues
# (`npx wrangler queues create my-app-jobs`). Soli decides retries with each
# job's max_retries, so let Cloudflare redeliver as often as it asks.
# [[queues.producers]]
# binding = "JOBS"
# queue = "my-app-jobs"
# [[queues.consumers]]
# queue = "my-app-jobs"
# max_retries = 100

# Models on PostgreSQL through Hyperdrive
# (`npx wrangler hyperdrive create my-db --connection-string="postgres://…"`
# prints the id); set SOLI_DB_ADAPTER = "postgres" under [vars].
# DATABASE_URL defaults to hyperdrive://HYPERDRIVE.
# [[hyperdrive]]
# binding = "HYPERDRIVE"
# id = "…"
# localConnectionString = "postgres://user:password@localhost:5432/my_app"
"#,
    );
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_runtime(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        for name in RUNTIME_FILES {
            fs::write(dir.join(name), name).unwrap();
        }
    }

    fn fake_app(dir: &Path) {
        fs::create_dir_all(dir.join("app/controllers")).unwrap();
        fs::create_dir_all(dir.join("config")).unwrap();
        fs::create_dir_all(dir.join("public/css")).unwrap();
        fs::write(
            dir.join("app/controllers/home_controller.sl"),
            "class HomeController < Controller\nend\n",
        )
        .unwrap();
        fs::write(dir.join("config/routes.sl"), "get(\"/\", \"home#index\")\n").unwrap();
        fs::write(dir.join("app/logo.bin"), [0xff, 0xfe, 0x00]).unwrap();
        fs::write(dir.join("public/css/site.css"), "body{}").unwrap();
        fs::write(dir.join(".env"), "SECRET=1").unwrap();
    }

    #[test]
    fn packages_source_assets_and_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let (app, runtime, out) = (
            tmp.path().join("My App"),
            tmp.path().join("rt"),
            tmp.path().join("out"),
        );
        fake_app(&app);
        fake_runtime(&runtime);

        let summary = build(&Options {
            app_dir: &app,
            out_dir: &out,
            runtime_dir: Some(&runtime),
            sql: None,
            npm_install: false,
        })
        .unwrap();

        assert_eq!(summary.source_files, 2);
        assert_eq!(summary.skipped, vec![PathBuf::from("app/logo.bin")]);
        assert_eq!(summary.asset_files, 1);
        assert!(summary.wrote_wrangler_toml);
        let files: Vec<(String, String)> =
            serde_json::from_str(&fs::read_to_string(out.join("src/app.json")).unwrap()).unwrap();
        let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/app/app/controllers/home_controller.sl",
                "/app/config/routes.sl"
            ]
        );
        assert!(out.join("public/css/site.css").is_file());
        // The SQL drivers' glue stays out of an app that uses no SQL.
        for name in &RUNTIME_FILES[..4] {
            assert!(out.join("src").join(name).is_file(), "{name}");
        }
        let toml = fs::read_to_string(out.join("wrangler.toml")).unwrap();
        assert!(toml.contains("name = \"my-app\""), "{toml}");
        assert!(toml.contains("[assets]"));
        // `routes` must stay above the first table, or TOML files it under it.
        assert!(toml.find("# routes =").unwrap() < toml.find("[assets]").unwrap());
        // .env holds secrets: it is not part of the bundle.
        assert!(!fs::read_to_string(out.join("src/app.json"))
            .unwrap()
            .contains("SECRET"));
    }

    #[test]
    fn keeps_an_existing_wrangler_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let (app, runtime, out) = (
            tmp.path().join("app"),
            tmp.path().join("rt"),
            tmp.path().join("out"),
        );
        fake_app(&app);
        fake_runtime(&runtime);
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("wrangler.toml"), "name = \"mine\"\n").unwrap();

        let summary = build(&Options {
            app_dir: &app,
            out_dir: &out,
            runtime_dir: Some(&runtime),
            sql: None,
            npm_install: false,
        })
        .unwrap();

        assert!(!summary.wrote_wrangler_toml);
        assert_eq!(
            fs::read_to_string(out.join("wrangler.toml")).unwrap(),
            "name = \"mine\"\n"
        );
    }

    #[test]
    fn refuses_a_folder_that_is_not_an_app() {
        let tmp = tempfile::tempdir().unwrap();
        let runtime = tmp.path().join("rt");
        fake_runtime(&runtime);
        let err = build(&Options {
            app_dir: tmp.path(),
            out_dir: &tmp.path().join("out"),
            runtime_dir: Some(&runtime),
            sql: None,
            npm_install: false,
        })
        .unwrap_err();
        assert!(err.contains("not a Soli app"), "{err}");
    }

    #[test]
    fn explains_a_missing_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("app");
        fake_app(&app);
        let err = build(&Options {
            app_dir: &app,
            out_dir: &tmp.path().join("out"),
            runtime_dir: Some(&tmp.path().join("none")),
            sql: None,
            npm_install: false,
        })
        .unwrap_err();
        assert!(err.contains("scripts/build-edge.sh"), "{err}");
    }

    /// An app on Postgres gets node-postgres: its glue, `src/sql.js` exporting
    /// it, the package in `package.json`, and `nodejs_compat` in a new
    /// `wrangler.toml`. An app on nothing SQL carries no driver.
    #[test]
    fn bundles_the_sql_driver_the_app_uses() {
        let tmp = tempfile::tempdir().unwrap();
        let (app, runtime, out) = (
            tmp.path().join("app"),
            tmp.path().join("rt"),
            tmp.path().join("out"),
        );
        fake_app(&app);
        fake_runtime(&runtime);
        let options = |sql| Options {
            app_dir: &app,
            out_dir: &out,
            runtime_dir: Some(&runtime),
            sql,
            npm_install: false,
        };

        let summary = build(&options(None)).unwrap();
        assert!(summary.sql_drivers.is_empty());
        assert!(!out.join("src/sql-pg.js").exists());
        assert!(!out.join("package.json").exists());
        let module = fs::read_to_string(out.join("src/sql.js")).unwrap();
        assert!(module.contains("export const postgres = null;"), "{module}");

        fs::write(
            app.join("config/database.toml"),
            "[connections.primary]\nadapter = \"pg\" # Hyperdrive on the Worker\n",
        )
        .unwrap();
        fs::remove_file(out.join("wrangler.toml")).unwrap();
        let summary = build(&options(None)).unwrap();
        assert_eq!(summary.sql_drivers, vec!["postgres"]);
        assert!(out.join("src/sql-pg.js").is_file());
        let module = fs::read_to_string(out.join("src/sql.js")).unwrap();
        assert!(
            module.contains("export { connect as postgres } from \"./sql-pg.js\";"),
            "{module}"
        );
        let package: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(out.join("package.json")).unwrap()).unwrap();
        assert_eq!(package["dependencies"]["pg"], "^8.13.0");
        let toml = fs::read_to_string(out.join("wrangler.toml")).unwrap();
        assert!(
            toml.contains("compatibility_flags = [\"nodejs_compat\"]"),
            "{toml}"
        );
        assert!(
            toml.find("compatibility_flags").unwrap() < toml.find("[assets]").unwrap(),
            "{toml}"
        );
        // npm did not run here, so the summary says what to do.
        assert!(summary.warnings.iter().any(|w| w.contains("npm install")));

        // An existing wrangler.toml without nodejs_compat earns a warning
        // rather than an edit.
        fs::write(out.join("wrangler.toml"), "name = \"mine\"\n").unwrap();
        let summary = build(&options(Some("postgres"))).unwrap();
        assert!(summary.warnings.iter().any(|w| w.contains("nodejs_compat")));

        // --sql none wins over detection, and the stale driver goes.
        let summary = build(&options(Some("none"))).unwrap();
        assert!(summary.sql_drivers.is_empty());
        assert!(
            !out.join("src/sql-pg.js").exists(),
            "a stale driver is removed"
        );

        // MySQL is refused on the edge: by name with --sql, with a warning
        // when the configuration names it.
        let err = build(&options(Some("mysql"))).unwrap_err();
        assert!(err.contains("MySQL is not available"), "{err}");
        fs::write(
            app.join("config/database.toml"),
            "[connections.primary]\nadapter = \"mysql\"\n",
        )
        .unwrap();
        let summary = build(&options(None)).unwrap();
        assert!(summary.sql_drivers.is_empty());
        assert!(summary
            .warnings
            .iter()
            .any(|w| w.contains("MySQL is not available")));

        assert!(build(&options(Some("oracle"))).is_err());
    }
}
