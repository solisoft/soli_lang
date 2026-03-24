use ssh2::Session;
use std::io::Read;
use std::net::TcpStream;
use std::path::Path;

#[derive(Clone)]
pub struct ServerConfig {
    pub name: String,
    pub username: String,
    pub ip: String,
    pub folder: String,
    pub api_key: String,
    pub proxy_url: String,
}

pub struct DeployConfig {
    pub git_url: String,
    pub git_branch: String,
    pub git_folder: String,
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
        return Err(format!(
            "deploy.toml not found in {}",
            folder.display()
        ));
    }

    let content = std::fs::read_to_string(&deploy_path)
        .map_err(|e| format!("Failed to read deploy.toml: {}", e))?;

    parse_deploy_toml(&content)
}

fn parse_deploy_toml(content: &str) -> Result<DeployConfig, String> {
    let mut git_url = None;
    let mut git_branch = "main".to_string();
    let mut git_folder = "/".to_string();
    let mut servers: Vec<ServerConfig> = Vec::new();

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
                    api_key: String::new(),
                    proxy_url: String::new(),
                });
            } else if line == "[servers]" {
                in_servers = true;
            } else {
                in_servers = false;
            }
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'');

            if in_servers {
                if let Some(server) = servers.last_mut() {
                    match key {
                        "name" => server.name = value.to_string(),
                        "username" => server.username = value.to_string(),
                        "ip" => server.ip = value.to_string(),
                        "folder" => server.folder = value.to_string(),
                        "api_key" => server.api_key = value.to_string(),
                        "proxy_url" => server.proxy_url = value.to_string(),
                        _ => {}
                    }
                }
            } else {
                match key {
                    "git_url" => git_url = Some(value.to_string()),
                    "git_branch" => git_branch = value.to_string(),
                    "git_folder" => git_folder = value.to_string(),
                    _ => {}
                }
            }
        }
    }

    let git_url = git_url.ok_or("git_url is required in deploy.toml")?;

    Ok(DeployConfig {
        git_url,
        git_branch,
        git_folder,
        servers,
    })
}

pub async fn deploy(config: DeployConfig) -> Vec<DeployResult> {
    let mut handles = Vec::new();

    for server in config.servers.clone() {
        let git_url = config.git_url.clone();
        let git_branch = config.git_branch.clone();
        let git_folder = config.git_folder.clone();

        let handle = tokio::spawn(async move {
            deploy_to_server(&server, &git_url, &git_branch, &git_folder).await
        });

        handles.push(handle);
    }

    let mut results = Vec::new();
    for handle in handles {
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

    results
}

async fn deploy_to_server(
    server: &ServerConfig,
    git_url: &str,
    git_branch: &str,
    git_folder: &str,
) -> DeployResult {
    println!(
        "[{}] Connecting to {}@{}...",
        server.name, server.username, server.ip
    );

    match ssh_connect(server).await {
        Ok(session) => {
            if let Err(e) = deploy_session(&session, server, git_url, git_branch, git_folder).await {
                DeployResult {
                    server_name: server.name.clone(),
                    success: false,
                    message: e,
                    slot: None,
                }
            } else {
                DeployResult {
                    server_name: server.name.clone(),
                    success: true,
                    message: "Deployment successful".to_string(),
                    slot: None,
                }
            }
        }
        Err(e) => DeployResult {
            server_name: server.name.clone(),
            success: false,
            message: e,
            slot: None,
        },
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

    let identities = agent.identities()
        .map_err(|e| format!("Failed to get SSH identities: {}", e))?;
    if identities.is_empty() {
        return Err("No SSH identities found. Make sure you have SSH keys added to your agent.".to_string());
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

async fn deploy_session(
    session: &Session,
    server: &ServerConfig,
    git_url: &str,
    git_branch: &str,
    git_folder: &str,
) -> Result<(), String> {
    let folder_exists = check_remote_folder_exists(session, &server.folder)?;

    if folder_exists {
        println!(
            "[{}] Folder exists, pulling latest changes...",
            server.name
        );
        git_pull(session, &server.folder, git_folder, git_branch)?;
    } else {
        println!("[{}] Cloning repository...", server.name);
        git_clone(session, &server.folder, git_url, git_branch, git_folder)?;
    }

    let app_name = extract_app_name(&server.folder);
    println!(
        "[{}] Triggering blue-green deploy for app '{}'...",
        server.name, app_name
    );

    let slot = trigger_proxy_deploy(&server.proxy_url, &server.api_key, &app_name)?;

    println!(
        "[{}] Deploy started on slot {} ✓",
        server.name, slot
    );

    Ok(())
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
            target, parent, branch, git_url, git_folder.trim_end_matches('/')
        )
    };

    let mut channel = session
        .channel_session()
        .map_err(|e| format!("Failed to open channel: {}", e))?;

    channel.exec(&clone_cmd).map_err(|e| format!("Git clone failed: {}", e))?;

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

fn git_pull(
    session: &Session,
    folder: &str,
    git_folder: &str,
    branch: &str,
) -> Result<(), String> {
    let target = if git_folder == "/" || git_folder.is_empty() {
        folder.to_string()
    } else {
        format!("{}/{}", folder, git_folder.trim_end_matches('/'))
    };

    let mut channel = session
        .channel_session()
        .map_err(|e| format!("Failed to open channel: {}", e))?;

    let pull_cmd = format!(
        "cd {} && git pull origin {}",
        target,
        branch
    );

    channel.exec(&pull_cmd).map_err(|e| format!("Git pull failed: {}", e))?;

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
        .filter(|s| !s.is_empty())
        .last()
        .unwrap_or("app")
        .to_string()
}

fn trigger_proxy_deploy(proxy_url: &str, api_key: &str, app_name: &str) -> Result<String, String> {
    let url = format!("{}/api/v1/apps/{}/deploy", proxy_url.trim_end_matches('/'), app_name);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
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

    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse response: {}", e))?;

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
        println!("✗ {}/{} servers deployed successfully, {} failed", succeeded, total, failed);
        for result in results.iter().filter(|r| !r.success) {
            println!("  [{}] Failed: {}", result.server_name, result.message);
        }
    }
}
