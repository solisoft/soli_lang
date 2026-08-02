//! Naming and layout for immutable releases.
//!
//! Pure: no SSH, no filesystem, no proxy. Every rule about what a deployment is
//! called and where it lives is decided here, so the parts that are easy to get
//! wrong — and expensive to get wrong on someone else's production host — are
//! provable at the desk.
//!
//! # The model
//!
//! A deployment is **immutable**; the alias is what moves.
//!
//! ```text
//! releases/<app>/<release-id>/    a build, never modified after it lands
//! sites/<app>  ->  releases/<app>/<release-id>
//! ```
//!
//! That is not a new convention: 28 of the 31 apps in production are already
//! symlinks under `sites/`. This gives that pattern a name, a history, and a
//! rollback that is one `ln -sfn` rather than a redeploy.
//!
//! # Why the release id is what it is
//!
//! `<utc>-<short-sha>` — sortable first, identifiable second.
//!
//! Sorting matters more than it looks: "the previous release" is answered by
//! sorting the directory listing, and it has to stay correct at 3am on the day
//! two deploys land in the same minute. A timestamp alone collides; a SHA alone
//! does not sort; a SHA alone also repeats when the same commit is redeployed,
//! which would make a rollback target ambiguous.

use std::fmt;

/// How many releases to keep on disk before pruning the oldest.
///
/// Five, not one: rollback needs at least the previous, and the number of times
/// a rollback has itself needed rolling back is not zero. Disk is cheaper than
/// the alternative.
pub const KEEP_RELEASES: usize = 5;

/// A single immutable deployment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReleaseId(String);

impl ReleaseId {
    /// `20260801T220145Z-a3f21c9`.
    ///
    /// The timestamp is UTC and fixed-width, so lexical order *is* chronological
    /// order — no parsing, and no locale to get wrong.
    pub fn new(utc_seconds: u64, commit: &str) -> Self {
        Self(format!("{}-{}", stamp(utc_seconds), short(commit)))
    }

    /// Accepts a name found on disk, or rejects it.
    ///
    /// Strict, because this is used to decide what to *delete* during pruning.
    /// A loose parser that accepted `node_modules` would eventually remove it.
    pub fn parse(name: &str) -> Option<Self> {
        let (ts, sha) = name.split_once('-')?;
        if ts.len() != 16 || !ts.ends_with('Z') || ts.as_bytes()[8] != b'T' {
            return None;
        }
        if !ts[..8].bytes().all(|b| b.is_ascii_digit())
            || !ts[9..15].bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        if sha.is_empty() || !sha.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return None;
        }
        Some(Self(name.to_string()))
    }

    /// Used by the executor's guards and by anything that logs a release.
    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The commit half, for display.
    #[allow(dead_code)]
    pub fn commit(&self) -> &str {
        self.0.split_once('-').map(|(_, sha)| sha).unwrap_or("")
    }
}

impl fmt::Display for ReleaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `YYYYMMDDTHHMMSSZ` from a Unix timestamp, without pulling in a date crate.
///
/// Civil-from-days, the standard algorithm. Written out rather than
/// approximated because a release id that is wrong by a day sorts wrongly, and
/// sorting is the whole job.
fn stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Seven characters, like git's own short form.
fn short(commit: &str) -> String {
    let clean: String = commit
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(7)
        .collect();
    if clean.is_empty() {
        "nocommit".into()
    } else {
        clean
    }
}

/// Where things live on the target host.
#[derive(Debug, Clone)]
pub struct Layout {
    /// The proxy's root, holding `sites/`.
    pub root: String,
    pub app: String,
}

impl Layout {
    pub fn new(root: impl Into<String>, app: impl Into<String>) -> Self {
        Self {
            root: into_root(root.into()),
            app: app.into(),
        }
    }

    /// `<root>/releases/<app>` — beside `sites/`, never inside it.
    ///
    /// Inside would put every past release under the directory the proxy scans
    /// for apps, and it would try to run all of them.
    pub fn releases_dir(&self) -> String {
        format!("{}/releases/{}", self.root, self.app)
    }

