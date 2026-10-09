//! `soli mobile publish`: upload an artifact to an app's `/__soli/mobile`.
//!
//! The token is read from `SOLI_MOBILE_TOKEN`, never from argv, so it does not
//! show in `ps` or in a CI log that echoes commands. Without one the upload is
//! sent anyway: a `soli serve --dev` on this machine takes it unauthenticated,
//! and anywhere else the server's 401 or 404 is explained.

use std::path::Path;

use super::build::read_stub;
use super::Platform;

pub struct PublishOptions<'a> {
    pub file: &'a Path,
    /// Base URL of the app (`https://staging.example.com`).
    pub url: &'a str,
    pub notes: Option<&'a str>,
    pub token: Option<&'a str>,
}

#[derive(Debug)]
pub struct Published {
    pub install_url: String,
    pub download_url: String,
    pub version: String,
    pub build_number: String,
}

/// `<url>/__soli/mobile/builds`, for a base URL with or without a trailing `/`.
pub fn upload_url(base: &str) -> String {
    format!("{}/__soli/mobile/builds", base.trim_end_matches('/'))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn publish(opts: &PublishOptions<'_>) -> Result<Published, String> {
    let file_name = opts
        .file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let platform = Platform::from_file_name(&file_name)
        .ok_or_else(|| format!("{} is not a .apk or .ipa", opts.file.display()))?;
    if !opts.file.is_file() {
        return Err(format!("{} does not exist", opts.file.display()));
    }
    if !(opts.url.starts_with("https://") || opts.url.starts_with("http://")) {
        return Err(format!(
            "the publish URL must start with https:// or http://, got {:?}",
            opts.url
        ));
    }

    let mut form = reqwest::blocking::multipart::Form::new().text("platform", platform.as_str());
    // What `soli mobile build` recorded beside the artifact. Without it the
    // server defaults the version, and refuses an iOS build (no bundle id).
    if let Some(stub) = read_stub(opts.file) {
        form = form
            .text("version", stub.version)
            .text("build_number", stub.build_number)
            .text("name", stub.name);
        if !stub.bundle_id.is_empty() {
            form = form.text("bundle_id", stub.bundle_id);
        }
    }
    if let Some(notes) = opts.notes {
        form = form.text("notes", notes.to_string());
    }
    form = form
        .file("file", opts.file)
        .map_err(|e| format!("cannot read {}: {e}", opts.file.display()))?;

    // No overall timeout: an IPA over a slow uplink takes as long as it takes.
    let client = reqwest::blocking::Client::builder()
        .timeout(None)
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut request = client.post(upload_url(opts.url)).multipart(form);
    if let Some(token) = opts.token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .map_err(|e| format!("upload to {} failed: {e}", opts.url))?;
    let status = response.status().as_u16();
    let body: serde_json::Value = response.json().unwrap_or(serde_json::Value::Null);
    let field = |name: &str| {
        body.get(name)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    match status {
        201 => Ok(Published {
            install_url: field("install_url"),
            download_url: field("download_url"),
            version: field("version"),
            build_number: field("build_number"),
        }),
        401 if opts.token.is_none() => Err(
            "401: the server wants a token. Set SOLI_MOBILE_TOKEN to the value configured on it"
                .to_string(),
        ),
        401 => Err("401: the server refused SOLI_MOBILE_TOKEN".to_string()),
        404 => Err(format!(
            "404: {} is not enabled there. Set SOLI_MOBILE_TOKEN (or SOLI_ADMIN_TOKEN) on the \
             server, or run it with --dev",
            upload_url(opts.url)
        )),
        413 => Err(format!("413: {}", field("error"))),
        _ => Err(format!(
            "{status}: {}",
            Some(field("error"))
                .filter(|e| !e.is_empty())
                .unwrap_or_else(|| "the upload was refused".to_string())
        )),
    }
}
#[cfg(target_arch = "wasm32")]
pub fn publish(_opts: &PublishOptions<'_>) -> Result<Published, String> {
    Err(crate::platform::unsupported_on_edge("publish"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_url_joins_the_path_once() {
        assert_eq!(
            upload_url("https://a.test/"),
            "https://a.test/__soli/mobile/builds"
        );
        assert_eq!(
            upload_url("http://localhost:5011"),
            "http://localhost:5011/__soli/mobile/builds"
        );
    }

    #[test]
    fn refuses_a_file_that_is_not_an_artifact() {
        let err = publish(&PublishOptions {
            file: Path::new("notes.txt"),
            url: "https://a.test",
            notes: None,
            token: None,
        })
        .unwrap_err();
        assert!(err.contains(".apk or .ipa"), "{err}");
    }
}
