//! `/__soli/mobile`: the app distributes its own native builds.
//!
//! `soli mobile publish` uploads an APK or IPA here; the page lists the
//! builds with QR codes, and each build gets a public-by-secret install link a
//! phone can open without signing in.
//!
//! * The page and the upload are behind [`admin_auth`] (`SOLI_MOBILE_*`, or the
//!   shared `SOLI_ADMIN_*`), and answer 404 when nothing is configured, so the
//!   page is off in production unless an environment turns it on.
//! * An upload is streamed to disk with a hard cap. It never reaches a worker,
//!   whose buffered body path stops at 8 MiB, which is why [`dispatch`] owns
//!   the request the way `dev_routes::dispatch` does.
//! * A build lives in `SOLI_MOBILE_PATH/<id>/`: the binary under a name made
//!   here, never the client's, and `build.json` beside it. Metadata stays on
//!   the same disk as the file it describes; a database row would point at a
//!   file only one host has.
//! * Install links (`/__soli/mobile/i/<token>`) skip the gate. iOS fetches the
//!   manifest and the IPA itself, without Safari's cookies, and a QR code is
//!   scanned on a device that never signed in. Each build has its own random
//!   token, revoked by deleting the build; `SOLI_MOBILE_PUBLIC_INSTALL=0` turns
//!   them off.

use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::header::HeaderMap;
use hyper::{Request, Response, StatusCode};
use serde::{Deserialize, Serialize};

use crate::interpreter::builtins::crypto::do_secure_compare;
use crate::mobile::{ios_manifest, qr, stamp, Platform};

use super::operator_shell::{self, Section};
use super::{admin_auth, dev_bar, full, html_ok, static_files, ResponseBody};

const BASE: &str = "/__soli/mobile";
const DEFAULT_MAX_SIZE: u64 = 512 * 1024 * 1024;
const DEFAULT_KEEP: usize = 20;
const MAX_NOTES: usize = 10 * 1024;
const MAX_SHORT_FIELD: usize = 256;

const INTRO: &str = "Native builds of this app, uploaded with <code>soli mobile publish</code>. \
Scan a code with the phone's camera to install.";

/// One stored build: `build.json` in its directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Build {
    id: String,
    token: String,
    platform: String,
    name: String,
    bundle_id: String,
    version: String,
    build_number: String,
    notes: String,
    size: u64,
    sha256: String,
    file: String,
    created_at: String,
}

impl Build {
    fn platform(&self) -> Platform {
        Platform::parse(&self.platform).unwrap_or(Platform::Android)
    }
}

fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn root() -> PathBuf {
    env_nonempty("SOLI_MOBILE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("./storage/mobile"))
}

fn max_size() -> u64 {
    env_nonempty("SOLI_MOBILE_MAX_SIZE")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_MAX_SIZE)
}

fn keep() -> usize {
    env_nonempty("SOLI_MOBILE_KEEP")
        .and_then(|v| v.trim().parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(DEFAULT_KEEP)
}

fn public_install_on() -> bool {
    env_nonempty("SOLI_MOBILE_PUBLIC_INSTALL").as_deref() != Some("0")
}

/// Install links answer only where the page could exist: in `--dev`, or with
/// a credential configured. A build left on disk after the credentials are
/// removed is not served.
fn install_links_on(dev_mode: bool) -> bool {
    public_install_on() && (dev_mode || admin_auth::is_configured("MOBILE"))
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn valid_token(token: &str) -> bool {
    token.len() == 32
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn new_token() -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn esc(value: &str) -> String {
    dev_bar::html_escape(value)
}

fn human_size(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB)
    } else {
        format!("{:.0} KB", (bytes as f64 / 1024.0).ceil())
    }
}

// ---------------------------------------------------------------- storage

fn read_build(dir: &Path) -> Option<Build> {
    let text = std::fs::read_to_string(dir.join("build.json")).ok()?;
    let build: Build = serde_json::from_str(&text).ok()?;
    valid_id(&build.id).then_some(build)
}

/// Every stored build, newest first.
fn list_builds(root: &Path) -> Vec<Build> {
    let mut builds: Vec<Build> = std::fs::read_dir(root)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| valid_id(&e.file_name().to_string_lossy()))
                .filter_map(|e| read_build(&e.path()))
                .collect()
        })
        .unwrap_or_default();
    builds.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    builds
}

