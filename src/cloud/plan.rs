//! The deployment, as a sequence of steps — decided before anything runs.
//!
//! Pure. Producing the plan and executing it are separate so that `--dry-run` is
//! the *same* plan the real deploy uses rather than a second code path that
//! drifts from it, and so the ordering rules below can be asserted without a
//! host to deploy to.
//!
//! # The ordering, and why each step is where it is
//!
//! ```text
//! 1  upload      into releases/<id>/, a directory nothing points at yet
//! 2  repoint     sites/<app> -> releases/<id>          atomic: ln -sfn
//! 3  proxy deploy                                       blue/green starts here
//! 4  health gate                                        old slot still serving
//! 5  alias                                              traffic moves
//! 6  prune        oldest releases, never the live one
//! ```
//!
//! **Upload before repoint** so a transfer that dies half way leaves an unused
//! directory rather than a live symlink pointing at half an app.
//!
//! **Repoint before deploy** because the proxy reads the app from
//! `sites/<app>`; deploying first would start the release that is already live,
//! and report success.
//!
//! **Health before alias.** The proxy's own blue/green keeps the old slot
//! serving until the new one answers. Moving the alias first sends real traffic
//! at a release that may still be starting — or may never start.
//!
//! **Prune last, and never the live release.** A failed deploy that pruned first
//! would have thrown away the thing it needs to roll back to.

use super::release::{Layout, ReleaseId};

/// One thing the deploy will do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Create the release directory on the host.
    Mkdir { path: String },
    /// Copy the built artifact into it.
    Upload { local: String, remote: String },
    /// `ln -sfn <target> <link>` — atomic, unlike remove-then-create.
    Repoint { link: String, target: String },
    /// Ask the proxy to start the app from what the symlink now points at.
    ProxyDeploy { app: String },
    /// Wait for the app to answer 200 before any traffic is moved.
    HealthGate { url: String, timeout_secs: u64 },
    /// Point a domain at the app.
    Alias { app: String, domain: String },
    /// Remove an old release.
    Prune { path: String },
}

impl Step {
    /// One line, for `--dry-run` and for the log.
    pub fn describe(&self) -> String {
        match self {
            Step::Mkdir { path } => format!("mkdir     {path}"),
            Step::Upload { local, remote } => format!("upload    {local} -> {remote}"),
            Step::Repoint { link, target } => format!("repoint   {link} -> {target}"),
            Step::ProxyDeploy { app } => format!("deploy    {app} (blue/green, health-gated)"),
            Step::HealthGate { url, timeout_secs } => {
                format!("health    {url} within {timeout_secs}s")
            }
            Step::Alias { app, domain } => format!("alias     {domain} -> {app}"),
            Step::Prune { path } => format!("prune     {path}"),
        }
    }

    /// Whether this step changes what users see.
    ///
    /// Used to decide what a failure means: everything before the first
    /// user-visible step can be abandoned silently, and everything after it
    /// needs saying out loud.
    pub fn is_user_visible(&self) -> bool {
        matches!(
            self,
            Step::Repoint { .. } | Step::ProxyDeploy { .. } | Step::Alias { .. }
        )
    }
}

/// What a deploy needs to know.
#[derive(Debug, Clone)]
pub struct Deployment {
    pub layout: Layout,
    pub release: ReleaseId,
    /// The built artifact on this machine.
    pub artifact: String,
    /// Domains to point at the app once it is healthy.
    pub domains: Vec<String>,
    /// Where to probe. Empty skips the gate — and says so, loudly.
    pub health_url: String,
    pub health_timeout_secs: u64,
    /// Releases already on the host.
    pub existing: Vec<ReleaseId>,
    pub keep: usize,
}

/// Builds the ordered plan.
pub fn plan(deployment: &Deployment) -> Vec<Step> {
    let release_dir = deployment.layout.release_dir(&deployment.release);
    let mut steps = vec![
        Step::Mkdir {
            path: release_dir.clone(),
        },
        Step::Upload {
            local: deployment.artifact.clone(),
            remote: release_dir.clone(),
        },
        Step::Repoint {
            link: deployment.layout.live_link(),
            target: release_dir.clone(),
        },
        Step::ProxyDeploy {
            app: deployment.layout.app.clone(),
        },
    ];

    if !deployment.health_url.is_empty() {
        steps.push(Step::HealthGate {
            url: deployment.health_url.clone(),
            timeout_secs: deployment.health_timeout_secs,
        });
    }

    for domain in &deployment.domains {
        steps.push(Step::Alias {
            app: deployment.layout.app.clone(),
            domain: domain.clone(),
        });
    }

    // The release being deployed is live from step 3 onward, so it is passed as
    // the live one here: pruning must never target it even though it is not yet
    // in `existing`.
    let mut all = deployment.existing.clone();
    all.push(deployment.release.clone());
    for old in super::release::prunable(all, Some(&deployment.release), deployment.keep) {
        steps.push(Step::Prune {
            path: deployment.layout.release_dir(&old),
        });
    }
    steps
}

