//! One database per tenant, on SQLite files: a main database for the sites, and
//! a `site` connection whose url holds `{tenant}`, chosen per request with
//! `switch_tenant` / `with_tenant`. Driven from Soli code, checked on disk.
#![cfg(feature = "sqlite")]

use std::collections::HashMap;
use std::path::Path;

use solilang::db::registry::registry_test_lock;
use solilang::db::{
    clear_registry_override, set_registry_for_tests, Adapter, ConnectionRegistry, ConnectionSpec,
};

fn sqlite(name: &str, url: String) -> (String, ConnectionSpec) {
    (
        name.to_string(),
        ConnectionSpec {
            name: name.to_string(),
            adapter: Adapter::Sqlite,
            url: Some(url),
            solidb_host: None,
            solidb_database: None,
            solidb_username: None,
            solidb_password: None,
            solidb_api_key: None,
            pool_size: None,
            replicas: Vec::new(),
            replica_delay: None,
        },
    )
}

/// `primary` holds the sites; `site` one file per tenant under `dir/sites`.
fn registry(dir: &Path) -> ConnectionRegistry {
    let connections: HashMap<_, _> = [
        sqlite(
            "primary",
            format!("sqlite://{}/main.sqlite3", dir.display()),
        ),
        sqlite(
            "site",
            format!("sqlite://{}/sites/{{tenant}}.sqlite3", dir.display()),
        ),
    ]
    .into_iter()
    .collect();
    ConnectionRegistry {
        default: "primary".to_string(),
        connections,
        from_file: false,
    }
}

/// Scripts here run on the tree-walker: under the VM no class-body DSL
/// (`connection "site"`) receives its class in a plain script, while `soli
/// serve` loads model files on the tree-walker and runs actions on the VM —
/// which `a_served_app_keeps_each_sites_data_in_its_database` covers.
fn run(source: &str) -> Result<(), String> {
    let (_, result) = solilang::run_with_path_and_coverage(source, None, false, None, None, &[]);
    result.map_err(|e| e.to_string())
}

fn rows(file: &Path, table: &str) -> i64 {
    let conn = rusqlite::Connection::open(file).expect("open");
    conn.query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |r| {
        r.get(0)
    })
    .expect("count")
}

const MODELS: &str = r##"
class Site < Model
end

class Post < Model
  connection "site"
end

def check(label, got, want)
  throw "#{label}: got #{got}, want #{want}" unless got == want
end
"##;

#[test]
fn each_tenant_reads_and_writes_its_own_database() {
    let _lock = registry_test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    set_registry_for_tests(registry(dir.path()));
    solilang::db::tenancy::clear();
    let moved = dir.path().join("elsewhere.sqlite3");

    let script = format!(
        "{MODELS}\n{}",
        r##"
Site.create({"key": "acme"})
Site.create({"key": "globex"})

# No tenant chosen: the site connection refuses rather than guessing.
refused = Post.count rescue "refused"
check("no tenant", refused, "refused")
check("no tenant yet", current_tenant(), nil)

switch_tenant("acme")
Post.create({"title": "a1"})
Post.create({"title": "a2"})
check("acme", current_tenant(), "acme")

switch_tenant("globex")
Post.create({"title": "g1"})
check("globex posts", Post.count, 1)

with_tenant("acme") do
  check("acme posts inside with_tenant", Post.count, 2)
  check("acme titles", Post.order("title", "asc").all.map { |p| p.title }.join(","), "a1,a2")
end
check("back to globex", current_tenant(), "globex")
check("globex again", Post.count, 1)

# The main database does not move with the tenant.
check("sites", Site.count, 2)

# A site whose data lives elsewhere brings its own url.
with_tenant("moved", {"url": "MOVED_URL"}) do
  Post.create({"title": "m1"})
  check("moved posts", Post.count, 1)
end

bad = switch_tenant("../etc") rescue "bad key"
check("a key that leaves its directory", bad, "bad key")
check("a refused key leaves the tenant alone", current_tenant(), "globex")

switch_tenant(nil)
check("none again", current_tenant(), nil)
"##
        .replace("MOVED_URL", &format!("sqlite://{}", moved.display()))
    );
    let result = run(&script);
    clear_registry_override();
    result.unwrap_or_else(|e| panic!("the Soli script failed: {e}"));

    let sites = dir.path().join("sites");
    assert_eq!(rows(&sites.join("acme.sqlite3"), "posts"), 2);
    assert_eq!(rows(&sites.join("globex.sqlite3"), "posts"), 1);
    assert_eq!(rows(&moved, "posts"), 1);
    assert!(
        !sites.join("moved.sqlite3").exists(),
        "an explicit url replaces the template"
    );
    assert_eq!(rows(&dir.path().join("main.sqlite3"), "sites"), 2);
    let main = rusqlite::Connection::open(dir.path().join("main.sqlite3")).unwrap();
    let posts_in_main: i64 = main
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'posts'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(posts_in_main, 0, "no tenant data in the main database");
}

