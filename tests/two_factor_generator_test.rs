//! `soli generate auth --two-factor` — an authenticator app (TOTP) as a
//! second factor: what it writes, that it edits nothing of the auth scaffold,
//! that it is fmt/lint clean, and a served app where a user enrols, signs in
//! with a code, spends a backup code, and is refused a replayed one.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use hmac::{Hmac, Mac};
use serde_json::{json, Value as Json};
use sha1::Sha1;
use solilang::scaffold::{create_auth, create_auth_with, create_two_factor};

const TWO_FACTOR_FILES: [&str; 7] = [
    "app/models/two_factor.sl",
    "app/middleware/two_factor.sl",
    "app/controllers/two_factor_controller.sl",
    "app/views/two_factor/index.html.slv",
    "app/views/two_factor/setup.html.slv",
    "app/views/two_factor/backup_codes.html.slv",
    "app/views/two_factor/challenge.html.slv",
];

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

fn two_factor_app() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    let folder = dir.path().to_str().unwrap();
    create_auth(folder).expect("generate auth");
    create_two_factor(folder).expect("generate auth --two-factor");
    dir
}

#[test]
fn two_factor_writes_its_files_migration_and_routes() {
    let dir = two_factor_app();
    let root = dir.path();
    for rel in TWO_FACTOR_FILES {
        assert!(root.join(rel).exists(), "{rel} not created");
    }
    assert_eq!(migrations(root, "create_two_factors"), 1);
    let routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    for route in [
        r#"get("/two_factor", "two_factor#index")"#,
        r#"post("/two_factor/setup", "two_factor#setup")"#,
        r#"post("/two_factor/confirm", "two_factor#confirm")"#,
        r#"post("/two_factor/backup_codes", "two_factor#backup_codes")"#,
        r#"delete("/two_factor", "two_factor#destroy")"#,
        r#"get("/two_factor/challenge", "two_factor#challenge")"#,
        r#"post("/two_factor/challenge", "two_factor#verify")"#,
    ] {
        assert!(routes.contains(route), "missing {route}");
    }
    // The gate runs right after the loader it reads.
    let gate = fs::read_to_string(root.join("app/middleware/two_factor.sl")).unwrap();
    let loader = fs::read_to_string(root.join("app/middleware/current_user.sl")).unwrap();
    assert!(loader.contains("# order: 5"), "{loader}");
    assert!(gate.contains("# order: 6"), "{gate}");
}

#[test]
fn plain_auth_writes_no_two_factor_files() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    create_auth_with(dir.path().to_str().unwrap(), true).unwrap();
    for rel in TWO_FACTOR_FILES {
        assert!(!dir.path().join(rel).exists(), "{rel} written unasked");
    }
}

/// It adds; it never edits. An app that customized its auth files keeps them,
/// and a second run changes nothing.
#[test]
fn two_factor_on_an_existing_auth_app_edits_nothing_and_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    make_app_skeleton(root);
    let folder = root.to_str().unwrap();
    create_auth(folder).unwrap();
    let kept = [
        "app/models/user.sl",
        "app/controllers/sessions_controller.sl",
        "app/middleware/current_user.sl",
        "app/views/sessions/new.html.slv",
    ];
    for rel in kept {
        fs::write(root.join(rel), "# mine\n").unwrap();
    }

    create_two_factor(folder).unwrap();
    for rel in kept {
        assert_eq!(fs::read_to_string(root.join(rel)).unwrap(), "# mine\n");
    }
    fs::write(root.join("app/models/two_factor.sl"), "# mine too\n").unwrap();
    create_two_factor(folder).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("app/models/two_factor.sl")).unwrap(),
        "# mine too\n"
    );
    let routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    assert_eq!(routes.matches("\"two_factor#index\"").count(), 1);
    assert_eq!(migrations(root, "create_two_factors"), 1);
}

