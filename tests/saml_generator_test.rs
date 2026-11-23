//! `soli generate saml` — SAML 2.0 single sign-on on top of session auth:
//! what it writes, that it never clobbers, that it is fmt/lint clean, and a
//! served app signed in to by a stand-in identity provider
//! (`Saml.test_response`, run through the binary), with the replays and
//! forgeries it must refuse.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde_json::{json, Value as Json};
use solilang::scaffold::{create_auth, create_saml};

const SAML_FILES: [&str; 4] = [
    "app/models/saml_identity.sl",
    "app/models/saml_assertion.sl",
    "app/services/saml_sso.sl",
    "app/controllers/saml_controller.sl",
];

const IDP_KEY: &str = include_str!("fixtures/saml/idp.key");
const IDP_CERT: &str = include_str!("fixtures/saml/idp.crt");
const OTHER_KEY: &str = include_str!("fixtures/saml/other.key");

const ORIGIN: &str = "https://app.test";
/// Behind an https:// APP_BASE_URL the request cookie carries the `__Host-`
/// prefix.
const REQUEST_COOKIE: &str = "__Host-saml_request";
const EMAIL_FORMAT: &str = "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress";
const IDP: &str = "https://idp.test/metadata";
const SSO: &str = "https://idp.test/sso";

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

fn saml_app() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    let folder = dir.path().to_str().unwrap();
    create_auth(folder).expect("generate auth");
    create_saml(folder).expect("generate saml");
    dir
}

#[test]
fn saml_writes_its_files_migration_and_routes() {
    let dir = saml_app();
    let root = dir.path();
    for rel in SAML_FILES {
        assert!(root.join(rel).exists(), "{rel} not created");
    }
    assert_eq!(migrations(root, "create_saml_identities"), 1);
    assert_eq!(migrations(root, "create_saml_assertions"), 1);
    let routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    for route in [
        r#"get("/saml/login", "saml#login")"#,
        r#"post("/saml/acs", "saml#acs")"#,
        r#"get("/saml/metadata", "saml#metadata")"#,
        r#"skip_csrf("/saml/acs")"#,
    ] {
        assert!(routes.contains(route), "missing {route}");
    }
}

#[test]
fn saml_requires_auth_and_never_clobbers() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    let folder = dir.path().to_str().unwrap();
    let err = create_saml(folder).unwrap_err();
    assert!(err.contains("soli generate auth"), "{err}");

    create_auth(folder).unwrap();
    create_saml(folder).unwrap();
    fs::write(dir.path().join("app/services/saml_sso.sl"), "# mine\n").unwrap();
    create_saml(folder).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("app/services/saml_sso.sl")).unwrap(),
        "# mine\n"
    );
    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    assert_eq!(routes.matches("\"saml#acs\"").count(), 1);
    assert_eq!(migrations(dir.path(), "create_saml_identities"), 1);
}

fn soli(dir: &Path, args: &[&str]) -> (bool, String) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(args)
        .current_dir(dir)
        .env_remove("SOLI_ENGINE")
        .output()
        .expect("run soli");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

#[test]
fn cli_generates_saml_and_generated_code_passes_fmt_and_lint() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    let (ok, output) = soli(dir.path(), &["generate", "saml"]);
    assert!(!ok, "{output}");
    assert!(output.contains("soli generate auth"), "{output}");
    assert!(soli(dir.path(), &["generate", "auth"]).0);
    let (ok, output) = soli(dir.path(), &["generate", "saml"]);
    assert!(ok, "{output}");
    assert!(output.contains("/saml/metadata"), "{output}");
    assert!(dir.path().join("app/services/saml_sso.sl").exists());

    let (formatted, output) = soli(dir.path(), &["fmt", "--check", "."]);
    assert!(formatted, "soli fmt --check:\n{output}");
    let (clean, output) = soli(dir.path(), &["lint", "."]);
    assert!(clean, "soli lint:\n{output}");
}