/// The build an install token names. Every token is compared, so the time
/// taken does not say how close a guess came.
fn find_by_token(root: &Path, token: &str) -> Option<Build> {
    if !valid_token(token) {
        return None;
    }
    let mut found = None;
    for build in list_builds(root) {
        if do_secure_compare(&build.token, token) && found.is_none() {
            found = Some(build);
        }
    }
    found
}

fn delete_build(root: &Path, id: &str) -> bool {
    valid_id(id) && std::fs::remove_dir_all(root.join(id)).is_ok()
}

/// Keep the newest `keep` builds of `platform`; delete the rest with their files.
fn prune(root: &Path, platform: Platform, keep: usize) -> usize {
    list_builds(root)
        .into_iter()
        .filter(|b| b.platform() == platform)
        .skip(keep)
        .filter(|b| delete_build(root, &b.id))
        .count()
}

// ---------------------------------------------------------------- dispatch

/// Is this one of the page's paths (any method)?
pub(crate) fn is_mobile_path(path: &str) -> bool {
    path == BASE || path.starts_with("/__soli/mobile/")
}

/// The page, the upload, or an install link; the request back on a miss.
///
/// See `dev_routes::dispatch` for why the `Err` is the whole request.
#[allow(clippy::result_large_err)]
pub(super) async fn dispatch(
    req: Request<Incoming>,
    method: &str,
    path: &str,
    peer_addr: SocketAddr,
    dev_mode: bool,
) -> Result<Response<ResponseBody>, Request<Incoming>> {
    if !is_mobile_path(path) {
        return Err(req);
    }
    let root = root();

    // Public by secret: no gate, but only while the page itself is on.
    if let Some(rest) = path.strip_prefix("/__soli/mobile/i/") {
        if !matches!(method, "GET" | "HEAD") || !install_links_on(dev_mode) {
            return Ok(admin_auth::hidden_not_found());
        }
        return Ok(handle_install(&root, rest, req.headers()));
    }

    let decision = admin_auth::authorize(req.headers(), dev_mode, peer_addr.ip(), "MOBILE");
    if let Some(refused) = admin_auth::refusal(decision, "Soli mobile") {
        return Ok(refused);
    }

    match (method, path) {
        ("GET" | "HEAD", BASE) => Ok(handle_index(&root, req.headers(), dev_mode)),
        ("POST", "/__soli/mobile/builds") => {
            // A token, never a browser's Basic credentials: an upload is a
            // CLI or CI call, and the Origin gate already refused a
            // cross-site one.
            if bearer_less_basic(req.headers()) {
                return Ok(json_error(
                    StatusCode::UNAUTHORIZED,
                    "uploads take `Authorization: Bearer <SOLI_MOBILE_TOKEN>`",
                ));
            }
            Ok(handle_upload(&root, req, dev_mode).await)
        }
        ("POST", _) => {
            let id = path
                .strip_prefix("/__soli/mobile/builds/")
                .and_then(|rest| rest.strip_suffix("/delete"));
            match id {
                Some(id) if valid_id(id) => {
                    delete_build(&root, id);
                    Ok(see_other(BASE))
                }
                _ => Ok(admin_auth::hidden_not_found()),
            }
        }
        _ => Ok(admin_auth::hidden_not_found()),
    }
}

fn bearer_less_basic(headers: &HeaderMap) -> bool {
    headers
        .get(hyper::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("Basic "))
}

fn see_other(location: &str) -> Response<ResponseBody> {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header("Location", location)
        .body(full(Bytes::new()))
        .unwrap()
}