#[test]
fn two_factor_requires_the_user_model() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    let err = create_two_factor(dir.path().to_str().unwrap()).unwrap_err();
    assert!(err.contains("app/models/user.sl"), "{err}");
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
fn cli_accepts_the_flag_alone_with_passkeys_and_on_an_existing_app() {
    for args in [
        &["generate", "auth", "--two-factor", "app"][..],
        &["generate", "auth", "app", "--passkeys", "--two-factor"][..],
    ] {
        let dir = tempfile::tempdir().unwrap();
        make_app_skeleton(&dir.path().join("app"));
        let (ok, output) = soli(dir.path(), args);
        assert!(ok, "{args:?}: {output}");
        assert!(
            dir.path().join("app/app/models/two_factor.sl").exists(),
            "{args:?}: {output}"
        );
        assert!(output.contains("SOLI_ENCRYPTION_KEY"), "{output}");
        // A passkey sign-in written by this version skips the challenge
        // itself, so there is nothing to tell the user to edit.
        assert!(!output.contains("was kept as it is"), "{output}");
    }

    // On an app whose passkey sign-in predates the flag, the message gives
    // the one line to add.
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    assert!(soli(dir.path(), &["generate", "auth", "--passkeys"]).0);
    fs::write(
        dir.path()
            .join("app/controllers/passkey_sessions_controller.sl"),
        "# an older scaffold\n",
    )
    .unwrap();
    let (ok, output) = soli(dir.path(), &["generate", "auth", "--two-factor"]);
    assert!(ok, "{output}");
    assert!(output.contains("was kept as it is"), "{output}");
    assert!(output.contains("user_verified"), "{output}");
}

#[test]
fn generated_two_factor_passes_fmt_and_lint() {
    let dir = two_factor_app();
    let (formatted, output) = soli(dir.path(), &["fmt", "--check", "."]);
    assert!(formatted, "soli fmt --check:\n{output}");
    let (clean, output) = soli(dir.path(), &["lint", "."]);
    assert!(clean, "soli lint:\n{output}");
}

/// Route comments above every public action, `_` helpers under `private`,
/// `@` rather than `this.` outside `static`, `nil` rather than `null`.
#[test]
fn generated_two_factor_follows_the_house_style() {
    let dir = two_factor_app();
    for rel in [
        "app/controllers/two_factor_controller.sl",
        "app/models/two_factor.sl",
        "app/middleware/two_factor.sl",
    ] {
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
                        above.starts_with("# GET ")
                            || above.starts_with("# POST ")
                            || above.starts_with("# DELETE "),
                        "{rel}: action {name} has no route comment"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Served end-to-end: a software authenticator against the generated app
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
        .env_remove("TWO_FACTOR_ISSUER")
        .env("AUTH_SIGNUP", "open")
        // The lockout is what is under test, not the per-IP throttle.
        .env("AUTH_ATTEMPTS_PER_IP", "1000")
        .env("AUTH_IP_WINDOW_SECONDS", "300")
        .env(
            "SOLI_ENCRYPTION_KEY",
            "two-factor-test-key-0123456789abcdef",
        )
        .env("SOLI_CSRF_TOKENS", "require")
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
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

/// What a request answered: status, body, and where it redirects to.
struct Reply {
    status: u16,
    body: String,
    location: String,
}

/// A browser stand-in: a cookie jar. Posts are JSON, which the CSRF gate
/// accepts from the page's own origin.
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

    fn request(&mut self, method: &str, path: &str, body: Option<Json>) -> Reply {
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
            request = request.set("Origin", &format!("http://127.0.0.1:{}", self.port));
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

    /// Password sign-in; with two-factor on, the session is then pending.
    fn sign_in(&mut self) -> Reply {
        self.post("/login", json!({"email": EMAIL, "password": PASSWORD}))
    }

    /// Whether the app sees a signed-in user: /two_factor renders for one,
    /// and sends anyone else away.
    fn signed_in(&mut self) -> bool {
        self.get("/two_factor").status == 200
    }
}

const EMAIL: &str = "ada@example.com";
const PASSWORD: &str = "correct horse battery";

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Wait out the last seconds of a 30-second step, so a code computed now is
/// still inside the window the server checks it in.
fn settle() {
    let into_step = unix_now() % 30;
    if into_step >= 26 {
        std::thread::sleep(std::time::Duration::from_secs(31 - into_step));
    }
}

/// RFC 6238: the six digits for `secret` (Base32) at `time`.
fn totp(secret: &str, time: u64) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut key, mut buffer, mut bits) = (Vec::new(), 0u64, 0);
    for c in secret.bytes() {
        let value = ALPHABET.iter().position(|&x| x == c).expect("base32") as u64;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            key.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    let mut mac = Hmac::<Sha1>::new_from_slice(&key).unwrap();
    mac.update(&(time / 30).to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let code = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]) % 1_000_000;
    format!("{code:06}")
}