/// Route comments above every public action, `_` helpers under `private`,
/// `@` rather than `this.` outside `static`, `nil` rather than `null`.
#[test]
fn generated_saml_follows_the_house_style() {
    let dir = saml_app();
    for rel in SAML_FILES {
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
                        above.starts_with("# GET ") || above.starts_with("# POST "),
                        "{rel}: action {name} has no route comment"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Served end-to-end
// ---------------------------------------------------------------------------

/// A port for a test server, never handed out twice by this test binary.
///
/// Binding port 0 and releasing it gives a free port — which the kernel may
/// give again to the next test that asks before the first server has bound
/// it. The second server then failed to start, and its test talked to the
/// first one: another app, another database ("sign-up is closed" on a fresh
/// app, a table that does not exist).
fn unused_port() -> u16 {
    static TAKEN: std::sync::Mutex<Vec<u16>> = std::sync::Mutex::new(Vec::new());
    loop {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .unwrap();
        let mut taken = TAKEN.lock().unwrap_or_else(|e| e.into_inner());
        if !taken.contains(&port) {
            taken.push(port);
            return port;
        }
    }
}

struct Server(std::process::Child, u16);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn serve(root: &Path, provision: &str) -> Server {
    serve_with(root, provision, &[])
}

/// `serve`, with more environment (it wins over the defaults).
fn serve_with(root: &Path, provision: &str, extra: &[(&str, &str)]) -> Server {
    serve_workers(root, provision, extra, 1)
}

/// `serve_with`, on `workers` request workers: requests sent together run
/// side by side.
fn serve_workers(root: &Path, provision: &str, extra: &[(&str, &str)], workers: usize) -> Server {
    let port = unused_port();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_soli"));
    command
        .arg("serve")
        .arg(root)
        .args([
            "--port",
            &port.to_string(),
            "--workers",
            &workers.to_string(),
        ])
        .current_dir(root)
        .env_remove("SOLI_ENGINE")
        .env_remove("SAML_IDP_METADATA_FILE")
        .env_remove("SAML_SP_ENTITY_ID")
        .env("AUTH_SIGNUP", "open")
        // A test run: the first account needs no AUTH_SETUP_TOKEN.
        .env("APP_ENV", "test")
        .env("AUTH_ATTEMPTS_PER_IP", "1000")
        .env("APP_BASE_URL", ORIGIN)
        .env("SAML_IDP_ENTITY_ID", IDP)
        .env("SAML_IDP_SSO_URL", SSO)
        .env("SAML_IDP_CERTIFICATE", IDP_CERT)
        .env("SAML_PROVISION", provision)
        .env(
            "SOLI_SESSION_SECRET",
            "saml-test-secret-0123456789abcdef-0123",
        )
        .env("SOLI_CSRF_TOKENS", "require")
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
        .env("SOLI_MAIL_DELIVERY_METHOD", "logger")
        .env("SOLI_FAIL_ON_VM_DEMOTION", "1")
        .env_remove("SAML_LINK_EXISTING")
        .env_remove("SAML_LINK_DIRECTORY")
        .env_remove("SAML_SESSION_TTL")
        .envs(extra.iter().copied())
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

struct Reply {
    status: u16,
    body: String,
    location: String,
}

/// A browser stand-in: a cookie jar.
struct Browser {
    port: u16,
    jar: BTreeMap<String, String>,
}

impl Browser {
    fn new(server: &Server) -> Self {
        Browser {
            port: server.1,
            jar: BTreeMap::new(),
        }
    }

    /// `origin` is the site the request comes from: the application's own
    /// for its forms, the identity provider's for the response it posts.
    fn request(&mut self, method: &str, path: &str, body: Option<Json>, origin: &str) -> Reply {
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
        if method != "GET" {
            request = request.set("Origin", origin);
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
        Reply {
            status: response.status(),
            location: response.header("location").unwrap_or_default().to_string(),
            body: response.into_string().unwrap_or_default(),
        }
    }

    fn get(&mut self, path: &str) -> Reply {
        self.request("GET", path, None, "")
    }

    /// Start a sign-in; the id of the request the identity provider receives.
    fn start(&mut self, path: &str) -> String {
        self.start_at(path, ORIGIN, REQUEST_COOKIE)
    }

    /// `start`, for an application at `origin` whose request cookie is `cookie`.
    fn start_at(&mut self, path: &str, origin: &str, cookie: &str) -> String {
        let reply = self.get(path);
        assert_eq!(reply.status, 302, "{}", reply.body);
        assert!(
            reply.location.starts_with(&format!("{SSO}?SAMLRequest=")),
            "{}",
            reply.location
        );
        assert!(self.jar.contains_key(cookie), "no request cookie {cookie}");
        let encoded = reply.location.split("SAMLRequest=").nth(1).unwrap();
        let deflated = STANDARD
            .decode(urlencoding::decode(encoded).unwrap().as_bytes())
            .unwrap();
        let mut xml = String::new();
        flate2::read::DeflateDecoder::new(&deflated[..])
            .read_to_string(&mut xml)
            .unwrap();
        assert!(
            xml.contains(&format!(
                "AssertionConsumerServiceURL=\"{origin}/saml/acs\""
            )),
            "{xml}"
        );
        let id = xml
            .split("ID=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        id.to_string()
    }

    /// The identity provider's cross-site POST of `response`.
    fn post_response(&mut self, response: &str) -> Reply {
        self.request(
            "POST",
            "/saml/acs",
            Some(json!({"SAMLResponse": response})),
            "https://idp.test",
        )
    }

    fn whoami(&mut self) -> String {
        self.get("/whoami").body
    }
}

/// What the stand-in identity provider signs, with overrides.
fn issue(root: &Path, request_id: &str, overrides: Json) -> String {
    let mut opts = json!({
        "private_key": IDP_KEY, "issuer": IDP, "acs_url": format!("{ORIGIN}/saml/acs"),
        "audience": format!("{ORIGIN}/saml/metadata"), "name_id": "ada-at-idp",
        "name_id_format": "urn:oasis:names:tc:SAML:2.0:nameid-format:persistent",
        "request_id": request_id, "attributes": {"Mail": "Ada@Example.com"},
    });
    for (key, value) in overrides.as_object().unwrap() {
        opts[key] = value.clone();
    }
    // A JSON object is a Soli hash literal.
    fs::write(
        root.join("idp.sl"),
        format!("print(Saml.test_response({opts}))\n"),
    )
    .unwrap();
    let (ok, output) = soli(root, &["idp.sl"]);
    assert!(ok, "{output}");
    output.trim().to_string()
}

fn served_app() -> tempfile::TempDir {
    let dir = saml_app();
    let root = dir.path();
    fs::create_dir_all(root.join("app/views/layouts")).unwrap();
    fs::write(
        root.join("app/views/layouts/application.html.slv"),
        "<main><%= yield %></main>\n",
    )
    .unwrap();
    fs::write(
        root.join("app/controllers/whoami_controller.sl"),
        "class WhoamiController < Controller\n  # GET /whoami\n  def index\n    user = current_user()\n    \
         render_text(user.nil? ? \"guest\" : \"#{user.email} #{user.role} #{user._key}\")\n  end\nend\n",
    )
    .unwrap();
    let mut routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    routes.push_str("get(\"/whoami\", \"whoami#index\")\n");
    fs::write(root.join("config/routes.sl"), routes).unwrap();
    dir
}

#[test]
fn served_app_signs_in_through_the_identity_provider() {
    let dir = served_app();
    let root = dir.path();
    let server = serve_with(root, "on", &[("SAML_LINK_EXISTING", "on")]);

    let mut guest = Browser::new(&server);
    let metadata = guest.get("/saml/metadata");
    assert_eq!(metadata.status, 200, "{}", metadata.body);
    assert!(
        metadata
            .body
            .contains(&format!("entityID=\"{ORIGIN}/saml/metadata\"")),
        "{}",
        metadata.body
    );
    assert!(
        metadata
            .body
            .contains(&format!("Location=\"{ORIGIN}/saml/acs\"")),
        "{}",
        metadata.body
    );
    // A persistent NameID, not an address the identity provider may give
    // to someone else later.
    assert!(
        metadata.body.contains(
            "<md:NameIDFormat>urn:oasis:names:tc:SAML:2.0:nameid-format:persistent</md:NameIDFormat>"
        ),
        "{}",
        metadata.body
    );

    // A response with no sign-in in progress in this browser.
    let unasked = issue(root, "_nobody", json!({}));
    assert_eq!(guest.post_response(&unasked).status, 401);
    assert_eq!(guest.whoami(), "guest");

    // Sign in: a first visit creates the account, as a member.
    let mut ada = Browser::new(&server);
    let request_id = ada.start("/saml/login?return_to=/whoami");
    let response = issue(root, &request_id, json!({}));
    // Whoever copies the response and the request cookie on the way.
    let mut thief = Browser {
        port: server.1,
        jar: ada.jar.clone(),
    };
    let reply = ada.post_response(&response);
    assert_eq!(
        (reply.status, reply.location.as_str()),
        (302, "/whoami"),
        "{}",
        reply.body
    );
    let who = ada.whoami();
    assert!(who.starts_with("ada@example.com member "), "{who}");
    assert!(!ada.jar.contains_key(REQUEST_COOKIE), "request cookie kept");
    // Posted again with the cookie that came with it: an assertion signs in once.
    assert_eq!(thief.post_response(&response).status, 401, "replayed");
    assert_eq!(thief.whoami(), "guest");

    // The same response again: the request was answered once.
    let mut again = Browser::new(&server);
    assert_eq!(again.post_response(&response).status, 401);
    // Even from a browser with its own sign-in in progress.
    again.start("/saml/login");
    assert_eq!(again.post_response(&response).status, 401);
    assert_eq!(again.whoami(), "guest");

    // Signed by someone else, or changed after signing.
    let mut eve = Browser::new(&server);
    let id = eve.start("/saml/login");
    let forged = issue(root, &id, json!({"private_key": OTHER_KEY}));
    assert_eq!(eve.post_response(&forged).status, 401);
    let id = eve.start("/saml/login");
    let genuine = issue(
        root,
        &id,
        json!({"name_id": "eve-at-idp", "attributes": {}}),
    );
    let xml = String::from_utf8(STANDARD.decode(&genuine).unwrap()).unwrap();
    let edited = STANDARD.encode(xml.replace("eve-at-idp", "ada-at-idp"));
    assert_eq!(eve.post_response(&edited).status, 401);
    // Meant for another application, or sent without an address to go by.
    let id = eve.start("/saml/login");
    let elsewhere = issue(root, &id, json!({"audience": "https://other.test"}));
    assert_eq!(eve.post_response(&elsewhere).status, 401);
    let id = eve.start("/saml/login");
    assert_eq!(eve.post_response(&genuine).status, 401, "stale request id");
    let nameless = issue(
        root,
        &id,
        json!({"name_id": "eve-at-idp", "attributes": {}}),
    );
    assert_eq!(eve.post_response(&nameless).status, 401, "no email");
    assert_eq!(eve.whoami(), "guest");

    // A second sign-in is the same account, even with another address: the
    // link is the identity, not the email.
    let mut later = Browser::new(&server);
    let id = later.start("/saml/login?return_to=https://evil.test/");
    let response = issue(
        root,
        &id,
        json!({"attributes": {"Mail": "renamed@example.com"}}),
    );
    let reply = later.post_response(&response);
    assert_eq!((reply.status, reply.location.as_str()), (302, "/"));
    assert_eq!(later.whoami(), who);

    // With SAML_LINK_EXISTING=on, an account that already exists is linked by
    // its email, and keeps its role.
    let mut grace = Browser::new(&server);
    sign_up(&mut grace, "grace@example.com");
    let mut sso = Browser::new(&server);
    let id = sso.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "grace@example.com",
               "name_id_format": EMAIL_FORMAT,
               "attributes": {}}),
    );
    assert_eq!(sso.post_response(&response).status, 302);
    let who = sso.whoami();
    assert!(who.starts_with("grace@example.com member"), "{who}");
    // Grace never confirmed her address, so whoever signed up with it may not
    // be Grace: linking it to the identity provider drops that password, and
    // the session opened with it.
    let login = grace.request(
        "POST",
        "/login",
        Some(json!({"email": "grace@example.com", "password": "correct horse battery"})),
        &format!("http://127.0.0.1:{}", server.1),
    );
    assert_eq!(
        login.status, 401,
        "the sign-up password still works: {}",
        login.body
    );
    assert_eq!(grace.whoami(), "guest", "the sign-up session survived");

    // The CSRF exemption is that one route.
    let cross = guest.request(
        "POST",
        "/login",
        Some(json!({"email": "x@example.com", "password": "x"})),
        "https://idp.test",
    );
    assert_eq!(cross.status, 403, "{}", cross.body);
}

#[test]
fn provisioning_off_refuses_a_user_with_no_account() {
    let dir = served_app();
    let server = serve(dir.path(), "off");
    let mut ada = Browser::new(&server);
    let id = ada.start("/saml/login");
    let response = issue(dir.path(), &id, json!({}));
    assert_eq!(ada.post_response(&response).status, 401);
    assert_eq!(ada.whoami(), "guest");
}

/// An admin's account is never taken over by an assertion that carries its
/// email: the identity provider may let a user set that attribute.
#[test]
fn an_admin_account_is_not_linked_by_its_email() {
    let dir = served_app();
    let root = dir.path();
    let server = serve_with(
        root,
        "on",
        &[("AUTH_SIGNUP", "first_user"), ("SAML_LINK_EXISTING", "on")],
    );
    let mut admin = Browser::new(&server);
    let signup = admin.request(
        "POST",
        "/signup",
        Some(json!({"email": "root@example.com", "password": "correct horse battery"})),
        &format!("http://127.0.0.1:{}", server.1),
    );
    assert_eq!(signup.status, 302, "{}", signup.body);
    assert!(admin.whoami().starts_with("root@example.com admin "));

    let mut eve = Browser::new(&server);
    let id = eve.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"attributes": {"email": "root@example.com"}}),
    );
    assert_eq!(eve.post_response(&response).status, 401);
    assert_eq!(eve.whoami(), "guest");
}

