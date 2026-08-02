//! Executing a plan.
//!
//! The plan is decided in [`super::plan`] and printed identically whether it
//! runs or not — `--dry-run` is the same plan, not a second description of it.
//!
//! # What happens when a step fails
//!
//! Everything up to and including the upload is invisible: a failure there
//! leaves an unused directory and nothing else, and the message says to retry.
//!
//! From the repoint onward the deployment is live, so a failure is reported with
//! the release that is *currently serving* and the exact command to go back. It
//! deliberately does **not** roll back on its own: an automatic rollback in the
//! middle of a half-applied change is a second uncontrolled change on top of the
//! first, at the moment when least is known about what is wrong.

use super::plan::Step;
use super::proxy::{wait_healthy, Admin};
use super::release::{Layout, ReleaseId};
use std::process::Command;
use std::time::Duration;

/// Ten seconds to establish the connection. Generous for a LAN, short enough
/// that an unreachable host is an error rather than a wait.
const CONNECT_TIMEOUT: &str = "ConnectTimeout=10";

/// How a step ended.
pub enum Outcome {
    Done(String),
    Failed(String),
}

/// Where the deployment is going.
pub struct Target {
    /// `user@host`, run through the system `ssh` so the agent, `~/.ssh/config`,
    /// jump hosts and everything else an operator has already configured keep
    /// working. A hand-rolled client would re-implement all of it, worse.
    pub ssh: String,
    pub admin: Admin,
}

