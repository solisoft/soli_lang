//! `soli generate oauth` integration tests.

use std::fs;
use std::path::Path;

use solilang::scaffold::{create_auth, create_oauth};

fn make_app_with_auth(root: &Path) {
    for sub in ["app/controllers", "app/models", "app/views", "config"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("config/routes.sl"), "# routes\n").unwrap();
    create_auth(root.to_str().unwrap()).unwrap();
}

#[test]
fn create_oauth_github_writes_files() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());

    create_oauth(dir.path().to_str().unwrap(), "github").unwrap();

    for rel in [
        "app/models/oauth_identity.sl",
        "app/services/oauth_client.sl",
        "app/services/github_oauth.sl",
        "app/controllers/oauth_controller.sl",
    ] {
        assert!(dir.path().join(rel).exists(), "{rel} missing");
    }

    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    assert!(routes.contains("/auth/:provider"));
    assert!(routes.contains("oauth#callback"));

    let migrations: Vec<String> = fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        migrations
            .iter()
            .any(|n| n.contains("create_oauth_identities")),
        "missing identities migration: {migrations:?}"
    );
}

#[test]
fn create_oauth_requires_auth() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("app")).unwrap();
    let err = create_oauth(dir.path().to_str().unwrap(), "github").unwrap_err();
    assert!(err.contains("generate auth"), "{err}");
}

#[test]
fn create_oauth_rejects_unknown_provider() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    let err = create_oauth(dir.path().to_str().unwrap(), "facebook").unwrap_err();
    assert!(err.contains("Unknown OAuth provider"), "{err}");
}

#[test]
fn create_oauth_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    create_oauth(dir.path().to_str().unwrap(), "github").unwrap();
    create_oauth(dir.path().to_str().unwrap(), "google").unwrap();
    assert!(dir.path().join("app/services/google_oauth.sl").exists());
    // Shared files still present, not wiped.
    assert!(dir.path().join("app/models/oauth_identity.sl").exists());
}

#[test]
fn create_oauth_writes_every_provider_service() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    for provider in [
        "apple",
        "github",
        "google",
        "gitlab",
        "discord",
        "linkedin",
        "microsoft",
    ] {
        create_oauth(dir.path().to_str().unwrap(), provider).unwrap();
        let service = dir.path().join(format!("app/services/{provider}_oauth.sl"));
        assert!(service.exists(), "{provider} service missing");
    }
    // The controller finds providers by class name instead of listing them.
    let controller =
        fs::read_to_string(dir.path().join("app/controllers/oauth_controller.sl")).unwrap();
    assert!(controller.contains("oauth_service_for"), "{controller}");
    assert!(controller.contains("redirect_external"), "{controller}");
}

#[test]
fn create_oauth_accepts_provider_aliases() {
    use solilang::scaffold::oauth_generator::normalize_provider;
    assert_eq!(normalize_provider("GL").unwrap(), "gitlab");
    assert_eq!(normalize_provider("azure").unwrap(), "microsoft");
    assert_eq!(normalize_provider("li").unwrap(), "linkedin");
}

#[test]
fn emitted_provider_services_parse() {
    use solilang::lexer::Scanner;
    use solilang::parser::Parser;

    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    for provider in ["apple", "gitlab", "discord", "linkedin", "microsoft"] {
        create_oauth(dir.path().to_str().unwrap(), provider).unwrap();
        let rel = format!("app/services/{provider}_oauth.sl");
        let source = fs::read_to_string(dir.path().join(&rel)).unwrap();
        let tokens = Scanner::new(&source).scan_tokens().unwrap();
        Parser::new(tokens)
            .parse()
            .unwrap_or_else(|e| panic!("{rel} does not parse: {e:?}"));
    }
}