/// The text of every element carrying `marker` (`data-backup-code>…<`).
fn marked(html: &str, marker: &str) -> Vec<String> {
    html.split(&format!("{marker}>"))
        .skip(1)
        .map(|rest| {
            rest.split('<')
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        })
        .collect()
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
    make_app_skeleton(dir.path());
    let folder = dir.path().to_str().unwrap();
    create_auth(folder).unwrap();
    create_two_factor(folder).unwrap();
    fs::create_dir_all(dir.path().join("app/views/layouts")).unwrap();
    fs::write(
        dir.path().join("app/views/layouts/application.html.slv"),
        "<main><%= yield %></main>\n",
    )
    .unwrap();
    dir
}

/// What clicking the mailed confirmation link does, written straight to the
/// database: open sign-up answers "check your mailbox" and signs nobody in
/// until the address is confirmed.
fn confirm_address(root: &Path, email: &str) {
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
}

#[test]
fn served_app_enrols_challenges_and_refuses_replays() {
    let dir = served_app();
    let server = serve(dir.path());

    // A guest has nothing to set up and no challenge to answer.
    let mut guest = Browser::new(&server);
    assert_eq!(guest.get("/two_factor").location, "/login");
    assert_eq!(guest.get("/two_factor/challenge").location, "/login");
    assert_eq!(
        guest
            .post("/two_factor/challenge", json!({"code": "000000"}))
            .location,
        "/login"
    );

    // Sign up; a second browser signs in to the same account.
    let mut ada = Browser::new(&server);
    let signup = ada.post("/signup", json!({"email": EMAIL, "password": PASSWORD}));
    assert_eq!(signup.status, 200, "{}", signup.body);
    confirm_address(dir.path(), EMAIL);
    let login = ada.post("/login", json!({"email": EMAIL, "password": PASSWORD}));
    assert_eq!(login.status, 302, "{}", login.body);
    let page = ada.get("/two_factor");
    assert_eq!(page.status, 200, "{}", page.body);
    assert!(
        page.body.contains("Set up an authenticator app"),
        "{}",
        page.body
    );
    let mut other = Browser::new(&server);
    assert_eq!(other.sign_in().status, 302);
    assert!(other.signed_in());

    // Enrol: the page shows a QR code and the secret; a wrong code keeps it off.
    let setup = ada.post("/two_factor/setup", json!({}));
    assert_eq!(setup.status, 200, "{}", setup.body);
    assert!(setup.body.contains("<svg"), "{}", setup.body);
    let secret = marked(&setup.body, "data-two-factor-secret").remove(0);
    assert_eq!(secret.len(), 32, "{secret}");
    let wrong = ada.post("/two_factor/confirm", json!({"code": "000000"}));
    assert_eq!(wrong.status, 422, "{}", wrong.body);
    // Not confirmed, so not asked for: a fresh sign-in goes straight through.
    let mut early = Browser::new(&server);
    early.sign_in();
    assert!(early.signed_in());

    // The secret is encrypted at rest.
    let database = database_bytes(dir.path());
    assert!(
        database.contains("ada@example.com"),
        "the data is not where this looks"
    );
    assert!(
        !database.contains(&secret),
        "the TOTP secret is stored in plaintext"
    );

    // Confirm with the previous step's code (still in the window), which
    // leaves the current and next steps for the sign-ins below.
    settle();
    let confirmed = ada.post(
        "/two_factor/confirm",
        json!({"code": totp(&secret, unix_now() - 30)}),
    );
    assert_eq!(confirmed.status, 200, "{}", confirmed.body);
    let backup = marked(&confirmed.body, "data-backup-code");
    assert_eq!(backup.len(), 10, "{}", confirmed.body);
    assert!(ada.signed_in(), "the enrolling session stays signed in");
    let page = ada.get("/two_factor");
    assert!(page.body.contains("10 backup codes left"), "{}", page.body);
    // Turning it on signed every other session out.
    assert!(!other.signed_in());
    assert!(!early.signed_in());

    // Password alone is now one factor: a guest, sent to the challenge.
    let mut phone = Browser::new(&server);
    assert_eq!(phone.sign_in().status, 302);
    let pending = phone.get("/two_factor");
    assert_eq!(
        (pending.status, pending.location.as_str()),
        (302, "/two_factor/challenge")
    );
    assert_eq!(
        phone.post("/two_factor/setup", json!({})).location,
        "/login"
    );
    let challenge = phone.get("/two_factor/challenge");
    assert_eq!(challenge.status, 200, "{}", challenge.body);

    // A wrong code, a malformed one and the code already spent on enrolling
    // are all refused the same way.
    settle();
    let spent = totp(&secret, unix_now() - 30);
    for code in ["000000", "12", "", spent.as_str()] {
        let refused = phone.post("/two_factor/challenge", json!({"code": code}));
        assert_eq!(refused.status, 401, "{code:?}: {}", refused.body);
        assert!(!phone.signed_in());
    }
    // The current code signs in; typed with a space, as apps display it.
    let code = totp(&secret, unix_now());
    let spaced = format!("{} {}", &code[..3], &code[3..]);
    let passed = phone.post("/two_factor/challenge", json!({"code": spaced}));
    assert_eq!((passed.status, passed.location.as_str()), (302, "/"));
    assert!(phone.signed_in());
    // Nothing left to answer once through.
    assert_eq!(phone.get("/two_factor/challenge").location, "/");

    // The same code again, from another browser: a replay, refused.
    let mut thief = Browser::new(&server);
    thief.sign_in();
    let replayed = thief.post("/two_factor/challenge", json!({"code": code}));
    assert_eq!(replayed.status, 401, "{}", replayed.body);
    assert!(!thief.signed_in());

    // A backup code signs in once, however it is typed.
    let typed = backup[0].to_uppercase().replace('-', " ");
    let passed = thief.post("/two_factor/challenge", json!({"code": typed}));
    assert_eq!(passed.status, 302, "{}", passed.body);
    assert!(thief.signed_in());
    let mut again = Browser::new(&server);
    again.sign_in();
    let reused = again.post("/two_factor/challenge", json!({"code": backup[0]}));
    assert_eq!(reused.status, 401, "{}", reused.body);
    let page = ada.get("/two_factor");
    assert!(page.body.contains("9 backup codes left"), "{}", page.body);

    // New backup codes need the password, and retire the old ones.
    let refused = ada.post("/two_factor/backup_codes", json!({"password": "nope"}));
    assert_eq!(refused.status, 401, "{}", refused.body);
    // That guess counts towards the lockout, as one at the sign-in form
    // does: a session left open is not a place to guess the password from.
    let attempts = r##"print(User.find_by("email", "ada@example.com").failed_attempts.to_i)"##;
    let counted = soli_with_database(dir.path(), attempts);
    assert!(
        counted.0 && counted.1.lines().any(|line| line.trim() == "1"),
        "{}",
        counted.1
    );
    let fresh = ada.post("/two_factor/backup_codes", json!({"password": PASSWORD}));
    assert_eq!(fresh.status, 200, "{}", fresh.body);
    let counted = soli_with_database(dir.path(), attempts);
    assert!(
        counted.0 && counted.1.lines().any(|line| line.trim() == "0"),
        "{}",
        counted.1
    );
    let fresh = marked(&fresh.body, "data-backup-code");
    assert_eq!(fresh.len(), 10);
    assert!(!fresh.contains(&backup[1]));
    let retired = again.post("/two_factor/challenge", json!({"code": backup[1]}));
    assert_eq!(retired.status, 401, "{}", retired.body);

    // Ten wrong codes in a row lock the second factor: a good backup code is
    // then refused like a wrong one, on this session and on a new one.
    // (`again` is at three already: the reused code, the retired one.)
    for _ in 0..8 {
        again.post("/two_factor/challenge", json!({"code": "000000"}));
    }
    let locked = again.post("/two_factor/challenge", json!({"code": fresh[0]}));
    assert_eq!(locked.status, 401, "{}", locked.body);
    assert!(!again.signed_in());

    // Turning it off takes the password and a code; a wrong one keeps it on.
    // Lift the lock first, as its expiry would.
    drop(server);
    let (ok, output) = soli_with_database(
        dir.path(),
        "TwoFactor.all.each { |record| record.update({\"locked_at\": nil, \"failed_codes\": 0}) }",
    );
    assert!(ok, "{output}");
    let server = serve(dir.path());
    let mut ada = Browser::new(&server);
    ada.sign_in();
    assert_eq!(
        ada.post("/two_factor/challenge", json!({"code": fresh[0]}))
            .status,
        302
    );
    let kept = ada.request(
        "DELETE",
        "/two_factor",
        Some(json!({"password": PASSWORD, "code": "000000"})),
    );
    assert_eq!(kept.status, 401, "{}", kept.body);
    let off = ada.request(
        "DELETE",
        "/two_factor",
        Some(json!({"password": PASSWORD, "code": fresh[1]})),
    );
    assert_eq!((off.status, off.location.as_str()), (302, "/two_factor"));
    let page = ada.get("/two_factor");
    assert!(
        page.body.contains("Set up an authenticator app"),
        "{}",
        page.body
    );

    // Off again: the password is enough.
    let mut plain = Browser::new(&server);
    plain.sign_in();
    assert!(plain.signed_in());
}