/// `soli runner -e code` against the served app's database: the last line it
/// prints.
fn runner(root: &Path, code: &str) -> String {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(["runner", "-e", code])
        .current_dir(root)
        .env_remove("SOLI_ENGINE")
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
        .output()
        .expect("run soli runner");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout.trim().lines().last().unwrap_or_default().to_string()
}

/// Sign up through the form, as a browser on the application's own origin.
fn sign_up(browser: &mut Browser, email: &str) {
    let origin = format!("http://127.0.0.1:{}", browser.port);
    let signup = browser.request(
        "POST",
        "/signup",
        Some(json!({"email": email, "password": "correct horse battery"})),
        &origin,
    );
    // Open sign-up answers "check your mailbox": the account exists,
    // unconfirmed, and nobody is signed in to it.
    assert_eq!(signup.status, 200, "{}", signup.body);
}

/// `sign_up`, then what the mailed link does (written straight to the
/// database), then a sign-in: a confirmed account with a session.
fn join(browser: &mut Browser, root: &Path, email: &str) {
    sign_up(browser, email);
    // The server's write may not be visible to a second connection the
    // instant its response arrives (the table itself is created by the first
    // sign-up): retry briefly.
    let mut changed = 0;
    for _ in 0..60 {
        changed = rusqlite::Connection::open(root.join("app.sqlite3"))
            .and_then(|conn| {
                conn.execute(
                    "UPDATE users SET doc = json_set(doc, '$.confirmed_at', 1) \
                     WHERE json_extract(doc, '$.email') = ?1",
                    [email],
                )
            })
            .unwrap_or(0);
        if changed == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(changed, 1, "no account for {email}");
    let origin = format!("http://127.0.0.1:{}", browser.port);
    let login = browser.request(
        "POST",
        "/login",
        Some(json!({"email": email, "password": "correct horse battery"})),
        &origin,
    );
    assert_eq!(login.status, 302, "{}", login.body);
}

/// Linking an existing (confirmed) account by its email is opt-in: unset or
/// `off`, the account is refused and a newcomer is still provisioned.
#[test]
fn existing_accounts_are_linked_only_with_link_existing_on() {
    for setting in [None, Some("off")] {
        let dir = served_app();
        let root = dir.path();
        let extra: Vec<(&str, &str)> = setting
            .map(|value| vec![("SAML_LINK_EXISTING", value)])
            .unwrap_or_default();
        let server = serve_with(root, "on", &extra);
        let mut grace = Browser::new(&server);
        join(&mut grace, root, "grace@example.com");
        let existing = grace.whoami();
        // Confirmed: an unconfirmed account is taken over even so (see
        // `an_unconfirmed_account_is_taken_over_with_linking_off`).
        assert_eq!(
            runner(
                root,
                r#"User.find_by("email", "grace@example.com").confirm_email
print("confirmed")"#
            ),
            "confirmed"
        );

        let mut sso = Browser::new(&server);
        let id = sso.start("/saml/login");
        let response = issue(
            root,
            &id,
            json!({"attributes": {"email": "grace@example.com"}}),
        );
        assert_eq!(sso.post_response(&response).status, 401, "{setting:?}");
        assert_eq!(sso.whoami(), "guest", "{setting:?}");
        assert_eq!(grace.whoami(), existing, "{setting:?}");
        // A newcomer is still provisioned.
        let id = sso.start("/saml/login");
        let response = issue(
            root,
            &id,
            json!({"name_id": "new-at-idp", "attributes": {"email": "new@example.com"}}),
        );
        assert_eq!(sso.post_response(&response).status, 302, "{setting:?}");
        assert!(sso.whoami().starts_with("new@example.com member "));
    }
}

/// An address already linked to someone at this identity provider names
/// another person there now (a reused mailbox): it is not linked again.
#[test]
fn an_account_linked_at_the_issuer_is_not_linked_again_by_email() {
    let dir = served_app();
    let root = dir.path();
    let server = serve_with(root, "on", &[("SAML_LINK_EXISTING", "on")]);
    let mut alice = Browser::new(&server);
    let id = alice.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "emp-001", "attributes": {"email": "alice@corp.test"}}),
    );
    assert_eq!(alice.post_response(&response).status, 302);

    let mut successor = Browser::new(&server);
    let id = successor.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "emp-999", "attributes": {"email": "alice@corp.test"}}),
    );
    assert_eq!(successor.post_response(&response).status, 401);
    assert_eq!(successor.whoami(), "guest");
}

