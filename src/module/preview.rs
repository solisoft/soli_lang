//! Per-branch preview environments (`soli env`).
//!
//! A preview environment is one git worktree of an app, registered with
//! soli-proxy as a site directory whose name *is* its domain, backed by its own
//! SoliDB database seeded from migrations. The proxy already supplies every
//! runtime concern — port allocation, blue/green slots, health gating on `/up`,
//! TLS — so this module only has to create the four things it does not:
//! the worktree, the `.env`, the database, and the symlink.
//!
//! # Why the naming rules are strict
//!
//! Preview domains are flat (`<slug>--<app>.<base>`) rather than nested
//! (`<slug>.<app>.<base>`) because DNS wildcards *and* the proxy's SNI resolver
//! both match exactly one label deep. A flat scheme means one `*.<base>` DNS
//! record and one wildcard certificate cover every app and branch; a nested one
//! would need a record and a certificate per app.
//!
//! Branch names are hostile input for DNS: `task/`-prefixed branches routinely
//! exceed 63 characters and contain `/`, which is illegal in a label.

use std::path::{Path, PathBuf};

/// Longest slug kept verbatim. Beyond this a hash suffix replaces the tail, so
/// two long branches sharing a prefix cannot collapse onto the same domain.
const MAX_SLUG_LEN: usize = 30;
/// Prefix retained when a slug is truncated; the rest is `-` + 6 hex chars.
const TRUNCATED_SLUG_LEN: usize = 24;
/// Maximum length of a single DNS label (RFC 1035).
const MAX_DNS_LABEL: usize = 63;

pub const DEFAULT_LOCAL_DOMAIN_BASE: &str = "dev.solisoft.test";

/// `APP_ENV` written into a preview `.env`.
pub const PREVIEW_APP_ENV: &str = "preview";

/// Default template copied into a preview worktree.
///
/// The `.example` suffix is load-bearing, not decoration. `load_env_files`
/// layers `.env.{APP_ENV}` **over** `.env` with override, so a template named
/// `.env.preview` — which git would check out into every worktree — would
/// silently win over the generated database name and point the preview back at
/// whatever the template says. Keeping the template outside the `.env.{APP_ENV}`
/// namespace makes that impossible. See [`guard_env_overlay`].
pub const DEFAULT_ENV_TEMPLATE: &str = ".env.preview.example";

/// The `[preview]` section of an app's `deploy.toml`.
#[derive(Debug, Clone)]
pub struct PreviewConfig {
    /// Domain suffix for remote environments, e.g. `dev.solisoft.net`.
    /// `None` means `--server` is unusable until it is configured.
    pub domain_base: Option<String>,
    /// Domain suffix for local environments. Defaults to `dev.solisoft.test`,
    /// which the proxy skips for ACME (`.test` is never publicly resolvable).
    pub local_domain_base: String,
    /// Where the proxy looks for site directories. `None` falls back to
    /// `$SOLI_SITES_DIR`, then to `../proxy/sites` relative to the app.
    pub sites_dir: Option<PathBuf>,
    /// Where worktrees are checked out. Defaults to `~/.soli/previews`.
    pub worktrees_dir: Option<PathBuf>,
    /// Template copied to the worktree's `.env` before per-environment values
    /// are layered on top. Never the app's own `.env` — see [`env_template`].
    pub env_template: String,
    /// Shell command run in the worktree after checkout (asset build).
    pub build_command: Option<String>,
    /// Whether `soli db:seed` runs after migrations.
    pub seed: bool,
}

impl Default for PreviewConfig {
    fn default() -> Self {
        Self {
            domain_base: None,
            local_domain_base: DEFAULT_LOCAL_DOMAIN_BASE.to_string(),
            sites_dir: None,
            worktrees_dir: None,
            env_template: DEFAULT_ENV_TEMPLATE.to_string(),
            build_command: None,
            seed: true,
        }
    }
}

