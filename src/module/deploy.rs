//! Remote deploy over SSH (Unix-only).
//!
//! Every remote step runs the system `ssh`, as the rsync step always did, so
//! the user's agent, keys, `~/.ssh/config` (ports, `ProxyJump`, aliases) and
//! `known_hosts` all apply. The deploy used to carry its own SSH client
//! (`ssh2`, with OpenSSL compiled from source) that verified no host key at
//! all, connected to port 22 only, and tried the agent's first key only.
//!
//! `deploy.toml` parsing lives in [`super::deploy_config`] so that the commands
//! which only need to read the file still compile on Windows.

use super::deploy_config::{DeployConfig, DeployMode, ServerConfig};
use crate::platform::process::shell_quote;
use std::path::Path;
use std::process::Stdio;

/// Options for every `ssh` call, rsync's included: a host seen for the first
/// time is trusted and remembered, a changed host key is refused; `BatchMode`
/// makes a missing key an error instead of a password prompt nobody answers.
const SSH_OPTIONS: &[&str] = &[
    "-o",
    "StrictHostKeyChecking=accept-new",
    "-o",
    "BatchMode=yes",
];

/// What a remote command printed, and whether it exited 0.
struct Remote {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Run `command` through the remote shell. `stdin` feeds it (a file for the
/// bundle upload), or nothing. An `ssh` that cannot connect or log in exits
/// 255, which is an error here rather than a failed command.
async fn ssh_run(server: &ServerConfig, command: &str, stdin: Stdio) -> Result<Remote, String> {
    let output = tokio::process::Command::new("ssh")
        .args(SSH_OPTIONS)
        .arg("--")
        .arg(format!("{}@{}", server.username, server.ip))
        .arg(command)
        .stdin(stdin)
        .output()
        .await
        .map_err(|e| format!("ssh spawn failed: {} (is OpenSSH installed?)", e))?;
    let remote = Remote {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    };
    if output.status.code() == Some(255) {
        return Err(format!(
            "SSH to {}@{} failed: {}",
            server.username,
            server.ip,
            remote.stderr.trim()
        ));
    }
    Ok(remote)
}

#[derive(Clone)]
pub struct DeployResult {
    pub server_name: String,
    pub success: bool,
    pub message: String,
    pub slot: Option<String>,
}

pub async fn deploy(config: DeployConfig) -> Result<Vec<DeployResult>, String> {
    if config.servers.is_empty() {
        return Ok(vec![]);
    }

    let api_key = crate::platform::env::var("SOLI_DEPLOY_API_KEY").map_err(|_| {
        "SOLI_DEPLOY_API_KEY env var is required for the proxy deploy step. Set it before running `soli deploy`.".to_string()
    })?;

    println!("Phase 1: Syncing code to all servers...");
    let mut handles = Vec::new();

    for server in config.servers.clone() {
        match config.mode {
            DeployMode::Git => {
                let git_url = config.git_url.clone();
                let git_branch = config.git_branch.clone();
                let git_folder = config.git_folder.clone();
                let name = server.name.clone();
                handles.push((
                    name,
                    tokio::spawn(async move {
                        sync_code_git(&server, &git_url, &git_branch, &git_folder).await
                    }),
                ));
            }
            DeployMode::Bundle => {
                let bundle_local = config.bundle_source.clone().unwrap();
                let bundle_path = if bundle_local.starts_with('/') {
                    Path::new(&bundle_local).to_path_buf()
                } else {
                    config.source_path.join(&bundle_local)
                };
                if !bundle_path.exists() {
                    return Err(format!("Bundle not found at '{}'", bundle_path.display()));
                }
                let name = server.name.clone();
                handles.push((
                    name,
                    tokio::spawn(async move { sync_code_bundle(&server, &bundle_path).await }),
                ));
            }
            DeployMode::Local => {
                let source = config.source_path.clone();
                let mut excludes: Vec<String> =
                    DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).collect();
                excludes.extend(config.local_excludes.iter().cloned());
                let name = server.name.clone();
                handles.push((
                    name,
                    tokio::spawn(async move { sync_code_rsync(&server, &source, &excludes).await }),
                ));
            }
        }
    }

    // A server whose code did not sync must not be switched over: its new
    // slot would start on whatever was there before. (A failed sync used to be
    // dropped here, and phase 2 deployed every server regardless.)
    let mut results = Vec::new();
    let mut unsynced = std::collections::HashSet::new();
    for (name, handle) in handles {
        let failure = match handle.await {
            Ok(Ok(())) => continue,
            Ok(Err(e)) => e,
            Err(e) => format!("Task join error: {}", e),
        };
        eprintln!("[{}] Sync failed: {}", name, failure);
        results.push(DeployResult {
            server_name: name.clone(),
            success: false,
            message: format!("Sync failed: {}", failure),
            slot: None,
        });
        unsynced.insert(name);
    }
    if unsynced.len() == config.servers.len() {
        return Ok(results);
    }

    // On the first server that synced: the others may still hold old code.
    let migrate_on = config
        .servers
        .iter()
        .find(|server| !unsynced.contains(&server.name))
        .expect("at least one server synced");
    if let Err(e) = run_migrations(migrate_on, &config.git_folder, config.mode).await {
        eprintln!("[{}] Migration warning: {}", migrate_on.name, e);
    }

    println!();
    println!("Phase 2: Triggering blue-green deploy on all servers...");

    let mut deploy_handles = Vec::new();

    for server in config.servers.clone() {
        if unsynced.contains(&server.name) {
            continue;
        }
        let key = api_key.clone();
        let handle = tokio::spawn(async move { trigger_deploy(&server, &key).await });
        deploy_handles.push(handle);
    }

    for handle in deploy_handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => results.push(DeployResult {
                server_name: "unknown".to_string(),
                success: false,
                message: format!("Task join error: {}", e),
                slot: None,
            }),
        }
    }

    Ok(results)
}

