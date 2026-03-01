//! Package installer for git-based dependencies.
//!
//! Downloads and caches packages from GitHub/GitLab repositories using
//! their archive download APIs. No git binary required.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::lockfile::{LockEntry, LockFile};
use super::package::{Dependency, Package};

/// Cache directory for downloaded packages (~/.soli/packages/).
fn cache_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".soli")
        .join("packages")
}

/// Supported git hosting providers.
#[derive(Debug)]
enum GitHost {
    GitHub { owner: String, repo: String },
    GitLab { owner: String, repo: String },
}

/// Parse a git URL into a GitHost.
fn parse_git_url(url: &str) -> Result<GitHost, String> {
    // Normalize: strip trailing .git
    let url = url.strip_suffix(".git").unwrap_or(url);

    if url.contains("github.com") {
        let parts = extract_owner_repo(url, "github.com")?;
        Ok(GitHost::GitHub {
            owner: parts.0,
            repo: parts.1,
        })
    } else if url.contains("gitlab.com") {
        let parts = extract_owner_repo(url, "gitlab.com")?;
        Ok(GitHost::GitLab {
            owner: parts.0,
            repo: parts.1,
        })
    } else {
        Err(format!(
            "Unsupported git host: {}. Only GitHub and GitLab are supported.",
            url
        ))
    }
}

/// Extract owner/repo from a URL like https://github.com/owner/repo
fn extract_owner_repo(url: &str, host: &str) -> Result<(String, String), String> {
    let host_pos = url
        .find(host)
        .ok_or_else(|| format!("Could not find {} in URL: {}", host, url))?;
    let after_host = &url[host_pos + host.len()..];
    let path = after_host.trim_start_matches('/');
    let parts: Vec<&str> = path.splitn(3, '/').collect();
    if parts.len() < 2 {
        return Err(format!("Could not extract owner/repo from URL: {}", url));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Resolve a git ref (tag, branch, or rev) to a full commit SHA.
fn resolve_ref(host: &GitHost, git_ref: &str) -> Result<String, String> {
    match host {
        GitHost::GitHub { owner, repo } => {
            let api_url = format!(
                "https://api.github.com/repos/{}/{}/commits/{}",
                owner, repo, git_ref
            );
            let response = ureq::get(&api_url)
                .set("Accept", "application/vnd.github.v3+json")
                .set("User-Agent", "soli-package-manager")
                .call()
                .map_err(|e| format!("Failed to resolve ref '{}': {}", git_ref, e))?;

            let body: serde_json::Value = response
                .into_json()
                .map_err(|e| format!("Failed to parse GitHub API response: {}", e))?;

            body["sha"].as_str().map(|s| s.to_string()).ok_or_else(|| {
                format!("Could not resolve ref '{}' for {}/{}", git_ref, owner, repo)
            })
        }
        GitHost::GitLab { owner, repo } => {
            let project = format!("{}/{}", owner, repo);
            let encoded_project = urlencoding::encode(&project);
            let api_url = format!(
                "https://gitlab.com/api/v4/projects/{}/repository/commits/{}",
                encoded_project, git_ref
            );
            let response = ureq::get(&api_url)
                .set("User-Agent", "soli-package-manager")
                .call()
                .map_err(|e| format!("Failed to resolve ref '{}': {}", git_ref, e))?;

            let body: serde_json::Value = response
                .into_json()
                .map_err(|e| format!("Failed to parse GitLab API response: {}", e))?;

            body["id"].as_str().map(|s| s.to_string()).ok_or_else(|| {
                format!("Could not resolve ref '{}' for {}/{}", git_ref, owner, repo)
            })
        }
    }
}

/// Get the archive download URL for a given commit SHA.
fn archive_url(host: &GitHost, sha: &str) -> String {
    match host {
        GitHost::GitHub { owner, repo } => {
            format!(
                "https://github.com/{}/{}/archive/{}.tar.gz",
                owner, repo, sha
            )
        }
        GitHost::GitLab { owner, repo } => {
            let project = format!("{}/{}", owner, repo);
            let encoded_project = urlencoding::encode(&project);
            format!(
                "https://gitlab.com/api/v4/projects/{}/repository/archive.tar.gz?sha={}",
                encoded_project, sha
            )
        }
    }
}

/// Download and extract a package archive to the cache directory.
fn download_and_extract(url: &str, dest: &Path) -> Result<(), String> {
    use flate2::read::GzDecoder;

    let response = ureq::get(url)
        .set("User-Agent", "soli-package-manager")
        .call()
        .map_err(|e| format!("Failed to download archive: {}", e))?;

    let reader = response.into_reader();
    let decoder = GzDecoder::new(reader);
    let mut archive = tar::Archive::new(decoder);

    // Create destination directory
    fs::create_dir_all(dest).map_err(|e| format!("Failed to create cache directory: {}", e))?;

    // Extract, stripping the top-level directory (GitHub/GitLab archives have one)
    for entry in archive
        .entries()
        .map_err(|e| format!("Failed to read archive entries: {}", e))?
    {
        let mut entry = entry.map_err(|e| format!("Failed to read archive entry: {}", e))?;
        let path = entry
            .path()
            .map_err(|e| format!("Failed to read entry path: {}", e))?
            .into_owned();

        // Strip the first component (e.g., "repo-sha/")
        let components: Vec<_> = path.components().collect();
        if components.len() <= 1 {
            continue; // Skip the top-level directory itself
        }

        let mut out_path = dest.to_path_buf();
        for component in &components[1..] {
            out_path.push(component);
        }

        if entry.header().entry_type() == tar::EntryType::Directory {
            fs::create_dir_all(&out_path)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent directory: {}", e))?;
            }
            let mut out_file =
                fs::File::create(&out_path).map_err(|e| format!("Failed to create file: {}", e))?;
            io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("Failed to extract file: {}", e))?;
        }
    }

    Ok(())
}