#[test]
fn with_tenant_puts_the_tenant_back_when_the_block_raises() {
    let _lock = registry_test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    set_registry_for_tests(registry(dir.path()));
    solilang::db::tenancy::clear();
    let result = run(&format!(
        "{MODELS}\n{}",
        r##"
switch_tenant("acme")
caught = nil
try
  with_tenant("globex") do
    throw "boom"
  end
catch e
  caught = "caught"
end
check("raised", caught, "caught")
check("tenant restored", current_tenant(), "acme")

missing = nil
try
  with_tenant("globex")
catch e
  missing = "no block"
end
check("a block is required", missing, "no block")
"##
    ));
    clear_registry_override();
    solilang::db::tenancy::clear();
    result.unwrap_or_else(|e| panic!("the Soli script failed: {e}"));
}

// ---------------------------------------------------------------------------
// Through `soli serve`: the middleware picks the site, actions run on the VM,
// one worker serves every request in turn, and a job runs under the tenant it
// was enqueued under.
// ---------------------------------------------------------------------------

fn write(root: &Path, path: &str, body: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, body).unwrap();
}

fn served_app(root: &Path) {
    write(
        root,
        "config/database.toml",
        r#"default = "primary"

[connections.primary]
adapter = "sqlite"
url = "sqlite://db/main.sqlite3"

[connections.site]
adapter = "sqlite"
url = "sqlite://db/sites/{tenant}.sqlite3"
"#,
    );
    write(
        root,
        "config/routes.sl",
        r#"get("/posts", "posts#index")
post("/posts", "posts#create")
post("/later", "posts#later")
get("/whoami", "posts#whoami")
"#,
    );
    write(root, "app/models/site.sl", "class Site < Model\nend\n");
    write(
        root,
        "app/models/post.sl",
        "class Post < Model\n  connection \"site\"\nend\n",
    );
    write(
        root,
        "app/middleware/site.sl",
        r#"# order: 10
def choose_site(req)
  key = req["headers"]["x-site"]
  switch_tenant(key) unless key.blank?
  req
end
"#,
    );
    write(
        root,
        "app/jobs/stamp_job.sl",
        r#"class StampJob
  static def perform(args: Hash)
    Post.create({"title": "from job #{current_tenant()}"})
  end
end
"#,
    );
    write(
        root,
        "app/controllers/posts_controller.sl",
        r#"class PostsController < Controller
  # GET /posts
  def index(req)
    titles = Post.order("title", "asc").all.map { |p| p.title }
    render_json({"tenant": current_tenant(), "titles": titles})
  end

  # POST /posts
  def create(req)
    Post.create({"title": params["title"]})
    render_json({"tenant": current_tenant(), "count": Post.count})
  end

  # POST /later
  def later(req)
    StampJob.perform_later({})
    render_json({"queued": current_tenant()})
  end

  # GET /whoami
  def whoami(req)
    render_json({"tenant": current_tenant()})
  end
end
"#,
    );
}

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
        .unwrap_or(28731);
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("serve")
        .arg(root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        // `sqlite://db/…` is relative to the working directory, as for an app
        // run from its own folder.
        .current_dir(root)
        .env("SOLI_SESSION_SECRET", "tenant-test-secret-0123456789abcdef")
        .env("SOLI_FAIL_ON_VM_DEMOTION", "1")
        .env("SOLI_JOBS_POLL_MS", "100")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn soli serve");
    let server = Server(child, port);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if ureq::get(&format!("http://127.0.0.1:{port}/whoami"))
            .call()
            .is_ok()
        {
            return server;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("soli serve did not answer on {port}");
}

fn call(
    server: &Server,
    method: &str,
    path: &str,
    site: Option<&str>,
    body: Option<&str>,
) -> (u16, serde_json::Value) {
    let mut request = ureq::request(method, &format!("http://127.0.0.1:{}{path}", server.1));
    if let Some(site) = site {
        request = request.set("X-Site", site);
    }
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
    let status = response.status();
    let text = response.into_string().unwrap_or_default();
    (
        status,
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text)),
    )
}