/// An OAuth controller written before account linking has no `identities`
/// action: the new routes are not appended next to it (they would be 500s),
/// and the generator says how to get them.
#[test]
fn linking_routes_are_only_added_next_to_a_controller_that_has_the_actions() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    fs::create_dir_all(dir.path().join("app/controllers")).unwrap();
    fs::write(
        dir.path().join("app/controllers/oauth_controller.sl"),
        "def oauth_service_for(provider)\n  nil\nend\n",
    )
    .unwrap();
    create_oauth(dir.path().to_str().unwrap(), "github").unwrap();
    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    assert!(routes.contains("oauth#callback"), "{routes}");
    assert!(!routes.contains("oauth#identities"), "{routes}");

    let fresh = tempfile::tempdir().unwrap();
    make_app_with_auth(fresh.path());
    create_oauth(fresh.path().to_str().unwrap(), "github").unwrap();
    create_oauth(fresh.path().to_str().unwrap(), "apple").unwrap();
    let routes = fs::read_to_string(fresh.path().join("config/routes.sl")).unwrap();
    assert_eq!(routes.matches("oauth#identities").count(), 1, "{routes}");
    assert!(
        routes.contains(r#"skip_csrf("/auth/apple/callback")"#),
        "{routes}"
    );
    assert!(fresh
        .path()
        .join("app/views/oauth/identities.html.slv")
        .exists());
}

// ---------------------------------------------------------------------------
// Served end-to-end, against stand-in providers
// ---------------------------------------------------------------------------

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde_json::{json, Value as Json};

const APPLE_KEY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/oauth/apple_test_key.p8"
);
const APPLE_PUB: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/oauth/apple_test_key.pub"
);

/// What the stand-in providers answer with, and what they were sent.
#[derive(Default)]
struct Provider {
    /// GitLab's userinfo.
    profile: Json,
    /// The claims of the id_token Apple and Microsoft return.
    claims: Json,
    /// The form body of the last token request.
    last_token_request: String,
}

/// An id_token as a token endpoint returns it. The services read it without
/// checking its signature (it comes from the provider over TLS), so this one
/// carries a placeholder.
fn id_token(claims: &Json) -> String {
    format!(
        "{}.{}.{}",
        URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(claims.to_string()),
        URL_SAFE_NO_PAD.encode("signature")
    )
}

/// GitLab, Apple and Microsoft in one loopback server.
fn start_providers(state: Arc<Mutex<Provider>>) -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; length];
            let _ = reader.read_exact(&mut body);
            let path = request_line.split_whitespace().nth(1).unwrap_or_default();

            let mut state = state.lock().unwrap();
            let answer = if path == "/oauth/userinfo" {
                state.profile.clone()
            } else if path.ends_with("/token") {
                state.last_token_request = String::from_utf8_lossy(&body).into_owned();
                if path == "/oauth/token" {
                    json!({"access_token": "gitlab-token", "token_type": "bearer"})
                } else {
                    json!({"access_token": "t", "id_token": id_token(&state.claims)})
                }
            } else {
                json!({"error": "not_found"})
            };
            let text = answer.to_string();
            let mut stream = stream;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                text.len(),
                text
            );
        }
    });
    port
}

struct Server(std::process::Child, u16);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn database_url(root: &Path) -> String {
    format!("sqlite://{}/app.sqlite3", root.display())
}

