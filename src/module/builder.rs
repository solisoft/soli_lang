//! Reproducible build pipeline: source → artifact.
//!
//! Turns a git ref or a working directory into a `.soli` bundle, running the
//! stages a Soli app needs on the way (dependency install, asset build,
//! bundling). This is the piece a PaaS runs on every push; `soli build` alone
//! only does the last stage.
//!
//! # This runs untrusted code
//!
//! `npm ci` executes arbitrary `postinstall` scripts, and a build script can do
//! anything the process can. Two consequences are handled here:
//!
//! - **The environment is scrubbed.** A build inherits only what it needs to
//!   run (`PATH`, `HOME`, `LANG`, `TZ`), never the database credentials, proxy
//!   admin key or provider tokens that happen to be exported in the operator's
//!   shell. See [`build_env`].
//! - **Every stage is bounded.** A build that hangs must fail, not occupy a
//!   worker forever.
//!
//! Process-level isolation — a container with no route to the database, a disk
//! quota — is the caller's job; this module cannot impose it on itself. What it
//! can do is refuse to be the thing that hands secrets to a build script.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Default ceiling for a single stage.
const DEFAULT_STAGE_TIMEOUT: Duration = Duration::from_secs(600);

/// Environment variables a build is allowed to inherit.
///
/// An allowlist rather than a denylist: a new secret added to the operator's
/// environment must not silently become visible to every tenant's build script.
const INHERITED_ENV: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL", "TZ"];

/// Sink for per-stage progress lines, so a caller can stream a build log.
type LogSink = Box<dyn Fn(&str) + Send + Sync>;

/// Where the source comes from.
#[derive(Debug, Clone)]
pub enum BuildSource {
    /// Clone `url` and check out `reference` (branch, tag or SHA).
    Git { url: String, reference: String },
    /// Build an existing directory in place — the CLI path, which skips the
    /// clone stage but is otherwise identical.
    Local(PathBuf),
}

/// What a project needs built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectKind {
    /// MVC app: `app/controllers/` or `config/routes.sl`.
    SoliApp,
    /// A directory served as files; still bundleable.
    Static,
}

/// One stage of a build, for the log the dashboard streams.
#[derive(Debug, Clone)]
pub struct BuildStep {
    pub name: String,
    pub duration: Duration,
    pub skipped: bool,
}

#[derive(Debug, Clone)]
pub struct BuildOutcome {
    pub artifact: PathBuf,
    pub kind: ProjectKind,
    /// Hash of the inputs that decide whether an identical build can be reused.
    pub cache_key: String,
    pub steps: Vec<BuildStep>,
}

pub struct Builder {
    workdir: PathBuf,
    timeout: Duration,
    soli_binary: PathBuf,
    /// Emitted per stage so a caller can stream progress.
    on_log: Option<LogSink>,
}

impl Builder {
    pub fn new(workdir: impl Into<PathBuf>) -> Self {
        Self {
            workdir: workdir.into(),
            timeout: DEFAULT_STAGE_TIMEOUT,
            soli_binary: default_soli_binary(),
            on_log: None,
        }
    }