/// The NameID stands for an address only in the emailAddress format; an
/// address with anything but ASCII in it is refused (lowercasing folds the
/// Kelvin sign into a k).
#[test]
fn only_an_email_nameid_or_an_ascii_address_is_an_email() {
    let dir = served_app();
    let root = dir.path();
    let server = serve_with(root, "on", &[("SAML_LINK_EXISTING", "on")]);
    let mut kim = Browser::new(&server);
    join(&mut kim, root, "kim@kk.test");

    let mut eve = Browser::new(&server);
    let id = eve.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "nobody@example.com",
               "name_id_format": "urn:oasis:names:tc:SAML:1.1:nameid-format:unspecified",
               "attributes": {}}),
    );
    assert_eq!(eve.post_response(&response).status, 401, "a username");
    let id = eve.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "kelvin", "attributes": {"email": "kim@\u{212A}\u{212A}.test"}}),
    );
    assert_eq!(eve.post_response(&response).status, 401, "non-ASCII");
    assert_eq!(eve.whoami(), "guest");

    let id = eve.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "Eve@Example.com", "name_id_format": EMAIL_FORMAT, "attributes": {}}),
    );
    assert_eq!(eve.post_response(&response).status, 302);
    assert!(eve.whoami().starts_with("eve@example.com member "));
}

/// Unset, SAML_PROVISION follows AUTH_SIGNUP: a closed sign-up provisions
/// nobody through the identity provider either, unless SAML_PROVISION=on.
#[test]
fn provisioning_follows_a_closed_sign_up() {
    for (provision, status) in [("", 401), ("on", 302)] {
        let dir = served_app();
        let server = serve_with(dir.path(), provision, &[("AUTH_SIGNUP", "closed")]);
        let mut ada = Browser::new(&server);
        let id = ada.start("/saml/login");
        let response = issue(dir.path(), &id, json!({}));
        assert_eq!(
            ada.post_response(&response).status,
            status,
            "SAML_PROVISION={provision:?}"
        );
    }
}

/// An account the identity provider created signs in there only: no reset
/// link, no password sign-in, even with a password in its row.
#[test]
fn an_sso_only_account_has_no_password_way_in() {
    let dir = served_app();
    let root = dir.path();
    let server = serve(root, "on");
    let mut ada = Browser::new(&server);
    let id = ada.start("/saml/login");
    assert_eq!(ada.post_response(&issue(root, &id, json!({}))).status, 302);
    let origin = format!("http://127.0.0.1:{}", server.1);

    let mut guest = Browser::new(&server);
    let reset = guest.request(
        "POST",
        "/password/reset",
        Some(json!({"email": "ada@example.com"})),
        &origin,
    );
    assert_eq!(reset.status, 200, "{}", reset.body);
    assert_eq!(
        runner(
            root,
            r##"user = User.find_by("email", "ada@example.com")
print("#{user.sso_only?} #{user.reset_token_digest.nil?}")"##
        ),
        "true true"
    );

    // A token and a password planted in the row change nothing.
    assert_eq!(
        runner(
            root,
            r##"user = User.find_by("email", "ada@example.com")
user.set_password("brand new password 1")
user.reset_token_digest = Crypto.sha256("planted-token")
user.reset_sent_at = DateTime.utc.to_unix
user.save
print("planted")"##
        ),
        "planted"
    );
    let edit = guest.get("/password/edit?token=planted-token");
    assert!(
        edit.body.contains("invalid or has expired"),
        "{}",
        edit.body
    );
    let update = guest.request(
        "POST",
        "/password/update",
        Some(
            json!({"token": "planted-token", "password": "another password 22",
                    "password_confirmation": "another password 22"}),
        ),
        &origin,
    );
    assert!(
        update.body.contains("invalid or has expired"),
        "{}",
        update.body
    );
    let login = guest.request(
        "POST",
        "/login",
        Some(json!({"email": "ada@example.com", "password": "brand new password 1"})),
        &origin,
    );
    assert_eq!(login.status, 401, "{}", login.body);
    assert_eq!(guest.whoami(), "guest");
}