fn serve(root: &Path, providers: u16, signup: &str) -> Server {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap();
    let fake = format!("http://127.0.0.1:{providers}");
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_soli"));
    command
        .arg("serve")
        .arg(root)
        .args(["--port", &port.to_string(), "--workers", "1"])
        .current_dir(root)
        .env_remove("SOLI_ENGINE")
        .env("AUTH_SIGNUP", signup)
        .env("AUTH_ATTEMPTS_PER_IP", "1000")
        .env("SOLI_HTTP_ALLOW_HOSTS", format!("127.0.0.1:{providers}"))
        .env("GITLAB_BASE_URL", &fake)
        .env("GITLAB_CLIENT_ID", "gitlab-client")
        .env("GITLAB_CLIENT_SECRET", "gitlab-secret")
        .env(
            "GITLAB_REDIRECT_URI",
            "https://app.test/auth/gitlab/callback",
        )
        .env("APPLE_BASE_URL", &fake)
        .env("APPLE_CLIENT_ID", "com.example.web")
        .env("APPLE_TEAM_ID", "TEAM123456")
        .env("APPLE_KEY_ID", "KEY1234567")
        // A path inside the app: the server reads no file outside it.
        .env("APPLE_PRIVATE_KEY_FILE", "config/apple_key.p8")
        .env("APPLE_REDIRECT_URI", "https://app.test/auth/apple/callback")
        .env("MICROSOFT_BASE_URL", &fake)
        .env("MICROSOFT_CLIENT_ID", "ms-client")
        .env("MICROSOFT_CLIENT_SECRET", "ms-secret")
        .env(
            "MICROSOFT_REDIRECT_URI",
            "https://app.test/auth/microsoft/callback",
        )
        .env("MICROSOFT_TENANT", "organizations")
        .env("MICROSOFT_ALLOWED_TENANTS", "tenant-good, TENANT-ALSO")
        .env("SOLI_CSRF_TOKENS", "require")
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env("DATABASE_URL", database_url(root))
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

    /// `cookies: false` is a cross-site POST: the session cookie stays home.
    fn request(&mut self, method: &str, path: &str, body: Option<Json>, cookies: bool) -> Reply {
        let origin = format!("http://127.0.0.1:{}", self.port);
        let agent = ureq::AgentBuilder::new().redirects(0).build();
        let mut request = agent.request(method, &format!("{origin}{path}"));
        if cookies && !self.jar.is_empty() {
            let cookie = self
                .jar
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            request = request.set("Cookie", &cookie);
        }
        if method != "GET" {
            request = request.set(
                "Origin",
                if cookies {
                    &origin
                } else {
                    "https://appleid.apple.com"
                },
            );
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
        if cookies {
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
        }
        Reply {
            status: response.status(),
            location: response.header("location").unwrap_or_default().to_string(),
            body: response.into_string().unwrap_or_default(),
        }
    }

    fn get(&mut self, path: &str) -> Reply {
        self.request("GET", path, None, true)
    }

    /// Apple's answer: a form posted from its own site, so without the
    /// session cookie.
    fn apple_posts(&mut self, form: &str) -> Reply {
        let agent = ureq::AgentBuilder::new().redirects(0).build();
        let response = agent
            .post(&format!(
                "http://127.0.0.1:{}/auth/apple/callback",
                self.port
            ))
            .set("Origin", "https://appleid.apple.com")
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string(form);
        let response = match response {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(e) => panic!("POST /auth/apple/callback: {e}"),
        };
        Reply {
            status: response.status(),
            location: response.header("location").unwrap_or_default().to_string(),
            body: response.into_string().unwrap_or_default(),
        }
    }

    /// Start a flow; the state the provider is asked to send back.
    fn start(&mut self, provider: &str) -> String {
        let reply = self.get(&format!("/auth/{provider}"));
        assert_eq!(reply.status, 302, "{}", reply.body);
        let state = reply.location.split("state=").nth(1).expect("no state");
        state.split('&').next().unwrap().to_string()
    }

    /// A whole sign-in with a provider that redirects back.
    fn sign_in_with(&mut self, provider: &str) -> Reply {
        let state = self.start(provider);
        self.get(&format!(
            "/auth/{provider}/callback?code=the-code&state={state}"
        ))
    }

    fn whoami(&mut self) -> String {
        self.get("/whoami").body
    }
}

fn soli(dir: &Path, args: &[&str]) -> (bool, String) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(args)
        .current_dir(dir)
        .env_remove("SOLI_ENGINE")
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env("DATABASE_URL", database_url(dir))
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

/// Everything SQLite has on disk for the app: the database and, since recent
/// writes sit there until a checkpoint, its write-ahead log.
fn database_bytes(root: &Path) -> String {
    let mut bytes = fs::read(root.join("app.sqlite3")).unwrap_or_default();
    bytes.extend(fs::read(root.join("app.sqlite3-wal")).unwrap_or_default());
    assert!(!bytes.is_empty(), "no database on disk");
    String::from_utf8_lossy(&bytes).into_owned()
}

fn served_app() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_app_with_auth(root);
    for provider in ["gitlab", "apple", "microsoft"] {
        create_oauth(root.to_str().unwrap(), provider).unwrap();
    }
    fs::create_dir_all(root.join("app/views/layouts")).unwrap();
    fs::write(
        root.join("app/views/layouts/application.html.slv"),
        "<main><%= yield %></main>\n",
    )
    .unwrap();
    fs::write(
        root.join("app/controllers/whoami_controller.sl"),
        "class WhoamiController < Controller\n  # GET /whoami\n  def index\n    user = current_user()\n    \
         render_text(user.nil? ? \"guest\" : \"#{user.email} #{user.role} #{user.name}\")\n  end\nend\n",
    )
    .unwrap();
    let mut routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    routes.push_str("get(\"/whoami\", \"whoami#index\")\n");
    fs::write(root.join("config/routes.sl"), routes).unwrap();
    fs::copy(APPLE_KEY, root.join("config/apple_key.p8")).unwrap();
    fs::copy(APPLE_PUB, root.join("config/apple_key.pub")).unwrap();
    let (ok, output) = soli(root, &["db:migrate", "up", "."]);
    assert!(ok, "db:migrate up:\n{output}");
    dir
}

