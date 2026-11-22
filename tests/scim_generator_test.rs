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
        "app/controllers/scim_groups_controller.sl",
        "app/models/scim_group.sl",
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
    serve_with(root, &[])
}

/// `serve`, with more of the environment set (or a variable overridden).
fn serve_with(root: &Path, extra: &[(&str, &str)]) -> Server {
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
    for (name, value) in extra {
        command.env(name, value);
    }
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
    // What two of the directory's groups mean here — and one mapping the
    // app must not honour.
    let model = fs::read_to_string(root.join("app/models/scim_group.sl")).unwrap();
    assert!(model.contains("const SCIM_GROUP_ROLES = {}"), "{model}");
    fs::write(
        root.join("app/models/scim_group.sl"),
        model.replace(
            "const SCIM_GROUP_ROLES = {}",
            "const SCIM_GROUP_ROLES = {\"Finance\": \"accountant\", \"Support\": \"agent\", \"Owners\": \"admin\"}",
        ),
    )
    .unwrap();
    fs::write(
        root.join("app/controllers/roles_controller.sl"),
        "class RolesController < Controller\n  # GET /roles — every account's role and groups\n  def index\n    \
         rows = {}\n    User.all.each do |user|\n      rows[user.email] = [user.role] + ScimGroup.names_for(user).sort\n    end\n    \
         render_json(rows)\n  end\nend\n",
    )
    .unwrap();
    let mut routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    routes.push_str("get(\"/roles\", \"roles#index\")\n");
    // A fixture, not part of the scaffold: marks an account as one the `okta`
    // directory provisioned, which is what a directory account that later set
    // a password looks like — and reads an account's address and password
    // state back.
    fs::write(
        root.join("app/controllers/fixture_controller.sl"),
        "class FixtureController < Controller\n  # GET /fixture/provisioned?email=\n  def provisioned\n    \
         user = User.find_by(\"email\", params[\"email\"].to_s)\n    user.scim_provisioned_at = DateTime.utc.to_unix\n    \
         user.scim_directory = \"okta\"\n    user.save\n    render_text(\"ok\")\n  end\n\n  \
         # GET /fixture/account?email=\n  def account\n    user = User.find_by(\"email\", params[\"email\"].to_s)\n    \
         return render_text(\"none\") if user.nil?\n\n    \
         render_text(\"#{user.email} password=#{user.password_digest.present?} active=#{user.deactivated_at.nil?} \
         directory=#{user.scim_directory.to_s} key=#{user._key}\")\n  end\nend\n",
    )
    .unwrap();
    routes.push_str("get(\"/fixture/provisioned\", \"fixture#provisioned\")\n");
    routes.push_str("get(\"/fixture/account\", \"fixture#account\")\n");
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

    // She is a directory account (one it provisioned, that later set a
    // password): the directory finds her and deactivates her.
    assert_eq!(
        grace
            .request("GET", "/fixture/provisioned?email=grace@example.com", None)
            .1,
        "ok"
    );
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

const GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";

/// Every account's role followed by its groups, by email.
fn roles(server: &Server) -> Json {
    let text = ureq::get(&format!("http://127.0.0.1:{}/roles", server.1))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn groups_give_roles_and_take_them_back_but_never_admin() {
    let dir = served_app();
    let server = serve(dir.path());
    let user = |name: &str| {
        let body = json!({"userName": format!("{name}@example.com")});
        let (status, created) = scim(&server, "POST", "/Users", Some(body), TOKEN);
        assert_eq!(status, 201, "{created}");
        created["id"].as_str().unwrap().to_string()
    };
    let (ada, grace, linus) = (user("ada"), user("grace"), user("linus"));

    // The server says it has groups.
    let (_, types) = scim(&server, "GET", "/ResourceTypes", None, TOKEN);
    assert_eq!(types["totalResults"], json!(2));
    assert_eq!(types["Resources"][1]["endpoint"], json!("/Groups"));
    let (_, schemas) = scim(&server, "GET", "/Schemas", None, TOKEN);
    assert_eq!(schemas["Resources"][1]["id"], json!(GROUP_SCHEMA));
    assert_eq!(scim(&server, "GET", "/Groups", None, "").0, 401);

    // A group created with a member; found by name; a duplicate refused.
    let (status, finance) = scim(
        &server,
        "POST",
        "/Groups",
        Some(
            json!({"schemas": [GROUP_SCHEMA], "displayName": "Finance", "externalId": "g-1",
                    "members": [{"value": ada}]}),
        ),
        TOKEN,
    );
    assert_eq!(status, 201, "{finance}");
    let finance_id = finance["id"].as_str().unwrap().to_string();
    assert_eq!(finance["members"][0]["value"], json!(ada));
    assert_eq!(finance["members"][0]["display"], json!("ada@example.com"));
    let (status, error) = scim(
        &server,
        "POST",
        "/Groups",
        Some(json!({"displayName": "Finance"})),
        TOKEN,
    );
    assert_eq!(
        (status, error["scimType"].clone()),
        (409, json!("uniqueness"))
    );
    let (_, found) = scim(
        &server,
        "GET",
        &format!(
            "/Groups?filter={}",
            urlencoding::encode("displayName eq \"Finance\"")
        ),
        None,
        TOKEN,
    );
    assert_eq!(found["Resources"][0]["id"], json!(finance_id));
    assert_eq!(
        roles(&server)["ada@example.com"],
        json!(["accountant", "Finance"])
    );
    assert_eq!(roles(&server)["grace@example.com"], json!(["member"]));

    // The user resource shows its groups.
    let (_, shown) = scim(&server, "GET", &format!("/Users/{ada}"), None, TOKEN);
    assert_eq!(shown["groups"][0]["display"], json!("Finance"));

    // Members in and out, in the shapes directories send.
    let patch = |id: &str, operations: Json| {
        let (status, group) = scim(
            &server,
            "PATCH",
            &format!("/Groups/{id}"),
            Some(
                json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": operations}),
            ),
            TOKEN,
        );
        assert_eq!(status, 200, "{group}");
        let mut members: Vec<String> = group["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                m["display"]
                    .as_str()
                    .unwrap()
                    .split('@')
                    .next()
                    .unwrap()
                    .to_string()
            })
            .collect();
        members.sort();
        members.join(",")
    };
    // Okta adds; adding twice changes nothing; an unknown user is skipped.
    let added = patch(
        &finance_id,
        json!([{"op": "add", "path": "members", "value": [{"value": grace}, {"value": ada}, {"value": "nobody"}]}]),
    );
    assert_eq!(added, "ada,grace");
    assert_eq!(
        roles(&server)["grace@example.com"],
        json!(["accountant", "Finance"])
    );
    // Entra removes one by a filtered path; Okta by value.
    let removed = patch(
        &finance_id,
        json!([{"op": "Remove", "path": format!("members[value eq \"{ada}\"]")}]),
    );
    assert_eq!(removed, "grace");
    assert_eq!(
        roles(&server)["ada@example.com"],
        json!(["member"]),
        "the role stayed"
    );
    let removed = patch(
        &finance_id,
        json!([{"op": "remove", "path": "members", "value": [{"value": grace}]}]),
    );
    assert_eq!(removed, "");
    assert_eq!(roles(&server)["grace@example.com"], json!(["member"]));
    // Replace sets the whole list.
    let replaced = patch(
        &finance_id,
        json!([{"op": "replace", "path": "members", "value": [{"value": ada}, {"value": linus}]}]),
    );
    assert_eq!(replaced, "ada,linus");

    // A group mapped to `admin` makes nobody an administrator.
    let (_, owners) = scim(
        &server,
        "POST",
        "/Groups",
        Some(json!({"displayName": "Owners", "members": [{"value": grace}]})),
        TOKEN,
    );
    assert_eq!(
        roles(&server)["grace@example.com"],
        json!(["member", "Owners"])
    );

    // An administrator made in the app keeps the role whatever the groups say.
    fs::write(
        dir.path().join("promote.sl"),
        "user = User.find_by(\"email\", \"linus@example.com\")\nuser.role = \"admin\"\nuser.save\n",
    )
    .unwrap();
    drop(server);
    let (ok, output) = soli(dir.path(), &["db:seed", ".", "promote.sl"]);
    assert!(ok, "{output}");
    let server = serve(dir.path());
    let (_, support) = scim(
        &server,
        "POST",
        "/Groups",
        Some(json!({"displayName": "Support", "members": [{"value": linus}, {"value": ada}]})),
        TOKEN,
    );
    let support_id = support["id"].as_str().unwrap().to_string();
    let all = roles(&server);
    // An administrator is out of the directory's reach: he keeps the group
    // he was in before, and is not put in a new one.
    assert_eq!(all["linus@example.com"], json!(["admin", "Finance"]));
    // Ada is in two mapped groups: the later one in the mapping wins.
    assert_eq!(
        all["ada@example.com"],
        json!(["agent", "Finance", "Support"])
    );

    // Renaming a group changes what it gives (Okta: no path).
    let (status, renamed) = scim(
        &server,
        "PATCH",
        &format!("/Groups/{support_id}"),
        Some(json!({"Operations": [{"op": "replace", "value": {"displayName": "Helpdesk"}}]})),
        TOKEN,
    );
    assert_eq!(
        (status, renamed["displayName"].clone()),
        (200, json!("Helpdesk"))
    );
    assert_eq!(
        roles(&server)["ada@example.com"],
        json!(["accountant", "Finance", "Helpdesk"])
    );

    // A PUT replaces the group, members included.
    let (status, put) = scim(
        &server,
        "PUT",
        &format!("/Groups/{finance_id}"),
        Some(json!({"displayName": "Finance", "members": [{"value": grace}]})),
        TOKEN,
    );
    assert_eq!(status, 200, "{put}");
    let all = roles(&server);
    assert_eq!(all["ada@example.com"], json!(["member", "Helpdesk"]));
    assert_eq!(
        all["grace@example.com"],
        json!(["accountant", "Finance", "Owners"])
    );

    // A deleted group takes what it gave with it, at once.
    assert_eq!(
        scim(
            &server,
            "DELETE",
            &format!("/Groups/{finance_id}"),
            None,
            TOKEN
        )
        .0,
        204
    );
    assert_eq!(
        scim(
            &server,
            "GET",
            &format!("/Groups/{finance_id}"),
            None,
            TOKEN
        )
        .0,
        404
    );
    let all = roles(&server);
    assert_eq!(all["grace@example.com"], json!(["member", "Owners"]));
    assert_eq!(all["linus@example.com"], json!(["admin"]));

    // A deleted user leaves its groups.
    let owners_id = owners["id"].as_str().unwrap();
    assert_eq!(
        scim(&server, "DELETE", &format!("/Users/{grace}"), None, TOKEN).0,
        204
    );
    let (_, owners) = scim(&server, "GET", &format!("/Groups/{owners_id}"), None, TOKEN);
    assert_eq!(owners["members"], json!([]));
    let (_, page) = scim(&server, "GET", "/Groups?startIndex=1&count=1", None, TOKEN);
    assert_eq!(
        (page["totalResults"].clone(), page["itemsPerPage"].clone()),
        (json!(2), json!(1))
    );
}

const OTHER_TOKEN: &str = "entra-token-0123456789abcdef0123456789";

/// What `/fixture/account` says of an account, and its key.
fn account(server: &Server, email: &str) -> (String, String) {
    let text = ureq::get(&format!(
        "http://127.0.0.1:{}/fixture/account?email={email}",
        server.1
    ))
    .call()
    .unwrap()
    .into_string()
    .unwrap();
    let key = text
        .rsplit("key=")
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    let state = text.split(" key=").next().unwrap_or_default().to_string();
    (state, key)
}

fn create_user(server: &Server, email: &str, token: &str) -> String {
    let (status, user) = scim(
        server,
        "POST",
        "/Users",
        Some(json!({"schemas": [USER_SCHEMA], "userName": email, "active": true})),
        token,
    );
    assert_eq!(status, 201, "{user}");
    user["id"].as_str().unwrap().to_string()
}

fn find_count(server: &Server, email: &str, token: &str) -> Json {
    scim(
        server,
        "GET",
        &format!(
            "/Users?filter={}",
            urlencoding::encode(&format!("userName eq \"{email}\""))
        ),
        None,
        token,
    )
    .1["totalResults"]
        .clone()
}

/// A directory reaches the accounts it provisioned and no other: an
/// administrator's address cannot be moved (then reset), a local account
/// cannot be deactivated, deleted or given a group's role.
#[test]
fn a_directory_cannot_touch_an_admin_or_an_account_it_did_not_provision() {
    let dir = served_app();
    let server = serve(dir.path());
    for email in ["boss@example.com", "local@example.com"] {
        let mut visitor = Client::browser(&server);
        let signup = json!({"email": email, "password": "correct horse battery"});
        assert_eq!(visitor.request("POST", "/signup", Some(signup)).0, 302);
    }
    fs::write(
        dir.path().join("promote.sl"),
        "user = User.find_by(\"email\", \"boss@example.com\")\nuser.role = \"admin\"\n\
         user.scim_provisioned_at = 1\nuser.save\n",
    )
    .unwrap();
    drop(server);
    let (ok, output) = soli(dir.path(), &["db:seed", ".", "promote.sl"]);
    assert!(ok, "{output}");
    let server = serve(dir.path());

    let ada = create_user(&server, "ada@example.com", TOKEN);
    let (_, boss) = account(&server, "boss@example.com");
    let (_, local) = account(&server, "local@example.com");

    // Not found by name, not listed, and 404 by id — read or write.
    assert_eq!(find_count(&server, "boss@example.com", TOKEN), json!(0));
    assert_eq!(find_count(&server, "local@example.com", TOKEN), json!(0));
    assert_eq!(find_count(&server, "ada@example.com", TOKEN), json!(1));
    let (_, list) = scim(&server, "GET", "/Users", None, TOKEN);
    assert_eq!(list["totalResults"], json!(1), "{list}");
    let move_address = json!({"Operations": [
        {"op": "replace", "path": "userName", "value": "attacker@evil.test"},
        {"op": "replace", "path": "active", "value": false}
    ]});
    for target in [&boss, &local] {
        let path = format!("/Users/{target}");
        assert_eq!(scim(&server, "GET", &path, None, TOKEN).0, 404);
        assert_eq!(
            scim(&server, "PATCH", &path, Some(move_address.clone()), TOKEN).0,
            404
        );
        let put = json!({"userName": "attacker@evil.test", "active": false});
        assert_eq!(scim(&server, "PUT", &path, Some(put), TOKEN).0, 404);
        assert_eq!(scim(&server, "DELETE", &path, None, TOKEN).0, 404);
    }
    assert_eq!(
        account(&server, "boss@example.com").0,
        "boss@example.com password=true active=true directory="
    );
    assert_eq!(
        account(&server, "local@example.com").0,
        "local@example.com password=true active=true directory="
    );
    assert_eq!(account(&server, "attacker@evil.test").0, "none");

    // A group's role goes to the directory's own account only.
    scim(
        &server,
        "POST",
        "/Groups",
        Some(json!({"displayName": "Finance", "members": [
            {"value": boss}, {"value": local}, {"value": ada}
        ]})),
        TOKEN,
    );
    let all = roles(&server);
    assert_eq!(all["boss@example.com"], json!(["admin"]));
    assert_eq!(all["local@example.com"], json!(["member"]));
    assert_eq!(all["ada@example.com"], json!(["accountant", "Finance"]));

    // A patch that cannot be saved says so: the deactivation riding with a
    // userName that is another account's is not reported as done.
    create_user(&server, "linus@example.com", TOKEN);
    let collide = json!({"Operations": [
        {"op": "replace", "path": "active", "value": false},
        {"op": "replace", "path": "userName", "value": "linus@example.com"}
    ]});
    let (status, error) = scim(
        &server,
        "PATCH",
        &format!("/Users/{ada}"),
        Some(collide),
        TOKEN,
    );
    assert_eq!(status, 409, "{error}");
    assert_eq!(
        account(&server, "ada@example.com").0,
        "ada@example.com password=false active=true directory=okta"
    );

    // The attribute's full name is the attribute.
    let qualified = json!({"Operations": [{
        "op": "replace",
        "path": "urn:ietf:params:scim:schemas:core:2.0:User:active",
        "value": false
    }]});
    let (status, user) = scim(
        &server,
        "PATCH",
        &format!("/Users/{ada}"),
        Some(qualified),
        TOKEN,
    );
    assert_eq!((status, user["active"].clone()), (200, json!(false)));
}

/// `SCIM_ADOPT_EXISTING=on`: an existing account is found and managed, and
/// what it could sign in with is revoked — never an administrator.
#[test]
fn adopting_an_existing_account_revokes_its_credentials() {
    let dir = served_app();
    let server = serve_with(dir.path(), &[("SCIM_ADOPT_EXISTING", "on")]);
    let mut squatter = Client::browser(&server);
    let signup = json!({"email": "newhire@example.com", "password": "correct horse battery"});
    assert_eq!(
        squatter.request("POST", "/signup", Some(signup.clone())).0,
        302
    );
    assert_eq!(squatter.whoami(), "newhire@example.com member");

    assert_eq!(find_count(&server, "newhire@example.com", TOKEN), json!(1));
    let (_, key) = account(&server, "newhire@example.com");
    let adopt = json!({"Operations": [{"op": "replace", "path": "externalId", "value": "00u1"}]});
    let (status, user) = scim(
        &server,
        "PATCH",
        &format!("/Users/{key}"),
        Some(adopt),
        TOKEN,
    );
    assert_eq!(status, 200, "{user}");

    // The session is closed and the password is gone.
    assert_eq!(squatter.whoami(), "guest");
    assert_eq!(
        account(&server, "newhire@example.com").0,
        "newhire@example.com password=false active=true directory=okta"
    );
    let mut again = Client::browser(&server);
    again.request("POST", "/login", Some(signup));
    assert_eq!(again.whoami(), "guest");
}

/// With several directories, each sees and writes its own accounts.
#[test]
fn a_directory_cannot_reach_another_directory_s_accounts() {
    let dir = served_app();
    let tokens = format!("okta={TOKEN},entra={OTHER_TOKEN}");
    let server = serve_with(dir.path(), &[("SCIM_TOKENS", &tokens)]);
    let ada = create_user(&server, "ada@example.com", TOKEN);
    create_user(&server, "grace@example.com", OTHER_TOKEN);

    assert_eq!(
        find_count(&server, "ada@example.com", OTHER_TOKEN),
        json!(0)
    );
    assert_eq!(
        find_count(&server, "grace@example.com", OTHER_TOKEN),
        json!(1)
    );
    let (_, list) = scim(&server, "GET", "/Users", None, OTHER_TOKEN);
    assert_eq!(list["totalResults"], json!(1), "{list}");
    let off = json!({"Operations": [{"op": "replace", "path": "active", "value": false}]});
    let path = format!("/Users/{ada}");
    assert_eq!(
        scim(&server, "PATCH", &path, Some(off.clone()), OTHER_TOKEN).0,
        404
    );
    assert_eq!(scim(&server, "DELETE", &path, None, OTHER_TOKEN).0, 404);
    assert_eq!(scim(&server, "PATCH", &path, Some(off), TOKEN).0, 200);
    assert_eq!(
        account(&server, "ada@example.com").0,
        "ada@example.com password=false active=false directory=okta"
    );
}

/// A token that ends with `=` (base64 padding) is one token, a value that is
/// not a string is not stored, and an address that keeps guessing is refused
/// even once it guesses right.
#[test]
fn padded_tokens_work_odd_values_are_not_stored_and_guessing_is_throttled() {
    let dir = served_app();
    let padded = "b2t0YS10b2tlbi0wMTIzNDU2Nzg5YWJjZGVmMDEyMw==";
    let tokens = format!("okta={padded}");
    let server = serve_with(dir.path(), &[("SCIM_TOKENS", &tokens)]);
    let ada = create_user(&server, "ada@example.com", padded);
    let path = format!("/Users/{ada}");

    let odd = json!({"Operations": [
        {"op": "replace", "path": "userName", "value": {"evil": true}},
        {"op": "replace", "path": "displayName", "value": ["a", "b"]}
    ]});
    let (status, body) = scim(&server, "PATCH", &path, Some(odd), padded);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["userName"], json!("ada@example.com"), "{body}");
    assert!(body["displayName"].as_str().is_none(), "{body}");
    let odd = json!({"userName": ["x@example.com"], "name": "not a hash"});
    let (status, body) = scim(&server, "PUT", &path, Some(odd), padded);
    assert_eq!(status, 400, "{body}");
    assert!(
        account(&server, "ada@example.com")
            .0
            .starts_with("ada@example.com "),
        "the refused PUT moved the address"
    );

    // A number where a string goes changes nothing — the id is not erased.
    let keep = json!({"Operations": [{"op": "replace", "path": "externalId", "value": "ext-1"}]});
    assert_eq!(scim(&server, "PATCH", &path, Some(keep), padded).0, 200);
    let odd = json!({"Operations": [
        {"op": "replace", "path": "externalId", "value": 12345},
        {"op": "replace", "path": "emails[type eq \"home\"].value", "value": "home@example.com"},
        {"op": "replace", "value": "not a hash"},
        "not an operation"
    ]});
    let (status, body) = scim(&server, "PATCH", &path, Some(odd), padded);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["externalId"], json!("ext-1"), "{body}");
    assert_eq!(body["userName"], json!("ada@example.com"), "{body}");
    let not_a_list = json!({"Operations": "x"});
    assert_eq!(
        scim(&server, "PATCH", &path, Some(not_a_list), padded).0,
        400
    );
    // The work address is the login address.
    let work = json!({"Operations": [
        {"op": "replace", "path": "emails[type eq \"work\"].value", "value": "Ada.L@Example.com"}
    ]});
    let (status, body) = scim(&server, "PATCH", &path, Some(work), padded);
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["userName"], json!("ada.l@example.com"), "{body}");

    // Wrong tokens are slowed, the right one never: nobody can switch a
    // directory's provisioning off by guessing from its address.
    for _ in 0..20 {
        assert_eq!(scim(&server, "GET", "/Users", None, "a-wrong-token").0, 401);
    }
    assert_eq!(scim(&server, "GET", "/Users", None, "a-wrong-token").0, 429);
    assert_eq!(scim(&server, "GET", "/Users", None, padded).0, 200);
}
