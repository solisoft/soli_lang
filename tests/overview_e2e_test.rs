//! The `/__soli/` overview, end to end: boots `soli serve` in production
//! mode on a small app, sends it some traffic, and reads the overview back.
//!
//! Only `SOLI_OVERVIEW_TOKEN` is set, so the overview shows its own sources —
//! usage and engine events — and reads no database: the trackers that would
//! write one are turned off too.

use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const ROUTES: &str = r#"
get("/", "home#index")
get("/posts/:id", "posts#show")
get("/boom", "home#boom")
"#;

const TOKEN: &str = "overview-e2e-token";

fn write_app(dir: &Path) {
    for sub in ["app/controllers", "app/models", "app/views", "config"] {
        std::fs::create_dir_all(dir.join(sub)).expect("app dir");
    }
    std::fs::write(dir.join("config/routes.sl"), ROUTES).expect("routes");
    std::fs::write(
        dir.join("app/controllers/home_controller.sl"),
        "class HomeController < Controller\n  def index\n    render_text(\"home\")\n  end\n\n  def boom\n    throw \"boom\"\n  end\nend\n",
    )
    .expect("home controller");
    std::fs::write(
        dir.join("app/controllers/posts_controller.sl"),
        "class PostsController < Controller\n  def show\n    render_text(\"post \" + params[\"id\"])\n  end\nend\n",
    )
    .expect("posts controller");
}

struct Server {
    child: Child,
    port: u16,
}

impl Server {
    fn start(dir: &Path) -> Self {
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
            .arg("2")
            .env("SOLI_OVERVIEW_TOKEN", TOKEN)
            .env("SOLI_USAGE", "on")
            .env("SOLI_ERRORS", "off")
            .env("SOLI_SLOW_QUERIES", "off")
            .env("SOLI_QUERY_STATS", "off")
            .env("SOLI_SPAM", "off")
            .env_remove("SOLI_ADMIN_TOKEN")
            .env_remove("SOLI_ADMIN_USER")
            .env_remove("SOLI_ADMIN_PASSWORD")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let server = Server {
            child: command.spawn().expect("spawn soli serve"),
            port,
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while server.get("/up", None).0 != 200 {
            assert!(Instant::now() < deadline, "the server never answered /up");
            std::thread::sleep(Duration::from_millis(100));
        }
        server
    }

    fn get(&self, path: &str, token: Option<&str>) -> (u16, String) {
        let mut request = ureq::get(&format!("http://127.0.0.1:{}{}", self.port, path))
            .timeout(Duration::from_secs(5));
        if let Some(token) = token {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        match request.call() {
            Ok(resp) => {
                let status = resp.status();
                (status, resp.into_string().unwrap_or_default())
            }
            Err(ureq::Error::Status(code, resp)) => (code, resp.into_string().unwrap_or_default()),
            Err(_) => (0, String::new()),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn the_overview_counts_route_patterns_behind_its_gate() {
    let dir = tempfile::tempdir().expect("tempdir");
    write_app(dir.path());
    let server = Server::start(dir.path());

    for path in ["/", "/", "/", "/posts/41", "/posts/42", "/missing", "/boom"] {
        let (status, _) = server.get(path, None);
        assert_ne!(status, 0, "{path} answered");
    }
    // Not the app's traffic.
    server.get("/favicon.ico", None);
    server.get("/_health", None);

    // Configured, no credentials: a challenge, not the page.
    let (status, body) = server.get("/__soli/", None);
    assert_eq!(status, 401, "{body}");
    let (status, _) = server.get("/__soli/", Some("wrong"));
    assert_eq!(status, 401);

    let (status, html) = server.get("/__soli/", Some(TOKEN));
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("<h1>Overview</h1>"), "{html}");
    assert!(html.contains("<code>GET /posts/:id</code>"), "{html}");
    assert!(html.contains("<code>GET /</code>"));
    assert!(html.contains("<code>no route matched</code>"));
    assert!(!html.contains("/posts/41"), "raw paths never shown");
    assert!(!html.contains("favicon"), "asset misses are not counted");
    assert!(!html.contains("_health"), "probes are not counted");
    // 3 + 2 + the 404 + the 500, and nothing of the probes or this page.
    assert!(
        html.contains("<span class=\"tile-value\">7</span>"),
        "seven requests counted: {html}"
    );
    // No other page's credentials: no other page's tiles.
    assert!(!html.contains("Open errors"));
    assert!(!html.contains(">Jobs<span"));

    // The bare path answers too, and asking again counts nothing new.
    let (status, again) = server.get("/__soli", Some(TOKEN));
    assert_eq!(status, 200);
    assert!(again.contains("<span class=\"tile-value\">7</span>"));
    let (status, hour) = server.get("/__soli/?range=1h", Some(TOKEN));
    assert_eq!(status, 200);
    assert!(hour.contains("Requests \u{b7} 1 h"));
}