fn json_response(status: StatusCode, body: serde_json::Value) -> Response<ResponseBody> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(full(Bytes::from(body.to_string())))
        .unwrap()
}

fn json_error(status: StatusCode, message: &str) -> Response<ResponseBody> {
    json_response(status, serde_json::json!({ "error": message }))
}

/// `scheme://host` this request reached, for the absolute URLs iOS and QR
/// codes need. `X-Forwarded-*` count only under `SOLI_TRUST_PROXY`, as for
/// every other URL the server builds.
fn base_url(headers: &HeaderMap) -> String {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let trust = crate::interpreter::builtins::trust_proxy::is_trust_proxy_enabled();
    let scheme = if trust && header("x-forwarded-proto").as_deref() == Some("https") {
        "https"
    } else {
        "http"
    };
    let host = trust
        .then(|| header("x-forwarded-host"))
        .flatten()
        .or_else(|| header("host"))
        .unwrap_or_else(|| "localhost".to_string());
    format!("{scheme}://{host}")
}

/// The base for install links and QR codes, which a *phone* opens.
///
/// Under `--dev` the page is only open to this machine, so it is reached as
/// `localhost` — a name the phone cannot resolve to this machine. `--dev`
/// binds every interface, and install links skip the gate, so a loopback host
/// is swapped for this machine's LAN address, port kept.
fn install_base(headers: &HeaderMap, dev_mode: bool) -> String {
    let base = base_url(headers);
    if !dev_mode {
        return base;
    }
    match lan_ip() {
        Some(ip) => with_lan_host(&base, ip),
        None => base,
    }
}

/// `base` with a loopback host replaced by `ip`; any other host unchanged.
fn with_lan_host(base: &str, ip: std::net::IpAddr) -> String {
    let Some((scheme, authority)) = base.split_once("://") else {
        return base.to_string();
    };
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        match bracketed.split_once(']') {
            Some((host, tail)) => (host, tail.strip_prefix(':')),
            None => return base.to_string(),
        }
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        }
    };
    let host_lower = host.to_ascii_lowercase();
    let loopback = host_lower == "localhost"
        || host_lower.ends_with(".localhost")
        || host_lower
            .parse::<std::net::IpAddr>()
            .is_ok_and(|addr| addr.is_loopback());
    if !loopback {
        return base.to_string();
    }
    let ip = match ip {
        std::net::IpAddr::V6(v6) => format!("[{v6}]"),
        v4 => v4.to_string(),
    };
    match port {
        Some(port) => format!("{scheme}://{ip}:{port}"),
        None => format!("{scheme}://{ip}"),
    }
}

/// This machine's address on the network its default route uses. Connecting
/// a UDP socket sends nothing; it only makes the OS pick the source address.
fn lan_ip() -> Option<std::net::IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:80").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

// ---------------------------------------------------------------- upload

struct UploadFields {
    platform: Option<String>,
    version: Option<String>,
    build_number: Option<String>,
    notes: Option<String>,
    bundle_id: Option<String>,
    name: Option<String>,
}

enum UploadError {
    TooLarge,
    Invalid(String),
    Server(String),
}

async fn handle_upload(
    root: &Path,
    req: Request<Incoming>,
    dev_mode: bool,
) -> Response<ResponseBody> {
    let base = install_base(req.headers(), dev_mode);
    match receive(root, req).await {
        Ok(build) => {
            let pruned = prune(root, build.platform(), keep());
            let install = format!("{base}{BASE}/i/{}", build.token);
            json_response(
                StatusCode::CREATED,
                serde_json::json!({
                    "id": build.id,
                    "platform": build.platform,
                    "version": build.version,
                    "build_number": build.build_number,
                    "size": build.size,
                    "sha256": build.sha256,
                    "install_url": install,
                    "download_url": format!("{install}/download"),
                    "pruned": pruned,
                }),
            )
        }
        Err(UploadError::TooLarge) => json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            &format!(
                "the build is over SOLI_MOBILE_MAX_SIZE ({} bytes)",
                max_size()
            ),
        ),
        Err(UploadError::Invalid(message)) => {
            json_error(StatusCode::UNPROCESSABLE_ENTITY, &message)
        }
        Err(UploadError::Server(message)) => {
            eprintln!("[soli] /__soli/mobile upload failed: {message}");
            json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "the build could not be stored",
            )
        }
    }
}