    /// Use a specific `soli` for the bundle stage.
    ///
    /// A build service pinning the compiler version per deployment needs this;
    /// so does any caller that is not itself the `soli` binary.
    pub fn with_soli_binary(mut self, path: impl Into<PathBuf>) -> Self {
        self.soli_binary = path.into();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_logger(mut self, logger: impl Fn(&str) + Send + Sync + 'static) -> Self {
        self.on_log = Some(Box::new(logger));
        self
    }

    fn log(&self, message: &str) {
        match &self.on_log {
            Some(logger) => logger(message),
            None => println!("{}", message),
        }
    }

    /// Run the whole pipeline.
    pub fn build(&self, source: &BuildSource) -> Result<BuildOutcome, String> {
        let mut steps = Vec::new();

        let source_dir = match source {
            BuildSource::Git { url, reference } => {
                let dir = self.workdir.join("src");
                self.timed(&mut steps, "clone", false, || {
                    self.clone_at(url, reference, &dir)
                })?;
                dir
            }
            BuildSource::Local(path) => path.clone(),
        };

        if !source_dir.join("soli.toml").exists() {
            return Err(format!(
                "{} is not a Soli project: no soli.toml",
                source_dir.display()
            ));
        }

        let kind = detect_kind(&source_dir);
        let cache_key = cache_key(&source_dir);
        self.log(&format!(
            "detected {:?}, cache key {}",
            kind,
            &cache_key[..12]
        ));

        let has_node = source_dir.join("package.json").exists();
        self.timed(&mut steps, "dependencies", !has_node, || {
            self.install_dependencies(&source_dir)
        })?;
        self.timed(&mut steps, "assets", !has_node, || {
            self.build_assets(&source_dir)
        })?;

        let artifact = self.workdir.join("artifact.soli");
        self.timed(&mut steps, "bundle", false, || {
            self.bundle(&source_dir, &artifact)
        })?;

        Ok(BuildOutcome {
            artifact,
            kind,
            cache_key,
            steps,
        })
    }

    fn timed<F>(
        &self,
        steps: &mut Vec<BuildStep>,
        name: &str,
        skip: bool,
        run: F,
    ) -> Result<(), String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let started = Instant::now();
        if skip {
            self.log(&format!("  {} — skipped", name));
            steps.push(BuildStep {
                name: name.to_string(),
                duration: Duration::ZERO,
                skipped: true,
            });
            return Ok(());
        }
        self.log(&format!("  {} …", name));
        run()?;
        let duration = started.elapsed();
        self.log(&format!("  {} ok ({:.1}s)", name, duration.as_secs_f64()));
        steps.push(BuildStep {
            name: name.to_string(),
            duration,
            skipped: false,
        });
        Ok(())
    }

    /// Clone and check out an exact ref.
    ///
    /// Fetches the single ref rather than the whole history: a preview build
    /// wants one commit, not every branch the repository has ever had.
    fn clone_at(&self, url: &str, reference: &str, into: &Path) -> Result<(), String> {
        if let Some(parent) = into.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        self.run(
            Command::new("git")
                .arg("clone")
                .arg("--depth")
                .arg("1")
                .arg("--branch")
                .arg(reference)
                .arg(url)
                .arg(into),
            "git clone",
        )
        .or_else(|_| {
            // `--branch` only accepts a branch or tag. A raw SHA needs an
            // explicit fetch, so fall back rather than refusing to build a
            // commit — which is exactly what a deployment pins.
            std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
            self.run(
                Command::new("git").arg("init").arg("-q").current_dir(into),
                "git init",
            )?;
            self.run(
                Command::new("git")
                    .args(["remote", "add", "origin", url])
                    .current_dir(into),
                "git remote add",
            )?;
            self.run(
                Command::new("git")
                    .args(["fetch", "--depth", "1", "origin", reference])
                    .current_dir(into),
                "git fetch",
            )?;
            self.run(
                Command::new("git")
                    .args(["checkout", "FETCH_HEAD"])
                    .current_dir(into),
                "git checkout",
            )
        })
    }

    fn install_dependencies(&self, dir: &Path) -> Result<(), String> {
        // `npm ci` needs a lockfile and is the reproducible form; fall back to
        // `install` only when there is none, and say so.
        let has_lock = dir.join("package-lock.json").exists();
        if !has_lock {
            self.log("    no package-lock.json — falling back to `npm install`, build is not reproducible");
        }
        let subcommand = if has_lock { "ci" } else { "install" };
        self.run(
            Command::new("npm").arg(subcommand).current_dir(dir),
            &format!("npm {}", subcommand),
        )
    }

    fn build_assets(&self, dir: &Path) -> Result<(), String> {
        if !package_has_build_script(dir) {
            self.log("    package.json has no `build` script — nothing to compile");
            return Ok(());
        }
        self.run(
            Command::new("npm").args(["run", "build"]).current_dir(dir),
            "npm run build",
        )
    }