/// Taking over an unconfirmed account drops every credential of whoever
/// created it (`User#revoke_credentials`), the rows of the optional
/// generators included; left with no password, it is SSO-only.
#[test]
fn taking_over_an_unconfirmed_account_revokes_its_credentials() {
    let dir = served_app();
    let root = dir.path();
    let server = serve_with(root, "on", &[("SAML_LINK_EXISTING", "on")]);
    let mut grace = Browser::new(&server);
    sign_up(&mut grace, "grace@example.com");
    assert_eq!(
        runner(
            root,
            r##"user = User.find_by("email", "grace@example.com")
SamlIdentity.create({"user_id": user._key, "issuer": "https://old-idp.test", "name_id": "g"})
print(SamlIdentity.where({"user_id": user._key}).count)"##
        ),
        "1"
    );

    let mut sso = Browser::new(&server);
    let id = sso.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"attributes": {"email": "grace@example.com"}}),
    );
    assert_eq!(sso.post_response(&response).status, 302);
    assert_eq!(grace.whoami(), "guest", "the sign-up session survived");
    assert_eq!(
        runner(
            root,
            r##"user = User.find_by("email", "grace@example.com")
old = SamlIdentity.where({"issuer": "https://old-idp.test"}).count
print("#{user.password_digest.nil?} #{user.confirmed?} #{user.sso_only?} #{old}")"##
        ),
        "true true true 0"
    );
}

/// Anyone can make a browser post to /saml/acs: a post that does not verify
/// leaves the sign-in in progress alone.
#[test]
fn a_forged_post_does_not_cancel_a_sign_in() {
    let dir = served_app();
    let root = dir.path();
    let server = serve(root, "on");
    let mut ada = Browser::new(&server);
    let id = ada.start("/saml/login");
    let junk = ada.request("POST", "/saml/acs", Some(json!({})), "https://evil.test");
    assert_eq!(junk.status, 401);
    assert!(
        ada.jar.contains_key(REQUEST_COOKIE),
        "request cookie cleared"
    );
    let reply = ada.post_response(&issue(root, &id, json!({})));
    assert_eq!(reply.status, 302, "{}", reply.body);
    assert!(!ada.jar.contains_key(REQUEST_COOKIE), "request cookie kept");
}

/// Without the identity provider's entity id there is no Issuer to check:
/// nothing signs in.
#[test]
fn no_identity_provider_entity_id_no_sign_in() {
    let dir = served_app();
    let root = dir.path();
    let (jar, id) = {
        let server = serve(root, "on");
        let mut ada = Browser::new(&server);
        let id = ada.start("/saml/login");
        (ada.jar, id)
    };
    let server = serve_with(root, "on", &[("SAML_IDP_ENTITY_ID", "")]);
    let mut ada = Browser {
        port: server.1,
        jar,
    };
    let response = issue(root, &id, json!({"issuer": "https://elsewhere.test"}));
    assert_eq!(ada.post_response(&response).status, 401);
    assert_eq!(ada.whoami(), "guest");
    assert_ne!(ada.get("/saml/login").status, 302);
}

/// `__Host-` needs Secure, which a browser refuses over plain http: behind an
/// http:// APP_BASE_URL the request cookie has its plain name.
#[test]
fn a_plain_http_base_url_keeps_a_plain_cookie_name() {
    let dir = served_app();
    let root = dir.path();
    let origin = "http://app.test";
    let server = serve_with(root, "on", &[("APP_BASE_URL", origin)]);
    let mut ada = Browser::new(&server);
    let id = ada.start_at("/saml/login", origin, "saml_request");
    assert!(!ada.jar.contains_key(REQUEST_COOKIE));
    let response = issue(
        root,
        &id,
        json!({"acs_url": format!("{origin}/saml/acs"),
               "audience": format!("{origin}/saml/metadata")}),
    );
    assert_eq!(ada.post_response(&response).status, 302);
}