/// Stream the multipart body: the file to a temporary file under `root`
/// (same filesystem, so the final move is a rename), the fields to memory
/// with small caps. Then validate, and move the file into `<root>/<id>/`.
async fn receive(root: &Path, req: Request<Incoming>) -> Result<Build, UploadError> {
    let cap = max_size();
    let declared = req
        .headers()
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    // The multipart framing adds a few hundred bytes around the file.
    if declared.is_some_and(|n| n > cap.saturating_add(64 * 1024)) {
        return Err(UploadError::TooLarge);
    }
    let boundary = req
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|ct| multer::parse_boundary(ct).ok())
        .ok_or_else(|| UploadError::Invalid("expected multipart/form-data".into()))?;

    let tmp_dir = root.join(".tmp");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| UploadError::Server(e.to_string()))?;
    let tmp = tempfile::NamedTempFile::new_in(&tmp_dir)
        .map_err(|e| UploadError::Server(e.to_string()))?;

    let mut multipart = multer::Multipart::new(req.into_body().into_data_stream(), boundary);
    let mut fields = UploadFields {
        platform: None,
        version: None,
        build_number: None,
        notes: None,
        bundle_id: None,
        name: None,
    };
    let mut file_name: Option<String> = None;
    let mut size: u64 = 0;
    let mut hasher = {
        use sha2::Digest;
        sha2::Sha256::new()
    };

    let bad_body = |e: multer::Error| UploadError::Invalid(format!("malformed upload: {e}"));
    while let Some(mut field) = multipart.next_field().await.map_err(bad_body)? {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            if file_name.is_some() {
                return Err(UploadError::Invalid("more than one `file` part".into()));
            }
            file_name = Some(field.file_name().unwrap_or("").to_string());
            let mut out = tmp.as_file();
            while let Some(chunk) = field.chunk().await.map_err(bad_body)? {
                size += chunk.len() as u64;
                if size > cap {
                    return Err(UploadError::TooLarge);
                }
                {
                    use sha2::Digest;
                    hasher.update(&chunk);
                }
                out.write_all(&chunk)
                    .map_err(|e| UploadError::Server(e.to_string()))?;
            }
            continue;
        }
        let limit = if name == "notes" {
            MAX_NOTES
        } else {
            MAX_SHORT_FIELD
        };
        let mut value = Vec::new();
        while let Some(chunk) = field.chunk().await.map_err(bad_body)? {
            value.extend_from_slice(&chunk);
            if value.len() > limit {
                return Err(UploadError::Invalid(format!(
                    "`{name}` is over {limit} bytes"
                )));
            }
        }
        let value = String::from_utf8_lossy(&value).trim().to_string();
        let slot = match name.as_str() {
            "platform" => &mut fields.platform,
            "version" => &mut fields.version,
            "build_number" => &mut fields.build_number,
            "notes" => &mut fields.notes,
            "bundle_id" => &mut fields.bundle_id,
            "name" => &mut fields.name,
            _ => continue,
        };
        *slot = Some(value).filter(|v| !v.is_empty());
    }

    let Some(file_name) = file_name else {
        return Err(UploadError::Invalid("no `file` part".into()));
    };
    if size == 0 {
        return Err(UploadError::Invalid("the file is empty".into()));
    }
    let from_name = Platform::from_file_name(&file_name);
    let platform = match fields.platform.as_deref() {
        Some(raw) => Platform::parse(raw)
            .ok_or_else(|| UploadError::Invalid(format!("unknown platform {raw:?}")))?,
        None => from_name.ok_or_else(|| {
            UploadError::Invalid("pass `platform`, or upload a .apk or .ipa".into())
        })?,
    };
    if from_name != Some(platform) {
        return Err(UploadError::Invalid(format!(
            "a {} build must be a .{} file",
            platform.as_str(),
            platform.extension()
        )));
    }
    let version = fields.version.unwrap_or_else(|| "0.0.0".to_string());
    stamp::validate_version_name(&version).map_err(UploadError::Invalid)?;
    let build_number = fields.build_number.unwrap_or_default();
    if !build_number.chars().all(|c| c.is_ascii_digit()) || build_number.len() > 20 {
        return Err(UploadError::Invalid("`build_number` must be digits".into()));
    }
    let bundle_id = fields.bundle_id.unwrap_or_default();
    if !bundle_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return Err(UploadError::Invalid(
            "`bundle_id` has invalid characters".into(),
        ));
    }
    if platform == Platform::Ios && bundle_id.is_empty() {
        return Err(UploadError::Invalid(
            "an iOS build needs `bundle_id` (the install manifest names it)".into(),
        ));
    }
    let name = fields.name.unwrap_or_else(|| "App".to_string());

    let id = uuid::Uuid::new_v4().simple().to_string();
    let file = format!(
        "{}-{version}{}.{}",
        crate::mobile::build::slug(&name),
        if build_number.is_empty() {
            String::new()
        } else {
            format!("-{build_number}")
        },
        platform.extension()
    );
    let dir = root.join(&id);
    std::fs::create_dir_all(&dir).map_err(|e| UploadError::Server(e.to_string()))?;
    tmp.persist(dir.join(&file))
        .map_err(|e| UploadError::Server(e.to_string()))?;
    let build = Build {
        id,
        token: new_token(),
        platform: platform.as_str().to_string(),
        name,
        bundle_id,
        version,
        build_number,
        notes: fields.notes.unwrap_or_default(),
        size,
        sha256: {
            use sha2::Digest;
            hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        },
        file,
        created_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    };
    let json = serde_json::to_vec_pretty(&build).map_err(|e| UploadError::Server(e.to_string()))?;
    if let Err(e) = std::fs::write(dir.join("build.json"), json) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(UploadError::Server(e.to_string()));
    }
    Ok(build)
}

