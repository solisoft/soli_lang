//! Per-project Soli version pinning.
//!
//! A project declares the interpreter it runs on in its manifest:
//!
//! ```toml
//! [package]
//! soli_version = "=2.0.3"   # exact pin — this project runs on soli 2.0.3
//! soli_version = "2.0.3"    # unchanged: a minimum, the pre-existing form
//! ```
//!
//! When the pin names a version other than the running one, [`reexec_if_pinned`]
//! replaces this process with that version — fetching and verifying it first if
//! the cache is cold. It is the same idea as `.nvmrc` or a rustup toolchain
//! file, and it resolves the same way: by walking up from the current directory.
//!
//! The pin lives in `soli.toml` rather than a dotfile of its own for a concrete
//! reason: the manifest parser rejects unknown `[package]` keys outright, and
//! five commands exit on a manifest they cannot load. A *new* key would break
//! `soli add` on every older soli, whereas an unrecognised `=` prefix degrades
//! to "the floor is satisfied" and the old binary simply ignores the pin.

use std::path::{Path, PathBuf};

use super::args::VERSION;
use super::standalone::{ensure_cached_runtime, FetchPolicy};

/// Set on the child across the re-exec, carrying the version we switched to.
///
/// Any non-empty value stops the child from switching again. That is the
/// backstop the version-equality check cannot provide: a published tarball
/// whose compiled version disagrees with its release tag would otherwise
/// re-exec itself forever. Carrying the version rather than a bare `1` lets
/// `soli which` report the disagreement instead of hiding it.
const SHIM_GUARD_ENV: &str = "SOLI_PINNED_EXEC";

/// Operator escape hatch: skip the switch entirely.
const NO_PIN_ENV: &str = "SOLI_NO_PIN";

/// Replace this process with the version the current project pins.
///
/// Returns normally when there is nothing to do — no pin, the pin is already
/// the running version, the command is one that must not be redirected, or a
/// guard is set. Never returns when it switches.
///
/// Called from [`crate::cli::run`] immediately after
/// [`super::standalone::boot_if_standalone`]. The order matters: a standalone
/// artifact *is* an application, has no CLI, and must never be redirected by a
/// `soli.toml` that happens to sit in its deployment directory.
pub fn reexec_if_pinned() {
    let Some(plan) = resolve() else {
        return;
    };

    let exe = match ensure_toolchain(&plan.version, &plan.manifest) {
        Ok(path) => path,
        Err(message) => {
            eprintln!("\x1b[31mError:\x1b[0m {message}");
            std::process::exit(1);
        }
    };

    switch_to(&exe, &plan.version);
}

/// What the current directory's project asks for, when it asks for anything.
struct PinPlan {
    version: String,
    manifest: PathBuf,
}

/// Decide whether to switch, without touching the network or the cache.
fn resolve() -> Option<PinPlan> {
    if env_is_set(NO_PIN_ENV) || env_is_set(SHIM_GUARD_ENV) {
        return None;
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    if bypasses_pin(&args) {
        return None;
    }

    // `Package::find` walks by `PathBuf::pop`, which on a relative path pops
    // once and stops — `current_dir` is absolute, so the walk really walks.
    let cwd = std::env::current_dir().ok()?;
    let (version, manifest) = solilang::module::pinned_soli_version(&cwd)?;

    // The steady state: the pin names the version already running. Return
    // before any I/O, so the feature costs a manifest read and nothing more.
    if version == VERSION {
        return None;
    }

    Some(PinPlan { version, manifest })
}

fn env_is_set(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.trim().is_empty())
}

/// The subcommand token `parse_args` would dispatch on: the first argument that
/// is not a flag.
///
/// The shim runs before `parse_args`, so it re-derives this much by hand. It
/// only has to agree with `parse_args` about *which token is the subcommand*,
/// not about how any subcommand's own arguments are shaped.
fn leading_token(args: &[String]) -> Option<&str> {
    args.iter()
        .find(|a| !a.starts_with('-'))
        .map(|a| a.as_str())
}