/// Run a line of Soli against the app's database, models loaded.
fn soli_with_database(root: &Path, code: &str) -> (bool, String) {
    let script = root.join("unlock.sl");
    fs::write(&script, format!("{code}\n")).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(["db:seed", ".", "unlock.sl"])
        .current_dir(root)
        .env_remove("SOLI_ENGINE")
        .env(
            "SOLI_ENCRYPTION_KEY",
            "two-factor-test-key-0123456789abcdef",
        )
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
        .output()
        .expect("run soli db:seed");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

/// The second factor is the owner's to set, show and remove: not an API
/// token's, not an administrator's acting as the user. And wrong codes are
/// counted in the database, so a burst cannot hide behind one write.
#[test]
fn only_the_owner_at_the_keyboard_manages_the_second_factor() {
    let dir = tempfile::tempdir().unwrap();
    make_app_skeleton(dir.path());
    create_auth(dir.path().to_str().unwrap()).unwrap();
    create_two_factor(dir.path().to_str().unwrap()).unwrap();
    let read = |path: &str| fs::read_to_string(dir.path().join(path)).unwrap();
    let controller = read("app/controllers/two_factor_controller.sl");
    assert_eq!(
        controller
            .matches("return @_not_the_owner unless auth_own_session?(req)")
            .count(),
        4,
        "setup, confirm, backup_codes and destroy:\n{controller}"
    );
    assert!(controller.contains("auth_fresh_sign_in?()"), "{controller}");
    let model = read("app/models/two_factor.sl");
    assert!(model.contains(r#"@increment("failed_codes")"#), "{model}");
    assert!(model.contains("Crypto.random_hex(10)"), "{model}");
    // Two requests carrying the same code cannot both record it as used:
    // the save is a compare-and-swap.
    assert!(model.contains("optimistic_lock"), "{model}");
    assert_eq!(model.matches("@update == true").count(), 3, "{model}");
}

/// Two requests that both read the record before either wrote it — the
/// interleaving of two simultaneous posts — cannot both be accepted with the
/// same code: the first to save wins, the other's save is refused as stale.
#[test]
fn the_same_code_cannot_pass_twice_from_two_copies_of_the_record() {
    let dir = served_app();
    let root = dir.path();
    fs::write(
        root.join("race.sl"),
        r##"user = User.new
user.email = "race@example.com"
user.set_password("correct horse battery")
user.save
record = TwoFactor.start(user)
code = Crypto.totp_generate(record.secret)
print("enrolled #{!record.confirm(code).nil?}")

# An authenticator code: both copies are read, then both verify.
TwoFactor.where({"user_id": user._key}).update_all({"last_step": 0})
first = TwoFactor.find(record._key)
second = TwoFactor.find(record._key)
print("totp #{first.verify(code)} #{second.verify(code)}")

# A backup code, the same way.
backup = TwoFactor.find(record._key).new_backup_codes[0]
first = TwoFactor.find(record._key)
second = TwoFactor.find(record._key)
print("backup #{first.verify(backup)} #{second.verify(backup)}")
print("left #{TwoFactor.find(record._key).backup_codes_left}")
"##,
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(["runner", "race.sl"])
        .current_dir(root)
        .env_remove("SOLI_ENGINE")
        .env(
            "SOLI_ENCRYPTION_KEY",
            "two-factor-test-key-0123456789abcdef",
        )
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
        .output()
        .expect("run soli");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail: Vec<&str> = lines.iter().rev().take(4).rev().copied().collect();
    assert_eq!(
        tail,
        [
            "enrolled true",
            "totp true false",
            "backup true false",
            "left 9"
        ],
        "{text}"
    );
}

/// A session remembers "this account has no second factor" so a signed-in
/// request costs no query. That answer must not outlive the sign-in it was
/// given to: a browser holding it, signing in again once the account has
/// turned two-factor on, gets the challenge.
#[test]
fn a_parked_no_second_factor_answer_does_not_survive_a_new_sign_in() {
    let dir = served_app();
    let root = dir.path();
    let server = serve(root);
    let creds = json!({"email": EMAIL, "password": PASSWORD});

    // Someone who knows the password signs in while the account has no
    // second factor: their session records that.
    let mut parked = Browser::new(&server);
    assert_eq!(parked.post("/signup", creds.clone()).status, 200);
    confirm_address(root, EMAIL);
    assert_eq!(parked.post("/login", creds.clone()).status, 302);
    assert_eq!(parked.get("/two_factor").status, 200);

    // The owner turns two-factor on.
    fs::write(
        root.join("enrol.sl"),
        r##"user = User.find_by("email", "ada@example.com")
record = TwoFactor.start(user)
print("enrolled #{!record.confirm(Crypto.totp_generate(record.secret)).nil?}")
"##,
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(["runner", "enrol.sl"])
        .current_dir(root)
        .env_remove("SOLI_ENGINE")
        .env(
            "SOLI_ENCRYPTION_KEY",
            "two-factor-test-key-0123456789abcdef",
        )
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
        .output()
        .expect("run soli");
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(text.trim().ends_with("enrolled true"), "{text}");

    // The same browser signs in again with the password. It holds the old
    // answer; it must be asked for the code all the same.
    assert_eq!(parked.post("/login", creds).status, 302);
    let page = parked.get("/two_factor");
    assert_eq!(
        (page.status, page.location.as_str()),
        (302, "/two_factor/challenge"),
        "the second factor was skipped: {}",
        page.body
    );
}

/// Passing the challenge re-opens the session inside the sign-in the first
/// step began: what that step set stays — the end a SAML sign-in came with
/// (the only thing that ends such a session), a provider's vouching for the
/// same user. A new sign-in drops both: they were the previous one's.
#[test]
fn the_challenge_keeps_what_the_first_step_set_and_a_new_sign_in_drops_it() {
    let dir = served_app();
    let root = dir.path();
    fs::write(
        root.join("app/controllers/fixture_controller.sl"),
        r##"class FixtureController < Controller
  # POST /fixture/stamp — what a SAML or provider first step leaves
  def stamp
    session_set("sign_in_expires_at", 4102444800)
    session_set("oauth_reauth_user", session_get("user_id"))
    session_set("oauth_reauth_at", 1234)
    render_text("stamped")
  end

  # POST /fixture/read
  def read
    mine = session_get("oauth_reauth_user") == session_get("user_id")
    render_text("#{session_get("sign_in_expires_at")} #{mine} #{session_get("oauth_reauth_at")}")
  end
end
"##,
    )
    .unwrap();
    let mut routes = fs::read_to_string(root.join("config/routes.sl")).unwrap();
    routes.push_str(
        "post(\"/fixture/stamp\", \"fixture#stamp\")\npost(\"/fixture/read\", \"fixture#read\")\n",
    );
    fs::write(root.join("config/routes.sl"), routes).unwrap();
    let server = serve(root);
    let creds = json!({"email": EMAIL, "password": PASSWORD});
    let mut ada = Browser::new(&server);
    assert_eq!(ada.post("/signup", creds.clone()).status, 200);
    confirm_address(root, EMAIL);

    fs::write(
        root.join("enrol.sl"),
        r##"user = User.find_by("email", "ada@example.com")
record = TwoFactor.start(user)
record.confirm(Crypto.totp_generate(record.secret))
print("secret #{record.secret}")
"##,
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .args(["runner", "enrol.sl"])
        .current_dir(root)
        .env_remove("SOLI_ENGINE")
        .env(
            "SOLI_ENCRYPTION_KEY",
            "two-factor-test-key-0123456789abcdef",
        )
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
        .output()
        .expect("run soli");
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let secret = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("secret "))
        .unwrap_or_else(|| panic!("no secret: {text}"))
        .to_string();

    // First step, then what it would have left, then the challenge.
    assert_eq!(ada.post("/login", creds.clone()).status, 302);
    assert_eq!(ada.post("/fixture/stamp", json!({})).body, "stamped");
    let passed = ada.post(
        "/two_factor/challenge",
        json!({"code": totp(&secret, unix_now() + 30)}),
    );
    assert_eq!(passed.status, 302, "{}", passed.body);
    assert_eq!(
        ada.post("/fixture/read", json!({})).body,
        "4102444800 true 1234"
    );

    // A new sign-in in the same browser: none of it is this sign-in's.
    assert_eq!(ada.post("/login", creds).status, 302);
    assert_eq!(ada.post("/fixture/read", json!({})).body, "null false null");
}

/// Logging out drops the remember-me token — for a session past every step
/// of signing in. One that only got as far as the password (the account has
/// a second factor it did not pass) is not the owner's to end things with:
/// its logout leaves the owner's token alone.
#[test]
fn a_logout_before_the_second_factor_leaves_the_owners_remember_me_alone() {
    let dir = served_app();
    let root = dir.path();
    let server = serve(root);
    let creds = json!({"email": EMAIL, "password": PASSWORD});
    let mut owner = Browser::new(&server);
    assert_eq!(owner.post("/signup", creds.clone()).status, 200);
    confirm_address(root, EMAIL);
    // The owner signs in and is remembered, then turns two-factor on.
    let remembered = json!({"email": EMAIL, "password": PASSWORD, "remember_me": "1"});
    assert_eq!(owner.post("/login", remembered).status, 302);
    fs::write(
        root.join("enrol.sl"),
        r##"user = User.find_by("email", "ada@example.com")
record = TwoFactor.start(user)
record.confirm(Crypto.totp_generate(record.secret))
print("token kept #{User.find(user._key).remember_token_digest.present?}")
"##,
    )
    .unwrap();
    let held = |script: &str| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
            .args(["runner", script])
            .current_dir(root)
            .env_remove("SOLI_ENGINE")
            .env(
                "SOLI_ENCRYPTION_KEY",
                "two-factor-test-key-0123456789abcdef",
            )
            .env("SOLI_DB_ADAPTER", "sqlite")
            .env(
                "DATABASE_URL",
                format!("sqlite://{}/app.sqlite3", root.display()),
            )
            .output()
            .expect("run soli");
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    assert!(held("enrol.sl").ends_with("token kept true"));

    // Someone who knows the password, and nothing else, signs in and logs out.
    let mut stranger = Browser::new(&server);
    assert_eq!(stranger.post("/login", creds).status, 302);
    assert_eq!(
        stranger.get("/two_factor").location,
        "/two_factor/challenge",
        "the stranger's session is not pending the second factor"
    );
    stranger.request("DELETE", "/logout", None);

    fs::write(
        root.join("after.sl"),
        r##"user = User.find_by("email", "ada@example.com")
print("token kept #{user.remember_token_digest.present?}")
"##,
    )
    .unwrap();
    assert!(
        held("after.sl").ends_with("token kept true"),
        "a session that never passed the second factor dropped the owner's remember-me"
    );
}