async fn sync_code_git(
    server: &ServerConfig,
    git_url: &str,
    git_branch: &str,
    git_folder: &str,
) -> Result<(), String> {
    println!(
        "[{}] Connecting to {}@{}...",
        server.name, server.username, server.ip
    );

    let folder_exists = check_remote_folder_exists(server, &server.folder).await?;

    if folder_exists {
        println!("[{}] Folder exists, pulling latest changes...", server.name);
        git_pull(server, &server.folder, git_folder, git_branch).await?;
    } else {
        println!("[{}] Cloning repository...", server.name);
        git_clone(server, &server.folder, git_url, git_branch, git_folder).await?;
    }

    println!("[{}] Code synced ✓", server.name);

    Ok(())
}

async fn sync_code_bundle(server: &ServerConfig, bundle_path: &Path) -> Result<(), String> {
    let bundle_file = bundle_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("app.soli");
    let remote_path = format!("{}/{}", server.folder.trim_end_matches('/'), bundle_file);

    println!(
        "[{}] Copying bundle {} to {}@{}:{}...",
        server.name,
        bundle_path.display(),
        server.username,
        server.ip,
        remote_path
    );

    ensure_remote_folder(server).await?;

    // Streamed through `cat` on the server rather than `scp`, whose remote
    // path is shell-parsed by some OpenSSH versions and not by others.
    let bundle = std::fs::File::open(bundle_path)
        .map_err(|e| format!("Failed to read bundle '{}': {}", bundle_path.display(), e))?;
    let quoted = shell_quote(&remote_path);
    let upload = ssh_run(
        server,
        &format!("cat > {quoted} && chmod 644 {quoted}"),
        Stdio::from(bundle),
    )
    .await?;
    if !upload.success {
        return Err(format!(
            "Failed to copy bundle to {}: {}",
            remote_path,
            upload.stderr.trim()
        ));
    }

    println!("[{}] Bundle copied ✓", server.name);
    Ok(())
}

