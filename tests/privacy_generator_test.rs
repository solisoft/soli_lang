//! `soli generate privacy` — export and erasure of what the app holds about
//! a user: what it writes, that it is fmt/lint clean, and a served app
//! (SQLite, migrated) with three models under three rules, where a user
//! downloads their data without its secrets and erases their account.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Value as Json};
use solilang::scaffold::{create_api_tokens, create_auth, create_privacy};

fn make_app_skeleton(root: &Path) {
    for sub in ["app/controllers", "app/models", "app/views", "config"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("config/routes.sl"), "# routes\n").unwrap();
}

#[test]
fn privacy_writes_its_files_and_requires_auth() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_app_skeleton(root);
    let folder = root.to_str().unwrap();
    let err = create_privacy(folder).unwrap_err();
    assert!(err.contains("soli generate auth"), "{err}");

    create_auth(folder).unwrap();
    fs::write(root.join("app/models/user.sl"), "# mine\n").unwrap();
    create_privacy(folder).unwrap();
    fs::write(root.join("app/services/privacy.sl"), "# my register\n").unwrap();
    create_privacy(folder).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("app/models/user.sl")).unwrap(),
        "# mine\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("app/services/privacy.sl")).unwrap(),
        "# my register\n"
    );
    for rel in [
        "app/controllers/privacy_controller.sl",
        "app/views/privacy/show.html.slv",
    ] {
        assert!(root.join(rel).exists(), "{rel} not created");
    }
    let routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    assert_eq!(routes.matches("\"privacy#erase\"").count(), 1, "{routes}");
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
fn cli_generates_privacy_and_generated_code_passes_fmt_lint_and_check() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    assert!(!soli(dir.path(), &["generate", "privacy"]).0);
    assert!(soli(dir.path(), &["generate", "auth"]).0);
    let (ok, output) = soli(dir.path(), &["generate", "privacy"]);
    assert!(ok, "{output}");
    assert!(
        output.contains("PRIVACY_MODELS") && output.contains("/privacy"),
        "{output}"
    );

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
        .env("AUTH_SIGNUP", "open")
        // A test run: the first account needs no AUTH_SETUP_TOKEN.
        .env("APP_ENV", "test")
        .env("AUTH_ATTEMPTS_PER_IP", "1000")
        .env("SOLI_CSRF_TOKENS", "require")
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
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
    disposition: String,
}

/// A person in a browser (a cookie jar), or a script (`bearer`).
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
            disposition: response
                .header("content-disposition")
                .unwrap_or_default()
                .to_string(),
            body: response.into_string().unwrap_or_default(),
        }
    }

    fn get(&mut self, path: &str) -> Reply {
        self.request("GET", path, None)
    }

    fn post(&mut self, path: &str, body: Json) -> Reply {
        self.request("POST", path, Some(body))
    }
}

const PASSWORD: &str = "correct horse battery";

