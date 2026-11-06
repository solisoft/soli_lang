//! `Xml.each`, `Xml.reader` and `Xml.parse_file` read files through the same
//! jail as `File`: once a jail is installed (what `soli serve` and the job
//! worker do at boot, `serve::serve_folder…` → `set_file_jail(app_root)`), a
//! relative path resolves under the app root and a path outside it is
//! refused — on both engines.
//!
//! Lives in its own test binary on purpose: the jail is process state and
//! first-write-wins, so it must not leak into other tests.

use std::fs;
use std::path::PathBuf;

use solilang::interpreter::builtins::file::set_file_jail;
use solilang::{run_script, Engine};

fn run(source: &str, engine: Engine) -> Result<(), String> {
    run_script(source, None, false, engine).map_err(|e| e.to_string())
}

fn refused(source: &str, engine: Engine, needle: &str) {
    match run(source, engine) {
        Ok(()) => panic!("{engine:?}: expected a refusal for:\n{source}"),
        Err(message) => assert!(
            message.contains(needle),
            "{engine:?}: expected {needle:?} in {message:?} for:\n{source}"
        ),
    }
}

#[test]
fn streaming_reads_honour_the_app_root_jail_on_both_engines() {
    std::env::set_var("SOLI_VM_OPTIONAL_LET", "1");
    let base: PathBuf = std::env::temp_dir().join(format!("soli_xml_jail_{}", std::process::id()));
    let app = base.join("app");
    fs::create_dir_all(app.join("imports")).expect("create the app root");
    let base = fs::canonicalize(&base).expect("canonical temp dir");
    let app = base.join("app");
    let feed = "<r><item id=\"1\"/><item id=\"2\"/></r>";
    fs::write(app.join("imports/feed.xml"), feed).expect("write the feed");
    let outside = base.join("outside.xml");
    fs::write(&outside, feed).expect("write the outside file");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, app.join("imports/link.xml")).expect("symlink");

    set_file_jail(app.clone());

    for engine in [Engine::Tree, Engine::Vm] {
        // Inside the jail: a relative path resolves under the app root.
        let inside = r#"
n = Xml.each("imports/feed.xml", "item") do |item|
  nil
end
throw "each counted #{n}" unless n == 2
reader = Xml.reader("imports/feed.xml", "item")
first = reader.next
throw "reader gave #{first}" unless first["id"] == "1"
doc = Xml.parse_file("imports/feed.xml")
throw "parse_file gave #{doc}" unless doc.root.children.length == 2
"#;
        if let Err(message) = run(inside, engine) {
            panic!("{engine:?}: reading inside the jail failed: {message}");
        }

        let outside_path = outside.display().to_string();
        for (call, who) in [
            (
                format!("Xml.each({outside_path:?}, \"item\") do |item|\n  nil\nend"),
                "Xml.each",
            ),
            (
                format!("Xml.reader({outside_path:?}, \"item\")"),
                "Xml.reader",
            ),
            (
                format!("Xml.parse_file({outside_path:?})"),
                "Xml.parse_file",
            ),
            (
                "Xml.each(\"../outside.xml\", \"item\") do |item|\n  nil\nend".to_string(),
                "Xml.each",
            ),
            (
                "Xml.reader(\"imports/../../outside.xml\", \"item\")".to_string(),
                "Xml.reader",
            ),
        ] {
            refused(&call, engine, "escapes the app-root jail");
            refused(&call, engine, who);
        }

        // A symlink planted inside the jail that points outside is refused
        // too: the jail resolves it, and the open does not follow it.
        #[cfg(unix)]
        refused(
            "Xml.each(\"imports/link.xml\", \"item\") do |item|\n  nil\nend",
            engine,
            "Xml.each",
        );
    }

    let _ = fs::remove_dir_all(&base);
}

// ------------------------------------------------------------ under `soli serve`