impl PreviewConfig {
    /// Read the `[preview]` section from an app's `deploy.toml`. A missing file
    /// or missing section yields defaults rather than an error: previews should
    /// work on an app that has never been deployed.
    pub fn load(folder: &Path) -> Result<Self, String> {
        let path = folder.join("deploy.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        Ok(Self::parse(&content))
    }

    /// Parse just the `[preview]` table. Deliberately independent of
    /// `deploy_config::parse_deploy_toml`, whose top-level `else` arm matches keys in
    /// any non-`[[servers]]` section — adding keys there would make `[preview]`
    /// entries leak into the deploy config.
    pub fn parse(content: &str) -> Self {
        let mut config = Self::default();
        let mut in_preview = false;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                in_preview = line == "[preview]";
                continue;
            }
            if !in_preview {
                continue;
            }
            let Some((key, raw_value)) = line.split_once('=') else {
                continue;
            };
            let value = raw_value.trim().trim_matches('"').trim_matches('\'');
            if value.is_empty() {
                continue;
            }
            match key.trim() {
                "domain_base" => config.domain_base = Some(value.to_string()),
                "local_domain_base" => config.local_domain_base = value.to_string(),
                "sites_dir" => config.sites_dir = Some(expand_tilde(value)),
                "worktrees_dir" => config.worktrees_dir = Some(expand_tilde(value)),
                "env_template" => config.env_template = value.to_string(),
                "build_command" => config.build_command = Some(value.to_string()),
                "seed" => config.seed = value != "false",
                _ => {}
            }
        }
        config
    }

    /// Worktree root, honouring the config then `~/.soli/previews`.
    pub fn worktrees_root(&self) -> PathBuf {
        self.worktrees_dir
            .clone()
            .unwrap_or_else(|| home_dir().join(".soli").join("previews"))
    }

    /// Site directory the proxy watches. Config wins, then `SOLI_SITES_DIR`,
    /// then a sibling `proxy/sites` checkout next to the app.
    pub fn sites_root(&self, app_folder: &Path) -> PathBuf {
        if let Some(dir) = &self.sites_dir {
            return dir.clone();
        }
        if let Ok(dir) = std::env::var("SOLI_SITES_DIR") {
            if !dir.is_empty() {
                return expand_tilde(&dir);
            }
        }
        app_folder.join("..").join("proxy").join("sites")
    }
}

fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_default()
}

fn expand_tilde(value: &str) -> PathBuf {
    match value.strip_prefix("~/") {
        Some(rest) => home_dir().join(rest),
        None => PathBuf::from(value),
    }
}

/// Reduce a git branch name to a DNS-label-safe slug.
///
/// Lowercases, replaces every character outside `[a-z0-9-]` with `-`, collapses
/// runs, and trims. Slugs longer than [`MAX_SLUG_LEN`] keep a
/// [`TRUNCATED_SLUG_LEN`] prefix plus a hash of the *full* branch name, so
/// `task/very-long-thing-a` and `task/very-long-thing-b` stay distinct.
pub fn branch_slug(branch: &str) -> String {
    let mut slug = String::with_capacity(branch.len());
    let mut last_was_dash = false;
    for ch in branch.chars() {
        let mapped = match ch {
            'a'..='z' | '0'..='9' => ch,
            'A'..='Z' => ch.to_ascii_lowercase(),
            _ => '-',
        };
        if mapped == '-' {
            // Collapse runs rather than emitting `--`, which is the separator
            // between the slug and the app name in a preview domain.
            if !last_was_dash {
                slug.push('-');
            }
            last_was_dash = true;
        } else {
            slug.push(mapped);
            last_was_dash = false;
        }
    }
    let slug = slug.trim_matches('-').to_string();

    if slug.is_empty() {
        // A branch of only separators still needs a stable, unique identity.
        return format!("env-{}", short_hash(branch));
    }
    if slug.len() <= MAX_SLUG_LEN {
        return slug;
    }
    let head: String = slug.chars().take(TRUNCATED_SLUG_LEN).collect();
    format!("{}-{}", head.trim_end_matches('-'), short_hash(branch))
}

/// First 6 hex characters of the branch's SHA-256 — enough to separate the
/// handful of branches an app has open at once, short enough to stay readable.
fn short_hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    digest
        .iter()
        .take(3)
        .map(|byte| format!("{:02x}", byte))
        .collect()
}

/// Build the preview domain: `<slug>--<app>.<base>`.
///
/// If `<slug>--<app>` would exceed a DNS label, the slug is shortened rather
/// than the app name, so the domain still says which app it belongs to.
pub fn preview_domain(slug: &str, app: &str, base: &str) -> String {
    let app = branch_slug(app);
    let separator_len = 2;
    let mut slug = slug.to_string();
    let budget = MAX_DNS_LABEL.saturating_sub(app.len() + separator_len);
    if slug.len() > budget {
        slug = slug.chars().take(budget).collect::<String>();
        slug = slug.trim_end_matches('-').to_string();
    }
    format!("{}--{}.{}", slug, app, base)
}