/// Install a single git dependency.
fn install_git_dep(
    name: &str,
    url: &str,
    tag: &Option<String>,
    branch: &Option<String>,
    rev: &Option<String>,
    lock: &mut LockFile,
) -> Result<(), String> {
    // Check if already satisfied
    let dep = Dependency::Git {
        url: url.to_string(),
        tag: tag.clone(),
        branch: branch.clone(),
        rev: rev.clone(),
    };
    if lock.is_satisfied(name, &dep) {
        println!("  {} (cached)", name);
        return Ok(());
    }

    let host = parse_git_url(url)?;

    // Determine the git ref to resolve
    let git_ref = if let Some(r) = rev {
        r.clone()
    } else if let Some(t) = tag {
        t.clone()
    } else if let Some(b) = branch {
        b.clone()
    } else {
        "HEAD".to_string()
    };

    let ref_spec = tag.as_ref().or(branch.as_ref()).cloned();

    println!("  {} (resolving {}...)", name, git_ref);

    // Resolve to full SHA
    let sha = resolve_ref(&host, &git_ref)?;
    let short_sha = &sha[..12.min(sha.len())];

    // Check if we already have this exact SHA cached
    let cache_path = cache_dir().join(format!("{}-{}", name, short_sha));
    if cache_path.exists() {
        println!("  {} (already downloaded at {})", name, short_sha);
    } else {
        println!("  {} (downloading {}...)", name, short_sha);
        let url = archive_url(&host, &sha);
        download_and_extract(&url, &cache_path)?;
        println!("  {} (installed)", name);
    }

    // Update lock entry
    lock.packages.insert(
        name.to_string(),
        LockEntry {
            name: name.to_string(),
            url: url.to_string(),
            resolved_rev: sha,
            cache_path: cache_path.clone(),
            ref_spec,
        },
    );

    // Check for transitive dependencies
    let dep_toml = cache_path.join("soli.toml");
    if dep_toml.exists() {
        let sub_pkg = Package::load(&dep_toml)
            .map_err(|e| format!("Failed to parse soli.toml in '{}': {}", name, e))?;
        for (sub_name, sub_dep) in &sub_pkg.dependencies {
            if let Dependency::Git {
                url: sub_url,
                tag: sub_tag,
                branch: sub_branch,
                rev: sub_rev,
            } = sub_dep
            {
                install_git_dep(sub_name, sub_url, sub_tag, sub_branch, sub_rev, lock)?;
            }
        }
    }

    Ok(())
}

