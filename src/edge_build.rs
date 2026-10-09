//! `soli edge build`: package an app as a Cloudflare Worker.
//!
//! The Worker is the edge runtime (`soli_edge_bg.wasm` and its JS host, built
//! by `scripts/build-edge.sh`) plus the app's source as `src/app.json`, which
//! the host mounts in memory before booting — a Worker has no filesystem.
//! `public/` becomes the Worker's static assets. The output is a plain
//! Wrangler project: `npx wrangler dev` runs it, `npx wrangler deploy` ships it.

use std::fs;
use std::path::{Path, PathBuf};

/// The files `scripts/build-edge.sh` produces, all required.
pub const RUNTIME_FILES: [&str; 4] = ["soli_edge_bg.wasm", "soli_edge.js", "jspi.js", "worker.js"];

/// The app directories the Worker needs at boot; `public/` is served as assets.
const SOURCE_DIRS: [&str; 3] = ["app", "config", "lib"];

pub struct Options<'a> {
    pub app_dir: &'a Path,
    pub out_dir: &'a Path,
    /// Where the runtime lives; `None` searches the usual places.
    pub runtime_dir: Option<&'a Path>,
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

    for name in RUNTIME_FILES {
        let from = runtime.join(name);
        fs::copy(&from, src.join(name)).map_err(|e| format!("copy {}: {e}", from.display()))?;
    }
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
        let config = wrangler_config(&worker_name(app_dir), asset_files > 0);
        fs::write(&wrangler_toml, config).map_err(|e| format!("write wrangler.toml: {e}"))?;
    }

    Ok(Summary {
        out_dir: out.to_path_buf(),
        source_files: files.len(),
        source_bytes,
        skipped,
        asset_files,
        wasm_bytes,
        wrote_wrangler_toml,
    })
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

fn wrangler_config(name: &str, assets: bool) -> String {
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
        for name in RUNTIME_FILES {
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
        })
        .unwrap_err();
        assert!(err.contains("scripts/build-edge.sh"), "{err}");
    }
}