// ---------------------------------------------------------------- pages

fn handle_index(root: &Path, headers: &HeaderMap, dev_mode: bool) -> Response<ResponseBody> {
    let base = install_base(headers, dev_mode);
    let builds = list_builds(root);
    let links_on = install_links_on(dev_mode);
    let mut body = String::new();

    if !links_on {
        body.push_str(
            "<p class=\"notice\">Install links are off (<code>SOLI_MOBILE_PUBLIC_INSTALL=0</code>): \
builds can be listed and deleted here, not installed.</p>",
        );
    }
    if !base.starts_with("https://") {
        body.push_str(
            "<p class=\"notice\">This page was reached over <code>http</code>. iOS installs only \
over HTTPS, so iOS builds show no install button here. Behind a TLS proxy, set \
<code>SOLI_TRUST_PROXY=1</code> so the forwarded scheme is used.</p>",
        );
    }

    if builds.is_empty() {
        body.push_str(
            "<div class=\"empty\"><b>No builds yet</b><span>Build and upload one from the \
app's folder with the commands below.</span></div>",
        );
    } else {
        body.push_str("<div class=\"gallery\">");
        for platform in [Platform::Android, Platform::Ios] {
            if let Some(latest) = builds.iter().find(|b| b.platform() == platform) {
                body.push_str(&latest_card(latest, &base, links_on));
            }
        }
        body.push_str("</div>");
        body.push_str(&history_table(&builds, links_on));
    }

    body.push_str(&format!(
        "<h2>Publish from CI</h2><pre><code>soli mobile build android\n\
SOLI_MOBILE_TOKEN=\"$TOKEN\" soli mobile publish --latest android --url {}</code></pre>",
        esc(&base)
    ));
    html_ok(operator_shell::page(
        Section::Mobile,
        "Mobile",
        INTRO,
        &body,
    ))
}