/// Commands that must run as the soli the user actually invoked.
fn bypasses_pin(args: &[String]) -> bool {
    // `--version` and `--help` are probes. Scripts use them to ask what is
    // installed, and neither should ever trigger a download.
    if args
        .iter()
        .any(|a| matches!(a.as_str(), "--version" | "-v" | "--help" | "-h"))
    {
        return true;
    }

    match leading_token(args) {
        // `soli update` replaces the *installed* binary. Redirected into a
        // pinned toolchain it would overwrite a cache entry with a different
        // version, leaving a directory named 2.0.3 holding some other soli.
        Some("update") => true,
        // Creates a project; the target directory does not exist yet.
        Some("new") => true,
        // Release engineering, concerning no project.
        Some("update-keygen") | Some("sign-update") => true,
        // Reports the resolution rather than following it.
        Some("which") => true,
        _ => false,
    }
}

/// An executable soli of `version` for this host, fetching it when the cache is
/// cold. Every error names the manifest that asked for it.
fn ensure_toolchain(version: &str, manifest: &Path) -> Result<PathBuf, String> {
    let target = solilang::update::host_target().map_err(|e| {
        format!(
            "{} pins soli {version}, but there is no published soli for this platform ({e}).\n  \
             Set {NO_PIN_ENV}=1 to use the soli already installed.",
            manifest.display()
        )
    })?;

    let policy = FetchPolicy {
        // These bytes are about to be executed with the developer's
        // privileges — unlike a cross-target build, which only embeds them.
        strict_checksum: true,
        executable: true,
    };

    if !cached(&target, version) {
        // Never fetch silently. Entering an unfamiliar repository and running
        // `soli test` now runs an interpreter *that repository chose*; the risk
        // is not new in kind, but it is new in that the interpreter itself is
        // repo-selected, so say so.
        println!(
            "  Fetching soli {version} pinned by {} ...",
            manifest.display()
        );
    }

    ensure_cached_runtime(&target, version, policy).map_err(|e| {
        format!(
            "cannot use soli {version} pinned by {}.\n  {e}\n  \
             Set {NO_PIN_ENV}=1 to run this project on soli {VERSION} anyway.",
            manifest.display()
        )
    })
}

fn cached(target: &str, version: &str) -> bool {
    super::standalone::cached_runtime_path(target, version)
        .ok()
        .and_then(|p| p.metadata().ok())
        .is_some_and(|m| m.len() > 0)
}

/// Become `exe`. Does not return on success.
///
/// Unix uses `exec`, not spawn-and-wait, and that choice is load-bearing:
/// `soli serve` writes `soli.pid` and later reads it to kill the previous
/// process, so a wrapper process would put the wrong pid there; `soli serve -d`
/// daemonizes by forking, which under a live parent leaves the shell watching
/// the wrapper; and the repo's deliberate exit codes (64, 70, 1) reach the
/// caller without a relay. One process, one pid, one exit status.
fn switch_to(exe: &Path, version: &str) -> ! {
    let mut cmd = std::process::Command::new(exe);
    cmd.args(std::env::args_os().skip(1))
        .env(SHIM_GUARD_ENV, version);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Only returns on failure.
        let err = cmd.exec();
        exec_failed(exe, version, err);
    }

    #[cfg(not(unix))]
    {
        // Windows has no exec. Ctrl-C reaches the whole console process group,
        // so the child sees it too and this wait then returns with its status.
        match cmd.status() {
            Ok(status) => std::process::exit(status.code().unwrap_or(70)),
            Err(err) => exec_failed(exe, version, err),
        }
    }
}

/// A cached toolchain that will not run — truncated, wrong architecture, or
/// stripped of its executable bit. Remove it so the next run re-downloads,
/// then say so rather than leaving a permanently broken directory.
fn exec_failed(exe: &Path, version: &str, err: std::io::Error) -> ! {
    let removed = std::fs::remove_file(exe).is_ok();

    eprintln!(
        "\x1b[31mError:\x1b[0m the cached soli {version} at {} could not be executed ({err}).",
        exe.display()
    );
    if removed {
        eprintln!("  It has been removed — rerun to download it again.");
    }
    std::process::exit(1);
}