/// Database name for a preview environment.
///
/// Prefixed `p_` so every preview database is greppable in a `GET
/// /_api/databases` listing and safe to bulk-reap.
pub fn preview_database(app: &str, slug: &str) -> String {
    let sanitize = |value: &str| {
        value
            .chars()
            .map(|ch| match ch {
                'a'..='z' | '0'..='9' => ch,
                'A'..='Z' => ch.to_ascii_lowercase(),
                _ => '_',
            })
            .collect::<String>()
    };
    format!("p_{}_{}", sanitize(app), sanitize(slug))
}

/// Everything derived from `(app, branch)` before any side effect happens.
///
/// Resolving first and acting second means `up` and `down` agree on names even
/// when `down` runs long after the worktree was created.
#[derive(Debug, Clone)]
pub struct PreviewEnv {
    pub app: String,
    pub branch: String,
    pub slug: String,
    /// Both the public hostname and the site directory name — the proxy derives
    /// one from the other.
    pub domain: String,
    pub database: String,
    pub worktree: PathBuf,
    pub site_link: PathBuf,
}

impl PreviewEnv {
    pub fn url(&self) -> String {
        format!("https://{}", self.domain)
    }
}

/// Derive the full identity of a preview environment without touching disk.
pub fn resolve(
    folder: &Path,
    branch: &str,
    config: &PreviewConfig,
    remote: bool,
) -> Result<PreviewEnv, String> {
    let app = app_name(folder)?;
    let slug = branch_slug(branch);

    let base = if remote {
        config.domain_base.clone().ok_or_else(|| {
            "no `domain_base` in the [preview] section of deploy.toml — required for --server"
                .to_string()
        })?
    } else {
        config.local_domain_base.clone()
    };

    let domain = preview_domain(&slug, &app, &base);
    let worktree = config.worktrees_root().join(&app).join(&slug);
    let site_link = config.sites_root(folder).join(&domain);

    Ok(PreviewEnv {
        database: preview_database(&app, &slug),
        app,
        branch: branch.to_string(),
        slug,
        domain,
        worktree,
        site_link,
    })
}

/// App name from `soli.toml`, falling back to the directory name.
fn app_name(folder: &Path) -> Result<String, String> {
    let manifest = folder.join("soli.toml");
    if manifest.exists() {
        if let Ok(package) = super::package::Package::load(&manifest) {
            if !package.name.is_empty() {
                return Ok(package.name);
            }
        }
    }
    folder
        .canonicalize()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .ok_or_else(|| {
            format!(
                "cannot determine an app name: {} has no soli.toml and no usable directory name",
                folder.display()
            )
        })
}

/// The `.env` written into a preview worktree.
///
/// Built from a template plus per-environment overrides. The template is
/// deliberately **not** the app's own `.env`: a preview that inherits production
/// database credentials will migrate and seed straight into production, which is
/// the one failure in this feature that cannot be undone. `task-orchestrator`
/// copies only `.env.test` for the same reason.
pub fn render_env_file(
    folder: &Path,
    config: &PreviewConfig,
    env: &PreviewEnv,
) -> Result<String, String> {
    let template = load_env_template(folder, config)?;

    let overrides: Vec<(&str, String)> = vec![
        ("APP_ENV", PREVIEW_APP_ENV.to_string()),
        ("SOLIDB_DATABASE", env.database.clone()),
        // Sessions move to SoliDB so they land in this branch's database.
        // SoliKV has no namespaces and its session keys are globally prefixed,
        // so every preview sharing one SoliKV would share sessions.
        ("SOLI_SESSION_DRIVER", "solidb".to_string()),
        ("SOLI_SOLIDB_DATABASE", env.database.clone()),
        ("APP_BASE_URL", env.url()),
        ("APP_URL", env.url()),
        ("OAUTH_REDIRECT_BASE", env.url()),
    ];

    let overridden: Vec<&str> = overrides.iter().map(|(key, _)| *key).collect();
    let mut out = String::new();
    out.push_str("# Generated by `soli env` — do not edit.\n");
    out.push_str(&format!("# branch: {}\n", env.branch));
    out.push_str(&format!("# domain: {}\n\n", env.domain));

    for line in template.lines() {
        let key = line.split_once('=').map(|(k, _)| k.trim()).unwrap_or("");
        // Drop template values we are about to define, so the file has exactly
        // one binding per key regardless of what the template carried.
        if overridden.contains(&key) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }

    out.push_str("\n# --- preview environment ---\n");
    for (key, value) in &overrides {
        out.push_str(&format!("{}={}\n", key, value));
    }
    Ok(out)
}

