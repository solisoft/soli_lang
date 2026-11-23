//! Value objects in models, and domain errors in a served app: a
//! `value_field` stored as a hash and read back as its value object, model
//! invariants as validation errors, and an uncaught `DomainError` answering
//! 422 (or its `http_status`) from the VM, with no fallback to the
//! interpreter.

use std::path::Path;

fn write(root: &Path, path: &str, body: &str) {
    let file = root.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, body).unwrap();
}

/// `Price` lives in a subdirectory that loads after `invoice.sl`: value
/// objects load first, or `value_field :total, Price` would not find it.
const PRICE: &str = r#"class Price < ValueObject
  cents: Int
  currency: String
  invariant(:cents, "must not be negative") { @cents >= 0 }

  def add(other: Price) -> Price
    @with(cents: @cents + other.cents)
  end
end
"#;

const INVOICE: &str = r#"class Invoice < Model
  value_field :total, Price
  validates("number", {"presence": true})
  invariant(:total, "total must be positive") { @total.cents > 0 }
end
"#;

#[test]
fn a_value_field_round_trips_and_invariants_refuse_the_save() {
    let dir = tempfile::tempdir().unwrap();
    let script = format!(
        "{PRICE}\n{INVOICE}\n{}",
        r#"
ok = Invoice.create({"number": "F-1", "total": Price.new(500, "EUR")})
print(ok._errors)
loaded = Invoice.find(ok._key)
print(loaded.total, loaded.total == Price.new(500, "EUR"))
print(Invoice.create({"number": "F-2", "total": Price.new(0, "EUR")})._errors)
print(Invoice.create({"number": "F-3", "total": 12})._errors)
print(Invoice.create({"total": Price.new(0, "EUR")})._errors)
print(Invoice.update(ok._key, {"total": Price.new(0, "EUR")}))
loaded.total = Price.new(0, "EUR")
loaded.save
print(loaded._errors)
print(Invoice.count, Invoice.find(ok._key).total.cents)
"#
    );
    write(dir.path(), "invoices.sl", &script);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("invoices.sl")
        .current_dir(dir.path())
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", dir.path().display()),
        )
        .output()
        .expect("run soli");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec![
            "null",
            "Price(cents: 500, currency: \"EUR\") true",
            "[{field => total, message => total must be positive}]",
            "[{field => total, message => must be a Price}]",
            // Invariants wait for the other validations to pass.
            "[{field => number, message => can't be blank}]",
            "{_errors => [{field => total, message => total must be positive}]}",
            "[{field => total, message => total must be positive}]",
            "1 500",
        ],
        "{stdout}"
    );
}

