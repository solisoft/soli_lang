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
        .env_remove("SAML_LINK_EXISTING")
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
               "name_id_format": EMAIL_FORMAT,
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
    assert_eq!(signup.status, 302, "{}", signup.body);
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
        sign_up(&mut grace, "grace@example.com");
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
    sign_up(&mut kim, "kim@kk.test");

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
/// it from its owner: with linking off, the first sign-in through the
/// identity provider takes the unconfirmed account over (and drops the
/// squatter's password and session) — as long as a first sign-in may create
/// accounts at all.
#[test]
fn an_unconfirmed_account_is_taken_over_with_linking_off() {
    let dir = served_app();
    let root = dir.path();
    let server = serve(root, "on");
    let mut squatter = Browser::new(&server);
    sign_up(&mut squatter, "hire@corp.test");
    let account = squatter.whoami();

    let mut hire = Browser::new(&server);
    let id = hire.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "hire-at-idp", "attributes": {"email": "hire@corp.test"}}),
    );
    assert_eq!(hire.post_response(&response).status, 302);
    assert_eq!(hire.whoami(), account, "not the same account");
    assert_eq!(
        squatter.whoami(),
        "guest",
        "the squatter's session survived"
    );
    let login = squatter.request(
        "POST",
        "/login",
        Some(json!({"email": "hire@corp.test", "password": "correct horse battery"})),
        &format!("http://127.0.0.1:{}", server.1),
    );
    assert_eq!(login.status, 401, "the squatter's password still works");
    drop(server);

    // Provisioning off: nobody new signs in, an unconfirmed account neither.
    let dir = served_app();
    let root = dir.path();
    let server = serve(root, "off");
    let mut squatter = Browser::new(&server);
    sign_up(&mut squatter, "hire@corp.test");
    let mut hire = Browser::new(&server);
    let id = hire.start("/saml/login");
    let response = issue(
        root,
        &id,
        json!({"name_id": "hire-at-idp", "attributes": {"email": "hire@corp.test"}}),
    );
    assert_eq!(hire.post_response(&response).status, 401);
    assert_eq!(hire.whoami(), "guest");
}

/// An account a directory provisioned (SCIM) is linked by its first SAML
/// sign-in even with linking off, and, having no password, is SSO-only from
/// then on.
#[test]
fn a_directory_account_is_linked_and_becomes_sso_only() {
    let dir = served_app();
    let root = dir.path();
    let server = serve(root, "on");
    assert_eq!(
        runner(
            root,
            r#"user = User.create({"email": "dir@corp.test", "role": "member", "confirmed_at": 1, "scim_provisioned_at": 1})
print(user._errors.nil?)"#
        ),
        "true"
    );
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
            r#"user = User.find_by("email", "dir@corp.test")
    print("#{user.sso_only?} #{SamlIdentity.where({"user_id": user._key}).count}")"#
        ),
        "true 1"
    );
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
            r#"user = User.find_by("email", "ada@example.com")
    print("#{user.failed_attempts.nil?} #{user.locked_at.nil?}")"#
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
