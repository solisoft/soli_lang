//! `soli generate auth --magic-link | --api-tokens | --impersonation`: what
//! each writes, that none edits the auth scaffold, that the code is fmt/lint
//! clean, and a served app (SQLite, migrated) where a link signs in only the
//! browser that asked for it, a token authenticates a request within its
//! scopes, and an administrator acts as a user and comes back.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Value as Json};
use solilang::scaffold::{create_api_tokens, create_auth, create_impersonation, create_magic_link};

fn make_app_skeleton(root: &Path) {
    for sub in ["app/controllers", "app/models", "app/views", "config"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("config/routes.sl"), "# routes\n").unwrap();
}

fn full_app() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    let folder = dir.path().to_str().unwrap();
    create_auth(folder).expect("generate auth");
    create_magic_link(folder).expect("--magic-link");
    create_api_tokens(folder).expect("--api-tokens");
    create_impersonation(folder).expect("--impersonation");
    dir
}

#[test]
fn each_flag_writes_its_files_and_edits_none_of_the_auth_scaffold() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_app_skeleton(root);
    let folder = root.to_str().unwrap();
    for generate in [create_magic_link, create_api_tokens, create_impersonation] {
        let err = generate(folder).unwrap_err();
        assert!(err.contains("soli generate auth"), "{err}");
    }

    create_auth(folder).unwrap();
    let kept = [
        "app/models/user.sl",
        "app/controllers/sessions_controller.sl",
        "app/middleware/current_user.sl",
    ];
    for rel in kept {
        fs::write(root.join(rel), "# mine\n").unwrap();
    }
    for _ in 0..2 {
        create_magic_link(folder).unwrap();
        create_api_tokens(folder).unwrap();
        create_impersonation(folder).unwrap();
    }
    for rel in kept {
        assert_eq!(fs::read_to_string(root.join(rel)).unwrap(), "# mine\n");
    }
    for rel in [
        "app/models/magic_link.sl",
        "app/controllers/magic_links_controller.sl",
        "app/mailers/magic_link_mailer.sl",
        "app/views/magic_links/new.html.slv",
        "app/views/magic_link_mailer/sign_in.html.slv",
        "app/models/api_token.sl",
        "app/middleware/api_token.sl",
        "app/controllers/api_tokens_controller.sl",
        "app/views/api_tokens/index.html.slv",
        "app/controllers/impersonations_controller.sl",
        "app/helpers/impersonation_helper.sl",
        "app/views/impersonations/_banner.html.slv",
    ] {
        assert!(root.join(rel).exists(), "{rel} not created");
    }
    let routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    for route in [
        "magic_links#show",
        "magic_links#redeem",
        "api_tokens#destroy",
        "impersonations#create",
    ] {
        assert_eq!(routes.matches(route).count(), 1, "{route}: {routes}");
    }
    let migrations: Vec<String> = fs::read_dir(root.join("db/migrations"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    for name in ["create_magic_links", "create_api_tokens"] {
        assert_eq!(
            migrations.iter().filter(|m| m.contains(name)).count(),
            1,
            "{migrations:?}"
        );
    }
}

fn soli(dir: &Path, args: &[&str]) -> (bool, String) {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(args)
        .current_dir(dir)
        .env_remove("SOLI_ENGINE")
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", dir.display()),
        )
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
fn cli_takes_the_flags_and_generated_code_passes_fmt_lint_and_check() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    let (ok, output) = soli(
        dir.path(),
        &[
            "generate",
            "auth",
            "--magic-link",
            "--api-tokens",
            "--impersonation",
        ],
    );
    assert!(ok, "{output}");
    for hint in ["/login/link", "/api_tokens", "/impersonate/:id"] {
        assert!(output.contains(hint), "{hint}: {output}");
    }
    assert!(dir.path().join("app/models/api_token.sl").exists());

    let (formatted, output) = soli(dir.path(), &["fmt", "--check", "."]);
    assert!(formatted, "soli fmt --check:\n{output}");
    let (clean, output) = soli(dir.path(), &["lint", "."]);
    assert!(clean, "soli lint:\n{output}");
    let (checked, output) = soli(dir.path(), &["check", "."]);
    assert!(checked, "soli check:\n{output}");
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

fn serve(root: &Path) -> Server {
    let port = unused_port();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_soli"));
    command
        .arg("serve")
        .arg(root)
        .args(["--port", &port.to_string(), "--workers", "1"])
        .current_dir(root)
        .env_remove("SOLI_ENGINE")
        .env("AUTH_SIGNUP", "first_user")
        // A test run: the first account needs no AUTH_SETUP_TOKEN.
        .env("APP_ENV", "test")
        .env("AUTH_ATTEMPTS_PER_IP", "1000")
        .env("APP_BASE_URL", "https://app.test")
        .env("SOLI_CSRF_TOKENS", "require")
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
        // Mail is kept in memory, where /whoami/mail reads the link back.
        .env("SOLI_MAIL_DELIVERY_METHOD", "test")
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

/// A browser stand-in (a cookie jar), or a script (`bearer`).
struct Client {
    port: u16,
    jar: BTreeMap<String, String>,
    bearer: Option<String>,
}

impl Client {
    fn browser(server: &Server) -> Self {
        Client {
            port: server.1,
            jar: BTreeMap::new(),
            bearer: None,
        }
    }

    fn script(server: &Server, token: &str) -> Self {
        Client {
            port: server.1,
            jar: BTreeMap::new(),
            bearer: Some(token.to_string()),
        }
    }

    fn request(&mut self, method: &str, path: &str, body: Option<Json>) -> Reply {
        let origin = format!("http://127.0.0.1:{}", self.port);
        let agent = ureq::AgentBuilder::new().redirects(0).build();
        let mut request = agent.request(method, &format!("{origin}{path}"));
        if !self.jar.is_empty() {
            let cookie = self
                .jar
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            request = request.set("Cookie", &cookie);
        }
        match &self.bearer {
            Some(token) => request = request.set("Authorization", &format!("Bearer {token}")),
            // A browser names its origin on a state-changing request.
            None if method != "GET" => request = request.set("Origin", &origin),
            None => {}
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
        self.request("GET", path, None)
    }

    fn post(&mut self, path: &str, body: Json) -> Reply {
        self.request("POST", path, Some(body))
    }

    fn whoami(&mut self) -> String {
        self.get("/whoami").body
    }
}

const PASSWORD: &str = "correct horse battery";

fn sign_up(server: &Server, email: &str) -> Client {
    let mut client = Client::browser(server);
    let reply = client.post("/signup", json!({"email": email, "password": PASSWORD}));
    assert_eq!(reply.status, 302, "{email}: {}", reply.body);
    client
}

/// The newest sign-in link the server mailed, as a path.
fn mailed_link(server: &Server) -> String {
    let mail = Client::browser(server).get("/whoami/mail").body;
    let start = mail
        .rfind("https://app.test/login/link/")
        .unwrap_or_else(|| panic!("no link in the last mail:\n{mail}"));
    let url: String = mail[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || ":/._-".contains(*c))
        .collect();
    url.trim_start_matches("https://app.test").to_string()
}

fn served_app() -> tempfile::TempDir {
    let dir = full_app();
    let root = dir.path();
    fs::create_dir_all(root.join("app/views/layouts")).unwrap();
    fs::write(
        root.join("app/views/layouts/application.html.slv"),
        "<main><%- partial(\"impersonations/banner\") %><%= yield %></main>\n",
    )
    .unwrap();
    fs::write(
        root.join("app/controllers/whoami_controller.sl"),
        r##"class WhoamiController < Controller
  static {
    this.layout = "application"
  }

  # GET /whoami
  def index
    user = current_user()
    render_text(user.nil? ? "guest" : "#{user.email} #{user.role}")
  end

  # GET /whoami/page
  def page
    render("magic_links/new", {"title": "x", "notice": "", "error": ""})
  end

  # GET /whoami/mail — the last mail sent, for the test to read its link
  def mail
    render_json(Mailer.deliveries())
  end

  # GET /whoami/key
  def key
    render_text(current_user()._key)
  end

  # POST /whoami/write
  def write
    return render_json({"error": "scope"}, 403) unless api_scope?("write")

    render_json({"wrote": true})
  end
end
"##,
    )
    .unwrap();
    let mut routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    routes.push_str(
        "get(\"/whoami\", \"whoami#index\")\nget(\"/whoami/page\", \"whoami#page\")\n\
         get(\"/whoami/mail\", \"whoami#mail\")\nget(\"/whoami/key\", \"whoami#key\")\npost(\"/whoami/write\", \"whoami#write\")\n",
    );
    fs::write(root.join("config/routes.sl"), routes).unwrap();
    let (ok, output) = soli(root, &["db:migrate", "up", "."]);
    assert!(ok, "db:migrate up:\n{output}");
    dir
}

#[test]
fn a_magic_link_signs_in_the_browser_that_asked_once() {
    let dir = served_app();
    let server = serve(dir.path());
    drop(sign_up(&server, "ada@example.com"));

    // Asking answers the same for an address with no account, cookie included.
    let mut stranger = Client::browser(&server);
    let unknown = stranger.post("/login/link", json!({"email": "nobody@example.com"}));
    let mut ada = Client::browser(&server);
    let known = ada.post("/login/link", json!({"email": "Ada@Example.com"}));
    assert_eq!((unknown.status, known.status), (200, 200));
    // The same page, but for the form's per-session tokens.
    let notice = "If that email has an account, a sign-in link is on its way.";
    assert!(unknown.body.contains(notice) && known.body.contains(notice));
    assert_eq!(unknown.body.len(), known.body.len());
    assert!(
        stranger.jar.contains_key("__Host-magic_link") && ada.jar.contains_key("__Host-magic_link")
    );
    let link = mailed_link(&server);

    // Opened elsewhere — a mail scanner, another device — it signs nobody in
    // and is not used up.
    let mut scanner = Client::browser(&server);
    assert_eq!(scanner.get(&link).status, 401);
    assert_eq!(stranger.get(&link).status, 401);
    assert_eq!(scanner.whoami(), "guest");

    // In the browser that asked, it signs in. Once.
    // The link itself signs nobody in: it names the account and waits for
    // the button — on a shared browser, a link someone else asked for and
    // sent along would otherwise sign the visitor in to that someone's
    // account.
    let page = ada.get(&link);
    assert_eq!(page.status, 200, "{}", page.body);
    assert!(
        page.body.contains("Sign in as ada@example.com"),
        "{}",
        page.body
    );
    assert_eq!(ada.whoami(), "guest");
    let reply = ada.post(&link, json!({}));
    assert_eq!(
        (reply.status, reply.location.as_str()),
        (302, "/"),
        "{}",
        reply.body
    );
    assert_eq!(ada.whoami(), "ada@example.com admin");
    assert!(!ada.jar.contains_key("__Host-magic_link"));
    assert_eq!(ada.get(&link).status, 401, "the link worked twice");

    // Asking twice from one browser keeps the first link good: the browser
    // keeps its cookie, and no link is removed for another. Whichever is
    // opened signs in; the cookie goes with it, so the other is spent.
    let mut phone = Client::browser(&server);
    phone.post("/login/link", json!({"email": "ada@example.com"}));
    let first = mailed_link(&server);
    phone.post("/login/link", json!({"email": "ada@example.com"}));
    let second = mailed_link(&server);
    assert_ne!(first, second);
    assert_eq!(phone.post(&first, json!({})).status, 302);
    assert_eq!(phone.get(&second).status, 401);

    // Ada never confirmed her address, but she has been using the account
    // (she was signed in when she created it): a link to her mailbox signs
    // in and confirms, and takes nothing away — her password still works.
    // A mailbox alone must not strip an account in use of what its user set.
    let with_password = Client::browser(&server).post(
        "/login",
        json!({"email": "ada@example.com", "password": PASSWORD}),
    );
    assert_eq!(with_password.status, 302, "{}", with_password.body);

    // An account nobody ever entered is another matter: whoever created it
    // may not be its owner. The first proof of its address — here, a link —
    // takes it over, and the password it was created with opens nothing.
    let issued = runner(
        dir.path(),
        r##"placeholder = User.new
placeholder.email = "grace@example.com"
placeholder.role = "member"
placeholder.set_password("a squatter chose this one")
placeholder.awaiting_owner = true
placeholder.save
sent = MagicLink.issue(placeholder, "198.51.100.7")
print("#{sent["token"]} #{sent["nonce"]}")"##,
    );
    let (token, nonce) = issued.split_once(' ').expect("token and nonce");
    let mut grace = Client::browser(&server);
    grace
        .jar
        .insert("__Host-magic_link".to_string(), nonce.to_string());
    let reply = grace.post(&format!("/login/link/{token}"), json!({}));
    assert_eq!(reply.status, 302, "{}", reply.body);
    assert_eq!(grace.whoami(), "grace@example.com member");
    let squatter = Client::browser(&server).post(
        "/login",
        json!({"email": "grace@example.com", "password": "a squatter chose this one"}),
    );
    assert_eq!(
        squatter.status, 401,
        "the password a never-entered account was created with survived its takeover"
    );

    // Anyone can ask for a link to any address. Asking never removes a link
    // that is waiting — the owner's still works — and is refused instead:
    // past three for one address, past a hundred for the account. Expired
    // rows do not count, a browser that asks again holds both nonces, and
    // an IPv6 requester is its /64.
    assert_eq!(
        runner(
            dir.path(),
            r##"user = User.find_by("email", "ada@example.com")
MagicLink.where({"user_id": user._key}).delete_all
owners = MagicLink.issue(user, "198.51.100.7")
again = MagicLink.issue(user, "198.51.100.7")
held = MagicLink.cookie_with(MagicLink.cookie_with("junk." + owners["nonce"], again["nonce"]), "f" * 64)
refused = (0..10).map { |i| MagicLink.issue(user, "203.0.113.9") }.filter { |sent| sent.nil? }.length
after_one_address = MagicLink.where({"user_id": user._key}).count
(0..200).each do |i|
  MagicLink.issue(user, "192.0.2.#{i}")
end
full = MagicLink.where({"user_id": user._key}).count
at_ceiling = MagicLink.issue(user, "a new address").nil?
both = !MagicLink.redeem(owners["token"], held).nil? && !MagicLink.redeem(again["token"], held).nil?
MagicLink.where({"user_id": user._key}).update_all({"expires_at": 1})
fresh = !MagicLink.issue(user, "a new address").nil?
left = MagicLink.where({"user_id": user._key}).count
networks = ["2001:db8:0:1::5", "2001:0db8:0000:0001:ffff::9", "2001:db8:0:2::5", "::ffff:192.0.2.1", "192.0.2.1", "192.0.2.1:5678", "[2001:DB8:0:1::7]:443", "fe80::1%eth0", "not:an:address", "::ffff:c000:201"].map { |address| MagicLink.requester_for(address) }
print("#{networks.join(" ")} #{again["nonce"] != owners["nonce"]} #{refused} #{after_one_address} #{full} #{at_ceiling} #{both} #{fresh} #{left}")"##
        ),
        "8193:3512:0:1::/64 8193:3512:0:1::/64 8193:3512:0:2::/64 192.0.2.1 192.0.2.1 192.0.2.1 8193:3512:0:1::/64 65152:0:0:0::/64 not:an:address 192.0.2.1 true 7 5 100 true true true 1"
    );
}

#[test]
fn an_api_token_authenticates_a_request_within_its_scopes() {
    let dir = served_app();
    let server = serve(dir.path());
    let mut ada = sign_up(&server, "ada@example.com");

    // Created from the account, shown once, listed by its prefix after.
    assert_eq!(
        Client::browser(&server).get("/api_tokens").location,
        "/login"
    );
    let created = ada.post(
        "/api_tokens",
        json!({"name": "export", "scope_read": "1", "days": "30"}),
    );
    assert_eq!(created.status, 201, "{}", created.body);
    let start = created
        .body
        .find("data-new-token>")
        .expect("no token shown")
        + 15;
    let token = created.body[start..]
        .split('<')
        .next()
        .unwrap()
        .trim()
        .to_string();
    assert!(
        token.starts_with("soli_pat_") && token.len() == 73,
        "{token}"
    );
    let listed = ada.get("/api_tokens");
    assert!(!listed.body.contains(&token), "the token is shown twice");
    assert!(listed.body.contains(&token[..15]), "{}", listed.body);
    assert!(listed.body.contains("read") && listed.body.contains("never used"));
    // Only its digest is stored.
    let mut stored = fs::read(dir.path().join("app.sqlite3")).unwrap_or_default();
    stored.extend(fs::read(dir.path().join("app.sqlite3-wal")).unwrap_or_default());
    let stored = String::from_utf8_lossy(&stored);
    assert!(
        stored.contains("export"),
        "the data is not where this looks"
    );
    assert!(!stored.contains(&token), "the token is stored in plaintext");

    // A script with the token is Ada, within the token's scopes.
    let mut script = Client::script(&server, &token);
    assert_eq!(script.whoami(), "ada@example.com admin");
    assert_eq!(script.post("/whoami/write", json!({})).status, 403);
    assert_eq!(
        ada.post("/whoami/write", json!({})).status,
        200,
        "a session has no scopes"
    );
    assert!(ada.get("/api_tokens").body.contains("last used"));
    // It cannot manage tokens, nor reach any page that manages the account:
    // those are closed to tokens whatever their scopes.
    assert_eq!(script.get("/api_tokens").status, 403);
    assert_eq!(
        script
            .post("/api_tokens", json!({"name": "more", "scope_write": "1"}))
            .status,
        403
    );
    assert_eq!(script.post("/impersonate/anyone", json!({})).status, 403);

    // Ten wrong passwords lock the password, not the integration: anyone who
    // knows the address can send them.
    let mut stranger = Client::browser(&server);
    for _ in 0..10 {
        let reply = stranger.post(
            "/login",
            json!({"email": "ada@example.com", "password": "not the password"}),
        );
        assert_eq!(reply.status, 401);
    }
    let locked = stranger.post(
        "/login",
        json!({"email": "ada@example.com", "password": PASSWORD}),
    );
    assert_eq!(locked.status, 401, "the account did not lock");
    assert_eq!(script.whoami(), "ada@example.com admin");

    // A wrong token is a 401, not the session behind it.
    let mut wrong = Client::script(&server, "soli_pat_0000");
    wrong.jar = ada.jar.clone();
    let refused = wrong.get("/whoami");
    assert_eq!(refused.status, 401, "{}", refused.body);

    // A write token writes; revoked, it is refused.
    let created = ada.post(
        "/api_tokens",
        json!({"name": "sync", "scope_write": "1", "scope_admin": "1", "days": "0"}),
    );
    let start = created.body.find("data-new-token>").unwrap() + 15;
    let writer = created.body[start..]
        .split('<')
        .next()
        .unwrap()
        .trim()
        .to_string();
    let mut sync = Client::script(&server, &writer);
    assert_eq!(sync.post("/whoami/write", json!({})).status, 200);
    let listed = ada.get("/api_tokens");
    assert!(listed.body.contains("no expiry"), "{}", listed.body);
    assert!(
        !listed.body.contains("admin,") && !listed.body.contains(", admin"),
        "an unknown scope was kept"
    );
    let mut paths: Vec<String> = listed
        .body
        .split("action=\"/api_tokens/")
        .skip(1)
        .map(|rest| format!("/api_tokens/{}", rest.split('"').next().unwrap()))
        .collect();
    assert_eq!(paths.len(), 2, "{}", listed.body);
    for path in paths.drain(..) {
        assert_eq!(ada.request("DELETE", &path, Some(json!({}))).status, 302);
    }
    assert_eq!(sync.get("/whoami").status, 401);

    // A password reset takes the tokens with it: whoever had the account
    // for a while keeps nothing they added.
    let created = ada.post(
        "/api_tokens",
        json!({"name": "left behind", "scope_read": "1"}),
    );
    let start = created.body.find("data-new-token>").unwrap() + 15;
    let left = created.body[start..]
        .split('<')
        .next()
        .unwrap()
        .trim()
        .to_string();
    let mut left_behind = Client::script(&server, &left);
    assert_eq!(left_behind.whoami(), "ada@example.com admin");
    runner(
        dir.path(),
        "User.find_by(\"email\", \"ada@example.com\").finish_password_reset(\"another horse battery 9\")",
    );
    assert_eq!(left_behind.get("/whoami").status, 401);
    assert_eq!(script.get("/whoami").status, 401);
}

#[test]
fn an_administrator_acts_as_a_user_and_comes_back() {
    let dir = served_app();
    let server = serve(dir.path());
    // The first account runs the app; sign-up then closes, so the others are
    // created the way an administrator would.
    drop(sign_up(&server, "admin@example.com"));
    fs::write(
        dir.path().join("users.sl"),
        "for email in [\"grace@example.com\", \"second-admin@example.com\"]\n  user = User.new\n  \
         user.email = email\n  user.role = email.starts_with?(\"second\") ? \"admin\" : \"member\"\n  \
         user.set_password(\"correct horse battery\")\n  user.save\nend\n",
    )
    .unwrap();
    drop(server);
    let (ok, output) = soli(dir.path(), &["db:seed", ".", "users.sl"]);
    assert!(ok, "{output}");
    let server = serve(dir.path());
    let mut admin = Client::browser(&server);
    let login = |client: &mut Client, email: &str| {
        let reply = client.post("/login", json!({"email": email, "password": PASSWORD}));
        assert_eq!(reply.status, 302, "{email}: {}", reply.body);
    };
    login(&mut admin, "admin@example.com");
    let mut grace = Client::browser(&server);
    login(&mut grace, "grace@example.com");
    let mut second = Client::browser(&server);
    login(&mut second, "second-admin@example.com");
    let grace_key = grace.get("/whoami/key").body;
    let admin_key = admin.get("/whoami/key").body;
    let second_key = second.get("/whoami/key").body;

    // A member cannot; an administrator cannot take another administrator,
    // themself, or nobody.
    let start =
        |client: &mut Client, key: &str| client.post(&format!("/impersonate/{key}"), json!({}));
    assert_eq!(start(&mut grace, &admin_key).status, 403);
    assert_eq!(start(&mut admin, &second_key).status, 403);
    assert_eq!(start(&mut admin, &admin_key).status, 403);
    assert_eq!(start(&mut admin, "missing").status, 403);
    assert_eq!(
        start(&mut Client::browser(&server), &grace_key).location,
        "/login"
    );
    assert_eq!(admin.whoami(), "admin@example.com admin");

    // The administrator becomes the user, under a banner.
    let before = admin.jar.get("session_id").cloned();
    let reply = start(&mut admin, &grace_key);
    assert_eq!(
        (reply.status, reply.location.as_str()),
        (302, "/"),
        "{}",
        reply.body
    );
    assert_eq!(admin.whoami(), "grace@example.com member");
    assert_ne!(
        admin.jar.get("session_id").cloned(),
        before,
        "the session id was kept"
    );
    let page = admin.get("/whoami/page");
    assert!(
        page.body.contains("data-impersonation-banner"),
        "{}",
        page.body
    );
    assert!(page.body.contains("You are acting as"), "{}", page.body);
    assert!(!grace
        .get("/whoami/page")
        .body
        .contains("data-impersonation-banner"));
    // Not from inside an impersonation, and not to mint the user's tokens.
    assert_eq!(start(&mut admin, &grace_key).status, 403);
    assert_eq!(admin.get("/api_tokens").location, "/login");

    // The impersonation holds only while the administrator's own sessions
    // do: once they are ended (a password reset, a revocation), it is over,
    // and stopping it gives no administrator session back.
    let mut hijacked = Client::browser(&server);
    login(&mut hijacked, "admin@example.com");
    assert_eq!(start(&mut hijacked, &grace_key).status, 302);
    assert_eq!(hijacked.whoami(), "grace@example.com member");

    // And comes back.
    let reply = admin.request("DELETE", "/impersonate", Some(json!({})));
    assert_eq!((reply.status, reply.location.as_str()), (302, "/"));
    assert_eq!(admin.whoami(), "admin@example.com admin");
    assert!(!admin
        .get("/whoami/page")
        .body
        .contains("data-impersonation-banner"));
    // Stopping with nothing to stop is a no-op, for anyone.
    assert_eq!(
        grace
            .request("DELETE", "/impersonate", Some(json!({})))
            .status,
        302
    );
    assert_eq!(grace.whoami(), "grace@example.com member");

    runner(
        dir.path(),
        "user = User.find_by(\"email\", \"admin@example.com\")\n\
         user.session_version = user.current_session_version + 1\nuser.save",
    );
    assert_eq!(
        hijacked.whoami(),
        "guest",
        "the impersonation outlived the admin's sessions"
    );
    hijacked.request("DELETE", "/impersonate", Some(json!({})));
    assert_eq!(hijacked.whoami(), "guest");
}

/// `soli runner -e code` against the served app's database: the last line it
/// prints.
fn runner(root: &Path, code: &str) -> String {
    let (ok, output) = soli(root, &["runner", "-e", code]);
    assert!(ok, "{output}");
    output.trim().lines().last().unwrap_or_default().to_string()
}

/// An account that signs in through single sign-on only (`sso_only?`, set by
/// `soli generate saml`) gets no sign-in link and no API token, and a token or
/// a link it had before is refused: none of them would end with the user's
/// removal at the identity provider.
#[test]
fn a_single_sign_on_only_account_gets_no_link_and_no_token() {
    let dir = served_app();
    let root = dir.path();
    let server = serve(root);
    let mut ada = sign_up(&server, "ada@example.com");
    let created = ada.post("/api_tokens", json!({"name": "old", "scope_read": "1"}));
    assert_eq!(created.status, 201, "{}", created.body);
    let start = created.body.find("data-new-token>").unwrap() + 15;
    let token = created.body[start..]
        .split('<')
        .next()
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(
        Client::script(&server, &token).whoami(),
        "ada@example.com admin"
    );
    let link = runner(
        root,
        r##"user = User.find_by("email", "ada@example.com")
link = MagicLink.issue(user)
user.sso_only = true
user.save
print("#{link["token"]} #{link["nonce"]}")"##,
    );
    let (link_token, nonce) = link.split_once(' ').expect("token and nonce");

    // The token made before is refused, and no new one is made.
    assert_eq!(Client::script(&server, &token).get("/whoami").status, 401);
    let refused = ada.post("/api_tokens", json!({"name": "new", "scope_read": "1"}));
    assert_eq!(refused.status, 403, "{}", refused.body);
    assert_eq!(
        runner(root, r#"print(ApiToken.where({"name": "new"}).count)"#),
        "0"
    );

    // Asking for a link answers as for anyone, and sends none.
    let mut browser = Client::browser(&server);
    let asked = browser.post("/login/link", json!({"email": "ada@example.com"}));
    assert_eq!(asked.status, 200);
    assert!(asked
        .body
        .contains("If that email has an account, a sign-in link is on its way."));
    let mail = Client::browser(&server).get("/whoami/mail").body;
    assert!(!mail.contains("/login/link/"), "a link was mailed: {mail}");

    // A link issued before signs nobody in, even in the browser it was for.
    browser
        .jar
        .insert("__Host-magic_link".to_string(), nonce.to_string());
    assert_eq!(
        browser.get(&format!("/login/link/{link_token}")).status,
        401
    );
    assert_eq!(browser.whoami(), "guest");
}

/// "Nobody ever signed in to it" is what lets a proof of the address take an
/// account over, so every way of using one has to mark it as used: a
/// session that was already open when the mark did not exist yet (an app
/// regenerated over its users), and an API token made outside the browser.
/// Without the mark, a sign-in link to an unconfirmed account in use
/// stripped it of its password and second factor.
#[test]
fn an_open_session_and_a_token_mark_the_account_as_used() {
    let dir = served_app();
    let server = serve(dir.path());
    let mut ada = sign_up(&server, "ada@example.com");
    let forget = r##"user = User.find_by("email", "ada@example.com")
User.where({"_key": user._key}).update_all({"awaiting_owner": true})
print(User.find(user._key).never_signed_in?)"##;
    assert_eq!(runner(dir.path(), forget), "true");
    assert_eq!(ada.whoami(), "ada@example.com admin");
    let used = r##"print(User.find_by("email", "ada@example.com").never_signed_in?)"##;
    assert_eq!(runner(dir.path(), used), "false");

    let minted = runner(
        dir.path(),
        r##"user = User.find_by("email", "ada@example.com")
User.where({"_key": user._key}).update_all({"awaiting_owner": true})
print(ApiToken.issue(user, "seed", [], 0)["token"])"##,
    );
    let reply = ureq::get(&format!("http://127.0.0.1:{}/whoami", server.1))
        .set("Authorization", &format!("Bearer {minted}"))
        .call()
        .unwrap();
    assert_eq!(reply.into_string().unwrap(), "ada@example.com admin");
    assert_eq!(runner(dir.path(), used), "false");
}

/// A browser restoring its tabs sends the same remember-me cookie on several
/// requests at once. For real, against a served app: every one of them is
/// signed in, exactly one is handed a replacement (the others lose the
/// compare-and-swap or meet the replaced token within its minute, and set no
/// cookie), and that replacement is the token the account holds afterwards.
#[test]
fn parallel_requests_with_one_remember_me_cookie_get_one_replacement() {
    let dir = served_app();
    let server = serve(dir.path());
    drop(sign_up(&server, "ada@example.com"));
    let mut ada = Client::browser(&server);
    let reply = ada.post(
        "/login",
        json!({"email": "ada@example.com", "password": PASSWORD, "remember_me": "1"}),
    );
    assert_eq!(reply.status, 302, "{}", reply.body);
    let remembered = ada.jar.get("remember_token").expect("no cookie").clone();
    let port = server.1;

    let tabs: Vec<_> = (0..8)
        .map(|_| {
            let remembered = remembered.clone();
            std::thread::spawn(move || {
                let mut tab = Client {
                    port,
                    jar: BTreeMap::new(),
                    bearer: None,
                };
                tab.jar.insert("remember_token".to_string(), remembered);
                let who = tab.whoami();
                (who, tab.jar.get("remember_token").cloned())
            })
        })
        .collect();
    let results: Vec<_> = tabs.into_iter().map(|tab| tab.join().unwrap()).collect();
    for (who, _) in &results {
        assert_eq!(who, "ada@example.com admin", "{results:?}");
    }
    let replacements: Vec<&String> = results
        .iter()
        .filter_map(|(_, cookie)| cookie.as_ref())
        .filter(|cookie| **cookie != remembered)
        .collect();
    assert_eq!(replacements.len(), 1, "{results:?}");

    // The one replacement is the account's token: it restores a session,
    // and nothing was taken for a copy (the first session still stands).
    let mut later = Client::browser(&server);
    later
        .jar
        .insert("remember_token".to_string(), replacements[0].clone());
    assert_eq!(later.whoami(), "ada@example.com admin");
    assert_eq!(ada.whoami(), "ada@example.com admin");
}

/// A sign-in link is claimed before it is used, by a write that names the
/// digest it read. The second of two requests opening the same link finds it
/// changed and signs nobody in — a delete alone would not tell them apart on
/// the SQL adapters, where removing a row that is already gone succeeds.
#[test]
fn a_sign_in_link_is_claimed_by_one_request_only() {
    let dir = served_app();
    let server = serve(dir.path());
    drop(sign_up(&server, "ada@example.com"));
    // Two requests that both read the link before either wrote: each holds
    // the row as it was. The first claim takes it; the second finds the
    // digest changed. (`redeem` would not get that far on a second call —
    // it would not find the row — so the claim itself is what is called.)
    let code = r##"user = User.find_by("email", "ada@example.com")
sent = MagicLink.issue(user, "198.51.100.7")
digest = Crypto.sha256(sent["token"])
one_request = MagicLink.find_by("token_digest", digest)
another = MagicLink.find_by("token_digest", digest)
took = MagicLink.claim(one_request, digest)
lost = MagicLink.claim(another, digest)
held = MagicLink.find_by("_key", one_request._key).token_digest.starts_with?("claimed:")
gone = MagicLink.redeem(sent["token"], sent["nonce"])
fresh = MagicLink.issue(user, "198.51.100.7")
signed = MagicLink.redeem(fresh["token"], fresh["nonce"])
print("#{took} #{lost} #{held} #{gone.nil?} #{signed.email}")"##;
    assert_eq!(
        runner(dir.path(), code),
        "true false true true ada@example.com"
    );
    // And `redeem` goes through it (two calls in a row cannot show that:
    // the second does not find the row).
    let model = fs::read_to_string(dir.path().join("app/models/magic_link.sl")).unwrap();
    assert!(
        model.contains("return nil unless MagicLink.claim(link, digest)"),
        "redeem no longer claims the link before using it"
    );
}

/// Logging out drops the remember-me token: a copy of the cookie restores
/// nothing afterwards.
#[test]
fn logout_ends_remember_me_for_every_copy_of_the_cookie() {
    let dir = served_app();
    let server = serve(dir.path());
    drop(sign_up(&server, "ada@example.com"));
    let mut ada = Client::browser(&server);
    let reply = ada.post(
        "/login",
        json!({"email": "ada@example.com", "password": PASSWORD, "remember_me": "1"}),
    );
    assert_eq!(reply.status, 302, "{}", reply.body);
    let copied = ada.jar.get("remember_token").expect("no cookie").clone();

    // The copy works while the token lives…
    let mut thief = Client::browser(&server);
    thief
        .jar
        .insert("remember_token".to_string(), copied.clone());
    assert_eq!(thief.whoami(), "ada@example.com admin");

    // …the owner logs out, from the browser that was remembered.
    let out = ada.request("DELETE", "/logout", None);
    assert!(out.status == 302 || out.status == 200, "{}", out.body);
    assert_eq!(ada.whoami(), "guest");

    // The copy — the same token, which nothing replaced meanwhile — is dead.
    let mut later = Client::browser(&server);
    later.jar.insert("remember_token".to_string(), copied);
    assert_eq!(
        later.whoami(),
        "guest",
        "a copy of the remember-me cookie outlived the logout"
    );
}
