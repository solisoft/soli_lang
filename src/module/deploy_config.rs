//! `deploy.toml` parsing.
//!
//! Split out of `deploy` deliberately: the deploy machinery is built on ssh2
//! and so is Unix-only, but reading the file is plain string work and several
//! commands that *do* build everywhere need it — `soli cloud` reads its target
//! server from here, and `soli env down` reads the proxy URL. Keeping the
//! parser in the gated module made those commands fail to compile on Windows.

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