/// Install a single version-based dependency from the registry.
fn install_version_dep(name: &str, version: &str, lock: &mut LockFile) -> Result<(), String> {
    use super::registry;

    let dep = Dependency::Version(version.to_string());
    if lock.is_satisfied(name, &dep) {
        println!("  {} (cached)", name);
        return Ok(());
    }

    let registry_url = registry::DEFAULT_REGISTRY;

    println!("  {} (resolving {}@{}...)", name, name, version);

    let info = registry::resolve_version(registry_url, name, version)?;

    let cache_path = cache_dir().join(format!("{}-{}", name, version));
    if cache_path.exists() {
        println!("  {} (already downloaded at {})", name, version);
    } else {
        println!("  {} (downloading {}...)", name, version);
        registry::download_package(&info.download_url, &cache_path)?;
        println!("  {} (installed)", name);
    }

    lock.packages.insert(
        name.to_string(),
        LockEntry {
            name: name.to_string(),
            url: registry_url.to_string(),
            resolved_rev: version.to_string(),
            cache_path: cache_path.clone(),
            ref_spec: Some(version.to_string()),
        },
    );

    // Check for transitive dependencies
    let dep_toml = cache_path.join("soli.toml");
    if dep_toml.exists() {
        let sub_pkg = Package::load(&dep_toml)
            .map_err(|e| format!("Failed to parse soli.toml in '{}': {}", name, e))?;
        for (sub_name, sub_dep) in &sub_pkg.dependencies {
            match sub_dep {
                Dependency::Git {
                    url: sub_url,
                    tag: sub_tag,
                    branch: sub_branch,
                    rev: sub_rev,
                } => {
                    install_git_dep(sub_name, sub_url, sub_tag, sub_branch, sub_rev, lock)?;
                }
                Dependency::Version(sub_ver) => {
                    install_version_dep(sub_name, sub_ver, lock)?;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Install all dependencies from a package, writing the lock file.
pub fn install_all(pkg: &Package, lock: &mut LockFile, lock_path: &Path) -> Result<(), String> {
    let mut any_installed = false;

    for (name, dep) in &pkg.dependencies {
        match dep {
            Dependency::Git {
                url,
                tag,
                branch,
                rev,
            } => {
                install_git_dep(name, url, tag, branch, rev, lock)?;
                any_installed = true;
            }
            Dependency::Path(_) => {
                // Path dependencies don't need installation
            }
            Dependency::Version(ver) => {
                install_version_dep(name, ver, lock)?;
                any_installed = true;
            }
        }
    }

    if any_installed {
        lock.save(lock_path)?;
    }

    Ok(())
}

/// Update a specific package (re-resolve, ignoring lock).
pub fn update_package(
    name: &str,
    pkg: &Package,
    lock: &mut LockFile,
    lock_path: &Path,
) -> Result<(), String> {
    let dep = pkg
        .dependencies
        .get(name)
        .ok_or_else(|| format!("Package '{}' not found in dependencies", name))?;

    match dep {
        Dependency::Git {
            url,
            tag,
            branch,
            rev,
        } => {
            // Remove existing lock entry to force re-resolve
            lock.packages.remove(name);
            install_git_dep(name, url, tag, branch, rev, lock)?;
            lock.save(lock_path)?;
        }
        Dependency::Path(_) => {
            println!("  {} is a path dependency, nothing to update", name);
        }
        Dependency::Version(ver) => {
            lock.packages.remove(name);
            install_version_dep(name, ver, lock)?;
            lock.save(lock_path)?;
        }
    }

    Ok(())
}

/// Update all packages (re-resolve everything, ignoring lock).
pub fn update_all(pkg: &Package, lock: &mut LockFile, lock_path: &Path) -> Result<(), String> {
    // Clear all lock entries to force re-resolve
    let remote_deps: Vec<String> = pkg
        .dependencies
        .iter()
        .filter(|(_, d)| matches!(d, Dependency::Git { .. } | Dependency::Version(_)))
        .map(|(n, _)| n.clone())
        .collect();

    for name in &remote_deps {
        lock.packages.remove(name);
    }

    install_all(pkg, lock, lock_path)
}

/// Add a dependency to a package and save.
pub fn add_dependency(pkg: &mut Package, name: &str, dep: Dependency) {
    pkg.dependencies.insert(name.to_string(), dep);
}

/// Remove a dependency from a package.
pub fn remove_dependency(pkg: &mut Package, name: &str, lock: &mut LockFile) {
    pkg.dependencies.remove(name);
    lock.packages.remove(name);
}

/// Find installed packages summary for display.
pub fn installed_summary(lock: &LockFile) -> Vec<(String, String, String)> {
    let mut entries: Vec<_> = lock
        .packages
        .values()
        .map(|e| {
            let short_rev = &e.resolved_rev[..12.min(e.resolved_rev.len())];
            let ref_info = e
                .ref_spec
                .as_ref()
                .map(|r| format!(" ({})", r))
                .unwrap_or_default();
            (
                e.name.clone(),
                format!("{}{}", short_rev, ref_info),
                e.url.clone(),
            )
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_url() {
        let host = parse_git_url("https://github.com/user/soli-math").unwrap();
        match host {
            GitHost::GitHub { owner, repo } => {
                assert_eq!(owner, "user");
                assert_eq!(repo, "soli-math");
            }
            _ => panic!("Expected GitHub host"),
        }
    }

    #[test]
    fn test_parse_github_url_with_git_suffix() {
        let host = parse_git_url("https://github.com/user/soli-math.git").unwrap();
        match host {
            GitHost::GitHub { owner, repo } => {
                assert_eq!(owner, "user");
                assert_eq!(repo, "soli-math");
            }
            _ => panic!("Expected GitHub host"),
        }
    }

    #[test]
    fn test_parse_gitlab_url() {
        let host = parse_git_url("https://gitlab.com/group/project").unwrap();
        match host {
            GitHost::GitLab { owner, repo } => {
                assert_eq!(owner, "group");
                assert_eq!(repo, "project");
            }
            _ => panic!("Expected GitLab host"),
        }
    }

    #[test]
    fn test_unsupported_host() {
        let result = parse_git_url("https://bitbucket.org/user/repo");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported git host"));
    }

    #[test]
    fn test_archive_url_github() {
        let host = GitHost::GitHub {
            owner: "user".to_string(),
            repo: "repo".to_string(),
        };
        let url = archive_url(&host, "abc123");
        assert_eq!(url, "https://github.com/user/repo/archive/abc123.tar.gz");
    }

    #[test]
    fn test_cache_dir() {
        let dir = cache_dir();
        assert!(dir.to_string_lossy().contains(".soli/packages"));
    }
}