fn latest_card(build: &Build, base: &str, links_on: bool) -> String {
    let install = format!("{base}{BASE}/i/{}", build.token);
    let qr = if links_on {
        qr::svg(&install, 180).unwrap_or_default()
    } else {
        String::new()
    };
    let link = if links_on {
        format!(
            "<p><a href=\"{href}\">Install page</a></p>",
            href = esc(&install)
        )
    } else {
        String::new()
    };
    format!(
        "<figure class=\"card qr-card\">{qr}<figcaption><strong>{name}</strong> \u{b7} {platform}<br>\
{version}{build} \u{b7} {size}<br><small>{when}</small>{link}</figcaption></figure>",
        name = esc(&build.name),
        platform = build.platform,
        version = esc(&build.version),
        build = if build.build_number.is_empty() {
            String::new()
        } else {
            format!(" ({})", esc(&build.build_number))
        },
        size = human_size(build.size),
        when = esc(&build.created_at),
    )
}

fn history_table(builds: &[Build], links_on: bool) -> String {
    let mut rows = String::new();
    for build in builds {
        let install = if links_on {
            format!(
                "<a href=\"{BASE}/i/{token}\">install</a>",
                token = esc(&build.token)
            )
        } else {
            String::new()
        };
        rows.push_str(&format!(
            "<tr><td>{platform}</td><td>{version}</td><td>{number}</td><td>{notes}</td>\
<td>{size}</td><td>{when}</td><td>{install}</td><td>\
<form method=\"post\" action=\"{BASE}/builds/{id}/delete\" \
onsubmit=\"return confirm('Delete this build? Its install link stops working.')\">\
<button type=\"submit\">Delete</button></form></td></tr>",
            platform = build.platform,
            version = esc(&build.version),
            number = esc(&build.build_number),
            notes = esc(&build.notes),
            size = human_size(build.size),
            when = esc(&build.created_at),
            id = build.id,
        ));
    }
    format!(
        "<h2>History</h2><div class=\"table-wrap\"><table><thead><tr><th>Platform</th>\
<th>Version</th><th>Build</th><th>Notes</th><th>Size</th><th>Uploaded</th><th></th><th></th>\
</tr></thead><tbody>{rows}</tbody></table></div>"
    )
}

/// `/__soli/mobile/i/<token>[/download|/manifest.plist]`.
fn handle_install(root: &Path, rest: &str, headers: &HeaderMap) -> Response<ResponseBody> {
    let (token, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let Some(build) = find_by_token(root, token) else {
        return admin_auth::hidden_not_found();
    };
    let base = base_url(headers);
    let install = format!("{base}{BASE}/i/{}", build.token);
    match tail {
        "" => noindex(html_ok(install_page(&build, &install))),
        "download" => {
            let path = root.join(&build.id).join(&build.file);
            let mut response = static_files::serve_disk_file(
                &path,
                build.size,
                build.platform().content_type(),
                headers,
            );
            let headers = response.headers_mut();
            if let Ok(value) = format!("attachment; filename=\"{}\"", build.file).parse() {
                headers.insert(hyper::header::CONTENT_DISPOSITION, value);
            }
            headers.insert(
                "X-Content-Type-Options",
                hyper::header::HeaderValue::from_static("nosniff"),
            );
            noindex(response)
        }
        "manifest.plist" if build.platform() == Platform::Ios => {
            let plist = ios_manifest::render(&ios_manifest::ManifestInput {
                ipa_url: &format!("{install}/download"),
                bundle_id: &build.bundle_id,
                version: &build.version,
                title: &build.name,
            });
            noindex(
                Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "application/xml")
                    .body(full(Bytes::from(plist)))
                    .unwrap(),
            )
        }
        _ => admin_auth::hidden_not_found(),
    }
}

