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
        "app/services/oauth_sso.sl",
        "app/services/github_oauth.sl",
        "app/controllers/oauth_controller.sl",
        "app/mailers/oauth_mailer.sl",
        "app/views/oauth_mailer/provider_linked.html.slv",
        "app/views/oauth/reauth.html.slv",
    ] {
        assert!(dir.path().join(rel).exists(), "{rel} missing");
    }
    // The shared helpers are `OauthSso`: `OauthClient` is the model
    // `soli generate oidc_provider` writes, and the two used to clash.
    assert!(!dir.path().join("app/services/oauth_client.sl").exists());
    let service = fs::read_to_string(dir.path().join("app/services/oauth_sso.sl")).unwrap();
    assert!(service.contains("class OauthSso"), "{service}");
    assert!(!service.contains("class OauthClient"), "{service}");

    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    assert!(routes.contains("/auth/:provider"));
    assert!(routes.contains("oauth#callback"));
    assert!(
        routes.contains(r#"post("/oauth/reauth", "oauth#reauthenticate")"#),
        "{routes}"
    );
    // Linking starts from a CSRF-checked POST, never from the GET a
    // cross-site link can make a browser send.
    assert!(
        routes.contains(r#"post("/auth/:provider/link", "oauth#link_provider")"#),
        "{routes}"
    );
    assert!(
        !routes.contains(r#"get("/auth/:provider/link""#),
        "{routes}"
    );

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
    let parse = |rel: &str| {
        let source = fs::read_to_string(dir.path().join(rel)).unwrap();
        let tokens = Scanner::new(&source).scan_tokens().unwrap();
        Parser::new(tokens)
            .parse()
            .unwrap_or_else(|e| panic!("{rel} does not parse: {e:?}"));
    };
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
        parse(&format!("app/services/{provider}_oauth.sl"));
    }
    for rel in [
        "app/services/oauth_sso.sl",
        "app/controllers/oauth_controller.sl",
        "app/mailers/oauth_mailer.sl",
    ] {
        parse(rel);
    }
}

/// What the provider services trust: GitHub's address only from /user/emails
/// (primary and verified), Microsoft's only with `xms_edov` or
/// `verified_primary_email`, never `preferred_username`; every form body
/// encoded; PKCE on GitHub too.
#[test]
fn provider_services_take_only_verified_addresses_and_encode_their_bodies() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    for provider in ["github", "google", "microsoft"] {
        create_oauth(dir.path().to_str().unwrap(), provider).unwrap();
    }
    let read = |rel: &str| fs::read_to_string(dir.path().join(rel)).unwrap();

    let github = read("app/services/github_oauth.sl");
    assert!(github.contains("/user/emails"), "{github}");
    assert!(
        github.contains(r#"entry["primary"] == true && entry["verified"] == true"#),
        "{github}"
    );
    assert!(!github.contains(r#"profile["email"]"#), "{github}");
    assert!(github.contains("code_challenge_method=S256"), "{github}");
    assert!(github.contains("code_verifier="), "{github}");

    let google = read("app/services/google_oauth.sl");
    for field in ["code", "client_secret", "redirect_uri", "code_verifier"] {
        assert!(
            google.contains(&format!("\"{field}=#{{url_encode(")),
            "{field} is not encoded:\n{google}"
        );
    }

    let microsoft = read("app/services/microsoft_oauth.sl");
    assert!(
        !microsoft.contains(r#"claims["preferred_username"]"#),
        "{microsoft}"
    );
    assert!(microsoft.contains("xms_edov"), "{microsoft}");
    assert!(microsoft.contains("verified_primary_email"), "{microsoft}");

    // Provider error text reaches the log, not the page.
    let controller = read("app/controllers/oauth_controller.sl");
    assert!(
        !controller.contains("OAuth failed: #{error}"),
        "{controller}"
    );
    assert!(
        !controller.contains(r#"answer["error_description"] ||"#),
        "{controller}"
    );
    assert!(
        controller.contains("OauthSso.public_message(error)"),
        "{controller}"
    );
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
    assert!(!routes.contains("oauth#link_provider"), "{routes}");

    let fresh = tempfile::tempdir().unwrap();
    make_app_with_auth(fresh.path());
    create_oauth(fresh.path().to_str().unwrap(), "github").unwrap();
    create_oauth(fresh.path().to_str().unwrap(), "apple").unwrap();
    let routes = fs::read_to_string(fresh.path().join("config/routes.sl")).unwrap();
    assert_eq!(routes.matches("oauth#identities").count(), 1, "{routes}");
    assert_eq!(routes.matches("oauth#link_provider").count(), 1, "{routes}");
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
    /// How many token requests were received.
    token_requests: usize,
    /// The nonce of each authorization request, by its PKCE challenge — which
    /// is how the token request (it carries the verifier) finds its own.
    nonces: BTreeMap<String, String>,
    /// Answer with a nonce from some other sign-in.
    forge_nonce: bool,
    /// Run when GitLab is asked for a token: something that happens to the
    /// account while the callback is waiting for the provider.
    on_token: Option<Box<dyn FnMut() + Send>>,
}

/// One parameter of a query string or a form body, percent-decoded.
fn parameter(text: &str, name: &str) -> Option<String> {
    let query = text.split_once('?').map_or(text, |(_, query)| query);
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| urlencoding::decode(value).unwrap_or_default().into_owned())
    })
}

/// The id_token claims a token request is answered with: the provider's, and
/// the nonce of the authorization request the code came from.
fn claims_for(state: &Provider, token_request: &str) -> Json {
    use sha2::Digest;
    let mut claims = state.claims.clone();
    let verifier = parameter(token_request, "code_verifier").unwrap_or_default();
    let challenge = URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()));
    if state.forge_nonce {
        claims["nonce"] = json!("the-nonce-of-another-sign-in");
    } else if let Some(nonce) = state.nonces.get(&challenge) {
        claims["nonce"] = json!(nonce);
    }
    claims
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
            if path.contains("/authorize?") {
                if let (Some(challenge), Some(nonce)) =
                    (parameter(path, "code_challenge"), parameter(path, "nonce"))
                {
                    state.nonces.insert(challenge, nonce);
                }
            }
            let answer = if path == "/oauth/userinfo" {
                state.profile.clone()
            } else if path.ends_with("/token") {
                state.last_token_request = String::from_utf8_lossy(&body).into_owned();
                state.token_requests += 1;
                if path == "/oauth/token" {
                    if let Some(hook) = state.on_token.as_mut() {
                        hook();
                    }
                    json!({"access_token": "gitlab-token", "token_type": "bearer"})
                } else {
                    let claims = claims_for(&state, &state.last_token_request);
                    json!({"access_token": "t", "id_token": id_token(&claims)})
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

fn database_url(root: &Path) -> String {
    format!("sqlite://{}/app.sqlite3", root.display())
}

fn serve(root: &Path, providers: u16, signup: &str) -> Server {
    serve_workers(root, providers, signup, 1)
}

/// `serve`, on `workers` request workers: requests sent together run side
/// by side.
fn serve_workers(root: &Path, providers: u16, signup: &str, workers: usize) -> Server {
    let port = unused_port();
    let fake = format!("http://127.0.0.1:{providers}");
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
        .env("AUTH_SIGNUP", signup)
        // A test run: the first account needs no AUTH_SETUP_TOKEN.
        .env("APP_ENV", "test")
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
        let location = response.header("location").unwrap_or_default().to_string();
        // Sent to a provider: go there, as a browser would, so the stand-in
        // sees the authorization request (and its nonce).
        if location.starts_with("http://127.0.0.1") && location.contains("/authorize?") {
            let _ = ureq::get(&location).call();
        }
        Reply {
            status: response.status(),
            location,
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
        state_of(&reply.location)
    }

    /// Start linking `provider` to the signed-in account: the same-origin
    /// POST the Connect buttons of /oauth/identities send.
    fn link(&mut self, provider: &str) -> Reply {
        self.request(
            "POST",
            &format!("/auth/{provider}/link"),
            Some(json!({})),
            true,
        )
    }

    /// A link started; the state the provider is asked to send back.
    fn start_link(&mut self, provider: &str) -> String {
        let reply = self.link(provider);
        assert_eq!(reply.status, 302, "{}", reply.body);
        assert!(reply.location.contains("state="), "{}", reply.location);
        state_of(&reply.location)
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

/// The `state` parameter of a redirect to a provider.
fn state_of(location: &str) -> String {
    let state = location.split("state=").nth(1).expect("no state");
    state.split('&').next().unwrap().to_string()
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

    // What a provider says about a refusal is logged, never shown: the page
    // would otherwise print whatever the query string carries.
    let eve_state = eve.start("gitlab");
    let denied = eve.get(&format!(
        "/auth/gitlab/callback?state={eve_state}&error=access_denied&error_description=%3Cb%3Ephish%3C%2Fb%3E"
    ));
    assert_eq!(denied.status, 401, "{}", denied.body);
    assert!(!denied.body.contains("phish"), "{}", denied.body);

    // An account someone created in wait with the victim's address, never
    // confirmed.
    let mut squatter = Browser::new(&server);
    let signup = squatter.request(
        "POST",
        "/signup",
        Some(json!({"email": "grace@example.com", "password": "squatter password 1"})),
        true,
    );
    // Open sign-up answers "check your mailbox" and opens no session: the
    // squatter never read the victim's mail, so they are never signed in —
    // nothing to link a provider with, and the password they chose opens
    // nothing either.
    assert_eq!(signup.status, 200, "{}", signup.body);
    assert_eq!(squatter.whoami(), "guest");
    let login = squatter.request(
        "POST",
        "/login",
        Some(json!({"email": "grace@example.com", "password": "squatter password 1"})),
        true,
    );
    assert_eq!(login.status, 401, "an unconfirmed account signed in");
    // The same answer for an address that already has an account.
    let again = Browser::new(&server).request(
        "POST",
        "/signup",
        Some(json!({"email": "grace@example.com", "password": "another password 12"})),
        true,
    );
    let stable = |page: &str| {
        page.split("value=\"")
            .map(|part| part.split_once('"').map_or(part, |(_, rest)| rest))
            .collect::<String>()
    };
    assert_eq!(
        (again.status, stable(&again.body)),
        (signup.status, stable(&signup.body))
    );

    // A session opened after a password reset bumped the account's session
    // version stays open (it used to be refused on its next request). The
    // same seed links an Apple identity to the squatter's account, as one
    // linked before linking required a confirmed address would be, and makes
    // Hopper, who signs in with a password.
    set_profile(gitlab("gl-1", "ada@example.com", true));
    fs::write(
        root.join("bump.sl"),
        concat!(
            "user = User.find_by(\"email\", \"ada@example.com\")\n",
            "user.session_version = 3\n",
            "user.save\n",
            "squatter = User.find_by(\"email\", \"grace@example.com\")\n",
            "OauthIdentity.link!(squatter._key, \"apple\", \"apple-squatter\", \"squatter@privaterelay.appleid.com\", nil)\n",
            "hopper = User.new\n",
            "hopper.email = \"hopper@example.com\"\n",
            "hopper.name = \"Hopper\"\n",
            "hopper.set_password(\"hopper password 1\")\n",
            "hopper.confirmed_at = 1\n",
            "hopper.save\n",
        ),
    )
    .unwrap();
    drop(server);
    let (ok, output) = soli(root, &["db:seed", ".", "bump.sl"]);
    assert!(ok, "{output}");
    let server = serve(root, providers, "open");
    let mut ada = Browser::new(&server);
    assert_eq!(ada.sign_in_with("gitlab").status, 302);
    assert_eq!(ada.whoami(), "ada@example.com member From GitLab");

    // ...and the provider's proof of the address hands it over alone: the
    // squatter's password, session and linked Apple account all stop working.
    let mut squatter = Browser::new(&server);
    let login = squatter.request(
        "POST",
        "/login",
        Some(json!({"email": "grace@example.com", "password": "squatter password 1"})),
        true,
    );
    // (The squatter never could sign in: the address was never confirmed.)
    assert_eq!(login.status, 401, "{}", login.body);
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
    set_claims(json!({
        "iss": "https://appleid.apple.com", "aud": "com.example.web", "exp": now + 600,
        "sub": "apple-squatter", "email": "squatter@privaterelay.appleid.com", "email_verified": "true",
    }));
    let mut squatter_phone = Browser::new(&server);
    assert_eq!(squatter_phone.sign_in_with("apple").status, 302);
    assert!(
        !squatter_phone.whoami().starts_with("grace@example.com"),
        "the squatter's linked identity still signs in to the account taken over"
    );

    // Linking needs a sign-in from the last ten minutes. Hopper signed in
    // with a password, which this scaffold does not time: the password again.
    let mut hopper = Browser::new(&server);
    let login = hopper.request(
        "POST",
        "/login",
        Some(json!({"email": "hopper@example.com", "password": "hopper password 1"})),
        true,
    );
    assert_eq!(login.status, 302, "{}", login.body);
    let asked = hopper.link("apple");
    assert_eq!(
        (asked.status, asked.location.as_str()),
        (302, "/oauth/reauth?provider=apple"),
        "{}",
        asked.body
    );
    assert_eq!(hopper.get(&asked.location).status, 200);
    let wrong = hopper.request(
        "POST",
        "/oauth/reauth",
        Some(json!({"provider": "apple", "password": "not the password"})),
        true,
    );
    assert_eq!(wrong.status, 401, "{}", wrong.body);
    assert_eq!(hopper.link("apple").status, 302);
    assert!(hopper.link("apple").location.starts_with("/oauth/reauth"));
    // The password POST is CSRF-checked and names the provider: it goes
    // straight to the provider, linking.
    let confirmed = hopper.request(
        "POST",
        "/oauth/reauth",
        Some(json!({"provider": "apple", "password": "hopper password 1"})),
        true,
    );
    assert_eq!(confirmed.status, 302, "{}", confirmed.body);
    assert!(
        confirmed.location.contains("state="),
        "{}",
        confirmed.location
    );
    hopper.start_link("apple");

    // Linking: a signed-in user adds Apple. Apple posts its answer from its
    // own site, without the session cookie; the action hands it to a GET.
    let page = ada.get("/oauth/identities");
    assert_eq!(page.status, 200, "{}", page.body);
    assert!(page.body.contains("Gitlab"), "{}", page.body);
    assert!(page.body.contains("Connect Apple") && page.body.contains("Connect Microsoft"));
    // The Connect buttons are forms posting with the CSRF token, not links.
    assert!(
        page.body
            .contains(r#"action="/auth/apple/link" method="POST""#),
        "{}",
        page.body
    );
    assert!(
        !page.body.contains(r#"href="/auth/apple""#),
        "{}",
        page.body
    );
    // Ada, signed in a moment ago, follows a cross-site link to
    // /auth/apple: she lands on her connected accounts, and nothing starts.
    let get = ada.get("/auth/apple");
    assert_eq!(
        (get.status, get.location.as_str()),
        (302, "/oauth/identities"),
        "{}",
        get.body
    );
    // A cross-site POST is stopped at the origin gate.
    let cross_site = ada.request("POST", "/auth/apple/link", Some(json!({})), false);
    assert_eq!(cross_site.status, 403, "{}", cross_site.body);
    assert!(
        !page.body.contains("Disconnect"),
        "the only way in can be removed"
    );
    set_claims(json!({
        "iss": "https://appleid.apple.com", "aud": "com.example.web", "exp": now + 600,
        "sub": "apple-ada", "email": "relay-ada@privaterelay.appleid.com", "email_verified": "true",
    }));
    let apple_state = ada.start_link("apple");
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
            "claims = JWT.verify(\"{secret}\", \"\", {{\"algorithm\": \"ES256\", \"key\": File.read(\"config/apple_key.pub\"), \"audience\": \"https://appleid.apple.com\"}})\nprint(\"#{{claims[\"iss\"]}} #{{claims[\"sub\"]}} #{{claims[\"aud\"]}}\")\n"
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
    let grace_state = grace.start_link("apple");
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
    // Hopper signed in with a password a while ago as far as this scaffold
    // knows: disconnecting asks for the password again, and removes nothing.
    let mut stale = Browser::new(&server);
    let login = stale.request(
        "POST",
        "/login",
        Some(json!({"email": "hopper@example.com", "password": "hopper password 1"})),
        true,
    );
    assert_eq!(login.status, 302, "{}", login.body);
    let asked = stale.request("DELETE", &path, Some(json!({})), true);
    assert_eq!(
        (asked.status, asked.location.as_str()),
        (302, "/oauth/reauth"),
        "{}",
        asked.body
    );
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
            "xms_edov": true,
        })
    };
    // An address Microsoft does not say it verified — `email` without
    // `xms_edov`, or only `preferred_username` — finds and creates nothing.
    let mut unverified = microsoft("tenant-good", "tenant-good");
    unverified["xms_edov"] = json!(false);
    unverified["email"] = json!("ada@example.com");
    set_claims(unverified.clone());
    let mut intruder = Browser::new(&server);
    assert_eq!(intruder.sign_in_with("microsoft").status, 401);
    assert_eq!(intruder.whoami(), "guest");
    unverified.as_object_mut().unwrap().remove("email");
    unverified["preferred_username"] = json!("ada@example.com");
    set_claims(unverified);
    assert_eq!(intruder.sign_in_with("microsoft").status, 401);
    assert_eq!(intruder.whoami(), "guest");
    // `verified_primary_email` is taken over `email`.
    let mut primary = microsoft("tenant-also", "tenant-also");
    primary.as_object_mut().unwrap().remove("xms_edov");
    primary["sub"] = json!("ms-primary");
    primary["verified_primary_email"] = json!(["primary@tenant-also.example"]);
    set_claims(primary);
    let mut primary_user = Browser::new(&server);
    assert_eq!(primary_user.sign_in_with("microsoft").status, 302);
    assert_eq!(
        primary_user.whoami(),
        "primary@tenant-also.example member MS User"
    );

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

    // A token that answers another sign-in's nonce — or none — is refused,
    // from Microsoft and from Apple alike.
    set_claims(microsoft("tenant-good", "tenant-good"));
    state.lock().unwrap().forge_nonce = true;
    for provider in ["microsoft", "apple"] {
        if provider == "apple" {
            set_claims(json!({
                "iss": "https://appleid.apple.com", "aud": "com.example.web",
                "exp": 4102444800u64, "sub": "apple-nonce",
            }));
        }
        let refused = Browser::new(&server).sign_in_with(provider);
        assert_eq!(refused.status, 401, "{provider}: {}", refused.body);
        assert!(
            refused.body.contains("does not answer this sign-in"),
            "{provider}: {}",
            refused.body
        );
    }
    state.lock().unwrap().forge_nonce = false;
    set_claims(microsoft("tenant-good", "tenant-good"));
    assert_eq!(Browser::new(&server).sign_in_with("microsoft").status, 302);
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

/// Who a provider sign-in may be matched to: an identity with no subject is
/// nobody's; an address that only looks like an account's (a non-ASCII
/// letter that lowercases to an ASCII one) matches no account; and an
/// account already linked at the provider is not linked a second time because
/// its address now belongs to someone else there.
#[test]
fn a_provider_identity_needs_a_subject_and_an_address_matches_one_owner() {
    let dir = served_app();
    let state = Arc::new(Mutex::new(Provider::default()));
    let providers = start_providers(Arc::clone(&state));
    let server = serve(dir.path(), providers, "open");
    let set_profile = |profile: Json| state.lock().unwrap().profile = profile;

    // No subject: refused, and no account is made.
    set_profile(json!({"email": "ghost@example.com", "email_verified": true, "name": "Ghost"}));
    let mut ghost = Browser::new(&server);
    let refused = ghost.sign_in_with("gitlab");
    assert_eq!(refused.status, 401, "{}", refused.body);
    assert!(
        refused.body.contains("did not say who signed in"),
        "{}",
        refused.body
    );
    assert_eq!(ghost.whoami(), "guest");

    // Mike's account, linked to his GitLab identity.
    set_profile(gitlab("gl-mike", "mike@example.com", true));
    let mut mike = Browser::new(&server);
    assert_eq!(mike.sign_in_with("gitlab").status, 302);
    assert_eq!(mike.whoami(), "mike@example.com member From GitLab");

    // The mailbox was given to someone else, with another subject: the
    // address alone does not open Mike's account.
    set_profile(gitlab("gl-newcomer", "mike@example.com", true));
    let mut newcomer = Browser::new(&server);
    let refused = newcomer.sign_in_with("gitlab");
    assert_eq!(refused.status, 401, "{}", refused.body);
    assert!(
        refused
            .body
            .contains("already linked to another Gitlab account"),
        "{}",
        refused.body
    );
    assert_eq!(newcomer.whoami(), "guest");

    // KELVIN SIGN for the k: lowercased it would read `mike@example.com`.
    // It is matched to no account, and makes none of its own.
    set_profile(gitlab("gl-lookalike", "mi\u{212a}e@example.com", true));
    let mut lookalike = Browser::new(&server);
    let refused = lookalike.sign_in_with("gitlab");
    assert_eq!(refused.status, 401, "{}", refused.body);
    assert_eq!(lookalike.whoami(), "guest");
    assert_eq!(mike.whoami(), "mike@example.com member From GitLab");
}

/// An account that signs in through single sign-on only (`User#sso_only?`,
/// `soli generate saml`) gets no provider: not by its verified address, not
/// through an identity linked before. And a Microsoft identity stored under
/// the bare `sub` (before uids were `<tid>:<sub>`) is adopted on its owner's
/// next sign-in only when the tenant recorded with it is the token's.
#[test]
fn sso_only_accounts_get_no_provider_and_legacy_microsoft_identities_are_adopted() {
    let dir = served_app();
    let root = dir.path();
    fs::write(
        root.join("seed.sl"),
        concat!(
            "sso = User.new\n",
            "sso.email = \"sso@example.com\"\n",
            "sso.name = \"Sso\"\n",
            "sso.sso_only = true\n",
            "sso.confirmed_at = 1\n",
            "sso.save\n",
            "OauthIdentity.link!(sso._key, \"gitlab\", \"gl-sso-linked\", nil, nil)\n",
            "old = User.new\n",
            "old.email = \"old@example.com\"\n",
            "old.name = \"Old\"\n",
            "old.confirmed_at = 1\n",
            "old.save\n",
            "OauthIdentity.create({\"user_id\": old._key, \"provider\": \"microsoft\", \"uid\": \"ms-old\", \"raw\": {\"tid\": \"tenant-good\"}})\n",
            "OauthIdentity.create({\"user_id\": old._key, \"provider\": \"microsoft\", \"uid\": \"ms-elsewhere\", \"raw\": {\"tid\": \"tenant-also\"}})\n",
            "OauthIdentity.create({\"user_id\": old._key, \"provider\": \"microsoft\", \"uid\": \"ms-no-tenant\"})\n",
        ),
    )
    .unwrap();
    let (ok, output) = soli(root, &["db:seed", ".", "seed.sl"]);
    assert!(ok, "{output}");

    let state = Arc::new(Mutex::new(Provider::default()));
    let providers = start_providers(Arc::clone(&state));
    let server = serve(root, providers, "open");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    for profile in [
        gitlab("gl-sso-new", "sso@example.com", true),
        gitlab("gl-sso-linked", "other@example.com", true),
    ] {
        state.lock().unwrap().profile = profile;
        let mut visitor = Browser::new(&server);
        let refused = visitor.sign_in_with("gitlab");
        assert_eq!(refused.status, 401, "{}", refused.body);
        assert!(
            refused.body.contains("single sign-on only"),
            "{}",
            refused.body
        );
        assert_eq!(visitor.whoami(), "guest");
    }
    assert!(
        !database_bytes(root).contains("gl-sso-new"),
        "an identity was linked to the SSO-only account"
    );

    // No verified address in these tokens: only an adopted identity signs in.
    let microsoft = |sub: &str| {
        json!({
            "iss": "https://login.microsoftonline.com/tenant-good/v2.0",
            "aud": "ms-client", "exp": now + 600, "tid": "tenant-good",
            "sub": sub, "name": "MS User",
        })
    };
    state.lock().unwrap().claims = microsoft("ms-old");
    let mut old = Browser::new(&server);
    assert_eq!(old.sign_in_with("microsoft").status, 302);
    assert_eq!(old.whoami(), "old@example.com member Old");
    assert!(database_bytes(root).contains("tenant-good:ms-old"));
    // Adopted once: the next sign-in finds it under its new uid.
    let mut again = Browser::new(&server);
    assert_eq!(again.sign_in_with("microsoft").status, 302);
    assert_eq!(again.whoami(), "old@example.com member Old");

    // Recorded for another tenant, or with no tenant in a multi-tenant app:
    // the same `sub` may be someone else's.
    for sub in ["ms-elsewhere", "ms-no-tenant"] {
        state.lock().unwrap().claims = microsoft(sub);
        let mut visitor = Browser::new(&server);
        assert_eq!(visitor.sign_in_with("microsoft").status, 401, "{sub}");
        assert_eq!(visitor.whoami(), "guest", "{sub}");
    }
}

/// What the shared service and the controller hold to, read from the source:
/// a guest start drops a leftover link, a provider sign-in records
/// `auth_time` (the OIDC provider's `max_age`), and the GET never links.
#[test]
fn oauth_sso_clears_stale_links_and_stamps_auth_time() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    create_oauth(dir.path().to_str().unwrap(), "github").unwrap();
    let read = |rel: &str| fs::read_to_string(dir.path().join(rel)).unwrap();

    let service = read("app/services/oauth_sso.sl");
    let begin_state = service
        .split("static def begin_state")
        .nth(1)
        .and_then(|rest| rest.split("\n  end").next())
        .unwrap();
    assert!(
        begin_state.contains(r#"Session.delete("oauth_link_user")"#),
        "{begin_state}"
    );
    assert!(
        // A sign-in opens the session through the one helper that stamps
        // `auth_time` and drops what the previous occupant left.
        service.contains("auth_open_session(user)"),
        "{service}"
    );
    assert!(service.contains(r#"respond_to?("sso_only?")"#), "{service}");

    let controller = read("app/controllers/oauth_controller.sl");
    let start = controller
        .split("def start(req)")
        .nth(1)
        .and_then(|rest| rest.split("\nend").next())
        .unwrap();
    assert!(!start.contains("begin_link"), "{start}");
    assert!(
        start.contains(r#"redirect("/oauth/identities")"#),
        "{start}"
    );
    assert!(
        controller.contains("def link_provider(req)"),
        "{controller}"
    );
}

/// Apple and Microsoft are read from an unsigned id_token, trusted because it
/// came over TLS from their own token endpoint: a base-URL override may only
/// name this machine (a test stand-in), never another host.
#[test]
fn unsigned_id_token_providers_are_reached_only_at_their_own_host() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_app_with_auth(root);
    create_oauth(root.to_str().unwrap(), "apple").unwrap();
    for (url, expected) in [
        ("", "https://appleid.apple.com"),
        ("https://appleid.apple.com", "https://appleid.apple.com"),
        ("http://127.0.0.1:9", "http://127.0.0.1:9"),
        ("http://localhost:9/", "http://localhost:9"),
        ("https://evil.example", "refused"),
        ("http://localhost.evil.example", "refused"),
        ("http://127.0.0.1@evil.example", "refused"),
        ("http://127.0.0.1:9/path", "refused"),
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
            .args([
                "runner",
                "-e",
                r#"print(OauthSso.token_endpoint_base("APPLE_BASE_URL", "https://appleid.apple.com") rescue "refused")"#,
            ])
            .current_dir(root)
            .env_remove("SOLI_ENGINE")
            .env("APPLE_BASE_URL", url)
            .output()
            .expect("run soli");
        let text = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            text.trim().lines().last().unwrap_or_default(),
            expected,
            "{url}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// The guards the served tests cannot reach: a self-hosted provider over
/// plain http, a token from another Microsoft tenant, a guessed password.
#[test]
fn the_generated_services_hold_their_configuration_to_account() {
    let dir = tempfile::tempdir().unwrap();
    make_app_with_auth(dir.path());
    for provider in ["github", "gitlab", "microsoft", "apple"] {
        create_oauth(dir.path().to_str().unwrap(), provider).unwrap();
    }
    let read = |path: &str| fs::read_to_string(dir.path().join(path)).unwrap();
    let sso = read("app/services/oauth_sso.sl");
    assert!(sso.contains("must be an https:// URL (plain http only for this machine)"));
    for (file, names) in [
        (
            "app/services/github_oauth.sl",
            vec!["GITHUB_BASE_URL", "GITHUB_API_URL"],
        ),
        ("app/services/gitlab_oauth.sl", vec!["GITLAB_BASE_URL"]),
    ] {
        let service = read(file);
        for name in names {
            assert!(
                service.contains(&format!(r#"OauthSso.instance_base("{name}""#)),
                "{file}: {name} is not held to https\n{service}"
            );
        }
    }
    let microsoft = read("app/services/microsoft_oauth.sl");
    assert!(microsoft.contains("tenant_id != configured"), "{microsoft}");
    assert!(
        microsoft.contains("MICROSOFT_TENANT must be a tenant id (a GUID)"),
        "{microsoft}"
    );
    assert!(microsoft.contains(r#"OauthSso.check_nonce(claims["nonce"], "Microsoft")"#));
    assert!(read("app/services/apple_oauth.sl")
        .contains(r#"OauthSso.check_nonce(claims["nonce"], "Apple")"#));
    let controller = read("app/controllers/oauth_controller.sl");
    assert!(
        controller
            .contains("rate_limiter_from_ip(req, AUTH_ATTEMPTS_PER_IP, AUTH_IP_WINDOW_SECONDS)"),
        "{controller}"
    );
    assert!(controller.contains("OauthSso.log_safe("), "{controller}");
}

/// A link is added by the session that asked for it. If the account's
/// sessions are ended while the provider is answering — a password reset —
/// the session that comes back from the provider links nothing, and is not
/// stamped back to life.
#[test]
fn a_session_ended_while_the_provider_answered_links_nothing() {
    let dir = served_app();
    let root = dir.path().to_path_buf();
    let state = Arc::new(Mutex::new(Provider::default()));
    let providers = start_providers(Arc::clone(&state));
    let server = serve(&root, providers, "open");
    state.lock().unwrap().profile = gitlab("gl-1", "ada@example.com", true);
    let mut ada = Browser::new(&server);
    assert_eq!(ada.sign_in_with("gitlab").status, 302);
    assert_eq!(ada.whoami(), "ada@example.com member From GitLab");

    // Another GitLab account, to connect — and the account's sessions end
    // when the provider is asked for the token, in the middle of the
    // callback.
    state.lock().unwrap().profile = gitlab("gl-2", "second@example.com", true);
    let link_state = ada.start_link("gitlab");
    let app = root.clone();
    state.lock().unwrap().on_token = Some(Box::new(move || {
        let code = r##"User.find_by("email", "ada@example.com").end_sessions"##;
        let (ok, output) = soli(&app, &["runner", "-e", code]);
        assert!(ok, "{output}");
    }));
    let reply = ada.get(&format!(
        "/auth/gitlab/callback?code=the-code&state={link_state}"
    ));
    state.lock().unwrap().on_token = None;
    assert_ne!(reply.location, "/oauth/identities", "{}", reply.body);
    assert!(
        reply
            .body
            .contains("sign in again before connecting an account"),
        "{} {}",
        reply.status,
        reply.body
    );
    let (ok, output) = soli(
        &root,
        &[
            "runner",
            "-e",
            r##"print("linked #{OauthIdentity.where({"uid": "gl-2"}).count}")"##,
        ],
    );
    assert!(ok && output.trim_end().ends_with("linked 0"), "{output}");
    assert_eq!(ada.whoami(), "guest", "the ended session is back");
}

/// One provider callback sent several times together — on separate workers,
/// for real — against a provider that would answer the same code each time.
/// The `state` answers one callback: one copy signs in, the others are
/// refused before the provider is asked anything.
#[test]
fn a_callback_sent_together_is_answered_once() {
    let dir = served_app();
    let state = Arc::new(Mutex::new(Provider::default()));
    let providers = start_providers(Arc::clone(&state));
    let server = serve_workers(dir.path(), providers, "open", 4);
    state.lock().unwrap().profile = gitlab("gl-9", "ada@example.com", true);
    let mut ada = Browser::new(&server);
    let sent_state = ada.start("gitlab");
    let gate = Arc::new(std::sync::Barrier::new(4));
    let copies: Vec<_> = (0..4)
        .map(|_| {
            let mut copy = Browser {
                port: ada.port,
                jar: ada.jar.clone(),
            };
            let gate = Arc::clone(&gate);
            let path = format!("/auth/gitlab/callback?code=the-code&state={sent_state}");
            std::thread::spawn(move || {
                gate.wait();
                let reply = copy.get(&path);
                (reply.status, reply.location)
            })
        })
        .collect();
    let replies: Vec<_> = copies.into_iter().map(|one| one.join().unwrap()).collect();
    let signed_in = replies
        .iter()
        .filter(|(status, location)| *status == 302 && location == "/")
        .count();
    assert_eq!(signed_in, 1, "{replies:?}");
    assert_eq!(
        replies.iter().filter(|(status, _)| *status == 403).count(),
        3,
        "{replies:?}"
    );
    // The copies that were refused took nothing from the one answered: the
    // provider was asked once, with the verifier.
    let sent = state.lock().unwrap().last_token_request.clone();
    let verifier = parameter(&sent, "code_verifier").unwrap_or_default();
    assert!(verifier.len() >= 43, "the verifier was gone: {sent}");
    assert_eq!(state.lock().unwrap().token_requests, 1, "{replies:?}");
}
