//! HTTP client built-in functions (deprecated).
//!
//! This module is deprecated. Use the HTTP class instead:
//! - HTTP.get(url) instead of http_get(url)
//! - HTTP.post(url, body) instead of http_post(url, body)
//! - etc.
//!
//! The standalone functions have been removed in favor of the HTTP class API.

use std::net::{IpAddr, ToSocketAddrs};

use crate::interpreter::environment::Environment;

const BLOCKED_SCHEMES: &[&str] = &["javascript", "file", "ftp", "ssh", "telnet", "gopher"];

pub fn validate_url_for_ssrf(url: &str) -> Result<(), String> {
    let url = url.trim();

    if url.is_empty() {
        return Err("URL cannot be empty".to_string());
    }

    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) => (s.to_lowercase(), r),
        None => {
            return Err("URL must have a scheme (e.g., http:// or https://)".to_string());
        }
    };

    if scheme.is_empty() {
        return Err("URL scheme cannot be empty".to_string());
    }

    if BLOCKED_SCHEMES.contains(&scheme.as_str()) {
        return Err(format!(
            "URL scheme '{}:' is not allowed for security reasons",
            scheme
        ));
    }

    if scheme != "http" && scheme != "https" {
        return Err("Only HTTP and HTTPS URLs are allowed".to_string());
    }

    let host = if let Some((h, _)) = rest.split_once('/') {
        if let Some((_, h2)) = h.split_once('@') {
            h2
        } else {
            h
        }
    } else if let Some((_, h)) = rest.split_once('@') {
        h
    } else {
        rest
    };

    let host = if let Some((h, _)) = host.split_once(':') {
        h
    } else {
        host
    };

    if host.is_empty() {
        return Err("URL host cannot be empty".to_string());
    }

    if is_blocked_host(host) {
        return Err("Access to private/localhost addresses is not allowed".to_string());
    }

    Ok(())
}

fn is_blocked_host(host: &str) -> bool {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_blocked_ip(ip);
    }

    let lower_host = host.to_lowercase();
    if lower_host == "localhost"
        || lower_host == "localhost."
        || lower_host.starts_with("localhost.")
    {
        return true;
    }

    if let Ok(addrs) = (host, 0u16).to_socket_addrs() {
        for addr in addrs {
            if is_blocked_ip(addr.ip()) {
                return true;
            }
        }
    }

    false
}

fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            let octets = ipv4.octets();
            if octets[0] == 127 {
                return true;
            }
            if octets[0] == 10 {
                return true;
            }
            if octets[0] == 172 && (octets[1] & 0xf0) == 16 {
                return true;
            }
            if octets[0] == 192 && octets[1] == 168 {
                return true;
            }
            if octets[0] == 169 && octets[1] == 254 {
                return true;
            }
            if octets[0] == 0 {
                return true;
            }
            false
        }
        IpAddr::V6(ipv6) => {
            if ip.is_loopback() {
                return true;
            }
            let octets = ipv6.octets();
            if octets[0] & 0xfe == 0xfc {
                return true;
            }
            if octets[0] == 0xfe && octets[1] == 0x80 {
                return true;
            }
            false
        }
    }
}

pub fn register_http_builtins(_env: &mut Environment) {}