/// One identity per account at each identity provider, held by the database
/// too: two first sign-ins racing cannot both link one account.
#[test]
fn the_identities_migration_holds_one_identity_per_account_and_issuer() {
    let dir = saml_app();
    let migration = fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .contains("create_saml_identities")
        })
        .expect("no identities migration");
    let source = fs::read_to_string(migration.path()).unwrap();
    assert!(
        source.contains(r#"["user_id", "issuer"], {"unique": true}"#),
        "{source}"
    );
    assert!(
        source.contains(r#"["issuer", "name_id"], {"unique": true}"#),
        "{source}"
    );
}

/// Whoever registered an address first, without proving it, does not keep
/// it from its owner — where an unconfirmed account cannot have been used.
/// With `auth_require_confirmed_email` on nobody signs in to one, so with
/// linking off the first sign-in through the identity provider takes it over
/// (dropping the squatter's password), as long as a first sign-in may create
/// accounts at all.
#[test]
fn an_unconfirmed_account_is_taken_over_only_where_it_cannot_have_been_used() {
    let squat = |server: &Server| {
        let mut squatter = Browser::new(server);
        let origin = format!("http://127.0.0.1:{}", server.1);
        squatter.request(
            "POST",
            "/signup",
            Some(json!({"email": "hire@corp.test", "password": "correct horse battery"})),
            &origin,
        );
        squatter
    };
    let assertion = json!({"name_id": "hire-at-idp", "attributes": {"email": "hire@corp.test"}});
    let login = json!({"email": "hire@corp.test", "password": "correct horse battery"});

    let dir = served_app();
    let root = dir.path();
    let server = serve(root, "on");
    let mut squatter = squat(&server);
    assert_eq!(
        squatter.whoami(),
        "guest",
        "an unconfirmed account signed in"
    );
    let mut hire = Browser::new(&server);
    let id = hire.start("/saml/login");
    assert_eq!(
        hire.post_response(&issue(root, &id, assertion.clone()))
            .status,
        302
    );
    assert!(hire.whoami().starts_with("hire@corp.test member"));
    let origin = format!("http://127.0.0.1:{}", server.1);
    let attempt = squatter.request("POST", "/login", Some(login.clone()), &origin);
    assert_eq!(attempt.status, 401, "the squatter's password still works");
    drop(server);

    // Where an unconfirmed account does sign in — sign-up is not open, the
    // account was made by an administrator — it may be somebody's. The
    // identity provider's word for an address does not hand it over: the
    // account, its session and its password stay as they were.
    let dir = served_app();
    let root = dir.path();
    let server = serve_with(root, "on", &[("AUTH_SIGNUP", "closed")]);
    runner(
        root,
        r##"user = User.new
user.email = "hire@corp.test"
user.role = "member"
user.set_password("correct horse battery")
user.save
print(user.confirmed?)"##,
    );
    let origin = format!("http://127.0.0.1:{}", server.1);
    let mut owner = Browser::new(&server);
    assert_eq!(
        owner
            .request("POST", "/login", Some(login.clone()), &origin)
            .status,
        302,
        "an administrator-made account could not sign in"
    );
    let account = owner.whoami();
    assert!(account.starts_with("hire@corp.test member"), "{account}");
    let mut claimant = Browser::new(&server);
    let id = claimant.start("/saml/login");
    assert_eq!(
        claimant
            .post_response(&issue(root, &id, assertion.clone()))
            .status,
        401
    );
    assert_eq!(claimant.whoami(), "guest");
    assert_eq!(owner.whoami(), account, "the account's session was closed");
    let mut again = Browser::new(&server);
    assert_eq!(
        again.request("POST", "/login", Some(login), &origin).status,
        302,
        "the account's password was revoked"
    );
    drop(server);

    // Provisioning off: nobody new signs in, an unconfirmed account neither.
    let dir = served_app();
    let root = dir.path();
    let server = serve(root, "off");
    squat(&server);
    let mut hire = Browser::new(&server);
    let id = hire.start("/saml/login");
    assert_eq!(hire.post_response(&issue(root, &id, assertion)).status, 401);
    assert_eq!(hire.whoami(), "guest");
}

/// An account a directory provisioned (SCIM) is linked by its first SAML
/// sign-in with SAML_LINK_EXISTING off only when the directory named that
/// identity (externalId == NameID) or SAML_LINK_DIRECTORY=on; the email alone
/// is refused. Linked with no password, it is SSO-only from then on.
#[test]
fn a_directory_account_is_linked_only_on_its_external_id_or_by_opt_in() {
    let dir = served_app();
    let root = dir.path();
    assert_eq!(
        runner(
            root,
            r##"a = User.create({"email": "dir@corp.test", "role": "member", "confirmed_at": 1, "scim_provisioned_at": 1})
b = User.create({"email": "ext@corp.test", "role": "member", "confirmed_at": 1, "scim_provisioned_at": 1, "scim_external_id": "ext-at-idp"})
print("#{a._errors.nil?} #{b._errors.nil?}")"##
        ),
        "true true"
    );

    {
        let server = serve(root, "on");
        // The email of a directory account, under another NameID: refused.
        let mut stranger = Browser::new(&server);
        let id = stranger.start("/saml/login");
        let response = issue(
            root,
            &id,
            json!({"name_id": "someone-else", "attributes": {"email": "dir@corp.test"}}),
        );
        assert_eq!(stranger.post_response(&response).status, 401);
        assert_eq!(stranger.whoami(), "guest");

        // The directory named this identity: linked.
        let mut ext = Browser::new(&server);
        let id = ext.start("/saml/login");
        let response = issue(
            root,
            &id,
            json!({"name_id": "ext-at-idp", "attributes": {"email": "ext@corp.test"}}),
        );
        assert_eq!(ext.post_response(&response).status, 302);
        assert!(ext.whoami().starts_with("ext@corp.test member "));
    }

    let server = serve_with(root, "on", &[("SAML_LINK_DIRECTORY", "on")]);
    let mut dee = Browser::new(&server);
    let id = dee.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "dee-at-idp", "attributes": {"email": "dir@corp.test"}}),
    );
    assert_eq!(dee.post_response(&response).status, 302);
    assert!(dee.whoami().starts_with("dir@corp.test member "));
    assert_eq!(
        runner(
            root,
            r##"user = User.find_by("email", "dir@corp.test")
    print("#{user.sso_only?} #{SamlIdentity.where({"user_id": user._key}).count}")"##
        ),
        "true 1"
    );
}

/// There is no single logout: a SAML sign-in ends here after
/// SAML_SESSION_TTL, so a user removed at the identity provider does not keep
/// a session for the whole session-store TTL.
#[test]
fn a_saml_session_ends_after_its_ttl() {
    let dir = served_app();
    let root = dir.path();
    let server = serve_with(root, "on", &[("SAML_SESSION_TTL", "2")]);
    let mut ada = Browser::new(&server);
    let id = ada.start("/saml/login");
    assert_eq!(ada.post_response(&issue(root, &id, json!({}))).status, 302);
    assert!(ada.whoami().starts_with("ada@example.com member "));
    std::thread::sleep(std::time::Duration::from_secs(3));
    assert_eq!(ada.whoami(), "guest");
}

/// Wrong passwords typed for an address that has none count for nothing: a
/// stranger cannot lock its owner out, and the password lockout never stands
/// in the way of the identity provider's sign-in.
#[test]
fn wrong_passwords_do_not_lock_out_single_sign_on() {
    let dir = served_app();
    let root = dir.path();
    let server = serve(root, "on");
    let mut ada = Browser::new(&server);
    let id = ada.start("/saml/login");
    assert_eq!(ada.post_response(&issue(root, &id, json!({}))).status, 302);

    let mut stranger = Browser::new(&server);
    for attempt in 0..11 {
        let login = stranger.request(
            "POST",
            "/login",
            Some(json!({"email": "ada@example.com", "password": format!("wrong password {attempt}")})),
            &format!("http://127.0.0.1:{}", server.1),
        );
        assert_eq!(login.status, 401);
    }
    assert_eq!(
        runner(
            root,
            r##"user = User.find_by("email", "ada@example.com")
    print("#{user.failed_attempts.nil?} #{user.locked_at.nil?}")"##
        ),
        "true true"
    );
    let mut again = Browser::new(&server);
    let id = again.start("/saml/login");
    assert_eq!(
        again.post_response(&issue(root, &id, json!({}))).status,
        302
    );

    // A lock stamped all the same (an older app) does not refuse it either.
    assert_eq!(
        runner(
            root,
            r#"user = User.find_by("email", "ada@example.com")
user.locked_at = DateTime.utc.to_unix
user.failed_attempts = 10
user.save
print(user.locked?)"#
        ),
        "true"
    );
    let mut locked = Browser::new(&server);
    let id = locked.start("/saml/login");
    assert_eq!(
        locked.post_response(&issue(root, &id, json!({}))).status,
        302
    );
    assert!(locked.whoami().starts_with("ada@example.com member "));
}

