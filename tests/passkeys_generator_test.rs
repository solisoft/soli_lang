//! `soli generate auth --passkeys` — the passkey scaffold on top of session
//! auth: what it writes, that it never clobbers, that it is fmt/lint clean, and
//! a served app that registers a passkey and signs in with it, driven by a
//! software authenticator (P-256) so the whole ceremony runs over HTTP.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};
use solilang::scaffold::{create_auth, create_auth_with};

const PASSKEY_FILES: [&str; 6] = [
    "app/models/passkey_credential.sl",
    "app/controllers/passkeys_controller.sl",
    "app/controllers/passkey_sessions_controller.sl",
    "app/views/passkeys/index.html.slv",
    "app/views/passkeys/_sign_in.html.slv",
    "public/js/passkeys.js",
];

fn make_app_skeleton(root: &Path) {
    for sub in ["app/controllers", "app/models", "app/views", "config"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("config/routes.sl"), "# routes\n").unwrap();
}

fn migrations(root: &Path, name: &str) -> usize {
    fs::read_dir(root.join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(name))
        .count()
}

fn passkeys_app() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    create_auth_with(dir.path().to_str().unwrap(), true).expect("generate auth --passkeys");
    dir
}

#[test]
fn passkeys_flag_writes_the_passkey_files_migration_and_routes() {
    let dir = passkeys_app();
    let root = dir.path();
    for rel in PASSKEY_FILES {
        assert!(root.join(rel).exists(), "{rel} not created");
    }
    assert!(
        root.join("app/models/user.sl").exists(),
        "base auth missing"
    );
    assert_eq!(migrations(root, "create_passkey_credentials"), 1);

    let migration = fs::read_dir(root.join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| e.file_name().to_string_lossy().contains("passkey"))
        .map(|e| fs::read_to_string(e.path()).unwrap())
        .unwrap();
    assert!(migration.contains(r#"["credential_id"], {"unique": true}"#));
    assert!(migration.contains(r#"["user_id"]"#));

    let routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    for route in [
        r#"get("/passkeys", "passkeys#index")"#,
        r#"post("/passkeys/options", "passkeys#creation_options")"#,
        r#"post("/passkeys", "passkeys#create")"#,
        r#"delete("/passkeys/:id", "passkeys#destroy")"#,
        r#"post("/passkeys/login/options", "passkey_sessions#request_options")"#,
        r#"post("/passkeys/login", "passkey_sessions#create")"#,
    ] {
        assert!(routes.contains(route), "missing {route}");
    }
    assert!(routes.contains("\"sessions#new\""), "base routes missing");

    // A fresh sign-in view carries the button and the conditional-UI field.
    let sign_in = fs::read_to_string(root.join("app/views/sessions/new.html.slv")).unwrap();
    assert!(
        sign_in.contains(r#"<%- partial("passkeys/sign_in") %>"#),
        "{sign_in}"
    );
    assert!(
        sign_in.contains(r#"autocomplete="username webauthn""#),
        "{sign_in}"
    );
}

#[test]
fn plain_auth_writes_no_passkey_files() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    create_auth(dir.path().to_str().unwrap()).unwrap();
    for rel in PASSKEY_FILES {
        assert!(
            !dir.path().join(rel).exists(),
            "{rel} written without --passkeys"
        );
    }
    let sign_in = fs::read_to_string(dir.path().join("app/views/sessions/new.html.slv")).unwrap();
    assert!(!sign_in.contains("passkeys/sign_in"));
}

#[test]
fn passkeys_on_an_existing_auth_app_adds_only_passkey_files_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_app_skeleton(root);
    create_auth(root.to_str().unwrap()).unwrap();

    // A customized sign-in view and model must survive.
    fs::write(
        root.join("app/views/sessions/new.html.slv"),
        "<p>mine</p>\n",
    )
    .unwrap();
    fs::write(root.join("app/models/user.sl"), "# my user\n").unwrap();

    create_auth_with(root.to_str().unwrap(), true).unwrap();
    for rel in PASSKEY_FILES {
        assert!(root.join(rel).exists(), "{rel} not created");
    }
    assert_eq!(
        fs::read_to_string(root.join("app/views/sessions/new.html.slv")).unwrap(),
        "<p>mine</p>\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("app/models/user.sl")).unwrap(),
        "# my user\n"
    );

    // A second run changes nothing.
    fs::write(root.join("app/models/passkey_credential.sl"), "# mine\n").unwrap();
    create_auth_with(root.to_str().unwrap(), true).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("app/models/passkey_credential.sl")).unwrap(),
        "# mine\n"
    );
    let routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    assert_eq!(routes.matches("\"passkeys#index\"").count(), 1);
    assert_eq!(routes.matches("\"sessions#new\"").count(), 1);
    assert_eq!(migrations(root, "create_passkey_credentials"), 1);
    assert_eq!(migrations(root, "create_users"), 1);
}

#[test]
fn passkeys_require_the_user_model() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    let err = solilang::scaffold::create_passkeys(dir.path().to_str().unwrap()).unwrap_err();
    assert!(err.contains("app/models/user.sl"), "{err}");
}

fn soli(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> (bool, String) {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_soli"));
    command
        .args(args)
        .current_dir(dir)
        .env_remove("SOLI_ENGINE")
        .env_remove("AUTH_SIGNUP")
        .env_remove("AUTH_ATTEMPTS_PER_IP")
        .env_remove("AUTH_IP_WINDOW_SECONDS");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().expect("run soli");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

/// `--passkeys` parses before and after the folder; an unknown flag is an
/// error rather than being taken for "no folder".
#[test]
fn cli_accepts_the_flag_in_either_position() {
    for args in [
        ["generate", "auth", "--passkeys", "app"],
        ["generate", "auth", "app", "--passkeys"],
    ] {
        let dir = tempfile::tempdir().unwrap();
        make_app_skeleton(&dir.path().join("app"));
        let (ok, output) = soli(dir.path(), &args, &[]);
        assert!(ok, "{args:?}: {output}");
        assert!(
            dir.path()
                .join("app/app/models/passkey_credential.sl")
                .exists(),
            "{args:?}: {output}"
        );
    }

    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    let (ok, _) = soli(dir.path(), &["generate", "auth"], &[]);
    assert!(ok);
    assert!(!dir.path().join("app/models/passkey_credential.sl").exists());
    let (ok, output) = soli(dir.path(), &["generate", "auth", "--passkey"], &[]);
    assert!(!ok);
    assert!(output.contains("unknown option --passkey"), "{output}");
}

#[test]
fn generated_passkeys_pass_fmt_and_lint() {
    let dir = passkeys_app();
    let (formatted, output) = soli(dir.path(), &["fmt", "--check", "."], &[]);
    assert!(formatted, "soli fmt --check:\n{output}");
    let (clean, output) = soli(dir.path(), &["lint", "."], &[]);
    assert!(clean, "soli lint:\n{output}");
}

/// Route comments above every public action, `_` helpers under `private`,
/// `@` rather than `this.` outside `static`, `nil` rather than `null`.
#[test]
fn generated_passkeys_follow_the_house_style() {
    let dir = passkeys_app();
    for rel in [
        "app/controllers/passkeys_controller.sl",
        "app/controllers/passkey_sessions_controller.sl",
        "app/models/passkey_credential.sl",
    ] {
        let source = fs::read_to_string(dir.path().join(rel)).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        let mut private = false;
        for (number, line) in lines.iter().enumerate() {
            let code = line.split('#').next().unwrap_or_default();
            assert!(
                !code.contains("this.") || code.contains("this.layout"),
                "{rel}:{}: {line}",
                number + 1
            );
            assert!(
                !line
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|word| word == "null"),
                "{rel}:{}: {line}",
                number + 1
            );
            if line.trim() == "private" {
                private = true;
            }
            if rel.contains("controller") && line.starts_with("  def ") {
                let name = line.trim_start_matches("  def ");
                if private {
                    assert!(name.starts_with('_'), "{rel}: private helper {name}");
                } else {
                    let above = lines[number - 1].trim();
                    assert!(
                        above.starts_with("# GET ")
                            || above.starts_with("# POST ")
                            || above.starts_with("# DELETE "),
                        "{rel}: action {name} has no route comment"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Served end-to-end: a software authenticator against the generated app
// ---------------------------------------------------------------------------

const ORIGIN: &str = "https://app.test";
const RP_ID: &str = "app.test";

struct Server(std::process::Child, u16);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn serve(root: &Path) -> Server {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_soli"));
    command
        .arg("serve")
        .arg(root)
        .args(["--port", &port.to_string(), "--workers", "1"])
        .current_dir(root)
        .env_remove("SOLI_ENGINE")
        .env_remove("AUTH_ATTEMPTS_PER_IP")
        .env_remove("AUTH_IP_WINDOW_SECONDS")
        .env_remove("WEBAUTHN_RP_ID")
        .env_remove("WEBAUTHN_ORIGINS")
        .env("AUTH_SIGNUP", "open")
        .env("APP_BASE_URL", ORIGIN)
        // Form posts must carry a token; the passkey requests are JSON and
        // carry the page's token in X-CSRF-Token.
        .env("SOLI_CSRF_TOKENS", "require")
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
        .env("SOLI_MAIL_DELIVERY_METHOD", "logger")
        .env("SOLI_FAIL_ON_VM_DEMOTION", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let server = Server(command.spawn().expect("spawn soli serve"), port);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if ureq::get(&format!("http://127.0.0.1:{port}/login"))
            .call()
            .is_ok()
        {
            return server;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("soli serve did not answer on {port}");
}

/// A browser stand-in: a cookie jar and an optional CSRF token header.
struct Browser {
    port: u16,
    jar: BTreeMap<String, String>,
    csrf: Option<String>,
}

impl Browser {
    fn new(server: &Server) -> Self {
        Browser {
            port: server.1,
            jar: BTreeMap::new(),
            csrf: None,
        }
    }

    fn request(&mut self, method: &str, path: &str, body: Option<&Json>) -> (u16, String) {
        let agent = ureq::AgentBuilder::new().redirects(0).build();
        let mut request = agent.request(method, &format!("http://127.0.0.1:{}{path}", self.port));
        if !self.jar.is_empty() {
            let cookie = self
                .jar
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            request = request.set("Cookie", &cookie);
        }
        // A browser names its origin on every state-changing request; the
        // CSRF gate refuses a cookie-bearing one that does not.
        if method != "GET" {
            request = request.set("Origin", &format!("http://127.0.0.1:{}", self.port));
        }
        if let Some(token) = &self.csrf {
            request = request.set("X-CSRF-Token", token);
        }
        let response = match body {
            Some(body) => request
                .set("Content-Type", "application/json")
                .send_string(&body.to_string()),
            None => request.call(),
        };
        let response = match response {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => panic!("{method} {path}: {e}"),
        };
        for header in response.all("set-cookie") {
            let pair = header.split(';').next().unwrap_or_default();
            if let Some((name, value)) = pair.split_once('=') {
                if value.is_empty() || header.to_ascii_lowercase().contains("max-age=0") {
                    self.jar.remove(name.trim());
                } else {
                    self.jar.insert(name.trim().to_string(), value.to_string());
                }
            }
        }
        (
            response.status(),
            response.into_string().unwrap_or_default(),
        )
    }

    fn post(&mut self, path: &str, body: Json) -> (u16, Json) {
        let (status, text) = self.request("POST", path, Some(&body));
        (
            status,
            serde_json::from_str(&text).unwrap_or(Json::String(text)),
        )
    }
}

fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn unb64(text: &str) -> Vec<u8> {
    URL_SAFE_NO_PAD.decode(text).unwrap()
}

/// A software passkey: one P-256 key and its credential id.
struct Authenticator {
    key: SigningKey,
    id: Vec<u8>,
}

impl Authenticator {
    fn new() -> Self {
        Authenticator {
            key: SigningKey::from_slice(&[42u8; 32]).unwrap(),
            id: b"software-passkey-1".to_vec(),
        }
    }

    fn cose_key(&self) -> Vec<u8> {
        let point = self.key.verifying_key().to_sec1_point(false);
        let point = point.as_bytes();
        // {1: 2, 3: -7, -1: 1, -2: x, -3: y}
        let mut cose = vec![0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
        cose.extend_from_slice(&point[1..33]);
        cose.extend_from_slice(&[0x22, 0x58, 0x20]);
        cose.extend_from_slice(&point[33..65]);
        cose
    }

    fn auth_data(&self, count: u32, attested: bool) -> Vec<u8> {
        let mut data = Sha256::digest(RP_ID.as_bytes()).to_vec();
        // UP | UV | BE, plus AT on registration
        data.push(if attested { 0x4d } else { 0x0d });
        data.extend_from_slice(&count.to_be_bytes());
        if attested {
            data.extend_from_slice(&[0u8; 16]);
            data.extend_from_slice(&(self.id.len() as u16).to_be_bytes());
            data.extend_from_slice(&self.id);
            data.extend_from_slice(&self.cose_key());
        }
        data
    }

    fn client_data(kind: &str, challenge: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({"type": kind, "challenge": challenge, "origin": ORIGIN}))
            .unwrap()
    }

    /// The `toJSON()` of navigator.credentials.create() for these options.
    fn create(&self, options: &Json) -> Json {
        let client = Self::client_data("webauthn.create", options["challenge"].as_str().unwrap());
        let auth_data = self.auth_data(0, true);
        // {"fmt": "none", "attStmt": {}, "authData": bytes}
        let mut attestation = vec![0xa3, 0x63];
        attestation.extend_from_slice(b"fmt");
        attestation.push(0x64);
        attestation.extend_from_slice(b"none");
        attestation.push(0x67);
        attestation.extend_from_slice(b"attStmt");
        attestation.push(0xa0);
        attestation.push(0x68);
        attestation.extend_from_slice(b"authData");
        attestation.extend_from_slice(&[0x58, auth_data.len() as u8]);
        attestation.extend_from_slice(&auth_data);
        json!({
            "id": b64(&self.id), "rawId": b64(&self.id), "type": "public-key",
            "response": {
                "clientDataJSON": b64(&client),
                "attestationObject": b64(&attestation),
                "transports": ["internal"],
            },
            "clientExtensionResults": {},
        })
    }

    /// The `toJSON()` of navigator.credentials.get() for these options.
    fn get(&self, options: &Json, count: u32, user_handle: &[u8]) -> Json {
        let client = Self::client_data("webauthn.get", options["challenge"].as_str().unwrap());
        let auth_data = self.auth_data(count, false);
        let mut signed = auth_data.clone();
        signed.extend_from_slice(&Sha256::digest(&client));
        let signature: Signature = self.key.sign(&signed);
        json!({
            "id": b64(&self.id), "rawId": b64(&self.id), "type": "public-key",
            "response": {
                "clientDataJSON": b64(&client),
                "authenticatorData": b64(&auth_data),
                "signature": b64(signature.to_der().as_bytes()),
                "userHandle": b64(user_handle),
            },
            "clientExtensionResults": {},
        })
    }
}

/// The value of `data-csrf-token="…"` in a page.
fn csrf_from(html: &str) -> String {
    let start = html.find("data-csrf-token=\"").expect("no data-csrf-token") + 17;
    html[start..].split('"').next().unwrap().to_string()
}

fn served_passkeys_app() -> tempfile::TempDir {
    let dir = passkeys_app();
    fs::create_dir_all(dir.path().join("app/views/layouts")).unwrap();
    fs::write(
        dir.path().join("app/views/layouts/application.html.slv"),
        "<main><%= yield %></main>\n",
    )
    .unwrap();
    dir
}

#[test]
fn served_app_registers_a_passkey_and_signs_in_with_it() {
    let dir = served_passkeys_app();
    let server = serve(dir.path());
    let authenticator = Authenticator::new();

    // The sign-in page renders the passkey partial.
    let mut guest = Browser::new(&server);
    let (status, page) = guest.request("GET", "/login", None);
    assert_eq!(status, 200, "{page}");
    assert!(page.contains("Sign in with a passkey"), "{page}");
    assert!(
        page.contains(r#"autocomplete="username webauthn""#),
        "{page}"
    );

    // Managing passkeys needs an account.
    assert_eq!(guest.request("GET", "/passkeys", None).0, 302);
    assert_eq!(guest.post("/passkeys/options", json!({})).0, 401);

    // Sign up, then open /passkeys and pick up its CSRF token.
    let mut ada = Browser::new(&server);
    let signup = json!({"email": "ada@example.com", "password": "correct horse battery"});
    assert_eq!(ada.post("/signup", signup).0, 302);
    let (status, page) = ada.request("GET", "/passkeys", None);
    assert_eq!(status, 200, "{page}");
    assert!(page.contains("No passkey yet"), "{page}");
    ada.csrf = Some(csrf_from(&page));

    // Registration: options, then the authenticator's response.
    let (status, options) = ada.post("/passkeys/options", json!({}));
    assert_eq!(status, 200, "{options}");
    assert_eq!(options["rp"]["id"], json!(RP_ID));
    assert_eq!(options["user"]["name"], json!("ada@example.com"));
    assert_eq!(options["excludeCredentials"], json!([]));
    let user_handle = unb64(options["user"]["id"].as_str().unwrap());
    let credential = authenticator.create(&options);
    let (status, created) = ada.post(
        "/passkeys",
        json!({"credential": credential, "name": "Laptop"}),
    );
    assert_eq!(status, 201, "{created}");
    assert_eq!(created["name"], json!("Laptop"));

    // The challenge was single use: the same response again is refused.
    let (status, _) = ada.post(
        "/passkeys",
        json!({"credential": credential, "name": "Again"}),
    );
    assert_eq!(status, 422);
    // And the next options exclude the registered credential.
    let (_, options) = ada.post("/passkeys/options", json!({}));
    assert_eq!(
        options["excludeCredentials"][0]["id"],
        json!(b64(&authenticator.id))
    );
    let (_, page) = ada.request("GET", "/passkeys", None);
    assert!(page.contains("Laptop"), "{page}");

    // Sign in from a fresh browser with the passkey.
    let mut device = Browser::new(&server);
    // No options first: no challenge, uniform failure.
    let assertion = authenticator.get(&json!({"challenge": "AAAA"}), 1, &user_handle);
    let (status, failure) = device.post("/passkeys/login", assertion);
    assert_eq!(status, 401);
    assert_eq!(failure, json!({"error": "Passkey sign-in failed."}));

    let (status, options) = device.post("/passkeys/login/options", json!({}));
    assert_eq!(status, 200, "{options}");
    assert_eq!(options["rpId"], json!(RP_ID));
    assert_eq!(options["allowCredentials"], json!([]));
    // A tampered signature gets the same answer as an unknown credential.
    let mut tampered = authenticator.get(&options, 1, &user_handle);
    tampered["response"]["clientDataJSON"] =
        json!(b64(&Authenticator::client_data("webauthn.get", "b3RoZXI")));
    assert_eq!(
        device.post("/passkeys/login", tampered),
        (401, failure.clone())
    );
    let (_, options) = device.post("/passkeys/login/options", json!({}));
    let mut unknown = authenticator.get(&options, 1, &user_handle);
    unknown["id"] = json!(b64(b"nobody"));
    unknown["rawId"] = json!(b64(b"nobody"));
    assert_eq!(
        device.post("/passkeys/login", unknown),
        (401, failure.clone())
    );
    // Nothing named the user beforehand, so a validly signed assertion
    // without a user handle, or naming another account, is refused too.
    for handle in [&b""[..], &b"someone-else"[..]] {
        let (_, options) = device.post("/passkeys/login/options", json!({}));
        let assertion = authenticator.get(&options, 1, handle);
        assert_eq!(
            device.post("/passkeys/login", assertion),
            (401, failure.clone())
        );
    }

    let (_, options) = device.post("/passkeys/login/options", json!({}));
    let assertion = authenticator.get(&options, 1, &user_handle);
    let (status, result) = device.post("/passkeys/login", assertion.clone());
    assert_eq!(status, 200, "{result}");
    assert_eq!(result, json!({"redirect": "/"}));
    let (status, page) = device.request("GET", "/passkeys", None);
    assert_eq!(status, 200, "signed in by the passkey: {page}");
    assert!(
        page.contains("Laptop") && page.contains("last used"),
        "{page}"
    );

    // Replaying the accepted assertion with fresh options fails (the
    // challenge differs), and so does a counter that did not move forward.
    let mut replay = Browser::new(&server);
    replay.post("/passkeys/login/options", json!({}));
    assert_eq!(replay.post("/passkeys/login", assertion).0, 401);
    let (_, options) = replay.post("/passkeys/login/options", json!({}));
    let stale = authenticator.get(&options, 1, &user_handle);
    assert_eq!(replay.post("/passkeys/login", stale).0, 401);

    // Remove the passkey; it no longer signs in.
    let start = page.find("action=\"/passkeys/").expect("no remove button") + 8;
    let path = page[start..].split('"').next().unwrap().to_string();
    let (status, _) = device.request("DELETE", &path, None);
    assert_eq!(status, 302);
    let (_, page) = device.request("GET", "/passkeys", None);
    assert!(page.contains("No passkey yet"), "{page}");
    let mut later = Browser::new(&server);
    let (_, options) = later.post("/passkeys/login/options", json!({}));
    let assertion = authenticator.get(&options, 2, &user_handle);
    assert_eq!(later.post("/passkeys/login", assertion).0, 401);
}
