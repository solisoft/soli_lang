//! `/__soli/mobile` end to end: a real `soli serve`, real uploads.
//!
//! Covers the gate (404 unconfigured, 401 without the token), the streamed
//! upload and its cap, a byte-for-byte download, a `Range` request, the
//! install page and manifest, delete, and `soli mobile publish`'s client
//! against the server.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::multipart::{Form, Part};
use reqwest::blocking::Client;
use reqwest::StatusCode;

const TOKEN: &str = "mobile-e2e-token-0123456789";
const MAX_SIZE: usize = 4000;

fn pick_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("free port")
}

struct Server {
    child: Child,
    port: u16,
}

impl Server {
    /// `token`: `SOLI_MOBILE_TOKEN`, or `None` for an unconfigured server.
    fn start(storage: &Path, token: Option<&str>) -> Server {
        let binary = PathBuf::from(env!("CARGO_BIN_EXE_soli"));
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/_e2e_app");
        let port = pick_port();
        let mut command = Command::new(&binary);
        command
            .arg("serve")
            .arg(&fixture)
            .arg("--port")
            .arg(port.to_string())
            .arg("--workers")
            .arg("1")
            .env("SOLI_SESSION_SECRET", "e2e-test-secret-0123456789abcdef")
            .env("SOLI_MOBILE_PATH", storage)
            .env("SOLI_MOBILE_MAX_SIZE", MAX_SIZE.to_string())
            .env_remove("SOLI_ADMIN_TOKEN")
            .env_remove("SOLI_ADMIN_USER")
            .env_remove("SOLI_ADMIN_PASSWORD")
            .env_remove("SOLI_MOBILE_TOKEN")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(token) = token {
            command.env("SOLI_MOBILE_TOKEN", token);
        }
        let server = Server {
            child: command.spawn().expect("spawn soli serve"),
            port,
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while client().get(server.url("/ping")).send().is_err() {
            assert!(Instant::now() < deadline, "server never became ready");
            thread::sleep(Duration::from_millis(200));
        }
        server
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{}", self.port, path)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn client() -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

fn apk_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

fn upload(server: &Server, token: Option<&str>, data: Vec<u8>) -> reqwest::blocking::Response {
    let form = Form::new()
        .text("version", "1.4.0")
        .text("build_number", "42")
        .text("name", "Shop")
        .text("notes", "Fixes the cart")
        .part("file", Part::bytes(data).file_name("whatever.apk"));
    let mut request = client()
        .post(server.url("/__soli/mobile/builds"))
        .multipart(form);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().expect("upload")
}

#[test]
fn unconfigured_server_hides_the_page() {
    let storage = tempfile::tempdir().unwrap();
    let server = Server::start(storage.path(), None);
    let page = client().get(server.url("/__soli/mobile")).send().unwrap();
    assert_eq!(page.status(), StatusCode::NOT_FOUND);
    let posted = upload(&server, Some(TOKEN), apk_bytes(10));
    assert_eq!(posted.status(), StatusCode::NOT_FOUND);
}

#[test]
fn upload_download_install_and_delete() {
    let storage = tempfile::tempdir().unwrap();
    let server = Server::start(storage.path(), Some(TOKEN));
    let http = client();

    // The gate.
    let page = http.get(server.url("/__soli/mobile")).send().unwrap();
    assert_eq!(page.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        upload(&server, None, apk_bytes(10)).status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        upload(&server, Some("wrong"), apk_bytes(10)).status(),
        StatusCode::UNAUTHORIZED
    );

    // The cap, and validation.
    assert_eq!(
        upload(&server, Some(TOKEN), apk_bytes(MAX_SIZE + 500)).status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let empty = upload(&server, Some(TOKEN), Vec::new());
    assert_eq!(empty.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // A build.
    let data = apk_bytes(3000);
    let created = upload(&server, Some(TOKEN), data.clone());
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: serde_json::Value = created.json().unwrap();
    assert_eq!(created["version"], "1.4.0");
    assert_eq!(created["platform"], "android");
    let install_url = created["install_url"].as_str().unwrap().to_string();
    let download_url = created["download_url"].as_str().unwrap().to_string();
    let id = created["id"].as_str().unwrap().to_string();
    assert!(install_url.contains("/__soli/mobile/i/"), "{install_url}");

    // Byte for byte, then a Range.
    let download = http.get(&download_url).send().unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(
        download.headers()["content-type"],
        "application/vnd.android.package-archive"
    );
    assert!(download.headers()["content-disposition"]
        .to_str()
        .unwrap()
        .starts_with("attachment;"));
    assert_eq!(download.headers()["x-content-type-options"], "nosniff");
    assert_eq!(download.bytes().unwrap().to_vec(), data);
    let ranged = http
        .get(&download_url)
        .header("Range", "bytes=10-19")
        .send()
        .unwrap();
    assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(ranged.bytes().unwrap().to_vec(), data[10..20].to_vec());

    // The install page is public; the operator page is not, and lists it.
    let install = http.get(&install_url).send().unwrap();
    assert_eq!(install.status(), StatusCode::OK);
    assert_eq!(install.headers()["x-robots-tag"], "noindex");
    let install = install.text().unwrap();
    assert!(install.contains("Download the APK") && install.contains("Fixes the cart"));
    let page = http
        .get(server.url("/__soli/mobile"))
        .bearer_auth(TOKEN)
        .send()
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let page = page.text().unwrap();
    assert!(
        page.contains("<svg") && page.contains("1.4.0"),
        "QR and version listed"
    );

    // An Android build has no iOS manifest; a wrong token is a plain 404.
    let manifest = http
        .get(format!("{install_url}/manifest.plist"))
        .send()
        .unwrap();
    assert_eq!(manifest.status(), StatusCode::NOT_FOUND);
    let guessed = http
        .get(server.url("/__soli/mobile/i/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"))
        .send()
        .unwrap();
    assert_eq!(guessed.status(), StatusCode::NOT_FOUND);

    // Delete revokes the link and removes the files.
    let deleted = http
        .post(server.url(&format!("/__soli/mobile/builds/{id}/delete")))
        .bearer_auth(TOKEN)
        .send()
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        http.get(&install_url).send().unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert!(!storage.path().join(&id).exists());
}

#[test]
fn ios_upload_needs_a_bundle_id_and_serves_a_manifest() {
    let storage = tempfile::tempdir().unwrap();
    let server = Server::start(storage.path(), Some(TOKEN));
    let http = client();
    let send = |bundle: Option<&str>| {
        let mut form = Form::new()
            .text("version", "2.0.0")
            .part("file", Part::bytes(apk_bytes(100)).file_name("shop.ipa"));
        if let Some(bundle) = bundle {
            form = form.text("bundle_id", bundle.to_string());
        }
        http.post(server.url("/__soli/mobile/builds"))
            .bearer_auth(TOKEN)
            .multipart(form)
            .send()
            .unwrap()
    };
    assert_eq!(send(None).status(), StatusCode::UNPROCESSABLE_ENTITY);
    let created = send(Some("com.example.shop"));
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: serde_json::Value = created.json().unwrap();
    let install_url = created["install_url"].as_str().unwrap();
    let manifest = http
        .get(format!("{install_url}/manifest.plist"))
        .send()
        .unwrap()
        .text()
        .unwrap();
    assert!(manifest.contains("<string>com.example.shop</string>"));
    assert!(manifest.contains("/download</string>"));
    // Reached over http: the page says iOS needs HTTPS instead of a button.
    let install = http.get(install_url).send().unwrap().text().unwrap();
    assert!(!install.contains("itms-services"));
}

#[test]
fn publish_round_trip() {
    let storage = tempfile::tempdir().unwrap();
    let server = Server::start(storage.path(), Some(TOKEN));
    let dist = tempfile::tempdir().unwrap();
    let file = dist.path().join("shop-1.4.0-42.apk");
    std::fs::write(&file, apk_bytes(1200)).unwrap();
    std::fs::write(
        solilang::mobile::build::stub_path(&file),
        serde_json::json!({
            "platform": "android", "name": "Shop", "bundle_id": "com.example.shop",
            "version": "1.4.0", "build_number": "42", "sha256": "", "size": 1200
        })
        .to_string(),
    )
    .unwrap();

    let base = server.url("");
    let refused = solilang::mobile::publish::publish(&solilang::mobile::publish::PublishOptions {
        file: &file,
        url: &base,
        notes: None,
        token: None,
    })
    .unwrap_err();
    assert!(refused.contains("SOLI_MOBILE_TOKEN"), "{refused}");

    let published =
        solilang::mobile::publish::publish(&solilang::mobile::publish::PublishOptions {
            file: &file,
            url: &base,
            notes: Some("From the CLI"),
            token: Some(TOKEN),
        })
        .expect("publish");
    assert_eq!(published.version, "1.4.0", "read from the stub");
    assert_eq!(published.build_number, "42");
    let install = client()
        .get(&published.install_url)
        .send()
        .unwrap()
        .text()
        .unwrap();
    assert!(install.contains("From the CLI") && install.contains("Shop"));
}
