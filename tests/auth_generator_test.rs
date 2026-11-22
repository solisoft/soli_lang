//! `soli generate auth` integration tests — exercise the auth scaffold
//! generator in a tempdir so `scaffold/auth_generator.rs` and the embedded
//! templates get covered without polluting the workspace.

use std::fs;
use std::path::Path;

use solilang::scaffold::create_auth;

fn make_app_skeleton(root: &Path) {
    for sub in ["app/controllers", "app/models", "app/views", "config"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::write(root.join("config/routes.sl"), "# routes\n").unwrap();
}

#[test]
fn create_auth_writes_all_files() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    let result = create_auth(dir.path().to_str().unwrap());
    assert!(result.is_ok(), "generate auth failed: {:?}", result.err());

    for rel in [
        "app/models/user.sl",
        "app/policies/application_policy.sl",
        "app/policies/user_policy.sl",
        "app/helpers/auth_helper.sl",
        "app/middleware/current_user.sl",
        "app/controllers/sessions_controller.sl",
        "app/controllers/registrations_controller.sl",
        "app/controllers/passwords_controller.sl",
        "app/controllers/confirmations_controller.sl",
        "app/mailers/auth_mailer.sl",
        "app/views/sessions/new.html.slv",
        "app/views/registrations/new.html.slv",
        "app/views/passwords/new.html.slv",
        "app/views/passwords/edit.html.slv",
        "app/views/confirmations/new.html.slv",
        "app/views/auth_mailer/reset_password.html.slv",
        "app/views/auth_mailer/confirm_email.html.slv",
    ] {
        assert!(dir.path().join(rel).exists(), "{rel} not created");
    }

    // Timestamped migrations are generated: users + token indexes.
    let migration_names: Vec<String> = fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(
        migration_names.iter().any(|n| n.contains("create_users")),
        "no create_users migration generated"
    );
    assert!(
        migration_names
            .iter()
            .any(|n| n.contains("add_auth_token_indexes")),
        "no add_auth_token_indexes migration generated"
    );

    // The User model ships the Devise-style flow methods.
    let user_model = fs::read_to_string(dir.path().join("app/models/user.sl")).unwrap();
    for needle in [
        "start_password_reset",
        "start_email_confirmation",
        "start_remember_me",
        "register_failed_attempt",
        "AUTH_MAX_FAILED_ATTEMPTS",
        "def revoke_credentials",
        "def sso_only?",
    ] {
        assert!(user_model.contains(needle), "user model missing {needle}");
    }
}

#[test]
fn create_auth_appends_routes() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    create_auth(dir.path().to_str().unwrap()).expect("generate auth ok");

    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).expect("read routes");
    assert!(routes.contains("\"sessions#new\""), "login route missing");
    assert!(
        routes.contains("\"registrations#create\""),
        "signup route missing"
    );
    assert!(
        routes.contains("\"passwords#create\""),
        "password reset route missing"
    );
    assert!(
        routes.contains("\"confirmations#show\""),
        "email confirmation route missing"
    );
}

#[test]
fn create_auth_backfills_flow_routes_on_older_apps() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    // Simulate an app generated before the Devise-style flows: base auth
    // routes present, flow routes absent.
    fs::write(
        dir.path().join("config/routes.sl"),
        "# routes\nget(\"/login\", \"sessions#new\", name: \"login\")\n",
    )
    .unwrap();

    create_auth(dir.path().to_str().unwrap()).expect("generate auth ok");

    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    assert_eq!(
        routes.matches("\"sessions#new\"").count(),
        1,
        "base auth routes duplicated"
    );
    assert!(
        routes.contains("\"passwords#new\""),
        "flow routes not backfilled"
    );
}