fn served_app(root: &Path) {
    write(
        root,
        "config/routes.sl",
        r#"get("/up", "billing#up")
get("/vo", "billing#vo")
get("/insufficient", "billing#insufficient")
get("/stale", "billing#stale")
get("/plain", "billing#plain")
post("/invoices", "billing#create")
"#,
    );
    write(root, "app/models/values/price.sl", PRICE);
    write(root, "app/models/invoice.sl", INVOICE);
    write(
        root,
        "app/models/errors.sl",
        r#"class InsufficientFunds < DomainError
end

class StaleInvoice < DomainError
  static http_status: Int = 409
end
"#,
    );
    write(
        root,
        "app/controllers/billing_controller.sl",
        r#"class BillingController < Controller
  # GET /up
  def up(req)
    "ok"
  end

  # GET /vo
  def vo(req)
    total = Price.new(500, "EUR").add(Price.new(cents: 250, currency: "EUR"))
    render_json({"total": total.to_h, "same": total == Price.new(750, "EUR"), "shown": total.to_s, "price": total})
  end

  # GET /insufficient
  def insufficient(req)
    throw InsufficientFunds.new("balance too low", {"needed": 30})
  end

  # GET /stale
  def stale(req)
    throw new StaleInvoice("invoice changed since you loaded it")
  end

  # GET /plain
  def plain(req)
    throw "not a domain error"
  end

  # POST /invoices
  def create(req)
    invoice = Invoice.create({"number": params["number"], "total": Price.new(params["cents"], "EUR")})
    return render_json({"errors": invoice._errors}, 422) if invoice._errors

    loaded = Invoice.find(invoice._key)
    render_json({"total": loaded.total.to_h, "price": loaded.total.is_a?("Price")})
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
    let port = port_not_given_before();
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("serve")
        .arg(root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--workers")
        .arg("1")
        .current_dir(root)
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", root.display()),
        )
        // A handler the VM handed back to the interpreter fails the request.
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

fn call(server: &Server, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
    let request = ureq::request(method, &format!("http://127.0.0.1:{}{path}", server.1));
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

#[test]
fn a_served_app_answers_a_domain_error_with_its_status() {
    let dir = tempfile::tempdir().unwrap();
    served_app(dir.path());
    let server = serve(dir.path());

    let (status, body) = call(&server, "GET", "/vo", None);
    assert_eq!(status, 200, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["total"]["cents"], 750);
    assert_eq!(json["same"], true);
    assert_eq!(json["shown"], "Price(cents: 750, currency: \"EUR\")");
    // `render_json` writes a value object's fields in declaration order.
    assert!(
        body.contains(r#""price":{"cents":750,"currency":"EUR"}"#),
        "{body}"
    );

    let (status, body) = call(&server, "GET", "/insufficient", None);
    assert_eq!(status, 422);
    assert!(body.contains("balance too low"), "{body}");

    let (status, body) = call(&server, "GET", "/stale", None);
    assert_eq!(status, 409);
    assert!(
        body.contains("invoice changed since you loaded it"),
        "{body}"
    );

    // Any other uncaught throw is still a failure.
    let (status, _) = call(&server, "GET", "/plain", None);
    assert_eq!(status, 500);

    let (status, body) = call(
        &server,
        "POST",
        "/invoices",
        Some(r#"{"number": "F-1", "cents": 900}"#),
    );
    assert_eq!(status, 200, "{body}");
    assert!(body.contains(r#""price":true"#), "{body}");
    let (status, body) = call(
        &server,
        "POST",
        "/invoices",
        Some(r#"{"number": "F-2", "cents": 0}"#),
    );
    assert_eq!(status, 422, "{body}");
    assert!(body.contains("total must be positive"), "{body}");
}

/// Phase two: domain events published after the commit, a job subscriber as
/// the transactional outbox, `after_commit` in a VM-run action, and
/// `optimistic_lock` answering 409 through `StaleRecord`.
fn events_app(root: &Path) {
    write(
        root,
        "config/routes.sl",
        r#"get("/up", "orders#up")
post("/orders", "orders#create")
post("/orders/:id/pay", "orders#pay")
post("/orders/:id/pay_then_fail", "orders#pay_then_fail")
post("/orders/:id/stale", "orders#stale")
get("/counts", "orders#counts")
"#,
    );
    write(
        root,
        "app/models/order_paid.sl",
        "class OrderPaid < DomainEvent\n  number: String\n  cents: Int\nend\n",
    );
    write(
        root,
        "app/models/order.sl",
        r#"class Order < Model
  optimistic_lock

  def pay
    @status = "paid"
    @record_event(OrderPaid.new(@number, @cents))
  end
end
"#,
    );
    write(
        root,
        "app/models/ledger_entry.sl",
        "class LedgerEntry < Model\nend\n",
    );
    write(
        root,
        "app/models/receipt.sl",
        "class Receipt < Model\nend\n",
    );
    write(root, "app/models/note.sl", "class Note < Model\nend\n");
    write(
        root,
        "app/jobs/receipt_job.sl",
        r#"class ReceiptJob
  static def perform(args)
    Receipt.create({"number": args["number"]})
  end
end
"#,
    );
    write(
        root,
        "app/subscribers/orders.sl",
        r#"subscribe(OrderPaid) do |event|
  LedgerEntry.create({"number": event.number, "cents": event.cents})
end

subscribe(OrderPaid, ReceiptJob)
subscribe(OrderPaid, {"stream": "orders", "key": "number"})
"#,
    );
    write(
        root,
        "app/controllers/orders_controller.sl",
        r#"class OrdersController < Controller
  # GET /up
  def up(req)
    "ok"
  end

  # POST /orders
  def create(req)
    order = Order.create({"number": params["number"], "cents": params["cents"]})
    render_json({"key": order._key, "lock_version": order.lock_version}, 201)
  end

  # POST /orders/:id/pay
  def pay(req)
    Order.transaction do
      order = Order.find(params["id"])
      order.pay
      throw StaleRecord.new("order changed") unless order.save
      after_commit { Note.create({"text": "paid #{order.number}"}) }
    end
    render_json({"ok": true})
  end

  # POST /orders/:id/pay_then_fail
  def pay_then_fail(req)
    Order.transaction do
      order = Order.find(params["id"])
      order.pay
      order.save
      after_commit { Note.create({"text": "never"}) }
      throw DomainError.new("payment provider refused")
    end
  end

  # POST /orders/:id/stale
  def stale(req)
    first = Order.find(params["id"])
    second = Order.find(params["id"])
    first.number = first.number + "!"
    first.save
    second.number = "late"
    throw StaleRecord.new("order changed since it was read") unless second.save
    render_json({"saved": true})
  end

  # GET /counts
  def counts(req)
    render_json({"ledger": LedgerEntry.count, "receipts": Receipt.count, "notes": Note.count})
  end
end
"#,
    );
}

fn counts(server: &Server) -> serde_json::Value {
    let (_, body) = call(server, "GET", "/counts", None);
    serde_json::from_str(&body).unwrap()
}

#[test]
fn a_served_app_publishes_events_after_the_commit() {
    let dir = tempfile::tempdir().unwrap();
    events_app(dir.path());
    let server = serve(dir.path());

    let (status, body) = call(
        &server,
        "POST",
        "/orders",
        Some(r#"{"number": "A-1", "cents": 900}"#),
    );
    assert_eq!(status, 201, "{body}");
    let created: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(created["lock_version"], 0);
    let key = created["key"].as_str().unwrap().to_string();

    // A rolled-back payment publishes nothing and runs no after_commit block.
    let (status, body) = call(
        &server,
        "POST",
        &format!("/orders/{key}/pay_then_fail"),
        Some("{}"),
    );
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        counts(&server),
        serde_json::json!({"ledger": 0, "receipts": 0, "notes": 0})
    );

    // A committed one: the block subscriber and the after_commit block ran.
    let (status, body) = call(&server, "POST", &format!("/orders/{key}/pay"), Some("{}"));
    assert_eq!(status, 200, "{body}");
    let after = counts(&server);
    assert_eq!(after["ledger"], 1, "{after}");
    assert_eq!(after["notes"], 1, "{after}");

    // The job subscriber was enqueued with the commit; a worker delivers it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while counts(&server)["receipts"] != 1 {
        assert!(
            std::time::Instant::now() < deadline,
            "the receipt job never ran: {}",
            counts(&server)
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    // Two copies of one order: the second save is stale, and the StaleRecord
    // it throws answers 409.
    let (status, body) = call(&server, "POST", &format!("/orders/{key}/stale"), Some("{}"));
    assert_eq!(status, 409, "{body}");
    assert!(body.contains("order changed since it was read"), "{body}");

    // The stream subscription: one EventStreamJob, for the committed payment
    // only, carrying the record for the ES topic.
    drop(server);
    let conn = rusqlite::Connection::open(dir.path().join("app.sqlite3")).unwrap();
    let streamed: Vec<String> = conn
        .prepare(
            "SELECT json_extract(doc, '$.args') FROM _jobs \
             WHERE json_extract(doc, '$.handler') = 'EventStreamJob'",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(streamed.len(), 1, "{streamed:?}");
    let args: serde_json::Value = serde_json::from_str(&streamed[0]).unwrap();
    assert_eq!(args["topic"], "orders");
    assert_eq!(args["key"], "A-1");
    assert_eq!(args["event"]["type"], "OrderPaid");
    assert_eq!(args["event"]["version"], 1);
    assert_eq!(
        args["event"]["data"],
        serde_json::json!({"number": "A-1", "cents": 900})
    );
}

/// Phase three: bounded contexts. `module Billing` spread over two files,
/// another context with its own `Invoice`, each model in its own table, and
/// a relation that stays inside its context.
fn contexts_app(root: &Path) {
    write(
        root,
        "config/routes.sl",
        "get(\"/up\", \"contexts#up\")\npost(\"/run\", \"contexts#run\")\nget(\"/billing/invoices\", \"billing/invoices#index\")\n",
    );
    write(
        root,
        "app/models/billing/invoice.sl",
        "module Billing\n  class Invoice < Model\n    belongs_to(\"customer\")\n  end\nend\n",
    );
    write(
        root,
        "app/models/billing/customer.sl",
        "module Billing\n  class Customer < Model\n    has_many(\"invoices\")\n  end\n\n  def self.label\n    \"billing\"\n  end\nend\n",
    );
    write(
        root,
        "app/models/shipping/invoice.sl",
        "module Shipping\n  class Invoice < Model\n  end\nend\n",
    );
    // A controller of the context: app/controllers/billing/, in the module.
    write(
        root,
        "app/controllers/billing/invoices_controller.sl",
        "module Billing\n  class InvoicesController < Controller\n    # GET /billing/invoices\n    def index(req)\n      render_json({\"count\": Invoice.count, \"controller\": self.class})\n    end\n  end\nend\n",
    );
    // A service of the context names its models unqualified.
    write(
        root,
        "app/services/billing/ledger.sl",
        "module Billing\n  class Ledger\n    static def invoices -> Int\n      Invoice.count\n    end\n  end\nend\n",
    );
    write(
        root,
        "app/controllers/contexts_controller.sl",
        r#"class ContextsController < Controller
  # GET /up
  def up(req)
    "ok"
  end

  # POST /run
  def run(req)
    customer = Billing::Customer.create({"name": "Acme"})
    Billing::Invoice.create({"number": "B-1", "customer_id": customer._key})
    Shipping::Invoice.create({"number": "S-1"})
    render_json({
      "billing": Billing::Invoice.count,
      "shipping": Shipping::Invoice.count,
      "class": Billing::Invoice.all[0].class,
      "label": Billing.label,
      "first_shipping": Shipping::Invoice.all[0].number,
      "ledger": Billing::Ledger.invoices
    })
  end
end
"#,
    );
}

#[test]
fn bounded_contexts_keep_their_models_apart() {
    let dir = tempfile::tempdir().unwrap();
    contexts_app(dir.path());
    let server = serve(dir.path());
    let (status, body) = call(&server, "POST", "/run", Some("{}"));
    assert_eq!(status, 200, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["billing"], 1, "{body}");
    assert_eq!(json["shipping"], 1, "{body}");
    assert_eq!(json["class"], "Billing::Invoice", "{body}");
    assert_eq!(json["label"], "billing", "{body}");
    assert_eq!(json["first_shipping"], "S-1", "{body}");
    assert_eq!(json["ledger"], 1, "{body}");
    let (status, body) = call(&server, "GET", "/billing/invoices", None);
    assert_eq!(status, 200, "{body}");
    assert!(
        body.contains(r#""controller":"Billing::InvoicesController""#)
            && body.contains(r#""count":1"#),
        "{body}"
    );
    drop(server);
    let conn = rusqlite::Connection::open(dir.path().join("app.sqlite3")).unwrap();
    let mut tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE '\\_%' ESCAPE '\\' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    tables.retain(|t| !t.starts_with("sqlite"));
    assert_eq!(
        tables,
        vec!["billing_customers", "billing_invoices", "shipping_invoices"]
    );
}

fn sqlite_script(dir: &Path, script: &str) -> Vec<String> {
    write(dir, "script.sl", script);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg("script.sl")
        .current_dir(dir)
        .env("SOLI_DB_ADAPTER", "sqlite")
        .env(
            "DATABASE_URL",
            format!("sqlite://{}/app.sqlite3", dir.display()),
        )
        .output()
        .expect("run soli");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout.lines().map(str::to_string).collect()
}

#[test]
fn a_value_field_can_hold_a_list_of_value_objects() {
    let dir = tempfile::tempdir().unwrap();
    let lines = sqlite_script(
        dir.path(),
        r#"class Line < ValueObject
  label: String
  cents: Int
end
class Quote < Model
  value_field :lines, [Line]
  invariant(:lines, "a quote has at least one line") { @lines.length > 0 }
end
q = Quote.create({"lines": [Line.new("design", 300), Line.new("build", 900)]})
loaded = Quote.find(q._key)
print(loaded.lines[1], loaded.lines.length)
print(Quote.create({"lines": []})._errors)
print(Quote.create({"lines": [Line.new("x", 1), 5]})._errors)
"#,
    );
    assert_eq!(
        lines,
        vec![
            "Line(label: \"build\", cents: 900) 2",
            "[{field => lines, message => a quote has at least one line}]",
            "[{field => lines, message => must be a list of Line}]",
        ]
    );
}

#[test]
fn optimistic_lock_works_on_a_column_mode_table() {
    let dir = tempfile::tempdir().unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("app.sqlite3")).unwrap();
    conn.execute_batch(
        "CREATE TABLE orders (id INTEGER PRIMARY KEY AUTOINCREMENT, number TEXT NOT NULL, \
         lock_version INTEGER NOT NULL DEFAULT 0);",
    )
    .unwrap();
    drop(conn);
    let lines = sqlite_script(
        dir.path(),
        r#"class Order < Model
  table "orders"
  optimistic_lock
end
o = Order.create({"number": "C-1"})
a = Order.find(o.id)
b = Order.find(o.id)
a.number = "C-2"
print(a.save, a.lock_version)
b.number = "C-3"
print(b.save, b.stale?)
print(Order.find(o.id).number, Order.find(o.id).lock_version)
"#,
    );
    assert_eq!(lines, vec!["true 1", "false true", "C-2 1"]);
}

#[test]
fn an_aggregate_child_is_written_through_its_root() {
    let dir = tempfile::tempdir().unwrap();
    let lines = sqlite_script(
        dir.path(),
        r#"module Billing
  class Invoice < Model
    has_many("lines")
  end

  class Line < Model
    part_of :invoice
  end
end
inv = Billing::Invoice.create({"number": "F-1"})
print(Billing::Line.create({"invoice_id": inv._key, "cents": 100}) rescue "refused")
Billing::Invoice.transaction do
  line = Billing::Line.create({"invoice_id": inv._key, "cents": 100})
  line.cents = 200
  print(line.save)
end
line = Billing::Line.all[0]
line.cents = 300
print(line.save rescue "refused", Billing::Line.count)
"#,
    );
    assert_eq!(lines, vec!["refused", "true", "refused 1"]);
}

#[test]
fn increment_moves_the_lock_version() {
    let dir = tempfile::tempdir().unwrap();
    let lines = sqlite_script(
        dir.path(),
        r#"class Counter < Model
  optimistic_lock
end
c = Counter.create({"hits": 0, "name": "a"})
stale = Counter.find(c._key)
fresh = Counter.find(c._key)
fresh.increment(:hits)
print(fresh.hits, fresh.lock_version)
stale.name = "b"
print(stale.save, stale.stale?)
"#,
    );
    assert_eq!(lines, vec!["1 1", "false true"]);
}

#[test]
fn an_event_sourced_aggregate_replays_its_stream_and_refuses_a_stale_append() {
    let dir = tempfile::tempdir().unwrap();
    let lines = sqlite_script(
        dir.path(),
        r#"class AccountOpened < DomainEvent
  owner: String
end

class Deposited < DomainEvent
  cents: Int
end

class InsufficientFunds < DomainError
end

class Account < EventSourced
  owner: String
  balance: Int = 0

  def open(owner: String)
    @emit(AccountOpened.new(owner))
  end

  def deposit(cents: Int)
    @emit(Deposited.new(cents))
  end

  def withdraw(cents: Int)
    throw InsufficientFunds.new("balance #{@balance}, asked #{cents}") if cents > @balance

    @emit(Deposited.new(-cents))
  end

  def apply(event)
    match event.class {
      "AccountOpened" => @owner = event.owner,
      "Deposited" => @balance = @balance + event.cents,
      _ => nil
    }
  end
end

seen = []
subscribe(Deposited) do |event|
  seen.push(event.cents)
end

account = Account.new()
account.open("Acme")
account.deposit(500)
account.save
print("saved:", account.id.length > 0, account.version, account.balance, seen)

loaded = Account.load(account.id)
print("loaded:", loaded.owner, loaded.balance, loaded.version)
loaded.withdraw(200)
loaded.save
print("after withdraw:", Account.load(account.id).balance, Account.load(account.id).version)
try
  loaded.withdraw(1000)
catch InsufficientFunds e
  print("refused:", e.message)
end

first = Account.load(account.id)
second = Account.load(account.id)
first.deposit(1)
first.save
second.deposit(2)
try
  second.save
catch StaleRecord e
  print("stale:", e.message)
end
print("final:", Account.load(account.id).balance, Account.load(account.id).version, seen)
"#,
    );
    let stale = lines[4].clone();
    assert!(
        stale.starts_with("stale: Account:")
            && stale.ends_with("changed since it was loaded at version 3"),
        "{stale}"
    );
    assert_eq!(
        [&lines[..4], &lines[5..]].concat(),
        vec![
            "saved: true 2 500 [500]",
            "loaded: Acme 500 2",
            "after withdraw: 300 3",
            "refused: balance 300, asked 1000",
            "final: 301 4 [500, -200, 1]",
        ]
    );
}

/// `static snapshot_every`: a snapshot every N events, in the transaction of
/// the events; `load` starts from it and replays only what came after (a
/// marker written into the stored state shows through); another
/// `snapshot_version` makes it ignored; a field a snapshot cannot hold as it
/// is fails the save with its name.
#[test]
fn an_event_sourced_aggregate_loads_from_its_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let lines = sqlite_script(
        dir.path(),
        r##"class Deposited < DomainEvent
  cents: Int
end

class Account < EventSourced
  static snapshot_every: Int = 2
  balance: Int = 0
  history: Array = []

  def deposit(cents: Int)
    @emit(Deposited.new(cents))
  end

  def apply(event)
    @balance = @balance + event.cents
    @history = @history + [event.cents]
  end
end

account = Account.new
account.deposit(10)
account.save
print("v1 snapshots:", SoliEventSnapshot.where({"stream": "Account:#{account.id}"}).count)
account.deposit(20)
account.save
snap = SoliEventSnapshot.find_by("stream", "Account:#{account.id}")
print("v2 snapshot:", snap.version, snap.state)
account.deposit(5)
account.save
loaded = Account.load(account.id)
print("loaded:", loaded.balance, loaded.version, loaded.history)

# Prove the snapshot is used: change its state, load shows the marker.
snap.update({"state": {"balance": 1000, "history": ["marker"]}})
marked = Account.load(account.id)
print("from snapshot:", marked.balance, marked.history, marked.version)

# Another schema: the snapshot is ignored, everything replayed.
snap.update({"schema": 99})
print("schema mismatch:", Account.load(account.id).balance)

# take_snapshot writes one at the current version.
again = Account.load(account.id)
print("take_snapshot:", again.take_snapshot, SoliEventSnapshot.find_by("stream", "Account:#{account.id}").version)

class Bad < EventSourced
  static snapshot_every: Int = 1
  when_at: Any = nil
  def touch
    @emit(Deposited.new(1))
  end
  def apply(event)
    @when_at = DateTime.utc
  end
end
b = Bad.new
b.touch
try
  b.save
catch e
  print("refused:", "#{e}")
end
"##,
    );
    assert_eq!(
        lines,
        vec![
            "v1 snapshots: 0",
            "v2 snapshot: 2 {balance => 30, history => [10, 20]}",
            "loaded: 35 3 [10, 20, 5]",
            "from snapshot: 1005 [marker, 5] 3",
            "schema mismatch: 35",
            "take_snapshot: true 3",
            "refused: Bad: a snapshot cannot keep `when_at` (a DateTime) as it is — define snapshot_state and restore_snapshot",
        ]
    );
}

#[test]
fn create_with_a_chosen_key_refuses_a_taken_one() {
    let dir = tempfile::tempdir().unwrap();
    let lines = sqlite_script(
        dir.path(),
        r#"class Note < Model
end
print(Note.create({"body": "a"}, {"key": "n1"})._errors)
print(Note.create({"body": "b"}, {"key": "n1"})._errors)
print(Note.find("n1").body, Note.count)
"#,
    );
    assert_eq!(
        lines,
        vec![
            "null",
            "[{field => _key, message => has already been taken}]",
            "a 1",
        ]
    );
}

fn wallets_app(root: &Path) {
    write(
        root,
        "config/routes.sl",
        r#"get("/up", "wallets#up")
post("/wallets", "wallets#create")
post("/wallets/:id/credit", "wallets#credit")
post("/wallets/:id/race", "wallets#race")
get("/wallets/:id", "wallets#show")
"#,
    );
    write(
        root,
        "app/models/credited.sl",
        "class Credited < DomainEvent\n  cents: Int\nend\n",
    );
    write(
        root,
        "app/models/wallet.sl",
        r#"class Wallet < EventSourced
  balance: Int = 0

  def credit(cents: Int)
    @emit(Credited.new(cents))
  end

  def apply(event)
    @balance = @balance + event.cents
  end
end
"#,
    );
    write(
        root,
        "app/controllers/wallets_controller.sl",
        r#"class WalletsController < Controller
  # GET /up
  def up(req)
    "ok"
  end

  # POST /wallets
  def create(req)
    wallet = Wallet.new()
    wallet.credit(params["cents"])
    wallet.save
    render_json({"id": wallet.id, "version": wallet.version}, 201)
  end

  # POST /wallets/:id/credit
  def credit(req)
    wallet = Wallet.load(params["id"])
    wallet.credit(params["cents"])
    wallet.save
    render_json({"balance": wallet.balance, "version": wallet.version})
  end

  # POST /wallets/:id/race — two copies loaded at one version both append
  def race(req)
    first = Wallet.load(params["id"])
    second = Wallet.load(params["id"])
    first.credit(1)
    first.save
    second.credit(2)
    second.save
    render_json({"saved": true})
  end

  # GET /wallets/:id
  def show(req)
    wallet = Wallet.load(params["id"])
    render_json({"balance": wallet.balance, "version": wallet.version})
  end
end
"#,
    );
}

#[test]
fn a_served_app_loads_and_saves_an_event_sourced_aggregate() {
    let dir = tempfile::tempdir().unwrap();
    wallets_app(dir.path());
    let server = serve(dir.path());

    let (status, body) = call(&server, "POST", "/wallets", Some(r#"{"cents": 100}"#));
    assert_eq!(status, 201, "{body}");
    let created: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(created["version"], 1, "{body}");
    let id = created["id"].as_str().unwrap().to_string();

    let (status, body) = call(
        &server,
        "POST",
        &format!("/wallets/{id}/credit"),
        Some(r#"{"cents": 50}"#),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"balance": 150, "version": 2})
    );

    // The second copy's append is refused: StaleRecord answers 409 and its
    // event is not stored.
    let (status, body) = call(&server, "POST", &format!("/wallets/{id}/race"), Some("{}"));
    assert_eq!(status, 409, "{body}");
    let (_, body) = call(&server, "GET", &format!("/wallets/{id}"), None);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({"balance": 151, "version": 3})
    );
}

/// `soli test` preloads app/models and app/services for the spec process in
/// the order `soli serve` loads them: value objects, events and mixin modules
/// first, then each folder's files, then its subfolders — so a bounded
/// context's model (`app/models/billing/invoice.sl`) exists in a spec, as it
/// does in the app, and a model can include a concern of
/// `app/models/concerns/`.
#[test]
fn the_test_runner_preloads_contexts_in_the_servers_order() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "app/models/_base.sl", "module Shared\nend\n");
    write(root, "app/models/billing/invoice.sl", INVOICE);
    write(root, "app/models/zebra.sl", "class Zebra < Model\nend\n");
    write(root, "app/models/values/price.sl", PRICE);
    write(
        root,
        "app/models/concerns/publishable.sl",
        "# A concern.\nmodule Publishable\nend\n",
    );
    write(
        root,
        "app/models/article.sl",
        "class Article < Model\n  include Publishable\nend\n",
    );
    // A bounded context is a module too, but not a mixin: it loads after the
    // folder's own files, so it can read a `const` of one (2.36.0 loaded it
    // first: `Undefined variable 'CARE_KINDS'`, and the app did not boot).
    write(
        root,
        "app/models/_vocabulary.sl",
        "const CARE_KINDS = [\"note\"]\n",
    );
    write(
        root,
        "app/models/care/entry.sl",
        "module Care\n  class Entry < Model\n    validates(\"kind\", {\"inclusion\": CARE_KINDS})\n  end\nend\n",
    );

    let order: Vec<String> = solilang::serve::sl_load_order(&root.join("app/models"))
        .iter()
        .map(|path| {
            path.strip_prefix(root.join("app/models"))
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        order,
        vec![
            "_base.sl",
            "concerns/publishable.sl",
            "values/price.sl",
            "_vocabulary.sl",
            "article.sl",
            "zebra.sl",
            "billing/invoice.sl",
            "care/entry.sl"
        ]
    );
    assert!(solilang::serve::sl_load_order(&root.join("app/missing")).is_empty());
}

/// A controller in a bounded context: its hooks read the context's classes
/// unqualified, as its actions do, and its own layout is the one rendered.
/// Hooks are compiled from the file's text, apart from the class, and used
/// to miss the module (`Receipt` undefined: 500); the layout was looked up
/// under the qualified class name and the page fell back on `application`.
#[test]
fn a_context_controller_keeps_its_namespace_in_hooks_and_its_layout() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "config/routes.sl",
        "get(\"/up\", \"health#up\")\nget(\"/billing/receipts\", \"billing/receipts#index\")\n",
    );
    write(
        root,
        "app/controllers/health_controller.sl",
        "class HealthController < Controller\n  # GET /up\n  def up(req)\n    \"ok\"\n  end\nend\n",
    );
    write(
        root,
        "app/models/billing/receipt.sl",
        "module Billing\n  class Receipt < Model\n    static def label -> String\n      \"reçus\"\n    end\n  end\nend\n",
    );
    write(
        root,
        "app/controllers/billing/receipts_controller.sl",
        r#"module Billing
  class ReceiptsController < Controller
    static {
      this.layout = "counter"
      this.before_action = fn(req) {
        @label = Receipt.label()
        req
      }
      this.after_action = fn(req, response) {
        response["headers"]["X-Label"] = Receipt.label()
        response
      }
    }

    # GET /billing/receipts
    def index(req)
      @count = Receipt.count
    end
  end
end
"#,
    );
    write(
        root,
        "app/views/billing/receipts/index.html.slv",
        "<p><%= @label %> : <%= @count %></p>\n",
    );
    write(
        root,
        "app/views/layouts/counter.html.slv",
        "<main data-layout=\"counter\"><%- yield %></main>\n",
    );
    write(
        root,
        "app/views/layouts/application.html.slv",
        "<main data-layout=\"application\"><%- yield %></main>\n",
    );
    let server = serve(root);
    let (status, body) = call(&server, "GET", "/billing/receipts", None);
    assert_eq!(status, 200, "{body}");
    assert!(
        body.contains("reçus : 0"),
        "the before_action did not run in its context: {body}"
    );
    assert!(
        body.contains("data-layout=\"counter\""),
        "the controller's layout was not used: {body}"
    );
}

/// A free port no other test of this binary was given: the tests run side
/// by side, and a port released by one could be handed to another while the
/// first server was still going down.
fn port_not_given_before() -> u16 {
    static TAKEN: std::sync::Mutex<Vec<u16>> = std::sync::Mutex::new(Vec::new());
    loop {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|listener| listener.local_addr())
            .map(|addr| addr.port())
            .expect("a free port");
        let mut taken = TAKEN.lock().unwrap_or_else(|e| e.into_inner());
        if !taken.contains(&port) {
            taken.push(port);
            return port;
        }
    }
}
