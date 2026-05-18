use ssh2::Session;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeployMode {
    Git,
    Local,
    Bundle,
}

#[derive(Clone)]
pub struct ServerConfig {
    pub name: String,
    pub username: String,
    pub ip: String,
    pub folder: String,
    pub proxy_url: String,
}

pub struct DeployConfig {
    pub mode: DeployMode,
    pub source_path: PathBuf,
    pub git_url: String,
    pub git_branch: String,
    pub git_folder: String,
    pub local_excludes: Vec<String>,
    pub bundle_source: Option<String>,
    pub servers: Vec<ServerConfig>,
}

#[derive(Clone)]
pub struct DeployResult {
    pub server_name: String,
    pub success: bool,
    pub message: String,
    pub slot: Option<String>,
}

pub fn load_deploy_config(folder: &Path) -> Result<DeployConfig, String> {
    let deploy_path = folder.join("deploy.toml");
    if !deploy_path.exists() {
        return Err(format!("deploy.toml not found in {}", folder.display()));
    }

    let content = std::fs::read_to_string(&deploy_path)
        .map_err(|e| format!("Failed to read deploy.toml: {}", e))?;

    let mut config = parse_deploy_toml(&content)?;
    config.source_path = folder.to_path_buf();
    Ok(config)
}

fn parse_array_line(value: &str) -> Vec<String> {
    let inner = value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim();
    if inner.is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_deploy_toml(content: &str) -> Result<DeployConfig, String> {
    let mut mode = DeployMode::Git;
    let mut git_url: Option<String> = None;
    let mut git_branch = "main".to_string();
    let mut git_folder = "/".to_string();
    let mut local_excludes: Vec<String> = Vec::new();
    let mut bundle_source: Option<String> = None;
    let mut servers: Vec<ServerConfig> = Vec::new();
    let mut warned_about_api_key = false;

    let mut in_servers = false;

    for line in content.lines() {
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') {
            if line.starts_with("[[servers]]") {
                in_servers = true;
                servers.push(ServerConfig {
                    name: String::new(),
                    username: String::new(),
                    ip: String::new(),
                    folder: String::new(),
                    proxy_url: String::new(),
                });
            } else {
                in_servers = line == "[servers]";
            }
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let raw_value = value.trim();

            if in_servers {
                if let Some(server) = servers.last_mut() {
                    let value = raw_value.trim_matches('"').trim_matches('\'');
                    match key {
                        "name" => server.name = value.to_string(),
                        "username" => server.username = value.to_string(),
                        "ip" => server.ip = value.to_string(),
                        "folder" => server.folder = value.to_string(),
                        "proxy_url" => server.proxy_url = value.to_string(),
                        "api_key" if !warned_about_api_key => {
                            eprintln!(
                                "warning: deploy.toml `api_key` is ignored — set the SOLI_DEPLOY_API_KEY env var instead and remove this line (deploy.toml is committed)."
                            );
                            warned_about_api_key = true;
                        }
                        _ => {}
                    }
                }
            } else {
                match key {
                    "mode" => {
                        let value = raw_value.trim_matches('"').trim_matches('\'');
                        mode = match value {
                            "git" => DeployMode::Git,
                            "local" => DeployMode::Local,
                            "bundle" => DeployMode::Bundle,
                            other => {
                                return Err(format!(
                                    "invalid mode `{}` in deploy.toml (expected \"git\", \"local\", or \"bundle\")",
                                    other
                                ));
                            }
                        };
                    }
                    "bundle_source" => {
                        bundle_source =
                            Some(raw_value.trim_matches('"').trim_matches('\'').to_string());
                    }
                    "git_url" => {
                        git_url = Some(raw_value.trim_matches('"').trim_matches('\'').to_string());
                    }
                    "git_branch" => {
                        git_branch = raw_value.trim_matches('"').trim_matches('\'').to_string();
                    }
                    "git_folder" => {
                        git_folder = raw_value.trim_matches('"').trim_matches('\'').to_string();
                    }
                    "local_excludes" => {
                        local_excludes = parse_array_line(raw_value);
                    }
                    _ => {}
                }
            }
        }
    }

    if mode == DeployMode::Git && git_url.is_none() {
        return Err("git_url is required in deploy.toml when mode is \"git\"".to_string());
    }

    if mode == DeployMode::Bundle && bundle_source.is_none() {
        return Err("bundle_source is required in deploy.toml when mode is \"bundle\"".to_string());
    }

    Ok(DeployConfig {
        mode,
        source_path: PathBuf::new(),
        git_url: git_url.unwrap_or_default(),
        git_branch,
        git_folder,
        local_excludes,
        bundle_source,
        servers,
    })
}