/// The auth scaffold, API tokens and the privacy scaffold, with three models
/// of the app's own under three rules, and a page to count what is left.
fn served_app() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_app_skeleton(root);
    let folder = root.to_str().unwrap();
    create_auth(folder).unwrap();
    create_api_tokens(folder).unwrap();
    create_privacy(folder).unwrap();
    fs::create_dir_all(root.join("app/views/layouts")).unwrap();
    fs::write(
        root.join("app/views/layouts/application.html.slv"),
        "<main><%= yield %></main>\n",
    )
    .unwrap();
    for model in ["Order", "Note", "Invoice"] {
        fs::write(
            root.join(format!("app/models/{}.sl", model.to_lowercase())),
            format!("class {model} < Model\nend\n"),
        )
        .unwrap();
    }
    let register = fs::read_to_string(root.join("app/services/privacy.sl")).unwrap();
    assert!(register.contains("const PRIVACY_MODELS = {}"), "{register}");
    fs::write(
        root.join("app/services/privacy.sl"),
        register.replace(
            "const PRIVACY_MODELS = {}",
            "const PRIVACY_MODELS = {\n  \"Order\": {\"owner\": \"user_id\", \"erase\": \"anonymize\", \"fields\": [\"address\", \"phone\"]},\n  \
             \"Note\": {\"owner\": \"author_id\", \"erase\": \"delete\"},\n  \
             \"Invoice\": {\"owner\": \"user_id\", \"erase\": \"keep\"}\n}",
        ),
    )
    .unwrap();
    fs::write(
        root.join("app/controllers/probe_controller.sl"),
        r##"class ProbeController < Controller
  # POST /probe/data — what the signed-in user owns
  def data
    user = current_user()
    Order.create({"user_id": user._key, "address": params["address"], "phone": "0102", "total": 12})
    Note.create({"author_id": user._key, "body": "a note of #{user.email}"})
    Invoice.create({"user_id": user._key, "number": "F-1"})
    render_json({"ok": true})
  end

  # GET /probe/state — what is in the database
  def state
    render_json({
      "orders": Order.all.map { |order| order.to_h },
      "notes": Note.count,
      "invoices": Invoice.count,
      "tokens": ApiToken.count,
      "users": User.all.map { |user| user.to_h }
    })
  end
end
"##,
    )
    .unwrap();
    let mut routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    routes.push_str(
        "post(\"/probe/data\", \"probe#data\")\nget(\"/probe/state\", \"probe#state\")\n",
    );
    fs::write(root.join("config/routes.sl"), routes).unwrap();
    let (ok, output) = soli(root, &["db:migrate", "up", "."]);
    assert!(ok, "db:migrate up:\n{output}");
    dir
}

/// What clicking the mailed confirmation link does, written straight to the
/// database: open sign-up answers "check your mailbox" and signs nobody in
/// until the address is confirmed.
fn confirm_address(root: &Path, email: &str) {
    let conn = rusqlite::Connection::open(root.join("app.sqlite3")).unwrap();
    let changed = conn
        .execute(
            "UPDATE users SET doc = json_set(doc, '$.confirmed_at', 1) \
             WHERE json_extract(doc, '$.email') = ?1",
            [email],
        )
        .unwrap();
    assert_eq!(changed, 1, "no account for {email}");
}

fn sign_up(server: &Server, root: &Path, email: &str, address: &str) -> Client {
    let mut client = Client::browser(server);
    let reply = client.post("/signup", json!({"email": email, "password": PASSWORD}));
    assert_eq!(reply.status, 200, "{email}: {}", reply.body);
    confirm_address(root, email);
    let reply = client.post("/login", json!({"email": email, "password": PASSWORD}));
    assert_eq!(reply.status, 302, "{email}: {}", reply.body);
    assert_eq!(
        client
            .post("/probe/data", json!({"address": address}))
            .status,
        200
    );
    client
}