/// The plan for a rollback: repoint, redeploy, gate. No upload, no prune.
///
/// Nothing is built and nothing is removed. That is the point — the bytes it
/// goes back to are provably the bytes that were serving before, and a rollback
/// that pruned could not itself be rolled back.
pub fn rollback_plan(
    layout: &Layout,
    target: &ReleaseId,
    health_url: &str,
    timeout: u64,
) -> Vec<Step> {
    let mut steps = vec![
        Step::Repoint {
            link: layout.live_link(),
            target: layout.release_dir(target),
        },
        Step::ProxyDeploy {
            app: layout.app.clone(),
        },
    ];
    if !health_url.is_empty() {
        steps.push(Step::HealthGate {
            url: health_url.to_string(),
            timeout_secs: timeout,
        });
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(ts: u64, sha: &str) -> ReleaseId {
        ReleaseId::new(ts, sha)
    }

    fn deployment() -> Deployment {
        Deployment {
            layout: Layout::new("/home/rocky", "crm.solisoft.net"),
            release: id(1_785_614_400, "a3f21c9"),
            artifact: "/tmp/build/app.soli".into(),
            domains: vec!["crm.solisoft.net".into()],
            health_url: "https://crm.solisoft.net/up".into(),
            health_timeout_secs: 60,
            existing: Vec::new(),
            keep: 5,
        }
    }

    fn kinds(steps: &[Step]) -> Vec<&'static str> {
        steps
            .iter()
            .map(|s| match s {
                Step::Mkdir { .. } => "mkdir",
                Step::Upload { .. } => "upload",
                Step::Repoint { .. } => "repoint",
                Step::ProxyDeploy { .. } => "deploy",
                Step::HealthGate { .. } => "health",
                Step::Alias { .. } => "alias",
                Step::Prune { .. } => "prune",
            })
            .collect()
    }

    #[test]
    fn the_health_gate_sits_between_the_deploy_and_the_alias() {
        // The single most important ordering here. Moving the alias first sends
        // real traffic at a release that may still be starting — the proxy's
        // blue/green keeps the old slot serving precisely so that does not
        // have to happen.
        let steps = plan(&deployment());
        let order = kinds(&steps);
        let deploy = order.iter().position(|k| *k == "deploy").unwrap();
        let health = order.iter().position(|k| *k == "health").unwrap();
        let alias = order.iter().position(|k| *k == "alias").unwrap();
        assert!(deploy < health, "{order:?}");
        assert!(health < alias, "{order:?}");
    }

    #[test]
    fn the_upload_finishes_before_anything_points_at_it() {
        // A transfer that dies half way must leave an unused directory, not a
        // live symlink pointing at half an app.
        let order = kinds(&plan(&deployment()));
        assert_eq!(&order[..3], &["mkdir", "upload", "repoint"], "{order:?}");
    }

    #[test]
    fn the_symlink_moves_before_the_proxy_is_told_to_deploy() {
        // The proxy reads the app from `sites/<app>`. Deploying first would
        // start the release that is already live — and report success.
        let order = kinds(&plan(&deployment()));
        let repoint = order.iter().position(|k| *k == "repoint").unwrap();
        let deploy = order.iter().position(|k| *k == "deploy").unwrap();
        assert!(repoint < deploy, "{order:?}");
    }

    #[test]
    fn pruning_happens_last_and_never_touches_the_new_release() {
        // A deploy that pruned first would have thrown away the thing it needs
        // to roll back to.
        let mut d = deployment();
        d.existing = (0..8)
            .map(|i| id(1_700_000_000 + i * 86_400, &format!("old{i:04}")))
            .collect();
        d.keep = 3;

        let steps = plan(&d);
        let order = kinds(&steps);
        assert_eq!(*order.last().unwrap(), "prune", "{order:?}");

        let live = d.layout.release_dir(&d.release);
        for step in &steps {
            if let Step::Prune { path } = step {
                assert_ne!(*path, live, "the release being deployed was pruned");
            }
        }
    }

    #[test]
    fn an_absent_health_url_removes_the_gate_rather_than_passing_it() {
        // An app with no health endpoint is a real case. Silently treating the
        // gate as passed would be the dangerous reading; omitting the step
        // makes it visible in the plan that no gate ran.
        let mut d = deployment();
        d.health_url = String::new();
        let order = kinds(&plan(&d));
        assert!(!order.contains(&"health"), "{order:?}");
    }

    #[test]
    fn every_domain_gets_an_alias() {
        let mut d = deployment();
        d.domains = vec!["a.soli.app".into(), "b.soli.app".into()];
        let aliases = plan(&d)
            .iter()
            .filter(|s| matches!(s, Step::Alias { .. }))
            .count();
        assert_eq!(aliases, 2);
    }

    #[test]
    fn a_rollback_builds_nothing_and_removes_nothing() {
        // The bytes it returns to are provably the bytes that were serving
        // before, and a rollback that pruned could not itself be rolled back.
        let layout = Layout::new("/home/rocky", "crm.solisoft.net");
        let target = id(1_700_000_000, "old1234");
        let order = kinds(&rollback_plan(&layout, &target, "https://x/up", 60));
        assert_eq!(order, ["repoint", "deploy", "health"]);
    }

    #[test]
    fn the_plan_reads_as_a_dry_run() {
        // `--dry-run` prints exactly this, from exactly this plan — not a
        // second code path that drifts from what a real deploy does.
        let lines: Vec<String> = plan(&deployment()).iter().map(Step::describe).collect();
        assert!(lines[0].starts_with("mkdir     /home/rocky/releases/crm.solisoft.net/"));
        assert!(lines.iter().any(|l| l.contains("-> /home/rocky/releases/")));
        assert!(lines
            .iter()
            .any(|l| l.starts_with("alias     crm.solisoft.net")));
    }

    #[test]
    fn user_visible_steps_are_the_ones_after_the_point_of_no_return() {
        // Everything before the first user-visible step can be abandoned
        // silently; everything after it has to be said out loud.
        let steps = plan(&deployment());
        let first = steps.iter().position(Step::is_user_visible).unwrap();
        assert_eq!(kinds(&steps)[first], "repoint");
        assert!(!steps[0].is_user_visible());
        assert!(!steps[1].is_user_visible());
    }
}
