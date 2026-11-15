//! `soli generate scim` — a SCIM 2.0 server over the auth scaffold's `User`:
//! what it writes, that it is fmt/lint clean, and a served app (SQLite,
//! migrated) driven with the requests Okta and Microsoft Entra ID send —
//! create, find, patch in both their shapes, replace, delete — and where a
//! deactivated user is signed out and kept out.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Value as Json};
use solilang::scaffold::{create_api_tokens, create_auth, create_scim};

const TOKEN: &str = "okta-token-0123456789abcdef0123456789";
const USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";

fn make_app_skeleton(root: &Path) {
    for sub in ["app/controllers", "app/models", "app/views", "config"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("config/routes.sl"), "# routes\n").unwrap();
}

#[test]
fn scim_writes_its_files_and_requires_auth() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_app_skeleton(root);
    let folder = root.to_str().unwrap();
    let err = create_scim(folder).unwrap_err();
    assert!(err.contains("soli generate auth"), "{err}");

    create_auth(folder).unwrap();
    fs::write(root.join("app/models/user.sl"), "# mine\n").unwrap();
    create_scim(folder).unwrap();
    create_scim(folder).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("app/models/user.sl")).unwrap(),
        "# mine\n"
    );
    for rel in [
        "app/controllers/scim_controller.sl",
        "app/middleware/scim_active.sl",
    ] {
        assert!(root.join(rel).exists(), "{rel} not created");
    }
    let routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    for route in [
        r#"get("/scim/v2/ServiceProviderConfig", "scim#service_provider_config")"#,
        r#"patch("/scim/v2/Users/:id", "scim#update")"#,
        r#"skip_csrf("/scim/*")"#,
    ] {
        assert_eq!(routes.matches(route).count(), 1, "{route}: {routes}");
    }
    let migrations = fs::read_dir(root.join("db/migrations"))
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .contains("add_scim_user_indexes")
        })
        .count();
    assert_eq!(migrations, 1);
    // The gate runs after the session's user (5), two-factor (6), tokens (7).
    let gate = fs::read_to_string(root.join("app/middleware/scim_active.sl")).unwrap();
    assert!(gate.contains("# order: 8"), "{gate}");
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
fn cli_generates_scim_and_generated_code_passes_fmt_lint_and_check() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    assert!(!soli(dir.path(), &["generate", "scim"]).0);
    assert!(soli(dir.path(), &["generate", "auth"]).0);
    let (ok, output) = soli(dir.path(), &["generate", "scim"]);
    assert!(ok, "{output}");
    assert!(
        output.contains("/scim/v2") && output.contains("SCIM_TOKEN"),
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
        .env_remove("SCIM_TOKEN")
        .env("AUTH_SIGNUP", "open")
        .env("AUTH_ATTEMPTS_PER_IP", "1000")
        .env("APP_BASE_URL", "https://app.test")
        // One real directory, and one whose token is too short to count.
        .env("SCIM_TOKENS", format!("okta={TOKEN}, placeholder=changeme"))
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

/// A request from the directory: its bearer token, `application/scim+json`.
fn scim(server: &Server, method: &str, path: &str, body: Option<Json>, token: &str) -> (u16, Json) {
    let mut request = ureq::request(
        method,
        &format!("http://127.0.0.1:{}/scim/v2{path}", server.1),
    );
    if !token.is_empty() {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    let response = match body {
        Some(body) => request
            .set("Content-Type", "application/scim+json")
            .send_string(&body.to_string()),
        None => request.call(),
    };
    let response = match response {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("{method} {path}: {e}"),
    };
    let status = response.status();
    let content_type = response.content_type().to_string();
    let text = response.into_string().unwrap_or_default();
    if status != 204 {
        assert_eq!(
            content_type, "application/scim+json",
            "{method} {path}: {text}"
        );
    }
    (status, serde_json::from_str(&text).unwrap_or(Json::Null))
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

    fn request(&mut self, method: &str, path: &str, body: Option<Json>) -> (u16, String) {
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
        (
            response.status(),
            response.into_string().unwrap_or_default(),
        )
    }

    fn whoami(&mut self) -> String {
        self.request("GET", "/whoami", None).1
    }
}

fn served_app() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_app_skeleton(root);
    let folder = root.to_str().unwrap();
    create_auth(folder).unwrap();
    create_api_tokens(folder).unwrap();
    create_scim(folder).unwrap();
    fs::create_dir_all(root.join("app/views/layouts")).unwrap();
    fs::write(
        root.join("app/views/layouts/application.html.slv"),
        "<main><%= yield %></main>\n",
    )
    .unwrap();
    fs::write(
        root.join("app/controllers/whoami_controller.sl"),
        "class WhoamiController < Controller\n  # GET /whoami\n  def index\n    user = current_user()\n    \
         render_text(user.nil? ? \"guest\" : \"#{user.email} #{user.role}\")\n  end\nend\n",
    )
    .unwrap();
    let mut routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    routes.push_str("get(\"/whoami\", \"whoami#index\")\n");
    fs::write(root.join("config/routes.sl"), routes).unwrap();
    let (ok, output) = soli(root, &["db:migrate", "up", "."]);
    assert!(ok, "db:migrate up:\n{output}");
    dir
}

#[test]
fn a_directory_provisions_users_the_way_okta_and_entra_do() {
    let dir = served_app();
    let server = serve(dir.path());

    // No token, a wrong one, and one too short to be configured: 401.
    for token in ["", "nope", "changeme"] {
        let (status, error) = scim(&server, "GET", "/Users", None, token);
        assert_eq!(status, 401, "{token:?}");
        assert_eq!(error["status"], json!("401"));
    }

    let (status, config) = scim(&server, "GET", "/ServiceProviderConfig", None, TOKEN);
    assert_eq!(status, 200);
    assert_eq!(config["patch"]["supported"], json!(true));
    assert_eq!(config["bulk"]["supported"], json!(false));
    let (_, types) = scim(&server, "GET", "/ResourceTypes", None, TOKEN);
    assert_eq!(types["Resources"][0]["endpoint"], json!("/Users"));
    let (_, schemas) = scim(&server, "GET", "/Schemas", None, TOKEN);
    assert_eq!(schemas["Resources"][0]["id"], json!(USER_SCHEMA));

    // Before creating, a directory asks whether the user exists.
    let find = |name: &str| {
        scim(
            &server,
            "GET",
            &format!(
                "/Users?filter={}",
                urlencoding::encode(&format!("userName eq \"{name}\""))
            ),
            None,
            TOKEN,
        )
        .1
    };
    assert_eq!(find("ada@example.com")["totalResults"], json!(0));

    let (status, created) = scim(
        &server,
        "POST",
        "/Users",
        Some(json!({
            "schemas": [USER_SCHEMA], "userName": "Ada@Example.com", "externalId": "00u1",
            "name": {"givenName": "Ada", "familyName": "Lovelace"}, "displayName": "Ada L",
            "emails": [{"value": "Ada@Example.com", "primary": true}], "active": true,
        })),
        TOKEN,
    );
    assert_eq!(status, 201, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["userName"], json!("ada@example.com"));
    assert_eq!(created["externalId"], json!("00u1"));
    assert_eq!(created["name"]["familyName"], json!("Lovelace"));
    assert_eq!(created["active"], json!(true));
    assert_eq!(
        created["meta"]["location"],
        json!(format!("https://app.test/scim/v2/Users/{id}"))
    );

    // The same user again is a conflict, in SCIM's words.
    let (status, error) = scim(
        &server,
        "POST",
        "/Users",
        Some(json!({"userName": "ada@example.com"})),
        TOKEN,
    );
    assert_eq!(
        (status, error["scimType"].clone()),
        (409, json!("uniqueness"))
    );
    let (status, error) = scim(
        &server,
        "POST",
        "/Users",
        Some(json!({"displayName": "No name"})),
        TOKEN,
    );
    assert_eq!(
        (status, error["scimType"].clone()),
        (400, json!("invalidValue"))
    );

    // Found by userName, whatever its case, and by externalId; read by id.
    assert_eq!(find("ADA@example.com")["Resources"][0]["id"], json!(id));
    let (_, by_external) = scim(
        &server,
        "GET",
        &format!(
            "/Users?filter={}",
            urlencoding::encode("externalId eq \"00u1\"")
        ),
        None,
        TOKEN,
    );
    assert_eq!(by_external["totalResults"], json!(1));
    let (status, error) = scim(
        &server,
        "GET",
        &format!("/Users?filter={}", urlencoding::encode("title co \"x\"")),
        None,
        TOKEN,
    );
    assert_eq!(
        (status, error["scimType"].clone()),
        (400, json!("invalidFilter"))
    );
    assert_eq!(
        scim(&server, "GET", &format!("/Users/{id}"), None, TOKEN).1["id"],
        json!(id)
    );
    assert_eq!(scim(&server, "GET", "/Users/missing", None, TOKEN).0, 404);

    // Pages.
    for n in 0..3 {
        scim(
            &server,
            "POST",
            "/Users",
            Some(json!({"userName": format!("user{n}@example.com")})),
            TOKEN,
        );
    }
    let (_, page) = scim(&server, "GET", "/Users?startIndex=2&count=2", None, TOKEN);
    assert_eq!(page["totalResults"], json!(4));
    assert_eq!(
        (page["startIndex"].clone(), page["itemsPerPage"].clone()),
        (json!(2), json!(2))
    );
    assert_eq!(page["Resources"][0]["userName"], json!("user0@example.com"));

    // Okta deactivates with a path-less replace; Entra with a path and the
    // string "False", capitalised like its operation.
    let patch = |operations: Json| {
        scim(
            &server,
            "PATCH",
            &format!("/Users/{id}"),
            Some(json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": operations})),
            TOKEN,
        )
        .1
    };
    assert_eq!(
        patch(json!([{"op": "replace", "value": {"active": false}}]))["active"],
        json!(false)
    );
    assert_eq!(
        patch(json!([{"op": "Replace", "path": "active", "value": "True"}]))["active"],
        json!(true)
    );
    assert_eq!(
        patch(json!([{"op": "Replace", "path": "active", "value": "False"}]))["active"],
        json!(false)
    );
    let renamed = patch(json!([
        {"op": "Replace", "path": "emails[type eq \"work\"].value", "value": "Ada.L@example.com"},
        {"op": "Replace", "path": "name.familyName", "value": "King"},
        {"op": "Add", "path": "displayName", "value": "Ada King"},
        {"op": "Replace", "path": "active", "value": true},
    ]));
    assert_eq!(renamed["userName"], json!("ada.l@example.com"));
    assert_eq!(renamed["name"]["familyName"], json!("King"));
    assert_eq!(renamed["displayName"], json!("Ada King"));
    assert_eq!(renamed["active"], json!(true));

    // A replace takes the resource as sent; a userName someone has is refused.
    let (status, replaced) = scim(
        &server,
        "PUT",
        &format!("/Users/{id}"),
        Some(json!({"userName": "ada@example.com", "name": {"givenName": "A."}, "active": false})),
        TOKEN,
    );
    assert_eq!(status, 200, "{replaced}");
    assert_eq!(replaced["name"]["givenName"], json!("A."));
    assert_eq!(replaced["active"], json!(false));
    let (status, _) = scim(
        &server,
        "PUT",
        &format!("/Users/{id}"),
        Some(json!({"userName": "user1@example.com"})),
        TOKEN,
    );
    assert_eq!(status, 409);

    // Removed, the user is gone.
    assert_eq!(
        scim(&server, "DELETE", &format!("/Users/{id}"), None, TOKEN).0,
        204
    );
    assert_eq!(
        scim(&server, "GET", &format!("/Users/{id}"), None, TOKEN).0,
        404
    );
    assert_eq!(
        scim(&server, "DELETE", &format!("/Users/{id}"), None, TOKEN).0,
        404
    );
}

#[test]
fn a_deactivated_user_is_signed_out_and_kept_out() {
    let dir = served_app();
    let server = serve(dir.path());

    // Grace has an account of her own, a session, and an API token.
    let mut grace = Client::browser(&server);
    let password = "correct horse battery";
    let signup = json!({"email": "grace@example.com", "password": password});
    assert_eq!(grace.request("POST", "/signup", Some(signup)).0, 302);
    assert_eq!(grace.whoami(), "grace@example.com member");
    let (status, page) = grace.request(
        "POST",
        "/api_tokens",
        Some(json!({"name": "export", "scope_read": "1"})),
    );
    assert_eq!(status, 201, "{page}");
    let start = page.find("data-new-token>").expect("no token shown") + 15;
    let token = page[start..].split('<').next().unwrap().trim().to_string();
    let mut script = Client::browser(&server);
    script.bearer = Some(token);
    assert_eq!(script.whoami(), "grace@example.com member");

    // The directory finds her and deactivates her.
    let (_, found) = scim(
        &server,
        "GET",
        &format!(
            "/Users?filter={}",
            urlencoding::encode("userName eq \"grace@example.com\"")
        ),
        None,
        TOKEN,
    );
    let id = found["Resources"][0]["id"]
        .as_str()
        .expect("grace not found")
        .to_string();
    let deactivate = json!({"Operations": [{"op": "replace", "value": {"active": false}}]});
    let (status, user) = scim(
        &server,
        "PATCH",
        &format!("/Users/{id}"),
        Some(deactivate),
        TOKEN,
    );
    assert_eq!((status, user["active"].clone()), (200, json!(false)));

    // Her session is closed, her token is nobody's, and signing in again —
    // the password still verifies — opens nothing.
    assert_eq!(grace.whoami(), "guest");
    assert_eq!(script.whoami(), "guest");
    let mut again = Client::browser(&server);
    let login = json!({"email": "grace@example.com", "password": password});
    again.request("POST", "/login", Some(login.clone()));
    assert_eq!(again.whoami(), "guest");

    // Reactivated, she signs in; the session closed meanwhile stays closed.
    let reactivate = json!({"Operations": [{"op": "Replace", "path": "active", "value": "True"}]});
    scim(
        &server,
        "PATCH",
        &format!("/Users/{id}"),
        Some(reactivate),
        TOKEN,
    );
    assert_eq!(grace.whoami(), "guest");
    assert_eq!(script.whoami(), "grace@example.com member");
    let mut back = Client::browser(&server);
    assert_eq!(back.request("POST", "/login", Some(login)).0, 302);
    assert_eq!(back.whoami(), "grace@example.com member");

    // The directory's token is not a way into the app, and a personal token
    // is not a way into SCIM.
    let mut directory = Client::browser(&server);
    directory.bearer = Some(TOKEN.to_string());
    assert_eq!(directory.whoami(), "guest");
    let personal = script.bearer.clone().unwrap();
    assert_eq!(scim(&server, "GET", "/Users", None, &personal).0, 401);
}