/// `soli which` — report the version that would run here, and why.
///
/// A shim you cannot interrogate costs someone an afternoon, so this is not
/// optional. It reports rather than follows: it is on the bypass list.
pub fn print_which() {
    let cwd = std::env::current_dir().ok();
    let pin = cwd
        .as_deref()
        .and_then(solilang::module::pinned_soli_version);

    let running_exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());

    match pin {
        Some((version, manifest)) if version != VERSION => {
            let target = solilang::update::host_target();
            let binary = target
                .as_ref()
                .ok()
                .and_then(|t| super::standalone::cached_runtime_path(t, &version).ok());

            println!("soli {version}");
            match binary {
                Some(path) if path.is_file() => println!("  binary   {}", path.display()),
                Some(path) => println!("  binary   {} (not downloaded yet)", path.display()),
                None => println!("  binary   <no published build for this platform>"),
            }
            println!("  pinned   {}", manifest.display());
            if env_is_set(NO_PIN_ENV) {
                println!("  note     {NO_PIN_ENV} is set — running soli {VERSION} instead");
            }
        }
        Some((version, manifest)) => {
            println!("soli {version}");
            println!("  binary   {running_exe}");
            println!("  pinned   {} (already running it)", manifest.display());
        }
        None => {
            println!("soli {VERSION}");
            println!("  binary   {running_exe}");
            println!("  pinned   no (no exact soli_version in soli.toml)");
        }
    }

    if let Ok(guard) = std::env::var(SHIM_GUARD_ENV) {
        if !guard.trim().is_empty() && guard != VERSION {
            println!("  warning  switched here as soli {guard}, but this binary reports {VERSION}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn leading_token_skips_global_flags() {
        assert_eq!(leading_token(&argv(&["--vm", "run", "a.sl"])), Some("run"));
        assert_eq!(
            leading_token(&argv(&["--no-type-check", "app.sl"])),
            Some("app.sl")
        );
        assert_eq!(leading_token(&argv(&[])), None);
        assert_eq!(leading_token(&argv(&["--vm"])), None);
    }

    /// `soli update` redirected into a pinned toolchain would overwrite a cache
    /// entry with a different version. That is the one bypass that prevents
    /// actual corruption, so it gets its own assertion.
    #[test]
    fn update_never_follows_a_pin() {
        assert!(bypasses_pin(&argv(&["update"])));
        assert!(bypasses_pin(&argv(&["--vm", "update"])));
    }

    #[test]
    fn probes_and_project_less_commands_bypass() {
        for args in [
            vec!["--version"],
            vec!["-v"],
            vec!["--help"],
            vec!["-h"],
            vec!["new", "myapp"],
            vec!["update-keygen"],
            vec!["sign-update", "latest.json"],
            vec!["which"],
        ] {
            assert!(bypasses_pin(&argv(&args)), "{args:?} should bypass");
        }
    }

    #[test]
    fn project_commands_follow_the_pin() {
        for args in [
            vec!["serve", "."],
            vec!["test", "tests/"],
            vec!["run", "app.sl"],
            vec!["db:migrate"],
            vec!["add", "some-pkg"],
            vec!["install"],
            vec!["build", "app"],
            vec!["routes"],
            vec![],
        ] {
            assert!(!bypasses_pin(&argv(&args)), "{args:?} should not bypass");
        }
    }

    /// The pre-scan must agree with `parse_args` about which token is the
    /// subcommand. A *file* called `update.sl` is a script to run, not the
    /// update command, and running it should honour the project's pin.
    #[test]
    fn a_script_is_not_the_command_that_shares_its_name() {
        assert!(!bypasses_pin(&argv(&["update.sl"])));
        assert!(!bypasses_pin(&argv(&["./update"])));
        assert!(!bypasses_pin(&argv(&["new.sl"])));
    }

    /// `--help` anywhere in the line wins: `soli serve --help` must print help
    /// rather than download an interpreter to print it.
    #[test]
    fn a_help_flag_after_a_subcommand_still_bypasses() {
        assert!(bypasses_pin(&argv(&["serve", "--help"])));
        assert!(bypasses_pin(&argv(&["test", "--version"])));
    }
}