fn gitlab(sub: &str, email: &str, verified: bool) -> Json {
    json!({"sub": sub, "email": email, "email_verified": verified, "name": "From GitLab"})
}

#[test]
fn served_app_signs_in_links_and_refuses() {
    let dir = served_app();
    let root = dir.path();
    let state = Arc::new(Mutex::new(Provider::default()));
    let providers = start_providers(Arc::clone(&state));
    let server = serve(root, providers, "open");
    let set_profile = |profile: Json| state.lock().unwrap().profile = profile;
    let set_claims = |claims: Json| state.lock().unwrap().claims = claims;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // A first sign-in creates the account: confirmed, a member, no password.
    set_profile(gitlab("gl-1", "Ada@Example.com", true));
    let mut ada = Browser::new(&server);
    let reply = ada.sign_in_with("gitlab");
    assert_eq!(
        (reply.status, reply.location.as_str()),
        (302, "/"),
        "{}",
        reply.body
    );
    assert_eq!(ada.whoami(), "ada@example.com member From GitLab");
    let stored = database_bytes(root);
    assert!(
        !stored.contains("\"password\""),
        "a plaintext password field was stored"
    );
    assert!(
        stored.contains("confirmed_at"),
        "the account is not confirmed"
    );
    // The exchange carried the PKCE verifier and the code.
    let sent = state.lock().unwrap().last_token_request.clone();
    assert!(
        sent.contains("code=the-code") && sent.contains("code_verifier="),
        "{sent}"
    );

    // A wrong state, or an address the provider did not verify, signs nobody in.
    let mut eve = Browser::new(&server);
    eve.start("gitlab");
    assert_eq!(
        eve.get("/auth/gitlab/callback?code=c&state=forged").status,
        403
    );
    set_profile(gitlab("gl-2", "grace@example.com", false));
    assert_eq!(eve.sign_in_with("gitlab").status, 401);
    assert_eq!(eve.whoami(), "guest");

    // A session opened after a password reset bumped the account's session
    // version stays open (it used to be refused on its next request).
    set_profile(gitlab("gl-1", "ada@example.com", true));
    fs::write(
        root.join("bump.sl"),
        "user = User.find_by(\"email\", \"ada@example.com\")\nuser.session_version = 3\nuser.save\n",
    )
    .unwrap();
    drop(server);
    let (ok, output) = soli(root, &["db:seed", ".", "bump.sl"]);
    assert!(ok, "{output}");
    let server = serve(root, providers, "open");
    let mut ada = Browser::new(&server);
    assert_eq!(ada.sign_in_with("gitlab").status, 302);
    assert_eq!(ada.whoami(), "ada@example.com member From GitLab");

    // An account someone created in wait with the victim's address, never
    // confirmed: the provider's proof of the address hands it over alone.
    let mut squatter = Browser::new(&server);
    let signup = squatter.request(
        "POST",
        "/signup",
        Some(json!({"email": "grace@example.com", "password": "squatter password 1"})),
        true,
    );
    assert_eq!(signup.status, 302, "{}", signup.body);
    assert!(squatter.whoami().starts_with("grace@example.com"));
    set_profile(gitlab("gl-2", "grace@example.com", true));
    let mut grace = Browser::new(&server);
    assert_eq!(grace.sign_in_with("gitlab").status, 302);
    assert!(grace.whoami().starts_with("grace@example.com"));
    assert_eq!(
        squatter.whoami(),
        "guest",
        "the squatter's session survived"
    );
    let login = squatter.request(
        "POST",
        "/login",
        Some(json!({"email": "grace@example.com", "password": "squatter password 1"})),
        true,
    );
    assert_eq!(login.status, 401, "the squatter's password still works");

    // Linking: a signed-in user adds Apple. Apple posts its answer from its
    // own site, without the session cookie; the action hands it to a GET.
    let page = ada.get("/oauth/identities");
    assert_eq!(page.status, 200, "{}", page.body);
    assert!(page.body.contains("Gitlab"), "{}", page.body);
    assert!(page.body.contains("Connect Apple") && page.body.contains("Connect Microsoft"));
    assert!(
        !page.body.contains("Disconnect"),
        "the only way in can be removed"
    );
    set_claims(json!({
        "iss": "https://appleid.apple.com", "aud": "com.example.web", "exp": now + 600,
        "sub": "apple-ada", "email": "relay-ada@privaterelay.appleid.com", "email_verified": "true",
    }));
    let apple_state = ada.start("apple");
    let posted = ada.apple_posts(&format!(
        "code=apple-code&state={apple_state}&user={}",
        urlencoding::encode(r#"{"name":{"firstName":"Ada","lastName":"L"}}"#)
    ));
    assert_eq!(posted.status, 302, "{}", posted.body);
    assert!(
        posted
            .location
            .starts_with("/auth/apple/callback?code=apple-code&state="),
        "{}",
        posted.location
    );
    let linked = ada.get(&posted.location);
    assert_eq!(
        (linked.status, linked.location.as_str()),
        (302, "/oauth/identities"),
        "{}",
        linked.body
    );
    assert_eq!(ada.whoami(), "ada@example.com member From GitLab");
    let page = ada.get("/oauth/identities");
    assert!(
        page.body.contains("Apple") && page.body.contains("relay-ada@"),
        "{}",
        page.body
    );
    assert!(page.body.contains("Disconnect"), "{}", page.body);

    // The client secret Apple received is an ES256 JWT signed with the key.
    let sent = state.lock().unwrap().last_token_request.clone();
    let secret = sent
        .split('&')
        .find_map(|pair| pair.strip_prefix("client_secret="))
        .expect("no client_secret");
    fs::write(
        root.join("secret.sl"),
        format!(
            "claims = jwt_verify(\"{secret}\", \"\", {{\"algorithm\": \"ES256\", \"key\": File.read(\"config/apple_key.pub\"), \"audience\": \"https://appleid.apple.com\"}})\nprint(\"#{{claims[\"iss\"]}} #{{claims[\"sub\"]}} #{{claims[\"aud\"]}}\")\n"
        ),
    )
    .unwrap();
    let (ok, output) = soli(root, &["secret.sl"]);
    assert!(ok, "{output}");
    assert!(
        output.contains("TEAM123456 com.example.web https://appleid.apple.com"),
        "{output}"
    );

    // Apple now signs Ada in from anywhere, whatever address it relays.
    let mut phone = Browser::new(&server);
    let phone_state = phone.start("apple");
    let reply = phone.get(&format!("/auth/apple/callback?code=c&state={phone_state}"));
    assert_eq!(reply.status, 302, "{}", reply.body);
    assert_eq!(phone.whoami(), "ada@example.com member From GitLab");

    // Grace cannot link the Apple account that is Ada's.
    let grace_state = grace.start("apple");
    let refused = grace.get(&format!("/auth/apple/callback?code=c&state={grace_state}"));
    assert_eq!(refused.status, 401, "{}", refused.body);
    assert!(
        refused.body.contains("linked to another user"),
        "{}",
        refused.body
    );

    // An id_token for another client, or expired, is refused.
    for (key, value) in [("aud", json!("com.other.app")), ("exp", json!(now - 10))] {
        let mut claims = json!({
            "iss": "https://appleid.apple.com", "aud": "com.example.web", "exp": now + 600,
            "sub": "apple-new", "email": "new@example.com", "email_verified": true,
        });
        claims[key] = value;
        set_claims(claims);
        let mut visitor = Browser::new(&server);
        assert_eq!(visitor.sign_in_with("apple").status, 401, "{key}");
    }

    // Disconnecting: one of two goes; the last way in stays.
    let page = ada.get("/oauth/identities");
    let start = page
        .body
        .find("action=\"/oauth/identities/")
        .expect("no disconnect button")
        + 8;
    let path = page.body[start..].split('"').next().unwrap().to_string();
    assert_eq!(
        ada.request("DELETE", &path, Some(json!({})), true).status,
        302
    );
    let page = ada.get("/oauth/identities");
    assert!(!page.body.contains("Disconnect"), "{}", page.body);
    assert_eq!(page.body.matches("<li").count(), 1, "{}", page.body);

    // Microsoft, several tenants: only the ones the app names, and only with
    // the issuer of the tenant the token claims.
    let microsoft = |tenant: &str, issuer_tenant: &str| {
        json!({
            "iss": format!("https://login.microsoftonline.com/{issuer_tenant}/v2.0"),
            "aud": "ms-client", "exp": now + 600, "tid": tenant,
            "sub": format!("ms-{tenant}"), "email": format!("user@{tenant}.example"), "name": "MS User",
        })
    };
    set_claims(microsoft("tenant-good", "tenant-good"));
    let mut employee = Browser::new(&server);
    assert_eq!(employee.sign_in_with("microsoft").status, 302);
    assert_eq!(employee.whoami(), "user@tenant-good.example member MS User");
    // Listed in another case, with spaces around it.
    set_claims(microsoft("tenant-also", "tenant-also"));
    assert_eq!(Browser::new(&server).sign_in_with("microsoft").status, 302);
    set_claims(microsoft("tenant-evil", "tenant-evil"));
    let refused = Browser::new(&server).sign_in_with("microsoft");
    assert_eq!(refused.status, 401, "{}", refused.body);
    assert!(refused.body.contains("not allowed"), "{}", refused.body);
    set_claims(microsoft("tenant-good", "tenant-evil"));
    let refused = Browser::new(&server).sign_in_with("microsoft");
    assert!(
        refused.body.contains("issuer does not match"),
        "{}",
        refused.body
    );
}

/// With sign-up closed, a provider signs in the accounts that exist and
/// creates none — it used to create one whatever AUTH_SIGNUP said.
#[test]
fn a_provider_creates_no_account_when_sign_up_is_closed() {
    let dir = served_app();
    let state = Arc::new(Mutex::new(Provider::default()));
    let providers = start_providers(Arc::clone(&state));
    state.lock().unwrap().profile = gitlab("gl-9", "newcomer@example.com", true);

    let server = serve(dir.path(), providers, "closed");
    let mut visitor = Browser::new(&server);
    let refused = visitor.sign_in_with("gitlab");
    assert_eq!(refused.status, 401, "{}", refused.body);
    assert!(
        refused.body.contains("sign-up is closed"),
        "{}",
        refused.body
    );
    assert_eq!(visitor.whoami(), "guest");
    drop(server);

    // The default policy: the first account opens the app and runs it.
    let server = serve(dir.path(), providers, "first_user");
    let mut founder = Browser::new(&server);
    assert_eq!(founder.sign_in_with("gitlab").status, 302);
    assert_eq!(founder.whoami(), "newcomer@example.com admin From GitLab");
    state.lock().unwrap().profile = gitlab("gl-10", "second@example.com", true);
    assert_eq!(Browser::new(&server).sign_in_with("gitlab").status, 401);
}