fn load_env_template(folder: &Path, config: &PreviewConfig) -> Result<String, String> {
    let primary = folder.join(&config.env_template);
    if primary.exists() {
        return std::fs::read_to_string(&primary)
            .map_err(|e| format!("Failed to read {}: {}", primary.display(), e));
    }
    let fallback = folder.join(".env.test");
    if fallback.exists() {
        eprintln!(
            "note: {} not found, using .env.test as the preview template",
            config.env_template
        );
        return std::fs::read_to_string(&fallback)
            .map_err(|e| format!("Failed to read {}: {}", fallback.display(), e));
    }
    Err(format!(
        "no preview env template: create {} (or .env.test) in {}.\n\
         It must NOT be a copy of your production .env — a preview inheriting production \
         database credentials will migrate and seed into production.",
        config.env_template,
        folder.display()
    ))
}

/// Refuse to run if the worktree contains a `.env.{PREVIEW_APP_ENV}` file.
///
/// Because the generated `.env` sets `APP_ENV=preview`, such a file is loaded
/// afterwards **with override**, so any `SOLIDB_DATABASE` in it would beat the
/// generated one and aim the preview at another database. Failing loudly beats
/// a preview that silently migrates the wrong target.
pub fn guard_env_overlay(worktree: &Path) -> Result<(), String> {
    let overlay = worktree.join(format!(".env.{}", PREVIEW_APP_ENV));
    if overlay.exists() {
        return Err(format!(
            "{} exists in the worktree.\n\
             The preview .env sets APP_ENV={}, so that file would be loaded afterwards with \
             override and could redirect the preview to another database. Rename it (the preview \
             template is `{}`) and retry.",
            overlay.display(),
            PREVIEW_APP_ENV,
            DEFAULT_ENV_TEMPLATE
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// Read `KEY=value` pairs out of a rendered preview `.env`.
///
/// Used to recover the SoliDB coordinates at teardown without re-deriving them
/// from the app's current configuration, which may have moved on since `up`.
fn parse_env_pairs(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            (key.trim().to_string(), value.to_string())
        })
        .collect()
}

fn env_value(pairs: &[(String, String)], key: &str) -> Option<String> {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .filter(|v| !v.is_empty())
}

fn run(command: &mut std::process::Command, what: &str) -> Result<(), String> {
    let status = command
        .status()
        .map_err(|e| format!("failed to run {}: {}", what, e))?;
    if !status.success() {
        return Err(format!("{} failed with {}", what, status));
    }
    Ok(())
}

/// Path to the running `soli` binary, so child commands use the same build.
fn soli_binary() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("soli"))
}

/// Create the worktree, `.env`, database and site symlink for `env`.
pub fn up(
    folder: &Path,
    config: &PreviewConfig,
    env: &PreviewEnv,
    seed: bool,
) -> Result<(), String> {
    println!("→ preview {} ({})", env.domain, env.branch);

    create_worktree(folder, env)?;
    guard_env_overlay(&env.worktree)?;

    let rendered = render_env_file(folder, config, env)?;
    std::fs::write(env.worktree.join(".env"), &rendered)
        .map_err(|e| format!("Failed to write the preview .env: {}", e))?;
    println!("  .env      {}", env.worktree.join(".env").display());

    if let Some(build) = &config.build_command {
        println!("  build     {}", build);
        run(
            std::process::Command::new("sh")
                .arg("-c")
                .arg(build)
                .current_dir(&env.worktree),
            "the build command",
        )?;
    }

    migrate_and_seed(env, seed)?;
    link_site(env)?;

    println!("\n  {}", env.url());
    println!("  database  {}", env.database);
    println!(
        "\nThe proxy discovers the site within a second or two; `soli env list` shows when it is up."
    );
    Ok(())
}

