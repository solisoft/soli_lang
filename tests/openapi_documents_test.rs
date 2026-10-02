//! OpenAPI documents: `openapi(name, options, -> { … })` in `config/routes.sl`.
//!
//! Boots `soli serve` on a small app with a public `v1` document, a private
//! `admin` one and two routes in neither, and checks what each endpoint
//! serves with the endpoints off (production) and on (`SOLI_OPENAPI=1`).

use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const ROUTES: &str = r#"
openapi("v1", {"title": "API v1", "version": "1.0", "description": "The public API.", "public": true}, -> {
  namespace("api/v1", -> {
    get("/posts", "posts#index")
    get("/posts/:id", "posts#show")
  })
})

openapi("admin", -> {
  get("/admin/stats", "stats#index")
})

get("/", "home#index")
get("/about", "home#about")
"#;

fn write_app(dir: &Path, routes: &str) {
    for sub in ["app/controllers", "app/models", "app/views", "config"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    std::fs::write(dir.join("config/routes.sl"), routes).unwrap();
    std::fs::write(
        dir.join("app/controllers/posts_controller.sl"),
        "class PostsController < Controller\n  # List posts.\n  # @query page Int  Page number\n  def index\n    render_json([])\n  end\n\n  def show\n    render_json({\"id\": params[\"id\"]})\n  end\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app/controllers/stats_controller.sl"),
        "class StatsController < Controller\n  def index\n    render_json({\"users\": 3})\n  end\nend\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app/controllers/home_controller.sl"),
        "class HomeController < Controller\n  def index\n    render_text(\"home\")\n  end\n\n  def about\n    render_text(\"about\")\n  end\nend\n",
    )
    .unwrap();
}

struct Server {
    child: Child,
    port: u16,
}

impl Server {
    fn start(dir: &Path, openapi: Option<&str>) -> Self {
        let port = TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .expect("a free port");
        let mut command = Command::new(env!("CARGO_BIN_EXE_soli"));
        command
            .arg("serve")
            .arg(dir)
            .arg("--port")
            .arg(port.to_string())
            .arg("--workers")
            .arg("1")
            .env_remove("SOLI_OPENAPI")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(value) = openapi {
            command.env("SOLI_OPENAPI", value);
        }
        let server = Server {
            child: command.spawn().expect("spawn soli serve"),
            port,
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while server.status("/up") != 200 {
            assert!(Instant::now() < deadline, "the server never answered /up");
            std::thread::sleep(Duration::from_millis(100));
        }
        server
    }

    fn get(&self, path: &str) -> (u16, String) {
        match ureq::get(&format!("http://127.0.0.1:{}{}", self.port, path))
            .timeout(Duration::from_secs(5))
            .call()
        {
            Ok(resp) => {
                let status = resp.status();
                (status, resp.into_string().unwrap_or_default())
            }
            Err(ureq::Error::Status(code, resp)) => (code, resp.into_string().unwrap_or_default()),
            Err(_) => (0, String::new()),
        }
    }

    fn status(&self, path: &str) -> u16 {
        self.get(path).0
    }

    fn json(&self, path: &str) -> serde_json::Value {
        let (status, body) = self.get(path);
        assert_eq!(status, 200, "{path}: {body}");
        serde_json::from_str(&body).expect("JSON")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn paths(spec: &serde_json::Value) -> Vec<String> {
    let mut paths: Vec<String> = spec["paths"]
        .as_object()
        .expect("paths")
        .keys()
        .cloned()
        .collect();
    paths.sort();
    paths
}

#[test]
fn a_public_document_is_served_where_the_endpoints_are_off() {
    let dir = tempfile::tempdir().unwrap();
    write_app(dir.path(), ROUTES);
    let server = Server::start(dir.path(), None);

    let v1 = server.json("/openapi/v1.json");
    assert_eq!(v1["info"]["title"], "API v1");
    assert_eq!(v1["info"]["version"], "1.0");
    assert_eq!(v1["info"]["description"], "The public API.");
    assert_eq!(paths(&v1), ["/api/v1/posts", "/api/v1/posts/{id}"]);
    assert_eq!(server.status("/openapi/v1"), 200);

    // The private document, and the merged one, stay out of reach.
    assert_eq!(server.status("/openapi/admin.json"), 404);
    assert_eq!(server.status("/openapi/admin"), 404);
    assert_eq!(server.status("/openapi.json"), 404);

    // The reference offers the public document only.
    let (status, page) = server.get("/openapi");
    assert_eq!(status, 200);
    assert!(page.contains("/openapi/v1.json"), "{page}");
    assert!(!page.contains("/openapi/admin.json"), "{page}");
}

#[test]
fn with_the_endpoints_on_every_document_is_served_and_the_rest_left_out() {
    let dir = tempfile::tempdir().unwrap();
    write_app(dir.path(), ROUTES);
    let server = Server::start(dir.path(), Some("1"));

    let admin = server.json("/openapi/admin.json");
    assert_eq!(
        admin["info"]["title"], "admin",
        "the name is the default title"
    );
    assert_eq!(paths(&admin), ["/admin/stats"]);

    // `/` and `/about` are in no document, so in no spec.
    let merged = server.json("/openapi.json");
    assert_eq!(
        paths(&merged),
        ["/admin/stats", "/api/v1/posts", "/api/v1/posts/{id}"]
    );

    let (_, page) = server.get("/openapi");
    assert!(
        page.contains("/openapi/v1.json") && page.contains("/openapi/admin.json"),
        "{page}"
    );

    // A name no document has is the application's, not a 404 of ours.
    assert_eq!(server.status("/openapi/nope.json"), 404);
}

#[test]
fn an_app_without_documents_keeps_every_route() {
    let dir = tempfile::tempdir().unwrap();
    write_app(
        dir.path(),
        "get(\"/\", \"home#index\")\nget(\"/api/posts\", \"posts#index\")\n",
    );
    let server = Server::start(dir.path(), Some("1"));
    assert_eq!(paths(&server.json("/openapi.json")), ["/", "/api/posts"]);
}

/// `soli routes` runs `config/routes.sl` the way the server boots it.
fn routes_error(routes: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    write_app(dir.path(), routes);
    let out = Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("routes")
        .current_dir(dir.path())
        .output()
        .expect("run soli routes");
    assert!(!out.status.success(), "routes.sl should be refused");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn documents_do_not_nest() {
    let err = routes_error(
        "openapi(\"a\", -> {\n  openapi(\"b\", -> {\n    get(\"/x\", \"home#index\")\n  })\n})\n",
    );
    assert!(err.contains("documents do not nest"), "{err}");
}

#[test]
fn an_unknown_option_or_a_bad_name_is_refused() {
    let err = routes_error(
        "openapi(\"v1\", {\"titel\": \"x\"}, -> {\n  get(\"/x\", \"home#index\")\n})\n",
    );
    assert!(err.contains("unknown option \"titel\""), "{err}");

    let err = routes_error("openapi(\"api v1\", -> {\n  get(\"/x\", \"home#index\")\n})\n");
    assert!(err.contains("is not a document name"), "{err}");
}