#[test]
fn a_served_app_keeps_each_sites_data_in_its_database() {
    let dir = tempfile::tempdir().expect("tempdir");
    served_app(dir.path());
    let server = serve(dir.path());

    for (site, title) in [("acme", "a1"), ("acme", "a2"), ("globex", "g1")] {
        let (status, body) = call(
            &server,
            "POST",
            "/posts",
            Some(site),
            Some(&format!(r#"{{"title": "{title}"}}"#)),
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["tenant"], site);
    }
    let (_, acme) = call(&server, "GET", "/posts", Some("acme"), None);
    assert_eq!(acme["titles"], serde_json::json!(["a1", "a2"]));
    let (_, globex) = call(&server, "GET", "/posts", Some("globex"), None);
    assert_eq!(globex["titles"], serde_json::json!(["g1"]));

    // The one worker last served globex: a request that chooses no site
    // starts with none, and a site-data read refuses instead of answering
    // with globex's rows.
    let (status, whoami) = call(&server, "GET", "/whoami", None, None);
    assert_eq!(status, 200);
    assert_eq!(whoami["tenant"], serde_json::Value::Null);
    let (status, _) = call(&server, "GET", "/posts", None, None);
    assert!(status >= 500, "a site read without a site: {status}");

    // A job enqueued under acme runs under acme.
    let (status, queued) = call(&server, "POST", "/later", Some("acme"), Some("{}"));
    assert_eq!(status, 200, "{queued}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        let (_, acme) = call(&server, "GET", "/posts", Some("acme"), None);
        if acme["titles"]
            .as_array()
            .is_some_and(|t| t.iter().any(|t| t == "from job acme"))
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the job never ran under acme: {acme}"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let (_, globex) = call(&server, "GET", "/posts", Some("globex"), None);
    assert_eq!(
        globex["titles"],
        serde_json::json!(["g1"]),
        "nothing leaked into globex"
    );

    let db = dir.path().join("db");
    assert_eq!(rows(&db.join("sites/acme.sqlite3"), "posts"), 3);
    assert_eq!(rows(&db.join("sites/globex.sqlite3"), "posts"), 1);
}

// ---------------------------------------------------------------------------
// SoliDB: a `database` with `{tenant}` in it. Needs a server, like the native
// driver test: `SOLIDB_DRIVER_TEST_HOST=127.0.0.1:16745` (admin/admin unless
// SOLIDB_USERNAME / SOLIDB_PASSWORD say otherwise). Skipped without one, and
// failed instead under `SOLI_REQUIRE_DB=1`, which CI sets.
// ---------------------------------------------------------------------------

fn solidb_server() -> Option<String> {
    match std::env::var("SOLIDB_DRIVER_TEST_HOST") {
        Ok(host) if !host.is_empty() => Some(host),
        _ => {
            if std::env::var("SOLI_REQUIRE_DB").is_ok_and(|flag| flag == "1") {
                panic!("SOLI_REQUIRE_DB=1 but SOLIDB_DRIVER_TEST_HOST is unset");
            }
            eprintln!("skip: SOLIDB_DRIVER_TEST_HOST unset (no SoliDB for the tenant test)");
            None
        }
    }
}

fn solidb_auth() -> String {
    use base64::Engine;
    let user = std::env::var("SOLIDB_USERNAME").unwrap_or_else(|_| "admin".into());
    let pass = std::env::var("SOLIDB_PASSWORD").unwrap_or_else(|_| "admin".into());
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))
    )
}