#[test]
fn create_auth_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());

    create_auth(dir.path().to_str().unwrap()).expect("first run ok");
    // Customize a file; a second run must not clobber it.
    let user_model = dir.path().join("app/models/user.sl");
    fs::write(&user_model, "# my custom user\n").unwrap();

    create_auth(dir.path().to_str().unwrap()).expect("second run ok");

    assert_eq!(
        fs::read_to_string(&user_model).unwrap(),
        "# my custom user\n",
        "second run clobbered an existing file"
    );

    // Routes were not duplicated.
    let routes = fs::read_to_string(dir.path().join("config/routes.sl")).unwrap();
    assert_eq!(
        routes.matches("\"sessions#new\"").count(),
        1,
        "auth routes appended twice"
    );

    // Only one users migration exists.
    let migrations = fs::read_dir(dir.path().join("db/migrations"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains("create_users"))
        .count();
    assert_eq!(migrations, 1, "duplicate users migration generated");
}

#[test]
fn create_auth_rejects_non_app_directory() {
    let dir = tempfile::tempdir().unwrap();
    // No app/ directory.
    let result = create_auth(dir.path().to_str().unwrap());
    assert!(result.is_err());
    assert!(
        result
            .err()
            .unwrap()
            .contains("does not look like a Soli app"),
        "expected app-structure error"
    );
}

/// Generate auth into a fresh skeleton and return its directory.
fn generated_app() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    create_auth(dir.path().to_str().unwrap()).expect("generate auth ok");
    dir
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

/// Card #677: what `generate auth` writes is formatted and lint-clean, so an
/// app's own `soli fmt --check` / `soli lint` gate passes right after it.
#[test]
fn generated_auth_passes_fmt_and_lint() {
    let dir = generated_app();
    let (formatted, output) = soli(dir.path(), &["fmt", "--check", "."], &[]);
    assert!(formatted, "soli fmt --check:\n{output}");
    let (clean, output) = soli(dir.path(), &["lint", "."], &[]);
    assert!(clean, "soli lint:\n{output}");
}