/// A `soli serve` child on a throwaway app, killed on drop.
struct Server {
    child: std::process::Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn start(app: &std::path::Path) -> Server {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .expect("a free port");
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
            .arg("serve")
            .arg(app)
            .arg("--port")
            .arg(port.to_string())
            .arg("--workers")
            .arg("1")
            .env("SOLI_HOST", "127.0.0.1")
            // The handler must run on the VM, not fall back to the tree-walker.
            .env("SOLI_FAIL_ON_VM_DEMOTION", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn soli serve");
        let server = Server { child, port };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while ureq::get(&format!("http://127.0.0.1:{port}/ping"))
            .timeout(std::time::Duration::from_millis(500))
            .call()
            .is_err()
        {
            assert!(
                std::time::Instant::now() < deadline,
                "soli serve never answered on port {port}"
            );
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        server
    }

    fn stream(&self, mode: &str, path: &str) -> String {
        ureq::get(&format!("http://127.0.0.1:{}/stream", self.port))
            .query("mode", mode)
            .query("path", path)
            .timeout(std::time::Duration::from_secs(10))
            .call()
            .expect("request")
            .into_string()
            .expect("body")
    }
}

const ROUTES: &str = r#"get("/ping", "api#ping")
get("/stream", "api#stream")
"#;

const CONTROLLER: &str = r#"def ping(req)
  {"status": 200, "body": "pong"}
end

def stream(req)
  path = req["query"]["path"]
  mode = req["query"]["mode"]
  body = "?"
  try
    if mode == "reader"
      body = "reader #{Xml.reader(path, "item").next["id"]}"
    elsif mode == "parse"
      body = "parse #{Xml.parse_file(path).root.name}"
    else
      n = Xml.each(path, "item") do |item|
        nil
      end
      body = "each #{n}"
    end
  catch e
    body = "ERR #{e}"
  end
  {"status": 200, "body": body}
end
"#;

#[test]
fn streaming_reads_honour_the_jail_under_soli_serve() {
    let base: PathBuf =
        std::env::temp_dir().join(format!("soli_xml_serve_jail_{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let app = base.join("app");
    fs::create_dir_all(app.join("config")).expect("config dir");
    fs::create_dir_all(app.join("app/controllers")).expect("controllers dir");
    fs::create_dir_all(app.join("data")).expect("data dir");
    let base = fs::canonicalize(&base).expect("canonical temp dir");
    let app = base.join("app");
    fs::write(app.join("config/routes.sl"), ROUTES).expect("routes");
    fs::write(app.join("app/controllers/api_controller.sl"), CONTROLLER).expect("controller");
    let feed = "<r><item id=\"1\"/><item id=\"2\"/></r>";
    fs::write(app.join("data/feed.xml"), feed).expect("feed");
    let outside = base.join("outside.xml");
    fs::write(&outside, feed).expect("outside file");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, app.join("data/link.xml")).expect("symlink");

    let server = Server::start(&app);

    // Inside the app root, by a relative path.
    assert_eq!(server.stream("each", "data/feed.xml"), "each 2");
    assert_eq!(server.stream("reader", "data/feed.xml"), "reader 1");
    assert_eq!(server.stream("parse", "data/feed.xml"), "parse r");

    let outside_path = outside.display().to_string();
    let mut refused = vec![
        outside_path.as_str(),
        "../outside.xml",
        "data/../../outside.xml",
    ];
    #[cfg(unix)]
    refused.push("data/link.xml");
    for path in refused {
        for (mode, who) in [
            ("each", "Xml.each"),
            ("reader", "Xml.reader"),
            ("parse", "Xml.parse_file"),
        ] {
            let body = server.stream(mode, path);
            assert!(
                body.starts_with(&format!("ERR {who}")) && body.contains("jail"),
                "{mode} {path:?} was not refused by the jail: {body:?}"
            );
        }
    }

    drop(server);
    let _ = fs::remove_dir_all(&base);
}