fn create_worktree(folder: &Path, env: &PreviewEnv) -> Result<(), String> {
    if env.worktree.exists() {
        println!("  worktree  {} (reusing)", env.worktree.display());
        return Ok(());
    }
    if let Some(parent) = env.worktree.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }

    // `-B` so an existing local branch is reused rather than erroring, matching
    // `task-orchestrator`'s worktree handling.
    let mut command = std::process::Command::new("git");
    command
        .current_dir(folder)
        .arg("worktree")
        .arg("add")
        .arg("-B")
        .arg(&env.branch)
        .arg(&env.worktree);

    // Prefer the remote ref when it exists so a preview tracks what was pushed,
    // not a stale local branch.
    let remote_ref = format!("origin/{}", env.branch);
    if git_ref_exists(folder, &remote_ref) {
        command.arg(&remote_ref);
    } else {
        command.arg(&env.branch);
    }

    run(&mut command, "git worktree add")?;
    println!("  worktree  {}", env.worktree.display());
    Ok(())
}

fn git_ref_exists(folder: &Path, reference: &str) -> bool {
    std::process::Command::new("git")
        .current_dir(folder)
        .arg("rev-parse")
        .arg("--verify")
        .arg("--quiet")
        .arg(reference)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn migrate_and_seed(env: &PreviewEnv, seed: bool) -> Result<(), String> {
    let soli = soli_binary();

    // `SOLI_PROTECT_ENV` stops the child's own `.env` reload from replacing the
    // database we just chose — the guard the parallel test runner uses.
    println!("  migrate   {}", env.database);
    run(
        std::process::Command::new(&soli)
            .arg("db:migrate")
            .arg("up")
            .arg(&env.worktree)
            .env("SOLIDB_DATABASE", &env.database)
            .env("SOLI_PROTECT_ENV", "SOLIDB_DATABASE"),
        "soli db:migrate up",
    )?;

    if seed {
        println!("  seed      {}", env.database);
        run(
            std::process::Command::new(&soli)
                .arg("db:seed")
                .arg(&env.worktree)
                .env("SOLIDB_DATABASE", &env.database)
                .env("SOLI_PROTECT_ENV", "SOLIDB_DATABASE"),
            "soli db:seed",
        )?;
    }
    Ok(())
}

fn link_site(env: &PreviewEnv) -> Result<(), String> {
    let sites_dir = env
        .site_link
        .parent()
        .ok_or_else(|| "invalid sites directory".to_string())?;
    if !sites_dir.exists() {
        return Err(format!(
            "sites directory {} does not exist — set `sites_dir` in the [preview] section of \
             deploy.toml or export SOLI_SITES_DIR",
            sites_dir.display()
        ));
    }
    if env.site_link.exists() || std::fs::symlink_metadata(&env.site_link).is_ok() {
        println!("  site      {} (already linked)", env.site_link.display());
        return Ok(());
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&env.worktree, &env.site_link)
            .map_err(|e| format!("Failed to link {}: {}", env.site_link.display(), e))?;
        println!("  site      {}", env.site_link.display());
        Ok(())
    }
    #[cfg(not(unix))]
    Err("preview environments require symlink support (unix)".to_string())
}