    fn bundle(&self, dir: &Path, output: &Path) -> Result<(), String> {
        self.run(
            Command::new(&self.soli_binary)
                .arg("build")
                .arg(dir)
                .arg("-o")
                .arg(output),
            "soli build",
        )?;
        // `soli build` reports success on stdout; make the postcondition
        // explicit rather than trusting the exit code, because a build service
        // hands this path straight to the artifact store.
        if !output.exists() {
            return Err(format!(
                "soli build reported success but wrote no artifact at {}",
                output.display()
            ));
        }
        Ok(())
    }

    /// Run a command with a scrubbed environment and a deadline.
    fn run(&self, command: &mut Command, what: &str) -> Result<(), String> {
        command.env_clear();
        for (key, value) in build_env() {
            command.env(key, value);
        }

        let mut child = command
            .spawn()
            .map_err(|e| format!("{} failed to start: {}", what, e))?;

        let deadline = Instant::now() + self.timeout;
        loop {
            match child.try_wait().map_err(|e| e.to_string())? {
                Some(status) if status.success() => return Ok(()),
                Some(status) => return Err(format!("{} failed with {}", what, status)),
                None => {
                    if Instant::now() >= deadline {
                        // A hung build must not hold a worker forever.
                        let _ = child.kill();
                        return Err(format!(
                            "{} exceeded the {}s limit and was killed",
                            what,
                            self.timeout.as_secs()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }
}

/// Which `soli` to shell out to for the bundle stage.
///
/// `current_exe()` is right only when the builder is running *inside* the soli
/// binary. Used unconditionally it makes a build service invoke itself, which
/// fails in confusing ways, so fall back to `soli` on `PATH` whenever the
/// running executable is not recognisably soli.
fn default_soli_binary() -> PathBuf {
    let Ok(exe) = std::env::current_exe() else {
        return PathBuf::from("soli");
    };
    let is_soli = exe
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| stem == "soli" || stem.starts_with("soli-"));
    if is_soli {
        exe
    } else {
        PathBuf::from("soli")
    }
}

/// The environment a build stage gets: an allowlist, never the caller's.
///
/// Returning owned pairs rather than mutating in place keeps this testable —
/// the property that matters is *what is absent*.
pub fn build_env() -> Vec<(String, String)> {
    INHERITED_ENV
        .iter()
        .filter_map(|key| {
            std::env::var(key)
                .ok()
                .map(|value| (key.to_string(), value))
        })
        // A build has no business talking to the database, so do not even give
        // it the chance: CI=true also makes npm quieter and non-interactive.
        .chain(std::iter::once(("CI".to_string(), "true".to_string())))
        .collect()
}

/// MVC app or plain directory, by the same rule `soli serve` uses.
pub fn detect_kind(dir: &Path) -> ProjectKind {
    if dir.join("app").join("controllers").exists() || dir.join("config").join("routes.sl").exists()
    {
        ProjectKind::SoliApp
    } else {
        ProjectKind::Static
    }
}

fn package_has_build_script(dir: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(dir.join("package.json")) else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) else {
        return false;
    };
    parsed
        .get("scripts")
        .and_then(|scripts| scripts.get("build"))
        .is_some()
}

/// Identify a build by its inputs, so an identical one can be reused.
///
/// Covers the lockfiles and the manifest — the things that change what the
/// build produces — plus the compiler version, because the same sources built
/// by a different `soli` are a different artifact. Application source is
/// deliberately **not** hashed: the caller already keys on the commit, and
/// walking the tree would make the cheap check expensive.
pub fn cache_key(dir: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
    for file in ["soli.toml", "package-lock.json", "soli.lock"] {
        hasher.update(file.as_bytes());
        if let Ok(bytes) = std::fs::read(dir.join(file)) {
            hasher.update(&bytes);
        }
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{:02x}", byte))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("soli-builder-test-{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn build_env_excludes_secrets_from_the_callers_environment() {
        // The failure this guards against: a build script reading the
        // operator's production database password out of its own environment.
        std::env::set_var("SOLIDB_PASSWORD", "super-secret");
        std::env::set_var("SOLI_DEPLOY_API_KEY", "admin-key");

        let env = build_env();
        let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();

        assert!(
            !keys.contains(&"SOLIDB_PASSWORD"),
            "secret leaked: {:?}",
            keys
        );
        assert!(
            !keys.contains(&"SOLI_DEPLOY_API_KEY"),
            "key leaked: {:?}",
            keys
        );
        assert!(keys.contains(&"CI"));

        std::env::remove_var("SOLIDB_PASSWORD");
        std::env::remove_var("SOLI_DEPLOY_API_KEY");
    }

    #[test]
    fn build_env_is_an_allowlist_not_a_denylist() {
        // A newly-invented secret must be excluded without anyone updating a
        // list of forbidden names.
        std::env::set_var("SOME_BRAND_NEW_TOKEN", "value");
        let env = build_env();
        assert!(!env.iter().any(|(k, _)| k == "SOME_BRAND_NEW_TOKEN"));
        std::env::remove_var("SOME_BRAND_NEW_TOKEN");
    }

    #[test]
    fn detects_an_mvc_app_and_a_plain_directory() {
        let dir = scratch("detect");
        assert_eq!(detect_kind(&dir), ProjectKind::Static);

        std::fs::create_dir_all(dir.join("app").join("controllers")).unwrap();
        assert_eq!(detect_kind(&dir), ProjectKind::SoliApp);

        let routes = scratch("detect-routes");
        std::fs::create_dir_all(routes.join("config")).unwrap();
        std::fs::write(routes.join("config").join("routes.sl"), "").unwrap();
        assert_eq!(detect_kind(&routes), ProjectKind::SoliApp);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&routes).ok();
    }

    #[test]
    fn cache_key_changes_with_the_lockfile_and_is_otherwise_stable() {
        let dir = scratch("cache-key");
        std::fs::write(dir.join("soli.toml"), "[package]\nname='x'\n").unwrap();

        let first = cache_key(&dir);
        assert_eq!(first, cache_key(&dir), "cache key is not deterministic");

        std::fs::write(dir.join("package-lock.json"), r#"{"a":1}"#).unwrap();
        let with_lock = cache_key(&dir);
        assert_ne!(first, with_lock, "adding a lockfile did not change the key");

        std::fs::write(dir.join("package-lock.json"), r#"{"a":2}"#).unwrap();
        assert_ne!(
            with_lock,
            cache_key(&dir),
            "changing a dependency did not change the key"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_script_detection_tolerates_broken_package_json() {
        let dir = scratch("pkg");
        std::fs::write(dir.join("package.json"), "{ not json").unwrap();
        assert!(!package_has_build_script(&dir));

        std::fs::write(dir.join("package.json"), r#"{"scripts":{"test":"x"}}"#).unwrap();
        assert!(!package_has_build_script(&dir));

        std::fs::write(
            dir.join("package.json"),
            r#"{"scripts":{"build":"tailwind"}}"#,
        )
        .unwrap();
        assert!(package_has_build_script(&dir));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_directory_without_a_manifest_is_refused() {
        let workdir = scratch("no-manifest-work");
        let source = scratch("no-manifest-src");
        let builder = Builder::new(&workdir).with_logger(|_| {});
        let err = builder
            .build(&BuildSource::Local(source.clone()))
            .unwrap_err();
        assert!(err.contains("soli.toml"), "unhelpful error: {}", err);

        std::fs::remove_dir_all(&workdir).ok();
        std::fs::remove_dir_all(&source).ok();
    }

    #[test]
    fn soli_binary_defaults_to_path_when_not_running_inside_soli() {
        // The test binary is not `soli`, so the default must not be itself —
        // otherwise a build service invokes its own executable with
        // `build <dir>` and fails in a way that looks like a compiler bug.
        let resolved = default_soli_binary();
        assert_eq!(
            resolved,
            PathBuf::from("soli"),
            "builder would have shelled out to {:?}",
            resolved
        );
    }

    #[test]
    fn a_hung_stage_is_killed_at_the_deadline() {
        let workdir = scratch("timeout");
        let builder = Builder::new(&workdir)
            .with_timeout(Duration::from_millis(300))
            .with_logger(|_| {});
        let err = builder
            .run(Command::new("sleep").arg("30"), "sleep")
            .unwrap_err();
        assert!(err.contains("limit"), "unexpected error: {}", err);

        std::fs::remove_dir_all(&workdir).ok();
    }
}