pub async fn deploy(config: DeployConfig) -> Result<Vec<DeployResult>, String> {
    if config.servers.is_empty() {
        return Ok(vec![]);
    }

    let api_key = std::env::var("SOLI_DEPLOY_API_KEY").map_err(|_| {
        "SOLI_DEPLOY_API_KEY env var is required for the proxy deploy step. Set it before running `soli deploy`.".to_string()
    })?;

    let first_server = config.servers[0].clone();

    println!("Phase 1: Syncing code to all servers...");
    let mut handles = Vec::new();

    for server in config.servers.clone() {
        match config.mode {
            DeployMode::Git => {
                let git_url = config.git_url.clone();
                let git_branch = config.git_branch.clone();
                let git_folder = config.git_folder.clone();
                handles.push(tokio::spawn(async move {
                    sync_code_git(&server, &git_url, &git_branch, &git_folder).await
                }));
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
                handles.push(tokio::spawn(async move {
                    sync_code_bundle(&server, &bundle_path).await
                }));
            }
            DeployMode::Local => {
                let source = config.source_path.clone();
                let mut excludes: Vec<String> =
                    DEFAULT_EXCLUDES.iter().map(|s| s.to_string()).collect();
                excludes.extend(config.local_excludes.iter().cloned());
                handles.push(tokio::spawn(async move {
                    sync_code_rsync(&server, &source, &excludes).await
                }));
            }
        }
    }

    for handle in handles {
        if let Err(e) = handle.await {
            eprintln!("Task join error: {}", e);
        }
    }

    if let Err(e) = run_migrations(&first_server, &config.git_folder, config.mode).await {
        eprintln!("[{}] Migration warning: {}", first_server.name, e);
    }

    println!();
    println!("Phase 2: Triggering blue-green deploy on all servers...");

    let mut deploy_handles = Vec::new();

    for server in config.servers.clone() {
        let key = api_key.clone();
        let handle = tokio::spawn(async move { trigger_deploy(&server, &key).await });
        deploy_handles.push(handle);
    }

    let mut results = Vec::new();
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

    let session = ssh_connect(server).await?;

    let folder_exists = check_remote_folder_exists(&session, &server.folder)?;

    if folder_exists {
        println!("[{}] Folder exists, pulling latest changes...", server.name);
        git_pull(&session, &server.folder, git_folder, git_branch)?;
    } else {
        println!("[{}] Cloning repository...", server.name);
        git_clone(&session, &server.folder, git_url, git_branch, git_folder)?;
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

    // Read bundle and scp it via SSH channel
    let bundle_data = std::fs::read(bundle_path)
        .map_err(|e| format!("Failed to read bundle '{}': {}", bundle_path.display(), e))?;

    let session = ssh_connect(server).await?;
    let size: u64 = bundle_data
        .len()
        .try_into()
        .map_err(|_| "Bundle too large for SCP".to_string())?;
    let mut channel = session
        .scp_send(std::path::Path::new(&remote_path), 0o644, size, None)
        .map_err(|e| format!("Failed to start SCP to {}: {}", remote_path, e))?;

    channel
        .write_all(&bundle_data)
        .map_err(|e| format!("Failed to write bundle via SCP: {}", e))?;

    channel.send_eof().ok();
    channel.wait_close().ok();

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
    cmd.arg("-e").arg("ssh -o StrictHostKeyChecking=accept-new");
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
    let session = ssh_connect(server).await?;
    let mut channel = session
        .channel_session()
        .map_err(|e| format!("Failed to open channel: {}", e))?;

    channel
        .exec(&format!("mkdir -p {}", server.folder))
        .map_err(|e| format!("Failed to mkdir on remote: {}", e))?;

    let mut stderr = String::new();
    channel.stderr().read_to_string(&mut stderr).ok();
    let mut stdout = String::new();
    channel.read_to_string(&mut stdout).ok();
    channel.wait_close().ok();

    if channel.exit_status().map(|s| s != 0).unwrap_or(false) {
        return Err(format!(
            "mkdir -p {} failed: {} {}",
            server.folder, stdout, stderr
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

    let session = ssh_connect(server).await?;

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

    let migration_cmd = format!("cd {} && soli db:migrate up", target);

    let mut channel = session
        .channel_session()
        .map_err(|e| format!("Failed to open channel: {}", e))?;

    channel
        .exec(&migration_cmd)
        .map_err(|e| format!("Migration command failed: {}", e))?;

    let mut stderr = String::new();
    channel
        .stderr()
        .read_to_string(&mut stderr)
        .map_err(|e| format!("Failed to read stderr: {}", e))?;

    let mut stdout = String::new();
    channel
        .read_to_string(&mut stdout)
        .map_err(|e| format!("Failed to read stdout: {}", e))?;

    channel.wait_close().ok();

    if channel.exit_status().map(|s| s != 0).unwrap_or(false) {
        return Err(format!("Migration failed: {} {}", stdout, stderr));
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

async fn ssh_connect(server: &ServerConfig) -> Result<Session, String> {
    let tcp = TcpStream::connect(format!("{}:22", server.ip))
        .map_err(|e| format!("TCP connection failed: {}", e))?;

    let mut session = Session::new().map_err(|e| format!("SSH session creation failed: {}", e))?;
    session.set_tcp_stream(tcp);
    session
        .handshake()
        .map_err(|e| format!("SSH handshake failed: {}", e))?;

    let mut agent = session
        .agent()
        .map_err(|e| format!("SSH agent failed: {}", e))?;
    agent
        .connect()
        .map_err(|e| format!("SSH agent connect failed: {}", e))?;

    agent
        .list_identities()
        .map_err(|e| format!("Failed to list SSH identities: {}", e))?;

    let identities = agent
        .identities()
        .map_err(|e| format!("Failed to get SSH identities: {}", e))?;
    if identities.is_empty() {
        return Err(
            "No SSH identities found. Make sure you have SSH keys added to your agent.".to_string(),
        );
    }

    let identity = &identities[0];
    agent
        .userauth(&server.username, identity)
        .map_err(|e| format!("SSH authentication failed: {}", e))?;

    if !session.authenticated() {
        return Err("SSH authentication failed".to_string());
    }

    Ok(session)
}

fn check_remote_folder_exists(session: &Session, folder: &str) -> Result<bool, String> {
    let mut channel = session
        .channel_session()
        .map_err(|e| format!("Failed to open channel: {}", e))?;

    channel
        .exec(&format!("test -d {} && echo 'exists'", folder))
        .map_err(|e| format!("Failed to execute: {}", e))?;

    let mut output = String::new();
    channel
        .read_to_string(&mut output)
        .map_err(|e| format!("Failed to read output: {}", e))?;

    channel.wait_close().ok();

    Ok(output.trim() == "exists")
}

fn git_clone(
    session: &Session,
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
            target, parent, branch, git_url
        )
    } else {
        format!(
            "mkdir -p {} && cd {} && git clone --branch {} {} {}",
            target,
            parent,
            branch,
            git_url,
            git_folder.trim_end_matches('/')
        )
    };

    let mut channel = session
        .channel_session()
        .map_err(|e| format!("Failed to open channel: {}", e))?;

    channel
        .exec(&clone_cmd)
        .map_err(|e| format!("Git clone failed: {}", e))?;

    let mut stderr = String::new();
    channel
        .stderr()
        .read_to_string(&mut stderr)
        .map_err(|e| format!("Failed to read stderr: {}", e))?;

    let mut stdout = String::new();
    channel
        .read_to_string(&mut stdout)
        .map_err(|e| format!("Failed to read stdout: {}", e))?;

    channel.wait_close().ok();

    if channel.exit_status().map(|s| s != 0).unwrap_or(false) {
        return Err(format!("Git clone failed: {} {}", stdout, stderr));
    }

    Ok(())
}

fn git_pull(session: &Session, folder: &str, git_folder: &str, branch: &str) -> Result<(), String> {
    let target = if git_folder == "/" || git_folder.is_empty() {
        folder.to_string()
    } else {
        format!("{}/{}", folder, git_folder.trim_end_matches('/'))
    };

    let mut channel = session
        .channel_session()
        .map_err(|e| format!("Failed to open channel: {}", e))?;

    let pull_cmd = format!("cd {} && git pull origin {}", target, branch);

    channel
        .exec(&pull_cmd)
        .map_err(|e| format!("Git pull failed: {}", e))?;

    let mut stderr = String::new();
    channel
        .stderr()
        .read_to_string(&mut stderr)
        .map_err(|e| format!("Failed to read stderr: {}", e))?;

    let mut stdout = String::new();
    channel
        .read_to_string(&mut stdout)
        .map_err(|e| format!("Failed to read stdout: {}", e))?;

    channel.wait_close().ok();

    if channel.exit_status().map(|s| s != 0).unwrap_or(false) {
        return Err(format!("Git pull failed: {} {}", stdout, stderr));
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
