//! `super` in the classes of a served application, on the VM.
//!
//! `soli serve` defines an application's classes on the tree-walker and runs
//! each action on a VM, which compiles a class's constructor or method from
//! that AST when it first reaches it. Those frames carried no class, so the
//! `super(...)` in a constructor — a custom error handing its message to
//! `Error`, a subclass handing its arguments up — and the `super.m()` of an
//! action, or of a method reached through `super`, raised "super used outside
//! of a subclass method". The handler was demoted to the interpreter; inside a
//! `try`, the VM caught its own error and answered with it.
//!
//! The server runs with `SOLI_FAIL_ON_VM_DEMOTION=1`: a handler that fails on
//! the VM and succeeds on the interpreter stops it, so a regression fails the
//! requests that follow instead of passing quietly. The demotion counter on
//! `/_metrics` catches the rest.

use std::path::Path;

fn write(root: &Path, path: &str, body: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, body).unwrap();
}

const ROUTES: &str = r#"get("/up", "probe#up")
get("/built", "probe#built")
get("/chain", "probe#chain")
get("/la", "probe#la")
get("/lb", "probe#lb")
get("/caught", "probe#caught")
get("/rescued", "probe#rescued")
get("/refused", "probe#refused")
get("/show", "probe#show")
"#;

const MODELS: &str = r##"class Point
  x: Int
  y: Int

  new(x, y)
    @x = x
    @y = y
  end
end

class Diagonal < Point
  new(n)
    super(n, n)
  end
end

class LA
  def m
    1
  end
end

class LB < LA
  def m
    super.m() + 10
  end
end

class LC < LB
  def m
    super.m() + 100
  end
end

class AppError < Error
end

class Overdrawn < AppError
  amount: Int

  new(amount: Int)
    super("overdrawn by #{amount}", {"amount": amount})
    @amount = amount
  end
end

class Refused < DomainError
  new(why: String)
    super("refused: #{why}")
  end
end
"##;

const CONTROLLER: &str = r##"class BaseProbeController < Controller
  # GET /show — reached through ProbeController#show's super call
  def show(req)
    "base"
  end
end

class ProbeController < BaseProbeController
  # GET /up
  def up(req)
    {"status": 200, "body": "up"}
  end

  # GET /built
  def built(req)
    point = Diagonal.new(4)
    {"status": 200, "body": "#{point.x},#{point.y}"}
  end

  # GET /chain
  def chain(req)
    {"status": 200, "body": "#{new LC().m()}"}
  end

  # GET /la — the chain's root alone
  def la(req)
    {"status": 200, "body": "#{LA.new().m()}"}
  end

  # GET /lb — the chain's middle level alone
  def lb(req)
    {"status": 200, "body": "#{LB.new().m()}"}
  end

  # GET /caught
  def caught(req)
    outcome = "none"
    try
      throw Overdrawn.new(7)
    catch err
      outcome = "#{err.class} #{err.message} #{err.amount}"
    end
    {"status": 200, "body": outcome}
  end

  # GET /rescued
  def rescued(req)
    error = Overdrawn.new(9) rescue "rescued"
    {"status": 200, "body": "#{error.class}"}
  end

  # GET /refused — an uncaught DomainError answers 422 from the VM
  def refused(req)
    throw Refused.new("closed")
  end

  # GET /show
  def show(req)
    {"status": 200, "body": "probe+" + super.show(req)}
  end
end
"##;

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
        .unwrap_or(28743);
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("serve")
        .arg(root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        .current_dir(root)
        // A handler the VM handed back to the interpreter stops the server.
        .env("SOLI_FAIL_ON_VM_DEMOTION", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn soli serve");
    let server = Server(child, port);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if ureq::get(&format!("http://127.0.0.1:{port}/up"))
            .call()
            .is_ok()
        {
            return server;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("soli serve did not answer on {port}");
}

fn get(server: &Server, path: &str) -> (u16, String) {
    let response = match ureq::get(&format!("http://127.0.0.1:{}{path}", server.1)).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(_, r)) => r,
        Err(e) => panic!("GET {path}: {e} — did the server stop on an engine divergence?"),
    };
    (
        response.status(),
        response.into_string().unwrap_or_default(),
    )
}

#[test]
fn super_in_served_classes_runs_on_the_vm() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "config/routes.sl", ROUTES);
    write(dir.path(), "app/models/shapes.sl", MODELS);
    write(
        dir.path(),
        "app/controllers/probe_controller.sl",
        CONTROLLER,
    );
    let server = serve(dir.path());

    // Twice: the second round runs what the first compiled.
    for _ in 0..2 {
        assert_eq!(get(&server, "/built"), (200, "4,4".to_string()));
        assert_eq!(get(&server, "/chain"), (200, "111".to_string()));
        assert_eq!(
            get(&server, "/caught"),
            (200, "Overdrawn overdrawn by 7 7".to_string())
        );
        assert_eq!(get(&server, "/rescued"), (200, "Overdrawn".to_string()));
        let (status, body) = get(&server, "/refused");
        assert_eq!(status, 422, "{body}");
        assert!(body.contains("refused: closed"), "{body}");
        assert_eq!(get(&server, "/show"), (200, "probe+base".to_string()));
    }

    // A handler that failed on the VM and failed again on the interpreter
    // stops nothing — `/refused` answers its 422 either way — but it is
    // still counted.
    let (status, metrics) = get(&server, "/_metrics");
    assert_eq!(status, 200, "{metrics}");
    assert!(
        metrics.contains("soli_vm_handler_demotions_total 0\n"),
        "a handler left the VM:\n{metrics}"
    );
}

/// `super.m()` reaches the nearest override however the worker warmed up.
/// On the branch before the 2.29.4 merge, GET /chain answered 111, then GET
/// /la compiled `LA#m` on its own, and from then on GET /chain answered 101
/// for the rest of the worker's life: `LC#m`'s `super` took the first
/// compiled method up the chain (`LA#m`) and skipped `LB#m`. The action was
/// not demoted, so nothing reported it.
#[test]
fn super_reaches_the_nearest_override_whatever_ran_first() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "config/routes.sl", ROUTES);
    write(dir.path(), "app/models/shapes.sl", MODELS);
    write(
        dir.path(),
        "app/controllers/probe_controller.sl",
        CONTROLLER,
    );
    let server = serve(dir.path());

    assert_eq!(get(&server, "/chain"), (200, "111".to_string()));
    assert_eq!(get(&server, "/la"), (200, "1".to_string()));
    assert_eq!(get(&server, "/chain"), (200, "111".to_string()));
    assert_eq!(get(&server, "/lb"), (200, "11".to_string()));
    assert_eq!(get(&server, "/chain"), (200, "111".to_string()));
}