fn noindex(mut response: Response<ResponseBody>) -> Response<ResponseBody> {
    response.headers_mut().insert(
        "X-Robots-Tag",
        hyper::header::HeaderValue::from_static("noindex"),
    );
    response
}

/// The page a phone opens from the QR code. Standalone, not the operator
/// shell: whoever holds the link sees this build and nothing else.
fn install_page(build: &Build, install: &str) -> String {
    let platform = build.platform();
    let action = match platform {
        Platform::Android => format!(
            "<a class=\"btn\" href=\"{href}/download\">Download the APK</a>\
<p class=\"hint\">Android asks once to allow installs from your browser.</p>",
            href = esc(install)
        ),
        Platform::Ios if install.starts_with("https://") => format!(
            "<a class=\"btn\" href=\"{href}\">Install</a>\
<p class=\"hint\">Open this page in Safari on a device registered for this build.</p>",
            href = esc(&ios_manifest::itms_services_link(&format!(
                "{install}/manifest.plist"
            )))
        ),
        Platform::Ios => "<p class=\"hint\">iOS installs only over HTTPS: open this link \
through the app's HTTPS address.</p>"
            .to_string(),
    };
    let notes = if build.notes.is_empty() {
        String::new()
    } else {
        format!("<pre class=\"notes\">{}</pre>", esc(&build.notes))
    };
    let qr = qr::svg(install, 200).unwrap_or_default();
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta name=\"robots\" content=\"noindex\"><title>Install {name}</title><style>{STYLE}</style>\
</head><body><main><h1>{name}</h1><p class=\"meta\">{platform} \u{b7} {version}{number} \u{b7} \
{size}</p>{action}{notes}<div class=\"qr\">{qr}<p class=\"hint\">On a computer? Scan this with \
the phone.</p></div></main></body></html>",
        name = esc(&build.name),
        platform = match platform {
            Platform::Android => "Android",
            Platform::Ios => "iOS",
        },
        version = esc(&build.version),
        number = if build.build_number.is_empty() {
            String::new()
        } else {
            format!(" ({})", esc(&build.build_number))
        },
        size = human_size(build.size),
    )
}

const STYLE: &str = "\
:root{color-scheme:light dark;--bg:#fff;--fg:#111;--muted:#666;--accent:#2563eb}\
@media (prefers-color-scheme:dark){:root{--bg:#111;--fg:#eee;--muted:#999;--accent:#60a5fa}}\
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,sans-serif}\
main{max-width:28rem;margin:0 auto;padding:2rem 1rem;text-align:center}\
h1{margin:0 0 .25rem;font-size:1.6rem}.meta,.hint{color:var(--muted);font-size:.9rem}\
.btn{display:inline-block;margin:1.25rem 0 .25rem;padding:.8rem 1.6rem;border-radius:.6rem;\
background:var(--accent);color:#fff;text-decoration:none;font-weight:600}\
.notes{text-align:left;white-space:pre-wrap;font:inherit;background:rgba(127,127,127,.1);\
padding:.75rem;border-radius:.5rem}.qr{margin-top:2rem}.qr svg{max-width:100%;height:auto}";

#[cfg(test)]
mod tests {
    use super::*;

    fn store(root: &Path, platform: &str, created_at: &str) -> Build {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let build = Build {
            id: id.clone(),
            token: new_token(),
            platform: platform.into(),
            name: "Shop".into(),
            bundle_id: "com.example.shop".into(),
            version: "1.0".into(),
            build_number: "1".into(),
            notes: String::new(),
            size: 3,
            sha256: String::new(),
            file: "shop-1.0-1.apk".into(),
            created_at: created_at.into(),
        };
        std::fs::create_dir_all(root.join(&id)).unwrap();
        std::fs::write(root.join(&id).join(&build.file), "apk").unwrap();
        std::fs::write(
            root.join(&id).join("build.json"),
            serde_json::to_vec(&build).unwrap(),
        )
        .unwrap();
        build
    }