impl Target {
    /// Runs a command on the host, returning stdout.
    ///
    /// `ConnectTimeout` is not optional. `BatchMode` only stops ssh asking for
    /// a password; without a connect timeout an unreachable host makes the
    /// whole command hang with no output — which reads as a hung deploy rather
    /// than as an unreachable server, and is the difference between a 10-second
    /// error and a phone call.
    pub fn remote(&self, command: &str) -> Result<String, String> {
        let output = Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                CONNECT_TIMEOUT,
                &self.ssh,
                command,
            ])
            .output()
            .map_err(|e| format!("ssh failed to start: {e}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    }

    fn upload(&self, local: &str, remote_dir: &str) -> Result<(), String> {
        let status = Command::new("scp")
            .args(["-o", "BatchMode=yes", "-o", CONNECT_TIMEOUT, "-r", local])
            .arg(format!("{}:{remote_dir}/", self.ssh))
            .status()
            .map_err(|e| format!("scp failed to start: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("scp exited with {status}"))
        }
    }

    /// Releases already on the host, newest last.
    ///
    /// Unparseable names are skipped rather than guessed at — this list decides
    /// what gets deleted, and a directory that is not a release must never
    /// become one by being in the same folder.
    pub fn releases(&self, layout: &Layout) -> Result<Vec<ReleaseId>, String> {
        let listing = self.remote(&format!(
            "ls -1 {} 2>/dev/null || true",
            shell_quote(&layout.releases_dir())
        ))?;
        let mut found: Vec<ReleaseId> = listing.lines().filter_map(ReleaseId::parse).collect();
        found.sort();
        Ok(found)
    }

    /// Which release the live symlink points at, if it points at one.
    pub fn live(&self, layout: &Layout) -> Result<Option<ReleaseId>, String> {
        let target = self.remote(&format!(
            "readlink -f {} 2>/dev/null || true",
            shell_quote(&layout.live_link())
        ))?;
        Ok(target.rsplit('/').next().and_then(ReleaseId::parse))
    }
}

/// Runs one step.
pub fn execute(step: &Step, target: &Target) -> Outcome {
    match step {
        Step::Mkdir { path } => match target.remote(&format!("mkdir -p {}", shell_quote(path))) {
            Ok(_) => Outcome::Done(String::new()),
            Err(e) => Outcome::Failed(e),
        },
        Step::Upload { local, remote } => match target.upload(local, remote) {
            Ok(()) => Outcome::Done(String::new()),
            Err(e) => Outcome::Failed(e),
        },
        // `ln -sfn`, one call: remove-then-create leaves a window where the
        // proxy sees no app at all, and the proxy scans on a timer.
        Step::Repoint { link, target: to } => {
            match target.remote(&format!(
                "ln -sfn {} {}",
                shell_quote(to),
                shell_quote(link)
            )) {
                Ok(_) => Outcome::Done(String::new()),
                Err(e) => Outcome::Failed(e),
            }
        }
        Step::ProxyDeploy { app } => match target.admin.deploy(app) {
            Ok(_) => Outcome::Done(String::new()),
            Err(e) => Outcome::Failed(e.to_string()),
        },
        Step::HealthGate { url, timeout_secs } => {
            match wait_healthy(url, Duration::from_secs(*timeout_secs)) {
                Ok(elapsed) => Outcome::Done(format!("200 in {:.1}s", elapsed.as_secs_f32())),
                Err(e) => Outcome::Failed(e),
            }
        }
        Step::Alias { app, domain } => match target.admin.set_alias(app, domain) {
            Ok(_) => Outcome::Done(String::new()),
            Err(e) => Outcome::Failed(e.to_string()),
        },
        // Guarded by the shape of the path, not by trust in the caller. This is
        // the only step that deletes, and it runs unattended.
        Step::Prune { path } => {
            if !is_release_path(path) {
                return Outcome::Failed(format!(
                    "refusing to remove {path}: it is not a release directory"
                ));
            }
            match target.remote(&format!("rm -rf {}", shell_quote(path))) {
                Ok(_) => Outcome::Done(String::new()),
                Err(e) => Outcome::Failed(e),
            }
        }
    }
}

/// Whether a path is safe to `rm -rf` unattended.
///
/// Belt and braces over [`ReleaseId::parse`]. The plan only ever produces
/// release paths, but this is the one command that destroys data and it runs
/// without a human watching — so it re-derives the answer from the path itself
/// rather than trusting that the plan was built correctly.
fn is_release_path(path: &str) -> bool {
    let mut parts = path.rsplit('/');
    let Some(last) = parts.next() else {
        return false;
    };
    if ReleaseId::parse(last).is_none() {
        return false;
    }
    // …/releases/<app>/<id>
    parts.next().is_some() && parts.any(|segment| segment == "releases")
}

/// Single-quotes a value for a POSIX shell.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_release_directory_can_be_pruned() {
        // The one step that destroys data, running unattended. It re-derives
        // the answer from the path rather than trusting the plan that built it.
        assert!(is_release_path(
            "/home/rocky/releases/crm/20260801T200000Z-a3f21c9"
        ));

        for bad in [
            "/home/rocky/releases/crm",
            "/home/rocky/sites/crm",
            "/home/rocky",
            "/",
            "/home/rocky/releases/crm/node_modules",
            "/home/rocky/other/crm/20260801T200000Z-a3f21c9",
            "20260801T200000Z-a3f21c9",
            "",
        ] {
            assert!(!is_release_path(bad), "would have removed {bad:?}");
        }
    }

    #[test]
    fn a_quoted_path_survives_a_shell() {
        // Paths come from a config file an operator edits. A space is ordinary;
        // a quote is a mistake that must not become a command.
        assert_eq!(shell_quote("/home/rocky"), "'/home/rocky'");
        assert_eq!(shell_quote("/home/my apps"), "'/home/my apps'");
        assert_eq!(shell_quote("a'b"), r"'a'\''b'");
        // What makes it safe is the envelope, not the absence of
        // dangerous-looking text: inside single quotes the shell
        // interprets nothing at all. So the property is that the value
        // is wholly enclosed and every inner quote is closed and
        // reopened around an escaped one.
        let quoted = shell_quote("a; rm -rf /");
        assert!(quoted.starts_with('\'') && quoted.ends_with('\''));
        assert_eq!(quoted, "'a; rm -rf /'");
        // An embedded quote cannot end the string early: the only way
        // out is the escape sequence, and it goes straight back in.
        assert_eq!(shell_quote("'; rm -rf /; '"), r"''\''; rm -rf /; '\'''");
    }

    #[test]
    fn the_live_release_is_read_from_the_symlink_target() {
        // `readlink -f` gives the resolved directory; the release id is its last
        // segment. Anything else — a broken link, a directory that is not a
        // release — reads as "nothing live" rather than as a wrong answer.
        let parse = |target: &str| target.rsplit('/').next().and_then(ReleaseId::parse);
        assert!(parse("/home/rocky/releases/crm/20260801T200000Z-a3f21c9").is_some());
        assert!(parse("/home/rocky/sites/crm").is_none());
        assert!(parse("").is_none());
    }
}