/// Card #677: the templates follow the house style — `<%= %>` already
/// escapes (an `h()` inside it double-escapes), `@field` rather than `this.`
/// outside a `static` block, and `nil` rather than `null`.
#[test]
fn generated_auth_follows_the_house_style() {
    let dir = generated_app();
    let mut stack = vec![dir.path().join("app")];
    let mut checked = 0;
    while let Some(path) = stack.pop() {
        if path.is_dir() {
            stack.extend(fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            continue;
        }
        let name = path.display().to_string();
        let source = fs::read_to_string(&path).unwrap();
        if name.ends_with(".slv") {
            assert!(!source.contains("<%= h("), "{name} double-escapes with h()");
        }
        if name.ends_with(".sl") {
            checked += 1;
            for (number, line) in source.lines().enumerate() {
                let code = line.split('#').next().unwrap_or_default();
                assert!(
                    !code.contains("this.") || code.contains("this.layout"),
                    "{name}:{}: `this.` where `@` applies: {line}",
                    number + 1
                );
                assert!(
                    !line
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .any(|word| word == "null"),
                    "{name}:{}: `null` where `nil` applies: {line}",
                    number + 1
                );
            }
        }
    }
    assert!(checked >= 10, "only {checked} .sl files checked");
}

/// The generated User model and policies, run against SQLite: AUTH_SIGNUP
/// decides who may sign up (`first_user` by default, a typo fails closed), the
/// per-IP throttle reads its numbers from the environment, and the rewritten
/// model and policies still behave.
#[test]
fn generated_auth_reads_its_policy_from_the_environment() {
    let dir = generated_app();
    let app = dir.path();
    let read = |rel: &str| fs::read_to_string(app.join(rel)).unwrap();
    // `req` is the request global a served app provides.
    let script = format!(
        "req = {{}}\n{}\n{}\n{}\n{}",
        read("app/models/user.sl"),
        read("app/policies/application_policy.sl"),
        read("app/policies/user_policy.sl"),
        r#"
print("before", auth_signup_mode(), auth_signup_open?(), AUTH_ATTEMPTS_PER_IP, AUTH_IP_WINDOW_SECONDS)
ada = User.new()
ada.email = " Ada@Example.com "
ada.set_password("correct horse battery")
print("saved", ada.save, ada.email, ada.role)
print("after", auth_signup_open?())
print("password", ada.authenticate("correct horse battery"), ada.authenticate("wrong"), ada.password_error("short"))
token = ada.start_password_reset
print("reset", ada.reset_token_expired?, Crypto.sha256(token) == User.find(ada._key).reset_token_digest)
ada.register_failed_attempt
print("attempts", User.find(ada._key).failed_attempts, ada.locked?)
ada.clear_failed_attempts
print("cleared", User.find(ada._key).failed_attempts)
print("resave", ada.save, ada._errors)
print("policy", UserPolicy.new({"user": ada, "record": ada}).edit?, UserPolicy.new({"user": nil, "record": ada}).show?, UserPolicy.new({"user": ada, "record": ada}).new?)
sso = User.new()
sso.email = "sso@example.com"
sso.sso_only = true
sso.save
sso.register_failed_attempt
print("no password, no count", User.find(sso._key).failed_attempts.nil?, sso.locked?)
"#
    );
    fs::write(app.join("auth_script.sl"), script).unwrap();
    let database = format!("sqlite://{}/app.sqlite3", app.display());
    let run = |extra: &[(&str, &str)]| {
        let _ = fs::remove_file(app.join("app.sqlite3"));
        let mut env = vec![
            ("SOLI_DB_ADAPTER", "sqlite"),
            ("DATABASE_URL", database.as_str()),
        ];
        env.extend_from_slice(extra);
        let (ok, output) = soli(app, &["auth_script.sl"], &env);
        assert!(ok, "{output}");
        output.lines().map(str::to_string).collect::<Vec<_>>()
    };

    let default = run(&[]);
    assert_eq!(
        default,
        vec![
            "before first_user true 15 300",
            "saved true ada@example.com member",
            "after false",
            "password true false Password must be at least 12 characters",
            "reset false true",
            "attempts 1 false",
            "cleared 0",
            "resave true []",
            "policy true false false",
            "no password, no count true false",
        ]
    );
    let open = run(&[
        ("AUTH_SIGNUP", "open"),
        ("AUTH_ATTEMPTS_PER_IP", "3"),
        ("AUTH_IP_WINDOW_SECONDS", "soon"),
    ]);
    assert_eq!(open[0], "before open true 3 300");
    assert_eq!(open[2], "after true");
    for typed in ["Closed", "yes"] {
        let closed = run(&[("AUTH_SIGNUP", typed)]);
        assert_eq!(closed[0], "before closed false 15 300", "{typed}");
    }
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

/// Serve the generated app on SQLite, with `env` on top. A handler the VM
/// hands back to the interpreter fails the request, so the controllers are
/// checked on the engine `soli serve` runs them on.
fn serve(root: &Path, env: &[(&str, &str)]) -> Server {
    let port = unused_port();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_soli"));
    command
        .arg("serve")
        .arg(root)
        .args(["--port", &port.to_string(), "--workers", "1"])
        .current_dir(root)
        .env_remove("SOLI_ENGINE")
        .env_remove("AUTH_SIGNUP")
        .env_remove("AUTH_ATTEMPTS_PER_IP")
        .env_remove("AUTH_IP_WINDOW_SECONDS")
        .env_remove("APP_ENV")
        .env_remove("AUTH_SETUP_TOKEN")
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
        .env("SOLI_MAIL_DELIVERY_METHOD", "logger")
        .env("SOLI_FAIL_ON_VM_DEMOTION", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (key, value) in env {
        command.env(key, value);
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

fn call(server: &Server, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
    let agent = ureq::AgentBuilder::new().redirects(0).build();
    let request = agent.request(method, &format!("http://127.0.0.1:{}{path}", server.1));
    let response = match body {
        Some(body) => request
            .set("Content-Type", "application/json")
            .send_string(body),
        None => request.call(),
    };
    let response = match response {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("{method} {path}: {e}"),
    };
    (
        response.status(),
        response.into_string().unwrap_or_default(),
    )
}

/// Does the sign-in page link to /signup?
fn offers_signup(server: &Server) -> bool {
    call(server, "GET", "/login", None).1.contains("Create one")
}

/// A generated app with a layout, as `soli new` would give it.
fn servable_app() -> tempfile::TempDir {
    let dir = generated_app();
    fs::create_dir_all(dir.path().join("app/views/layouts")).unwrap();
    fs::write(
        dir.path().join("app/views/layouts/application.html.slv"),
        "<main><%= yield %></main>\n",
    )
    .unwrap();
    dir
}

const ADA: &str = r#"{"email": "ada@example.com", "password": "correct horse battery"}"#;
const BOB: &str = r#"{"email": "bob@example.com", "password": "correct horse battery"}"#;

/// Card #677: sign-up is no longer open to anyone by default. Under the
/// default `AUTH_SIGNUP=first_user` the first account is created (as the
/// admin) and /signup closes behind it.
#[test]
fn served_signup_closes_after_the_first_account_by_default() {
    let dir = servable_app();
    let server = serve(dir.path(), &[("APP_ENV", "test")]);

    let (status, body) = call(&server, "GET", "/login", None);
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("Create one"), "{body}");
    assert!(!body.contains("&amp;"), "double-escaped: {body}");
    assert_eq!(call(&server, "GET", "/signup", None).0, 200);

    let (status, body) = call(&server, "POST", "/signup", Some(ADA));
    assert_eq!(status, 302, "{body}");

    let (status, body) = call(&server, "GET", "/signup", None);
    assert_eq!(status, 403, "{body}");
    assert!(body.contains("Sign-up is closed"), "{body}");
    assert_eq!(call(&server, "POST", "/signup", Some(BOB)).0, 403);
    assert!(!offers_signup(&server));

    // The account that opened the app is its admin.
    drop(server);
    let conn = rusqlite::Connection::open(dir.path().join("app.sqlite3")).unwrap();
    let mut rows = conn.prepare("SELECT * FROM users").unwrap();
    let columns = rows.column_count();
    let dump: Vec<String> = rows
        .query_map([], |row| {
            Ok((0..columns)
                .map(|i| {
                    row.get_ref(i)
                        .ok()
                        .and_then(|v| v.as_str().ok().map(str::to_string))
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join(" "))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(dump.len(), 1, "{dump:?}");
    assert!(dump[0].contains(r#""role":"admin""#), "{dump:?}");
}

/// In production the first account (the admin) needs AUTH_SETUP_TOKEN: on a
/// freshly deployed app the first visitor to reach /signup was made admin.
#[test]
fn in_production_the_first_account_needs_the_setup_token() {
    let token = "a-setup-token-long-enough-1234";
    let dir = servable_app();
    let server = serve(
        dir.path(),
        &[
            ("APP_ENV", "production"),
            ("SOLI_APP_HOSTS", "127.0.0.1"),
            (
                "SOLI_SESSION_SECRET",
                "auth-test-secret-0123456789abcdef-0123",
            ),
            ("AUTH_SETUP_TOKEN", token),
        ],
    );
    let (status, body) = call(&server, "GET", "/signup", None);
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("setup_token"), "no setup token field: {body}");

    let without = r#"{"email": "eve@example.com", "password": "correct horse battery"}"#;
    assert_eq!(call(&server, "POST", "/signup", Some(without)).0, 403);
    let wrong = r#"{"email": "eve@example.com", "password": "correct horse battery", "setup_token": "guess"}"#;
    assert_eq!(call(&server, "POST", "/signup", Some(wrong)).0, 403);
    let right = format!(
        r#"{{"email": "ada@example.com", "password": "correct horse battery", "setup_token": "{token}"}}"#
    );
    let (status, body) = call(&server, "POST", "/signup", Some(&right));
    assert_eq!(status, 302, "{body}");
    drop(server);

    // Production with no token set (or one too short to guess-proof) creates
    // no first account at all.
    let dir = servable_app();
    let server = serve(
        dir.path(),
        &[
            ("APP_ENV", "production"),
            ("SOLI_APP_HOSTS", "127.0.0.1"),
            (
                "SOLI_SESSION_SECRET",
                "auth-test-secret-0123456789abcdef-0123",
            ),
            ("AUTH_SETUP_TOKEN", "short"),
        ],
    );
    let short = r#"{"email": "ada@example.com", "password": "correct horse battery", "setup_token": "short"}"#;
    assert_eq!(call(&server, "POST", "/signup", Some(short)).0, 403);
}

/// `AUTH_SIGNUP=open` lets anyone sign up, `closed` nobody, and the per-IP
/// throttle takes its limit from `AUTH_ATTEMPTS_PER_IP`.
#[test]
fn served_signup_and_throttle_follow_the_environment() {
    let dir = servable_app();
    let server = serve(
        dir.path(),
        &[("AUTH_SIGNUP", "open"), ("AUTH_ATTEMPTS_PER_IP", "3")],
    );
    assert_eq!(call(&server, "POST", "/signup", Some(ADA)).0, 200);
    assert_eq!(call(&server, "POST", "/signup", Some(BOB)).0, 200);
    let wrong = r#"{"email": "ada@example.com", "password": "wrong password!"}"#;
    // Two sign-ups spent two of the three; the next request is the last.
    assert_eq!(call(&server, "POST", "/login", Some(wrong)).0, 401);
    let (status, body) = call(&server, "POST", "/login", Some(wrong));
    assert_eq!(status, 429, "{body}");
    drop(server);

    let dir = servable_app();
    let server = serve(dir.path(), &[("AUTH_SIGNUP", "closed")]);
    assert_eq!(call(&server, "GET", "/signup", None).0, 403);
    assert_eq!(call(&server, "POST", "/signup", Some(ADA)).0, 403);
    assert!(!offers_signup(&server));
}

/// `User#revoke_credentials` leaves an account nothing but its email: the
/// password, the remember-me digest and every session go, and so do the rows
/// of the optional credential models the app has (here an `ApiToken`), while
/// those it lacks (`OauthIdentity`, `TwoFactor`…) are skipped, not raised on.
#[test]
fn revoke_credentials_leaves_only_the_email() {
    let dir = servable_app();
    let root = dir.path();
    fs::write(
        root.join("app/models/api_token.sl"),
        "class ApiToken < Model\nend\n",
    )
    .unwrap();
    let code = r##"user = User.new
user.email = "ada@example.com"
user.set_password("correct horse battery")
user.save
user.start_remember_me
ApiToken.create({"user_id": user._key, "name": "ci"})
ApiToken.create({"user_id": "someone-else", "name": "other"})
user.revoke_credentials
user = User.find_by("email", "ada@example.com")
gone = ApiToken.where({"user_id": user._key}).count
print("#{user.password_digest.nil?} #{user.remember_token_digest.nil?} #{user.current_session_version} #{user.authenticate("correct horse battery")} #{gone} #{ApiToken.count}")"##;
    let database = format!("sqlite://{}/app.sqlite3", root.display());
    for engine in ["tree", "vm"] {
        let _ = fs::remove_file(root.join("app.sqlite3"));
        let (ok, output) = soli(
            root,
            &["runner", "-e", code],
            &[
                ("SOLI_DB_ADAPTER", "sqlite"),
                ("DATABASE_URL", &database),
                ("SOLI_ENGINE", engine),
            ],
        );
        assert!(ok, "{engine}: {output}");
        assert!(
            output.trim_end().ends_with("true true 1 false 0 1"),
            "{engine}: {output}"
        );
    }
}

/// A page without what changes on every render (the CSRF token, the form's
/// timestamp), so two answers can be compared.
fn without_form_values(page: &str) -> String {
    let mut parts = page.split("value=\"");
    let mut out = parts.next().unwrap_or_default().to_string();
    for part in parts {
        out.push_str(part.split_once('"').map_or("", |(_, rest)| rest));
    }
    out
}

/// The link in the confirmation email shows a button; only the button
/// confirms. A mail scanner or a link preview that fetches the URL must not
/// turn an account someone else opened with an address into a confirmed one —
/// which is what provider and SAML sign-ins link an account by.
#[test]
fn the_confirmation_link_confirms_nothing_until_its_button_is_pressed() {
    let dir = servable_app();
    let root = dir.path();
    fs::write(
        root.join("app/controllers/fixture_controller.sl"),
        "class FixtureController < Controller\n  # GET /fixture/token?email=\n  def token\n    \
         user = User.find_by(\"email\", params[\"email\"].to_s)\n    render_text(user.start_email_confirmation)\n  end\n\n  \
         # GET /fixture/confirmed?email=\n  def confirmed\n    \
         user = User.find_by(\"email\", params[\"email\"].to_s)\n    render_text(user.confirmed?.to_s)\n  end\nend\n",
    )
    .unwrap();
    let mut routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    routes.push_str("get(\"/fixture/token\", \"fixture#token\")\n");
    routes.push_str("get(\"/fixture/confirmed\", \"fixture#confirmed\")\n");
    fs::write(root.join("config/routes.sl"), routes).unwrap();
    let (ok, output) = soli(
        root,
        &["db:migrate", "up", "."],
        &[
            ("SOLI_DB_ADAPTER", "sqlite"),
            (
                "DATABASE_URL",
                &format!("sqlite://{}/app.sqlite3", root.display()),
            ),
        ],
    );
    assert!(ok, "{output}");
    let server = serve(root, &[("AUTH_SIGNUP", "open")]);
    // Open sign-up: one answer, whether the address was free or is taken,
    // and no session until the mailbox answered.
    let first = call(&server, "POST", "/signup", Some(ADA));
    assert_eq!(first.0, 200, "{}", first.1);
    assert!(first.1.contains("Check your mailbox"), "{}", first.1);
    let taken = call(&server, "POST", "/signup", Some(ADA));
    assert_eq!(
        (taken.0, without_form_values(&taken.1)),
        (first.0, without_form_values(&first.1)),
        "a taken address answers differently"
    );
    assert_eq!(
        call(&server, "POST", "/login", Some(ADA)).0,
        401,
        "signed in before the address was confirmed"
    );
    let confirmed = |server: &Server| {
        call(
            server,
            "GET",
            "/fixture/confirmed?email=ada@example.com",
            None,
        )
        .1
    };
    assert_eq!(confirmed(&server), "false");
    let (_, token) = call(&server, "GET", "/fixture/token?email=ada@example.com", None);
    assert!(token.len() >= 20, "{token}");

    // Fetching the link — what a scanner does — shows the button and stops.
    let (status, page) = call(
        &server,
        "GET",
        &format!("/confirm_email?token={token}"),
        None,
    );
    assert_eq!(status, 200);
    assert!(page.contains("Confirm my email"), "{page}");
    assert!(page.contains("action=\"/confirm_email\""), "{page}");
    assert_eq!(confirmed(&server), "false", "a GET confirmed the address");

    // A wrong token confirms nothing either, by GET or by POST.
    let (_, page) = call(&server, "GET", "/confirm_email?token=nope", None);
    assert!(page.contains("invalid or was already used"), "{page}");
    let (_, page) = call(
        &server,
        "POST",
        "/confirm_email",
        Some(r#"{"token": "nope"}"#),
    );
    assert!(page.contains("invalid or was already used"), "{page}");
    assert_eq!(confirmed(&server), "false");

    // The button does, once. Pressed from another browser than the one
    // that signed up — the mailbox's owner, who may not be whoever chose the
    // password: the address is confirmed, the password chosen at sign-up is
    // gone, and the visitor is sent to choose one.
    let body = format!(r#"{{"token": "{token}"}}"#);
    let agent = ureq::AgentBuilder::new().redirects(0).build();
    let origin = format!("http://127.0.0.1:{}", server.1);
    let pressed = agent
        .post(&format!("{origin}/confirm_email"))
        .set("Content-Type", "application/json")
        .send_string(&body)
        .unwrap();
    assert_eq!(pressed.status(), 302);
    let location = pressed.header("location").unwrap_or_default().to_string();
    assert!(location.starts_with("/password/edit?token="), "{location}");
    assert_eq!(confirmed(&server), "true");
    assert_eq!(
        call(&server, "POST", "/login", Some(ADA)).0,
        401,
        "the password chosen by whoever signed up still opens the account"
    );
    let (_, page) = call(&server, "POST", "/confirm_email", Some(&body));
    assert!(page.contains("invalid or was already used"), "{page}");

    // From the browser that signed up (it kept the sign-up cookie), the
    // confirmation keeps the password chosen there.
    let signed_up = agent
        .post(&format!("{origin}/signup"))
        .set("Content-Type", "application/json")
        .send_string(BOB)
        .unwrap();
    let cookie = signed_up
        .all("set-cookie")
        .iter()
        .find(|c| c.starts_with("signup="))
        .map(|c| c.split(';').next().unwrap().to_string())
        .expect("no sign-up cookie");
    let (_, token) = call(&server, "GET", "/fixture/token?email=bob@example.com", None);
    let confirmed_here = agent
        .post(&format!("{origin}/confirm_email"))
        .set("Content-Type", "application/json")
        .set("Origin", &origin)
        .set("Cookie", &cookie)
        .send_string(&format!(r#"{{"token": "{token}"}}"#))
        .unwrap();
    assert_eq!(confirmed_here.status(), 200);
    assert!(confirmed_here
        .into_string()
        .unwrap()
        .contains("Your email address is confirmed"));
    assert_eq!(call(&server, "POST", "/login", Some(BOB)).0, 302);
}

/// Completing a password reset proves the mailbox, as a confirmation does:
/// the account is confirmed by it.
#[test]
fn a_completed_password_reset_confirms_the_address() {
    let dir = servable_app();
    let root = dir.path();
    let code = r##"user = User.new
user.email = "ada@example.com"
user.set_password("correct horse battery")
user.save
before = user.confirmed?
user.finish_password_reset("another horse battery")
print("#{before} #{User.find_by("email", "ada@example.com").confirmed?}")"##;
    let database = format!("sqlite://{}/app.sqlite3", root.display());
    let (ok, output) = soli(
        root,
        &["runner", "-e", code],
        &[("SOLI_DB_ADAPTER", "sqlite"), ("DATABASE_URL", &database)],
    );
    assert!(ok, "{output}");
    assert!(output.trim_end().ends_with("false true"), "{output}");
}

/// A deployment that never said APP_ENV=production is not a development run:
/// its first account — the admin — still needs AUTH_SETUP_TOKEN.
#[test]
fn a_deployment_that_names_no_environment_still_asks_for_the_setup_token() {
    let dir = servable_app();
    let server = serve(dir.path(), &[]);
    let (status, body) = call(&server, "POST", "/signup", Some(ADA));
    assert_ne!(status, 302, "the first visitor became the admin: {body}");
    drop(server);
    let conn = rusqlite::Connection::open(dir.path().join("app.sqlite3")).unwrap();
    let users: i64 = conn
        .query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))
        .unwrap_or(0);
    assert_eq!(users, 0);
}

/// The remember-me writes are compare-and-swaps. Two requests that both read
/// the account before either wrote — a browser restoring several tabs — do
/// not both replace the token: the second finds it changed and sets no
/// cookie, so the first one's stands. And a sign-in checked against a
/// password that was reset meanwhile leaves no token behind.
#[test]
fn remember_me_writes_do_not_overwrite_what_changed_meanwhile() {
    let dir = servable_app();
    let root = dir.path();
    let code = r##"account = User.new
account.email = "ada@example.com"
account.set_password("correct horse battery")
account.save
first_token = account.start_remember_me

one_tab = User.find(account._key)
other_tab = User.find(account._key)
replaced = one_tab.rotate_remember_me
lost = other_tab.rotate_remember_me
stored = User.find(account._key)
print("rotation #{!replaced.nil?} #{lost.nil?} #{stored.remembered_by?(replaced)} #{stored.remembered_by?(first_token)}")

signing_in = User.find(account._key)
User.find(account._key).finish_password_reset("another horse battery")
late = signing_in.start_remember_me
after = User.find(account._key)
print("reset #{late.nil?} #{after.remember_token_digest.nil?} #{after.authenticate("another horse battery")}")"##;
    let database = format!("sqlite://{}/app.sqlite3", root.display());
    let (ok, output) = soli(
        root,
        &["runner", "-e", code],
        &[("SOLI_DB_ADAPTER", "sqlite"), ("DATABASE_URL", &database)],
    );
    assert!(ok, "{output}");
    let lines: Vec<&str> = output.trim_end().lines().rev().take(2).collect();
    assert_eq!(
        lines,
        ["reset true true true", "rotation true true true true"],
        "{output}"
    );
}
