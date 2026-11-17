//! `soli edge build` and `soli edge dev`: see `solilang::edge_build`.

use std::path::Path;
use std::process;

pub fn build(folder: &str, out: Option<&str>, runtime: Option<&str>, sql: Option<&str>, pdf: bool) {
    let app_dir = Path::new(folder);
    let out_dir = out
        .map(Path::new)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| app_dir.join("dist").join("edge"));
    let summary = match solilang::edge_build::build(&solilang::edge_build::Options {
        app_dir,
        out_dir: &out_dir,
        runtime_dir: runtime.map(Path::new),
        sql,
        pdf,
        npm_install: true,
    }) {
        Ok(summary) => summary,
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    };

    println!(
        "Packaged {} source files ({} KB) and {} static assets into {}",
        summary.source_files,
        summary.source_bytes / 1024,
        summary.asset_files,
        summary.out_dir.display()
    );
    println!(
        "Runtime: {:.1} MB of wasm{}",
        summary.wasm_bytes as f64 / 1e6,
        if summary.pdf { ", with PDF" } else { "" }
    );
    for skipped in &summary.skipped {
        println!(
            "  skipped {} (not UTF-8 text; binary files belong in public/)",
            skipped.display()
        );
    }
    if !summary.sql_drivers.is_empty() {
        println!("SQL drivers: {}", summary.sql_drivers.join(", "));
    }
    if let Some(npm) = &summary.npm {
        println!("{npm}");
    }
    if summary.wrangler_toml_from_app {
        println!("Used the app's wrangler.toml");
    } else if summary.wrote_wrangler_toml {
        println!("Wrote wrangler.toml (later builds keep your edits)");
    }
    for warning in &summary.warnings {
        println!("Warning: {warning}");
    }
    println!();
    println!("Next:");
    println!("  cd {}", summary.out_dir.display());
    println!("  npx wrangler dev      # run it locally on workerd");
    println!("  npx wrangler deploy   # ship it");
}

/// `soli edge dev`: the app on workerd, the runtime Cloudflare runs, rebuilt as
/// you edit. It packages the app into `<folder>/dist/edge` (copying the app's
/// `.dev.vars`, wrangler's local-only variables), starts `wrangler dev` on
/// `port`, and repackages on every change under `app/`, `config/`, `lib/`,
/// `public/` or to `wrangler.toml` / `.dev.vars`: wrangler reloads the Worker,
/// and the page with it (`--live-reload`).
///
/// Behind soli-proxy it is a site like any other:
/// `start_script = "soli edge dev . --port $PORT"`. The proxy stops a site by
/// signalling its process group, which takes `wrangler dev` along.
pub fn dev(folder: &str, port: u16, runtime: Option<&str>, sql: Option<&str>, pdf: bool) {
    let app_dir = Path::new(folder);
    let out_dir = app_dir.join("dist").join("edge");
    let options = solilang::edge_build::Options {
        app_dir,
        out_dir: &out_dir,
        runtime_dir: runtime.map(Path::new),
        sql,
        pdf,
        npm_install: true,
    };
    if let Err(e) = package_for_dev(&options, port) {
        eprintln!("Error: {e}");
        process::exit(1);
    }

    let mut wrangler = match process::Command::new("npx")
        .args([
            "wrangler",
            "dev",
            "--local",
            "--live-reload",
            "--ip",
            "127.0.0.1",
        ])
        .args(["--port", &port.to_string()])
        .current_dir(&out_dir)
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            eprintln!("Error: could not start `npx wrangler dev` (is Node.js installed?): {e}");
            process::exit(1);
        }
    };
    println!("soli edge dev: http://127.0.0.1:{port} — watching the app for changes");

    let (app, out) = (app_dir.to_path_buf(), out_dir.clone());
    let runtime = runtime.map(std::path::PathBuf::from);
    let sql = sql.map(str::to_string);
    let rebuild = move || {
        let options = solilang::edge_build::Options {
            app_dir: &app,
            out_dir: &out,
            runtime_dir: runtime.as_deref(),
            sql: sql.as_deref(),
            pdf,
            npm_install: true,
        };
        match package_for_dev(&options, port) {
            Ok(()) => println!("soli edge dev: repackaged"),
            Err(e) => eprintln!("soli edge dev: {e}"),
        }
    };
    let _watcher = watch_app(app_dir, rebuild);

    let status = wrangler.wait().map(|s| s.code().unwrap_or(1)).unwrap_or(1);
    process::exit(status);
}