    pub fn release_dir(&self, id: &ReleaseId) -> String {
        format!("{}/{}", self.releases_dir(), id)
    }

    /// `<root>/sites/<app>` — the symlink the proxy follows.
    pub fn live_link(&self) -> String {
        format!("{}/sites/{}", self.root, self.app)
    }
}

fn into_root(root: String) -> String {
    let trimmed = root.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// What a rollback should point at, given what is on disk and what is live.
///
/// `None` when there is nothing to roll back to — reported rather than guessed.
/// Rolling back to "whatever sorts first" when the current release is the only
/// one would silently redeploy the thing being rolled back.
pub fn previous(mut releases: Vec<ReleaseId>, live: Option<&ReleaseId>) -> Option<ReleaseId> {
    releases.sort();
    releases.dedup();
    match live {
        Some(current) => releases.into_iter().rev().find(|r| r < current),
        // Nothing is live. The newest is the best guess and the only one.
        None => releases.pop(),
    }
}

/// Releases to delete, oldest first, keeping `KEEP_RELEASES` and never the
/// live one.
///
/// The live check is not redundant with the count: an operator who rolled back
/// several times can have the live release fall outside the newest five, and
/// pruning by count alone would delete the running deployment out from under
/// the symlink.
pub fn prunable(
    mut releases: Vec<ReleaseId>,
    live: Option<&ReleaseId>,
    keep: usize,
) -> Vec<ReleaseId> {
    releases.sort();
    releases.dedup();
    let keep_from = releases.len().saturating_sub(keep);
    releases
        .into_iter()
        .take(keep_from)
        .filter(|r| Some(r) != live)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(ts: u64, sha: &str) -> ReleaseId {
        ReleaseId::new(ts, sha)
    }

    #[test]
    fn a_release_id_sorts_chronologically_as_text() {
        // "The previous release" is answered by sorting a directory listing. If
        // lexical order ever diverges from chronological order, a rollback
        // silently targets the wrong build.
        let early = id(1_754_000_000, "aaaaaaa");
        let late = id(1_754_086_400, "bbbbbbb");
        assert!(early < late);
        assert!(early.as_str() < late.as_str());
    }

    #[test]
    fn the_stamp_is_a_real_date() {
        // 2026-08-01T20:00:00Z. A release id wrong by a day sorts wrongly, and
        // sorting is the whole job — so the civil-from-days arithmetic is
        // checked against a known instant rather than assumed.
        assert_eq!(stamp(1_785_614_400), "20260801T200000Z");
        assert_eq!(stamp(0), "19700101T000000Z");
        // A leap day, which is where a hand-rolled calendar goes wrong.
        assert_eq!(stamp(1_709_164_800), "20240229T000000Z");
    }

    #[test]
    fn two_deploys_in_the_same_second_are_still_distinct() {
        // Same second, different commits. A timestamp alone collides and the
        // second deploy would land in the first one's directory.
        let a = id(1_754_000_000, "aaaaaaa");
        let b = id(1_754_000_000, "bbbbbbb");
        assert_ne!(a, b);
    }

    #[test]
    fn parsing_is_strict_enough_to_delete_by() {
        // This decides what gets removed during pruning. A loose parser that
        // accepted a stray directory would eventually delete it.
        assert!(ReleaseId::parse("20260801T200000Z-a3f21c9").is_some());
        for bad in [
            "node_modules",
            "current",
            "20260801-a3f21c9",
            "20260801T200000-a3f21c9",
            "20260801T200000Z-",
            "20260801T20000ZZ-abc",
            "notadate T200000Z-abc",
            "",
        ] {
            assert!(ReleaseId::parse(bad).is_none(), "accepted {bad:?}");
        }
    }

    #[test]
    fn rollback_targets_the_one_before_the_live_release() {
        let a = id(1_754_000_000, "aaaaaaa");
        let b = id(1_754_086_400, "bbbbbbb");
        let c = id(1_754_172_800, "ccccccc");
        let all = vec![c.clone(), a.clone(), b.clone()];

        assert_eq!(previous(all.clone(), Some(&c)), Some(b.clone()));
        assert_eq!(previous(all.clone(), Some(&b)), Some(a.clone()));
    }

    #[test]
    fn rolling_back_past_the_oldest_release_says_no() {
        // Returning "whatever sorts first" here would silently redeploy the
        // very thing being rolled back, and report success.
        let a = id(1_754_000_000, "aaaaaaa");
        assert_eq!(previous(vec![a.clone()], Some(&a)), None);
        assert_eq!(previous(vec![], Some(&a)), None);
    }

    #[test]
    fn rolling_back_twice_walks_backwards_rather_than_oscillating() {
        // After one rollback the live release is no longer the newest. A
        // "previous = second newest" rule would send the next rollback back to
        // the one just abandoned, and the two would ping-pong forever.
        let a = id(1_754_000_000, "aaaaaaa");
        let b = id(1_754_086_400, "bbbbbbb");
        let c = id(1_754_172_800, "ccccccc");
        let all = vec![a.clone(), b.clone(), c.clone()];

        let first = previous(all.clone(), Some(&c)).unwrap();
        assert_eq!(first, b);
        let second = previous(all.clone(), Some(&first)).unwrap();
        assert_eq!(second, a, "the second rollback returned to the newest");
    }

    #[test]
    fn pruning_never_removes_the_live_release() {
        // An operator who rolled back several times can have the live release
        // fall outside the newest N. Pruning by count alone deletes the running
        // deployment out from under the symlink.
        let ids: Vec<ReleaseId> = (0..8)
            .map(|i| id(1_754_000_000 + i * 86_400, &format!("sha{i:04}")))
            .collect();
        let live = ids[0].clone();

        let doomed = prunable(ids.clone(), Some(&live), 3);
        assert!(!doomed.contains(&live), "the live release was pruned");
        assert_eq!(doomed.len(), 4, "got {doomed:?}");
        // Oldest first, so a partial failure leaves the newest behind.
        assert!(doomed.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn pruning_keeps_the_newest_n() {
        let ids: Vec<ReleaseId> = (0..10)
            .map(|i| id(1_754_000_000 + i * 86_400, &format!("sha{i:04}")))
            .collect();
        let doomed = prunable(ids.clone(), Some(&ids[9]), KEEP_RELEASES);
        assert_eq!(doomed.len(), 10 - KEEP_RELEASES);
        assert_eq!(doomed[0], ids[0]);
    }

    #[test]
    fn releases_live_beside_sites_never_inside_it() {
        // Inside `sites/` the proxy would discover every past release as an app
        // and try to run all of them.
        let layout = Layout::new("/home/rocky/", "crm.solisoft.net");
        assert_eq!(
            layout.releases_dir(),
            "/home/rocky/releases/crm.solisoft.net"
        );
        assert_eq!(layout.live_link(), "/home/rocky/sites/crm.solisoft.net");
        assert!(!layout.releases_dir().contains("/sites/"));
    }

    #[test]
    fn a_trailing_slash_on_the_root_does_not_double_up() {
        // `//sites/x` mostly works and then does not, on the one path that
        // compares strings.
        for root in ["/home/rocky", "/home/rocky/", "/home/rocky///"] {
            assert_eq!(
                Layout::new(root, "app").live_link(),
                "/home/rocky/sites/app"
            );
        }
    }

    #[test]
    fn a_commit_that_is_not_a_sha_still_produces_a_usable_id() {
        // A dirty working tree, a detached build, a caller that passed a branch
        // name. None of those should fail a deploy.
        assert!(ReleaseId::parse(id(1_754_000_000, "").as_str()).is_some());
        assert_eq!(id(1_754_000_000, "").commit(), "nocommit");
        // Seven characters after filtering, like git's own short form.
        assert_eq!(id(1_754_000_000, "feature/x-1").commit(), "feature");
    }
}