/// Documents in `collection` of `database`, or 0 when either is missing.
fn solidb_count(host: &str, database: &str, collection: &str) -> i64 {
    let url = format!("http://{host}/_api/database/{database}/collection");
    let Ok(response) = ureq::get(&url).set("Authorization", &solidb_auth()).call() else {
        return 0;
    };
    let body: serde_json::Value = response.into_json().unwrap_or_default();
    body["collections"]
        .as_array()
        .and_then(|all| all.iter().find(|c| c["name"] == collection))
        .and_then(|c| c["count"].as_i64())
        .unwrap_or(0)
}

#[test]
fn solidb_tenants_get_a_database_each() {
    let Some(host) = solidb_server() else { return };
    let _lock = registry_test_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let prefix = format!("soli_tenancy_test_{}", std::process::id());
    let main = format!("{prefix}_main");
    // The ORM reads its SoliDB host and default database from the environment.
    std::env::set_var("SOLIDB_HOST", format!("http://{host}"));
    std::env::set_var("SOLIDB_DATABASE", &main);

    let solidb = |name: &str, database: String| {
        (
            name.to_string(),
            ConnectionSpec {
                name: name.to_string(),
                adapter: Adapter::Solidb,
                url: None,
                solidb_host: Some(format!("http://{host}")),
                solidb_database: Some(database),
                solidb_username: None,
                solidb_password: None,
                solidb_api_key: None,
                pool_size: None,
                replicas: Vec::new(),
                replica_delay: None,
            },
        )
    };
    set_registry_for_tests(ConnectionRegistry {
        default: "primary".to_string(),
        connections: [
            solidb("primary", main.clone()),
            solidb("site", format!("{prefix}_{{tenant}}")),
        ]
        .into_iter()
        .collect(),
        from_file: false,
    });
    solilang::db::tenancy::clear();

    let result = run(r##"
class TenancySite < Model
end

class TenancyPost < Model
  connection "site"
end

def check(label, got, want)
  throw "#{label}: got #{got}, want #{want}" unless got == want
end

TenancySite.create({"key": "acme"})
switch_tenant("acme")
TenancyPost.create({"title": "a1"})
TenancyPost.create({"title": "a2"})
switch_tenant("globex")
TenancyPost.create({"title": "g1"})
check("globex", TenancyPost.count, 1)
with_tenant("acme") do
  check("acme", TenancyPost.count, 2)
end
switch_tenant(nil)
check("sites", TenancySite.count, 1)
"##);
    solilang::db::tenancy::clear();
    clear_registry_override();

    let counts = (
        solidb_count(&host, &main, "tenancy_sites"),
        solidb_count(&host, &main, "tenancy_posts"),
        solidb_count(&host, &format!("{prefix}_acme"), "tenancy_posts"),
        solidb_count(&host, &format!("{prefix}_globex"), "tenancy_posts"),
    );
    for database in [
        main.clone(),
        format!("{prefix}_acme"),
        format!("{prefix}_globex"),
    ] {
        let _ = ureq::delete(&format!("http://{host}/_api/database/{database}"))
            .set("Authorization", &solidb_auth())
            .call();
    }
    result.expect("the Soli script failed against SoliDB");
    assert_eq!(
        counts,
        (1, 0, 2, 1),
        "(main sites, main posts, acme posts, globex posts)"
    );
}