    #[test]
    fn tokens_and_ids_have_the_shape_the_routes_accept() {
        let token = new_token();
        assert!(valid_token(&token), "{token}");
        assert!(valid_id(&uuid::Uuid::new_v4().simple().to_string()));
        assert!(!valid_id("../../etc"));
        assert!(!valid_token("short"));
    }

    #[test]
    fn a_token_finds_its_build_and_nothing_else() {
        let root = tempfile::tempdir().unwrap();
        let a = store(root.path(), "android", "2026-01-01T00:00:00.000Z");
        let b = store(root.path(), "android", "2026-01-02T00:00:00.000Z");
        assert_eq!(find_by_token(root.path(), &a.token).unwrap().id, a.id);
        assert_eq!(find_by_token(root.path(), &b.token).unwrap().id, b.id);
        assert!(find_by_token(root.path(), &new_token()).is_none());
    }

    #[test]
    fn prune_keeps_the_newest_per_platform() {
        let root = tempfile::tempdir().unwrap();
        let old = store(root.path(), "android", "2026-01-01T00:00:00.000Z");
        let new = store(root.path(), "android", "2026-01-03T00:00:00.000Z");
        let ios = store(root.path(), "ios", "2026-01-02T00:00:00.000Z");
        assert_eq!(prune(root.path(), Platform::Android, 1), 1);
        let left: Vec<String> = list_builds(root.path()).into_iter().map(|b| b.id).collect();
        assert_eq!(left, vec![new.id, ios.id]);
        assert!(!root.path().join(&old.id).exists());
    }

    #[test]
    fn delete_refuses_a_path_that_is_not_an_id() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("keep")).unwrap();
        assert!(!delete_build(root.path(), "keep"));
        assert!(!delete_build(root.path(), ".."));
        assert!(root.path().join("keep").exists());
    }

    #[test]
    fn base_url_ignores_forwarded_headers_unless_trusted() {
        let mut headers = HeaderMap::new();
        headers.insert("host", "staging.example.com".parse().unwrap());
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        // SOLI_TRUST_PROXY is not set in the test environment.
        assert_eq!(base_url(&headers), "http://staging.example.com");
    }

    #[test]
    fn a_loopback_host_becomes_the_lan_address_for_the_phone() {
        let lan: std::net::IpAddr = "192.168.1.20".parse().unwrap();
        assert_eq!(
            with_lan_host("http://localhost:5011", lan),
            "http://192.168.1.20:5011"
        );
        assert_eq!(
            with_lan_host("http://127.0.0.1:5011", lan),
            "http://192.168.1.20:5011"
        );
        assert_eq!(
            with_lan_host("http://[::1]:5011", lan),
            "http://192.168.1.20:5011"
        );
        assert_eq!(with_lan_host("http://[::1]", lan), "http://192.168.1.20");
        assert_eq!(
            with_lan_host("http://app.localhost", lan),
            "http://192.168.1.20"
        );
        assert_eq!(
            with_lan_host("https://staging.example.com", lan),
            "https://staging.example.com",
            "a real host is kept"
        );
    }

    #[test]
    fn install_page_offers_the_platforms_action() {
        let root = tempfile::tempdir().unwrap();
        let mut build = store(root.path(), "android", "2026-01-01T00:00:00.000Z");
        let page = install_page(&build, "https://a.test/__soli/mobile/i/t");
        assert!(page.contains("/download\">Download the APK"));
        assert!(page.contains("noindex"));

        build.platform = "ios".into();
        let page = install_page(&build, "https://a.test/__soli/mobile/i/t");
        assert!(page.contains("itms-services://?action=download-manifest"));
        let page = install_page(&build, "http://a.test/__soli/mobile/i/t");
        assert!(!page.contains("itms-services"), "no iOS install over http");
    }
}