/// An emailAddress NameID is stored and matched lowercased: the identity
/// provider changing its case does not lock the user out. One stored exactly
/// (before NameIDs were normalised) is still found.
#[test]
fn an_email_nameid_is_matched_whatever_its_case() {
    let dir = served_app();
    let root = dir.path();
    let server = serve(root, "on");
    let mut alice = Browser::new(&server);
    let id = alice.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "Alice@Corp.Test", "name_id_format": EMAIL_FORMAT, "attributes": {}}),
    );
    assert_eq!(alice.post_response(&response).status, 302);
    let who = alice.whoami();
    assert_eq!(
        runner(root, r#"print(SamlIdentity.all[0].name_id)"#),
        "alice@corp.test"
    );
    let mut later = Browser::new(&server);
    let id = later.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "ALICE@corp.test", "name_id_format": EMAIL_FORMAT, "attributes": {}}),
    );
    assert_eq!(later.post_response(&response).status, 302);
    assert_eq!(later.whoami(), who);

    // Stored exactly by an older version of this file.
    assert_eq!(
        runner(
            root,
            &format!(
                r#"user = User.create({{"email": "bob@corp.test", "role": "member", "confirmed_at": 1}})
SamlIdentity.create({{"user_id": user._key, "issuer": "{IDP}", "name_id": "Bob@Corp.Test", "email": "bob@corp.test"}})
print("seeded")"#
            )
        ),
        "seeded"
    );
    let mut bob = Browser::new(&server);
    let id = bob.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "Bob@Corp.Test", "name_id_format": EMAIL_FORMAT, "attributes": {}}),
    );
    assert_eq!(bob.post_response(&response).status, 302);
    assert!(bob.whoami().starts_with("bob@corp.test member "));
}

/// The address comes from the first attribute of SAML_EMAIL_ATTRIBUTES the
/// assertion carries, in that order — not from whichever the response lists
/// first — and a blank one is passed over.
#[test]
fn email_attributes_follow_the_configured_order() {
    let dir = served_app();
    let root = dir.path();
    let server = serve(root, "on");
    let mut first = Browser::new(&server);
    let id = first.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "n1", "attributes": {"mail": "second@corp.test", "email": "first@corp.test"}}),
    );
    assert_eq!(first.post_response(&response).status, 302);
    assert!(first.whoami().starts_with("first@corp.test member "));
    let mut blank = Browser::new(&server);
    let id = blank.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "n2", "attributes": {"Email": "", "mail": "fallback@corp.test"}}),
    );
    assert_eq!(blank.post_response(&response).status, 302);
    assert!(blank.whoami().starts_with("fallback@corp.test member "));
}

/// In production with `first_user` sign-up, the first account is the admin,
/// made at /signup with AUTH_SETUP_TOKEN: a SAML sign-in does not take its
/// place (and close /signup) before it exists.
#[test]
fn production_first_user_waits_for_the_admin() {
    let dir = served_app();
    let root = dir.path();
    let server = serve_with(
        root,
        "on",
        &[
            ("AUTH_SIGNUP", "first_user"),
            ("APP_ENV", "production"),
            ("AUTH_SETUP_TOKEN", "a-setup-token-of-twenty-chars"),
            ("SOLI_APP_HOSTS", "app.test,127.0.0.1"),
        ],
    );
    let mut ada = Browser::new(&server);
    let id = ada.start("/saml/login");
    assert_eq!(ada.post_response(&issue(root, &id, json!({}))).status, 401);
    assert_eq!(ada.whoami(), "guest");
    assert_eq!(ada.get("/signup").status, 200, "sign-up closed");
    assert_eq!(runner(root, "print(User.count)"), "0");
}

/// A take-over that stopped before its identity row was stored leaves an
/// account that is confirmed and single sign-on only: nothing can enter it,
/// and it is not taken over again. The take-over marks it, and the next
/// sign-in links it. An account that merely looks the same — single sign-on
/// only, no identity, because its identity was revoked on purpose — carries
/// no mark and is still refused: its address alone proves nothing.
#[test]
fn a_half_done_take_over_is_finished_and_a_revoked_account_is_not_relinked() {
    let dir = served_app();
    let root = dir.path();
    // SAML_LINK_EXISTING off: an existing account is not linked by address.
    let server = serve(root, "on");
    let state = r##"half = User.new
half.email = "half@example.com"
half.role = "member"
half.confirmed_at = DateTime.utc.to_unix
half.sso_only = true
half.saml_link_pending = true
half.save
revoked = User.new
revoked.email = "revoked@example.com"
revoked.role = "member"
revoked.confirmed_at = DateTime.utc.to_unix
revoked.sso_only = true
revoked.save
print("ready")"##;
    assert_eq!(runner(root, state), "ready");

    let mut half = Browser::new(&server);
    let id = half.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "half-at-idp", "attributes": {"email": "half@example.com"}}),
    );
    assert_eq!(half.post_response(&response).status, 302);
    assert!(half.whoami().starts_with("half@example.com member "));

    let mut revoked = Browser::new(&server);
    let id = revoked.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "revoked-at-idp", "attributes": {"email": "revoked@example.com"}}),
    );
    assert_eq!(revoked.post_response(&response).status, 401);
    assert_eq!(revoked.whoami(), "guest");

    assert_eq!(
        runner(
            root,
            r##"half = User.find_by("email", "half@example.com")
revoked = User.find_by("email", "revoked@example.com")
print("#{half.saml_link_pending} #{SamlIdentity.where({"user_id": half._key}).count} #{SamlIdentity.where({"user_id": revoked._key}).count}")"##
        ),
        "false 1 0"
    );
}

/// The mark the previous test writes by hand is the take-over's own: set by
/// `SamlSso.take_over_unconfirmed`, with the account made single sign-on
/// only, and cleared by `revoke_credentials` — an account whose identity is
/// taken away on purpose is not waiting to be linked.
#[test]
fn the_take_over_sets_its_mark_and_a_revocation_clears_it() {
    let dir = served_app();
    let root = dir.path();
    let code = r##"user = User.new
user.email = "placeholder@example.com"
user.role = "member"
user.set_password("a squatter chose this one")
user.awaiting_owner = true
user.save
SamlSso.take_over_unconfirmed(user)
taken = User.find(user._key)
marked = "#{taken.saml_link_pending} #{taken.sso_only?} #{taken.password_digest.nil?} #{taken.confirmed?}"
taken.revoke_credentials
print("#{marked} | #{User.find(user._key).saml_link_pending}")"##;
    assert_eq!(runner(root, code), "true true true true | false");
}