const DEFAULT_EXCLUDES: &[&str] = &[
    ".git/",
    "target/",
    "node_modules/",
    ".env",
    ".env.*",
    "sessions/",
    "*.log",
    ".DS_Store",
];

async fn sync_code_rsync(
    server: &ServerConfig,
    source: &Path,
    excludes: &[String],
) -> Result<(), String> {
    println!(
        "[{}] Rsyncing local files to {}@{}:{}...",
        server.name, server.username, server.ip, server.folder
    );

    ensure_remote_folder(server).await?;

    let source_arg = format!("{}/", source.display());
    let dest_arg = format!(
        "{}@{}:{}/",
        server.username,
        server.ip,
        server.folder.trim_end_matches('/')
    );

    let mut cmd = tokio::process::Command::new("rsync");
    cmd.arg("-avz").arg("--delete");
    for ex in excludes {
        cmd.arg(format!("--exclude={}", ex));
    }
    cmd.arg("-e").arg(format!("ssh {}", SSH_OPTIONS.join(" ")));
    cmd.arg(&source_arg);
    cmd.arg(&dest_arg);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("rsync spawn failed: {} (is rsync installed?)", e))?;

    if !output.status.success() {
        return Err(format!(
            "rsync failed: {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ));
    }

    println!("[{}] Code synced ✓", server.name);
    Ok(())
}

async fn ensure_remote_folder(server: &ServerConfig) -> Result<(), String> {
    let remote = ssh_run(
        server,
        &format!("mkdir -p {}", shell_quote(&server.folder)),
        Stdio::null(),
    )
    .await?;
    if !remote.success {
        return Err(format!(
            "mkdir -p {} failed: {} {}",
            server.folder, remote.stdout, remote.stderr
        ));
    }

    Ok(())
}

async fn run_migrations(
    server: &ServerConfig,
    git_folder: &str,
    mode: DeployMode,
) -> Result<(), String> {
    println!("[{}] Running database migrations...", server.name);

    let target = match mode {
        DeployMode::Local => server.folder.clone(),
        DeployMode::Git => {
            if git_folder == "/" || git_folder.is_empty() {
                server.folder.clone()
            } else {
                format!("{}/{}", server.folder, git_folder.trim_end_matches('/'))
            }
        }
        DeployMode::Bundle => {
            return Err("Migrations are not supported in bundle mode. Run `soli db:migrate up` locally before building the bundle.".to_string());
        }
    };

    let migration_cmd = format!("cd {} && soli db:migrate up", shell_quote(&target));

    let remote = ssh_run(server, &migration_cmd, Stdio::null()).await?;
    if !remote.success {
        return Err(format!(
            "Migration failed: {} {}",
            remote.stdout, remote.stderr
        ));
    }

    println!("[{}] Migrations completed ✓", server.name);

    Ok(())
}

async fn trigger_deploy(server: &ServerConfig, api_key: &str) -> DeployResult {
    println!("[{}] Triggering blue-green deploy...", server.name);

    let app_name = extract_app_name(&server.folder);

    match trigger_proxy_deploy(&server.proxy_url, api_key, &app_name) {
        Ok(slot) => {
            println!("[{}] Deploy started on slot {} ✓", server.name, slot);
            DeployResult {
                server_name: server.name.clone(),
                success: true,
                message: "Deployment successful".to_string(),
                slot: Some(slot),
            }
        }
        Err(e) => {
            println!("[{}] Deploy failed: {}", server.name, e);
            DeployResult {
                server_name: server.name.clone(),
                success: false,
                message: e,
                slot: None,
            }
        }
    }
}

async fn check_remote_folder_exists(server: &ServerConfig, folder: &str) -> Result<bool, String> {
    let remote = ssh_run(
        server,
        &format!("test -d {} && echo 'exists'", shell_quote(folder)),
        Stdio::null(),
    )
    .await?;
    Ok(remote.stdout.trim() == "exists")
}

async fn git_clone(
    server: &ServerConfig,
    folder: &str,
    git_url: &str,
    branch: &str,
    git_folder: &str,
) -> Result<(), String> {
    let target = if git_folder == "/" || git_folder.is_empty() {
        folder.to_string()
    } else {
        format!("{}/{}", folder, git_folder.trim_end_matches('/'))
    };

    let parent = Path::new(&target)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or(".");

    let clone_cmd = if git_folder == "/" || git_folder.is_empty() {
        format!(
            "mkdir -p {} && cd {} && git clone --branch {} {} .",
            shell_quote(&target),
            shell_quote(parent),
            shell_quote(branch),
            shell_quote(git_url)
        )
    } else {
        format!(
            "mkdir -p {} && cd {} && git clone --branch {} {} {}",
            shell_quote(&target),
            shell_quote(parent),
            shell_quote(branch),
            shell_quote(git_url),
            shell_quote(git_folder.trim_end_matches('/'))
        )
    };

    let remote = ssh_run(server, &clone_cmd, Stdio::null()).await?;
    if !remote.success {
        return Err(format!(
            "Git clone failed: {} {}",
            remote.stdout, remote.stderr
        ));
    }

    Ok(())
}

async fn git_pull(
    server: &ServerConfig,
    folder: &str,
    git_folder: &str,
    branch: &str,
) -> Result<(), String> {
    let target = if git_folder == "/" || git_folder.is_empty() {
        folder.to_string()
    } else {
        format!("{}/{}", folder, git_folder.trim_end_matches('/'))
    };

    let pull_cmd = format!(
        "cd {} && git pull origin {}",
        shell_quote(&target),
        shell_quote(branch)
    );

    let remote = ssh_run(server, &pull_cmd, Stdio::null()).await?;
    if !remote.success {
        return Err(format!(
            "Git pull failed: {} {}",
            remote.stdout, remote.stderr
        ));
    }

    Ok(())
}

fn extract_app_name(folder: &str) -> String {
    folder
        .split('/')
        .rfind(|s| !s.is_empty())
        .unwrap_or("app")
        .to_string()
}

fn trigger_proxy_deploy(proxy_url: &str, api_key: &str, app_name: &str) -> Result<String, String> {
    let url = format!(
        "{}/api/v1/apps/{}/deploy",
        proxy_url.trim_end_matches('/'),
        app_name
    );

    // SEC-042a: same TLS-1.2 floor as the shared runtime clients in
    // `http_class.rs`. `proxy_url` is operator-configured and reaches
    // out over HTTPS in real deployments.
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .min_tls_version(reqwest::tls::Version::TLS_1_2)
        .build()
        .map_err(|e| format!("HTTP client failed: {}", e))?;

    let response = client
        .post(&url)
        .header("X-Api-Key", api_key)
        .send()
        .map_err(|e| format!("Deploy request failed: {}", e))?;

    let status = response.status();
    let body = response.text().unwrap_or_default();

    if !status.is_success() {
        return Err(format!("Deploy API returned {}: {}", status, body));
    }

    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Failed to parse response: {}", e))?;

    let slot = json["data"]["slot"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    Ok(slot)
}

pub fn print_summary(results: &[DeployResult]) {
    let total = results.len();
    let succeeded = results.iter().filter(|r| r.success).count();
    let failed = total - succeeded;

    println!();
    if failed == 0 {
        println!("✓ {}/{} servers deployed successfully", succeeded, total);
    } else {
        println!(
            "✗ {}/{} servers deployed successfully, {} failed",
            succeeded, total, failed
        );
        for result in results.iter().filter(|r| !r.success) {
            println!("  [{}] Failed: {}", result.server_name, result.message);
        }
    }
}