/// Tear an environment down.
///
/// Order matters: the proxy must be told to stop the app **before** the symlink
/// disappears. `discover_apps_inner` only drops vanished apps from its map, so
/// unlinking first leaves an orphan process holding its allocated ports.
pub fn down(
    config: &PreviewConfig,
    env: &PreviewEnv,
    proxy_url: Option<&str>,
    keep_data: bool,
) -> Result<(), String> {
    println!("→ tearing down {}", env.domain);
    let mut problems: Vec<String> = Vec::new();

    match proxy_url {
        Some(url) => match stop_app(url, &env.domain) {
            Ok(()) => println!("  stopped   {}", env.domain),
            Err(e) => problems.push(format!("could not stop the app via the proxy: {}", e)),
        },
        None => problems.push(
            "no proxy URL configured, so the app process was not stopped — it will keep running \
             and holding its ports. Set `proxy_url` on a [[servers]] entry or pass --proxy-url."
                .to_string(),
        ),
    }

    // Read the DB coordinates from the worktree's own .env before deleting it.
    let db_target = read_db_target(env);

    if std::fs::symlink_metadata(&env.site_link).is_ok() {
        std::fs::remove_file(&env.site_link)
            .map_err(|e| format!("Failed to unlink {}: {}", env.site_link.display(), e))?;
        println!("  unlinked  {}", env.site_link.display());
    }

    if env.worktree.exists() {
        let removed = std::process::Command::new("git")
            .current_dir(&env.worktree)
            .arg("worktree")
            .arg("remove")
            .arg("--force")
            .arg(&env.worktree)
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if removed {
            println!("  worktree  removed");
        } else {
            // A worktree whose branch was deleted upstream can refuse removal;
            // the directory still has to go or the next `up` reuses stale code.
            std::fs::remove_dir_all(&env.worktree)
                .map_err(|e| format!("Failed to remove {}: {}", env.worktree.display(), e))?;
            println!("  worktree  removed (forced)");
        }
    }

    if keep_data {
        println!("  database  {} (kept)", env.database);
    } else if let Some((host, auth)) = db_target {
        match drop_database(&host, auth.as_ref(), &env.database) {
            Ok(()) => println!("  database  {} dropped", env.database),
            Err(e) => problems.push(format!("could not drop {}: {}", env.database, e)),
        }
    } else {
        problems.push(format!(
            "no SOLIDB_HOST recorded for this environment, so {} was left in place",
            env.database
        ));
    }

    let _ = config;
    if problems.is_empty() {
        println!("\n  done");
        Ok(())
    } else {
        // Report everything rather than stopping at the first problem: a partial
        // teardown still needs the operator to know exactly what survived.
        Err(problems.join("\n  - "))
    }
}

/// Recover `(host, basic auth)` from the environment's generated `.env`.
fn read_db_target(env: &PreviewEnv) -> Option<(String, Option<(String, String)>)> {
    let content = std::fs::read_to_string(env.worktree.join(".env")).ok()?;
    let pairs = parse_env_pairs(&content);
    let host = env_value(&pairs, "SOLIDB_HOST")?;
    let auth = env_value(&pairs, "SOLIDB_USERNAME").map(|user| {
        (
            user,
            env_value(&pairs, "SOLIDB_PASSWORD").unwrap_or_default(),
        )
    });
    Some((host, auth))
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .build()
        .map_err(|e| format!("HTTP client failed: {}", e))
}

fn stop_app(proxy_url: &str, app: &str) -> Result<(), String> {
    let url = format!(
        "{}/api/v1/apps/{}/stop",
        proxy_url.trim_end_matches('/'),
        app
    );
    let mut request = http_client()?.post(&url);

    // The admin API accepts unauthenticated requests on a loopback bind, which
    // is the usual local setup, so a missing key is not an error here — let the
    // proxy answer 401 if it does require one.
    if let Some(key) = std::env::var("SOLI_DEPLOY_API_KEY")
        .ok()
        .filter(|key| !key.is_empty())
    {
        request = request.header("X-Api-Key", key);
    }

    let response = request
        .send()
        .map_err(|e| format!("request failed: {}", e))?;

    let status = response.status();
    // The app may already be gone; that is a successful teardown, not an error.
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    let body = response.text().unwrap_or_default();

    // The admin API answers 500 — not 404 — for an app it does not know, so a
    // teardown of an already-removed environment would look like a failure.
    // Confirm absence by listing instead of matching on the error text.
    if let Ok(false) = app_is_known(proxy_url, app) {
        return Ok(());
    }
    Err(format!("{}: {}", status, body))
}

/// Whether the proxy currently manages an app with this name/domain.
fn app_is_known(proxy_url: &str, app: &str) -> Result<bool, String> {
    let url = format!("{}/api/v1/apps", proxy_url.trim_end_matches('/'));
    let mut request = http_client()?.get(&url);
    if let Some(key) = std::env::var("SOLI_DEPLOY_API_KEY")
        .ok()
        .filter(|key| !key.is_empty())
    {
        request = request.header("X-Api-Key", key);
    }

    let response = request
        .send()
        .map_err(|e| format!("request failed: {}", e))?;
    if !response.status().is_success() {
        return Err(format!("apps listing returned {}", response.status()));
    }
    let body: serde_json::Value = response
        .json()
        .map_err(|e| format!("failed to parse the apps listing: {}", e))?;

    let apps = body
        .get("data")
        .and_then(|data| data.as_array())
        .ok_or_else(|| "unexpected apps listing shape".to_string())?;

    Ok(apps.iter().any(|entry| {
        let config = entry.get("config");
        let matches = |field: &str| {
            config
                .and_then(|config| config.get(field))
                .and_then(|value| value.as_str())
                == Some(app)
        };
        matches("name") || matches("domain")
    }))
}