/// Package the app and write the Worker's `.dev.vars`: the app's own, plus a
/// `SOLI_APP_HOSTS` naming where the page is opened from, unless the app's
/// file sets it. `wrangler dev` hands the Worker the production host of
/// `routes` as `Host`, so a form posted from `http://localhost:<port>` — or
/// from `https://<site>.test` through soli-proxy — would fail the same-origin
/// check. The list is this server's own addresses, the hosts soli-proxy puts
/// in the environment, and the ones the app's `.env` lists (where the proxy
/// reads them too); a Worker inherits none of that environment by itself.
fn package_for_dev(options: &solilang::edge_build::Options<'_>, port: u16) -> Result<(), String> {
    let summary = solilang::edge_build::build(options)?;
    for warning in &summary.warnings {
        eprintln!("Warning: {warning}");
    }
    let mut vars = std::fs::read_to_string(options.app_dir.join(".dev.vars")).unwrap_or_default();
    let sets_hosts = vars
        .lines()
        .any(|line| line.trim_start().starts_with("SOLI_APP_HOSTS"));
    if !sets_hosts {
        let from_env_file = std::fs::read_to_string(options.app_dir.join(".env"))
            .ok()
            .and_then(|text| {
                text.lines().find_map(|line| {
                    let value = line.trim().strip_prefix("SOLI_APP_HOSTS=")?;
                    Some(value.trim().trim_matches('"').to_string())
                })
            })
            .unwrap_or_default();
        let mut hosts = vec![format!("localhost:{port}"), format!("127.0.0.1:{port}")];
        for host in std::env::var("SOLI_APP_HOSTS")
            .unwrap_or_default()
            .split(',')
            .chain(from_env_file.split(','))
        {
            let host = host.trim().to_ascii_lowercase();
            if !host.is_empty() && !hosts.contains(&host) {
                hosts.push(host);
            }
        }
        if !vars.is_empty() && !vars.ends_with('\n') {
            vars.push('\n');
        }
        vars.push_str(&format!("SOLI_APP_HOSTS=\"{}\"\n", hosts.join(",")));
    }
    std::fs::write(options.out_dir.join(".dev.vars"), vars)
        .map_err(|e| format!("write .dev.vars: {e}"))
}

/// Watch what `soli edge build` packages and call `rebuild` once per burst of
/// changes. The returned watcher must stay alive.
fn watch_app(
    app_dir: &Path,
    rebuild: impl Fn() + Send + 'static,
) -> Option<notify::RecommendedWatcher> {
    use notify::{RecursiveMode, Watcher};
    use std::time::Duration;

    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match notify::recommended_watcher(tx) {
        Ok(watcher) => watcher,
        Err(e) => {
            eprintln!("soli edge dev: no file watcher ({e}); restart to pick up changes");
            return None;
        }
    };
    for dir in ["app", "config", "lib", "public"] {
        let path = app_dir.join(dir);
        if path.is_dir() {
            let _ = watcher.watch(&path, RecursiveMode::Recursive);
        }
    }
    for file in ["wrangler.toml", ".dev.vars"] {
        let path = app_dir.join(file);
        if path.is_file() {
            let _ = watcher.watch(&path, RecursiveMode::NonRecursive);
        }
    }
    std::thread::spawn(move || {
        while let Ok(event) = rx.recv() {
            let relevant = |e: &notify::Result<notify::Event>| {
                e.as_ref()
                    .is_ok_and(|e| !matches!(e.kind, notify::EventKind::Access(_)))
            };
            if !relevant(&event) {
                continue;
            }
            // An editor's save is several events; build once they settle.
            while rx.recv_timeout(Duration::from_millis(250)).is_ok() {}
            rebuild();
        }
    });
    Some(watcher)
}