/// A link finished by hand — the identity row created by an administrator
/// on an account the take-over had marked — is complete at its next
/// sign-in: the mark goes, so that deleting the identity later does not
/// leave an account that is linked again by its address alone.
#[test]
fn a_sign_in_through_a_known_identity_clears_the_take_overs_mark() {
    let dir = served_app();
    let root = dir.path();
    let server = serve(root, "on");
    let state = r##"user = User.new
user.email = "ada@example.com"
user.role = "member"
user.confirmed_at = DateTime.utc.to_unix
user.sso_only = true
user.saml_link_pending = true
user.save
SamlIdentity.create({"user_id": user._key, "issuer": "IDP_ISSUER", "name_id": "ada-at-idp", "email": "ada@example.com"})
print("ready #{SamlIdentity.where({"user_id": user._key}).count}")"##
        .replace("IDP_ISSUER", IDP);
    assert_eq!(runner(root, &state), "ready 1");

    let mut ada = Browser::new(&server);
    let id = ada.start("/saml/login");
    assert_eq!(ada.post_response(&issue(root, &id, json!({}))).status, 302);
    assert!(ada.whoami().starts_with("ada@example.com member "));
    // Through the identity that was there — still the only one: the
    // by-address path, which also clears the mark, would have added a row.
    assert_eq!(
        runner(
            root,
            r##"user = User.find_by("email", "ada@example.com")
print("#{user.saml_link_pending} #{SamlIdentity.where({"user_id": user._key}).count}")"##
        ),
        "false 1"
    );
}

/// One response posted several times together with the request cookie it
/// answers — on separate workers, for real. An assertion signs in once.
#[test]
fn a_response_posted_together_signs_in_once() {
    let dir = served_app();
    let root = dir.path();
    let server = serve_workers(root, "on", &[], 4);
    let mut ada = Browser::new(&server);
    let request_id = ada.start("/saml/login");
    let response = issue(root, &request_id, json!({}));
    let gate = std::sync::Arc::new(std::sync::Barrier::new(4));
    let copies: Vec<_> = (0..4)
        .map(|_| {
            let mut copy = Browser {
                port: ada.port,
                jar: ada.jar.clone(),
            };
            let gate = gate.clone();
            let response = response.clone();
            std::thread::spawn(move || {
                gate.wait();
                let status = copy.post_response(&response).status;
                (status, copy.whoami())
            })
        })
        .collect();
    let replies: Vec<_> = copies.into_iter().map(|one| one.join().unwrap()).collect();
    let signed_in = replies.iter().filter(|(status, _)| *status == 302).count();
    assert_eq!(signed_in, 1, "{replies:?}");
    for (status, who) in &replies {
        assert!(*status == 302 || *status == 401, "{replies:?}");
        assert_eq!(
            *status == 302,
            who.starts_with("ada@example.com"),
            "{replies:?}"
        );
    }
}

/// Two first sign-ins of the same person sent together, each with its own
/// request and its own assertion — two tabs. Whatever each is answered, the
/// account that remains is one, it is linked, and the next sign-in works:
/// the request that lost must not remove the account the other one linked.
#[test]
fn two_first_sign_ins_together_leave_one_linked_account() {
    let dir = served_app();
    let root = dir.path();
    let server = serve_workers(root, "on", &[("SAML_LINK_EXISTING", "on")], 4);
    let mut tabs = Vec::new();
    for _ in 0..2 {
        let mut tab = Browser::new(&server);
        let request_id = tab.start("/saml/login");
        let response = issue(root, &request_id, json!({}));
        tabs.push((tab, response));
    }
    let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
    let posting: Vec<_> = tabs
        .into_iter()
        .map(|(mut tab, response)| {
            let gate = gate.clone();
            std::thread::spawn(move || {
                gate.wait();
                tab.post_response(&response).status
            })
        })
        .collect();
    let statuses: Vec<u16> = posting.into_iter().map(|one| one.join().unwrap()).collect();
    assert!(statuses.contains(&302), "nobody signed in: {statuses:?}");
    assert!(statuses.iter().all(|status| *status < 500), "{statuses:?}");

    let mut later = Browser::new(&server);
    let request_id = later.start("/saml/login");
    let response = issue(root, &request_id, json!({}));
    let reply = later.post_response(&response);
    assert_eq!(reply.status, 302, "{statuses:?}: {}", reply.body);
    assert!(
        later.whoami().starts_with("ada@example.com"),
        "{}",
        later.whoami()
    );
}

/// The narrow case the test above cannot provoke on demand, made to happen:
/// while a first sign-in is between creating the account and linking it,
/// another sign-in of the same person links that account. The first one
/// fails — and must leave the account, which is the other's now: removed,
/// the identity would name nobody and that person could never sign in.
#[test]
fn a_first_sign_in_that_lost_the_link_leaves_the_account_to_the_winner() {
    let dir = served_app();
    let root = dir.path();
    let service_path = root.join("app/services/saml_sso.sl");
    let service = fs::read_to_string(&service_path).unwrap();
    let linking = "    link = SamlIdentity.create({\n";
    assert_eq!(service.matches(linking).count(), 1, "{service}");
    fs::write(
        &service_path,
        service.replace(
            linking,
            "    if created && email.starts_with?(\"raced\") && SamlIdentity.count == 0\n      \
             SamlIdentity.create({\"user_id\": user._key, \"issuer\": assertion[\"issuer\"], \
             \"name_id\": SamlSso.name_id_of(assertion), \"email\": email})\n    end\n    \
             link = SamlIdentity.create({\n",
        ),
    )
    .unwrap();
    // The unique indexes are what refuses the second link.
    let migrated = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(["db:migrate", "up", "."])
        .current_dir(root)
        .env_remove("SOLI_ENGINE")
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
        .output()
        .expect("run soli");
    assert!(
        migrated.status.success(),
        "db:migrate up: {}",
        String::from_utf8_lossy(&migrated.stderr)
    );
    // With linking by address on: that is how another sign-in comes to
    // link an account it did not create.
    let server = serve_with(root, "on", &[("SAML_LINK_EXISTING", "on")]);
    let raced = json!({"name_id": "raced-at-idp", "attributes": {"Mail": "raced@example.com"}});

    let mut first = Browser::new(&server);
    let request_id = first.start("/saml/login");
    let lost = first.post_response(&issue(root, &request_id, raced.clone()));
    assert_eq!(lost.status, 401, "{}", lost.body);

    // The account is there and linked: the person signs in.
    let mut later = Browser::new(&server);
    let request_id = later.start("/saml/login");
    let reply = later.post_response(&issue(root, &request_id, raced));
    assert_eq!(reply.status, 302, "{}", reply.body);
    assert!(
        later.whoami().starts_with("raced@example.com"),
        "{}",
        later.whoami()
    );
}
