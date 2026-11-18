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
        .env_remove("SAML_IDP_METADATA_FILE")
        .env_remove("SAML_SP_ENTITY_ID")
        .env("AUTH_SIGNUP", "open")
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
        let reply = self.get(path);
        assert_eq!(reply.status, 302, "{}", reply.body);
        assert!(
            reply.location.starts_with(&format!("{SSO}?SAMLRequest=")),
            "{}",
            reply.location
        );
        assert!(self.jar.contains_key("saml_request"), "no request cookie");
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
                "AssertionConsumerServiceURL=\"{ORIGIN}/saml/acs\""
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
    let server = serve(root, "on");

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
    assert!(!ada.jar.contains_key("saml_request"), "request cookie kept");
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

    // An account that already exists is linked by its email, and keeps its role.
    let mut grace = Browser::new(&server);
    let signup = grace.request(
        "POST",
        "/signup",
        Some(json!({"email": "grace@example.com", "password": "correct horse battery"})),
        &format!("http://127.0.0.1:{}", server.1),
    );
    assert_eq!(signup.status, 302, "{}", signup.body);
    let existing = grace.whoami();
    let mut sso = Browser::new(&server);
    let id = sso.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "grace@example.com",
               "name_id_format": "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress",
               "attributes": {}}),
    );
    assert_eq!(sso.post_response(&response).status, 302);
    assert_eq!(sso.whoami(), existing);
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
    let server = serve_with(root, "on", &[("AUTH_SIGNUP", "first_user")]);
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

#[test]
fn link_existing_off_refuses_an_account_that_exists() {
    let dir = served_app();
    let root = dir.path();
    let server = serve_with(root, "on", &[("SAML_LINK_EXISTING", "off")]);
    let mut grace = Browser::new(&server);
    let signup = grace.request(
        "POST",
        "/signup",
        Some(json!({"email": "grace@example.com", "password": "correct horse battery"})),
        &format!("http://127.0.0.1:{}", server.1),
    );
    assert_eq!(signup.status, 302, "{}", signup.body);

    let mut sso = Browser::new(&server);
    let id = sso.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"attributes": {"email": "grace@example.com"}}),
    );
    assert_eq!(sso.post_response(&response).status, 401);
    assert_eq!(sso.whoami(), "guest");
    // A newcomer is still provisioned.
    let id = sso.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "new-at-idp", "attributes": {"email": "new@example.com"}}),
    );
    assert_eq!(sso.post_response(&response).status, 302);
    assert!(sso.whoami().starts_with("new@example.com member "));
}