fn drop_database(
    host: &str,
    auth: Option<&(String, String)>,
    database: &str,
) -> Result<(), String> {
    let url = format!("{}/_api/database/{}", host.trim_end_matches('/'), database);
    let mut request = http_client()?.delete(&url);
    if let Some((user, password)) = auth {
        request = request.basic_auth(user, Some(password));
    }
    let response = request
        .send()
        .map_err(|e| format!("request failed: {}", e))?;

    let status = response.status();
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    Err(format!(
        "{}: {}",
        status,
        response.text().unwrap_or_default()
    ))
}

/// Preview environments discovered on disk, with their live state if the proxy
/// can be reached.
pub fn list(folder: &Path, config: &PreviewConfig, remote: bool) -> Result<Vec<String>, String> {
    let app = app_name(folder)?;
    let base = if remote {
        config
            .domain_base
            .clone()
            .ok_or_else(|| "no `domain_base` configured".to_string())?
    } else {
        config.local_domain_base.clone()
    };
    let suffix = format!("--{}.{}", branch_slug(&app), base);

    let sites_dir = config.sites_root(folder);
    let entries = std::fs::read_dir(&sites_dir)
        .map_err(|e| format!("Failed to read {}: {}", sites_dir.display(), e))?;

    let mut domains: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(&suffix))
        .collect();
    domains.sort();
    Ok(domains)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_lowercases_and_replaces_illegal_characters() {
        assert_eq!(branch_slug("Feat/Live_Cursors"), "feat-live-cursors");
        assert_eq!(branch_slug("fix/#123"), "fix-123");
    }

    #[test]
    fn slug_collapses_runs_and_trims() {
        // A `--` inside a slug would be ambiguous with the slug/app separator.
        assert_eq!(branch_slug("feat//weird__name"), "feat-weird-name");
        assert_eq!(
            branch_slug("/leading/and/trailing/"),
            "leading-and-trailing"
        );
        assert!(!branch_slug("a///b").contains("--"));
    }

    #[test]
    fn slug_truncates_long_branches_but_keeps_them_distinct() {
        let a = "task/why-no-dependency-matters-and-why-we-chose-soli-lang-approach";
        let b = "task/why-no-dependency-matters-and-why-we-chose-something-else";
        let slug_a = branch_slug(a);
        let slug_b = branch_slug(b);

        assert!(slug_a.len() <= MAX_SLUG_LEN + 1, "got {}", slug_a);
        // Shared 24-char prefix — only the hash keeps these apart.
        assert_ne!(slug_a, slug_b);
    }

    #[test]
    fn slug_handles_a_branch_of_only_separators() {
        let slug = branch_slug("///");
        assert!(!slug.is_empty());
        assert_ne!(slug, branch_slug("___"));
    }

    #[test]
    fn slug_is_stable() {
        let branch = "task/some-very-long-branch-name-that-needs-truncating-for-dns";
        assert_eq!(branch_slug(branch), branch_slug(branch));
    }

    #[test]
    fn domain_is_flat_and_fits_one_dns_label() {
        assert_eq!(
            preview_domain("feat-cart", "demo", "dev.solisoft.test"),
            "feat-cart--demo.dev.solisoft.test"
        );

        let long_slug = "a".repeat(80);
        let domain = preview_domain(&long_slug, "demo", "dev.solisoft.test");
        let label = domain.split('.').next().unwrap();
        assert!(label.len() <= MAX_DNS_LABEL, "label was {}", label.len());
        assert!(label.ends_with("--demo"));
    }

    #[test]
    fn domain_sanitises_the_app_name() {
        assert_eq!(
            preview_domain("main", "My_App", "dev.test"),
            "main--my-app.dev.test"
        );
    }

    #[test]
    fn database_name_is_prefixed_and_underscored() {
        assert_eq!(preview_database("demo", "feat-cart"), "p_demo_feat_cart");
        // Dots and dashes are legal in domains but not wanted in a DB name.
        assert_eq!(preview_database("my-app", "fix.1"), "p_my_app_fix_1");
    }

    #[test]
    fn config_parses_only_the_preview_section() {
        let toml = r#"
mode = "git"
git_branch = "main"

[[servers]]
name = "prod-1"
folder = "/srv/app"

[preview]
domain_base = "dev.solisoft.net"
env_template = ".env.staging"
seed = false
build_command = "npm ci && npm run build:css"
"#;
        let config = PreviewConfig::parse(toml);
        assert_eq!(config.domain_base.as_deref(), Some("dev.solisoft.net"));
        assert_eq!(config.env_template, ".env.staging");
        assert!(!config.seed);
        assert_eq!(
            config.build_command.as_deref(),
            Some("npm ci && npm run build:css")
        );
        // `git_branch` sits outside [preview] and must not leak in.
        assert_eq!(config.local_domain_base, DEFAULT_LOCAL_DOMAIN_BASE);
    }

    #[test]
    fn config_defaults_when_section_absent() {
        let config = PreviewConfig::parse("mode = \"git\"\n");
        assert!(config.domain_base.is_none());
        assert!(config.seed);
        assert_eq!(config.env_template, DEFAULT_ENV_TEMPLATE);
    }

    fn sample_env(database: &str) -> PreviewEnv {
        PreviewEnv {
            app: "demo".to_string(),
            branch: "feat/cart".to_string(),
            slug: "feat-cart".to_string(),
            domain: "feat-cart--demo.dev.solisoft.test".to_string(),
            database: database.to_string(),
            worktree: PathBuf::from("/tmp/wt"),
            site_link: PathBuf::from("/tmp/sites/feat-cart--demo.dev.solisoft.test"),
        }
    }

    fn render(template: &str, env: &PreviewEnv) -> String {
        let dir = std::env::temp_dir().join(format!("soli-preview-test-{}", short_hash(template)));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(DEFAULT_ENV_TEMPLATE), template).unwrap();
        let config = PreviewConfig::default();
        let rendered = render_env_file(&dir, &config, env).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        rendered
    }

    #[test]
    fn env_file_overrides_the_template_database() {
        // The whole point: a template that names production must not survive.
        let template = "SOLIDB_HOST=http://localhost:6745\nSOLIDB_DATABASE=production\n";
        let rendered = render(template, &sample_env("p_demo_feat_cart"));

        assert!(rendered.contains("SOLIDB_DATABASE=p_demo_feat_cart"));
        assert!(
            !rendered.contains("SOLIDB_DATABASE=production"),
            "production database leaked into the preview env:\n{}",
            rendered
        );
        // Non-overridden template keys are preserved.
        assert!(rendered.contains("SOLIDB_HOST=http://localhost:6745"));
    }

    #[test]
    fn env_file_binds_each_overridden_key_exactly_once() {
        let template =
            "SOLIDB_DATABASE=a\nAPP_BASE_URL=https://prod.example.com\nAPP_ENV=production\n";
        let rendered = render(template, &sample_env("p_demo_feat_cart"));

        for key in ["SOLIDB_DATABASE", "APP_BASE_URL", "APP_ENV"] {
            let count = rendered
                .lines()
                .filter(|line| line.split_once('=').map(|(k, _)| k.trim()) == Some(key))
                .count();
            assert_eq!(count, 1, "{} bound {} times in:\n{}", key, count, rendered);
        }
    }

    #[test]
    fn env_file_points_sessions_at_solidb() {
        // SoliKV has no namespaces, so preview sessions must not live there.
        let rendered = render("", &sample_env("p_demo_feat_cart"));
        assert!(rendered.contains("SOLI_SESSION_DRIVER=solidb"));
        assert!(rendered.contains("SOLI_SOLIDB_DATABASE=p_demo_feat_cart"));
    }

    #[test]
    fn env_template_missing_is_a_hard_error() {
        let dir = std::env::temp_dir().join("soli-preview-test-no-template");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::remove_file(dir.join(DEFAULT_ENV_TEMPLATE)).ok();
        std::fs::remove_file(dir.join(".env.test")).ok();
        // Falling back to the app's own `.env` here would be the one
        // unrecoverable bug in this feature, so absence must fail loudly.
        std::fs::write(dir.join(".env"), "SOLIDB_DATABASE=production\n").unwrap();

        let err = render_env_file(&dir, &PreviewConfig::default(), &sample_env("p_x")).unwrap_err();
        assert!(err.contains(".env.preview"), "unhelpful error: {}", err);
        std::fs::remove_dir_all(&dir).ok();
    }
}