#[test]
fn a_user_downloads_their_data_and_erases_their_account() {
    let dir = served_app();
    let server = serve(dir.path());
    let mut ada = sign_up(&server, dir.path(), "ada@example.com", "1 rue Ada");
    let mut grace = sign_up(&server, dir.path(), "grace@example.com", "2 rue Grace");
    let created = ada.post("/api_tokens", json!({"name": "export", "scope_read": "1"}));
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

    // The page is the person's own: not a guest's, not a token's.
    assert_eq!(Client::browser(&server).get("/privacy").location, "/login");
    let mut script = Client::browser(&server);
    script.bearer = Some(token.clone());
    assert_eq!(script.post("/privacy/export", json!({})).status, 403);
    assert_eq!(
        script
            .post(
                "/privacy/erase",
                json!({"email": "ada@example.com", "password": PASSWORD})
            )
            .status,
        403
    );
    let page = ada.get("/privacy");
    assert_eq!(page.status, 200, "{}", page.body);
    assert!(page.body.contains("ada@example.com"), "{}", page.body);

    // The download: the account and what is attached to it, hers only, and
    // no credential.
    let download = ada.post("/privacy/export", json!({}));
    assert_eq!(download.status, 200, "{}", download.body);
    assert!(
        download.disposition.contains("attachment"),
        "{}",
        download.disposition
    );
    let data: Json = serde_json::from_str(&download.body).expect("the export is JSON");
    assert_eq!(data["user"]["email"], json!("ada@example.com"));
    assert_eq!(data["Order"].as_array().unwrap().len(), 1);
    assert_eq!(data["Order"][0]["address"], json!("1 rue Ada"));
    assert_eq!(data["Note"][0]["body"], json!("a note of ada@example.com"));
    assert_eq!(data["Invoice"][0]["number"], json!("F-1"));
    assert_eq!(data["ApiToken"][0]["name"], json!("export"));
    assert!(data["exported_at"].is_string());
    for secret in [
        "password_digest",
        "token_digest",
        "session_version",
        "2 rue Grace",
        "grace@",
    ] {
        assert!(!download.body.contains(secret), "{secret} is in the export");
    }

    // Erasing asks for the address and the password.
    let refused = ada.post(
        "/privacy/erase",
        json!({"email": "grace@example.com", "password": PASSWORD}),
    );
    assert_eq!(refused.status, 422, "{}", refused.body);
    let refused = ada.post(
        "/privacy/erase",
        json!({"email": "ada@example.com", "password": "nope"}),
    );
    assert_eq!(refused.status, 401, "{}", refused.body);
    assert_eq!(
        ada.get("/privacy").status,
        200,
        "a refused erasure signed her out"
    );

    let erased = ada.post(
        "/privacy/erase",
        json!({"email": "Ada@Example.com", "password": PASSWORD}),
    );
    assert_eq!(
        (erased.status, erased.location.as_str()),
        (302, "/"),
        "{}",
        erased.body
    );

    // She is signed out, cannot sign in, and her token is nobody's.
    assert_eq!(ada.get("/privacy").location, "/login");
    let login = Client::browser(&server).post(
        "/login",
        json!({"email": "ada@example.com", "password": PASSWORD}),
    );
    assert_eq!(login.status, 401);
    assert_eq!(script.get("/probe/state").status, 401);

    // Each model by its rule; Grace's data untouched.
    let state: Json = serde_json::from_str(&grace.get("/probe/state").body).unwrap();
    let orders = state["orders"].as_array().unwrap();
    assert_eq!(orders.len(), 2, "{state}");
    let hers = orders
        .iter()
        .find(|o| o["total"] == json!(12) && o.get("address").is_none_or(Json::is_null));
    assert!(hers.is_some(), "the order was not anonymized: {state}");
    assert!(
        orders.iter().any(|o| o["address"] == json!("2 rue Grace")),
        "{state}"
    );
    assert_eq!(
        state["notes"],
        json!(1),
        "her note was not deleted, or Grace's was"
    );
    assert_eq!(state["invoices"], json!(2), "an invoice was not kept");
    assert_eq!(state["tokens"], json!(0));

    // The account is a shell: no address, no name, no credential.
    let users = state["users"].as_array().unwrap();
    assert_eq!(users.len(), 2);
    let shell = users
        .iter()
        .find(|u| u["email"].as_str().unwrap().ends_with("@erased.invalid"))
        .unwrap_or_else(|| panic!("no erased account: {state}"));
    for field in ["password_digest", "name", "remember_token_digest"] {
        assert!(
            shell.get(field).is_none_or(Json::is_null),
            "{field}: {shell}"
        );
    }
    assert!(shell["erased_at"].is_number(), "{shell}");
    assert!(
        !state.to_string().contains("ada@example.com"),
        "her address is still stored: {state}"
    );
    assert!(grace.get("/privacy").status == 200);
}
