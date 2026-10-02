//! `ES` against a real es broker, over the binary protocol and over HTTP.
//!
//! Needs an `es-broker` executable: `ES_BROKER_BIN`, or one on `PATH`. Without
//! one the tests return early, as the Postgres and MySQL adapter tests do;
//! `SOLI_REQUIRE_ES=1` turns that into a failure.
#![cfg(feature = "es")]

use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Broker {
    child: Child,
    http: u16,
    binary: u16,
    _data: tempfile::TempDir,
}

impl Drop for Broker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn broker_bin() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ES_BROKER_BIN") {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join("es-broker"))
            .find(|p| p.is_file())
    })
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("a free port")
}

fn start() -> Option<Broker> {
    let Some(bin) = broker_bin() else {
        assert!(
            std::env::var_os("SOLI_REQUIRE_ES").is_none(),
            "SOLI_REQUIRE_ES is set and no es-broker was found (ES_BROKER_BIN or PATH)"
        );
        eprintln!("skipped: no es-broker (set ES_BROKER_BIN)");
        return None;
    };
    let data = tempfile::tempdir().expect("tempdir");
    let (http, binary) = (free_port(), free_port());
    let child = Command::new(bin)
        .arg("--data-dir")
        .arg(data.path())
        .args(["--bind", &format!("127.0.0.1:{http}")])
        .args(["--bind-binary", &format!("127.0.0.1:{binary}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start es-broker");
    let broker = Broker {
        child,
        http,
        binary,
        _data: data,
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    for port in [http, binary] {
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "es-broker did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    Some(broker)
}

/// Run `source` against `broker`, over the binary listener or HTTP alone.
fn run(broker: &Broker, binary: bool, source: &str) -> (String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("s.sl");
    std::fs::write(&script, source).expect("write");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_soli"));
    cmd.arg(&script)
        .env("ES_BROKER", format!("http://127.0.0.1:{}", broker.http))
        .env_remove("ES_AUTH")
        .env_remove("ES_GZIP");
    if binary {
        cmd.env("ES_BINARY", format!("127.0.0.1:{}", broker.binary));
    } else {
        cmd.env_remove("ES_BINARY");
    }
    let out = cmd.output().expect("run soli");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const ROUND_TRIP: &str = r#"
topic = "orders_#{getenv("ES_TRANSPORT_LABEL") ?? "x"}"
ES.create_topic(topic, {"partitions": 2})
print(ES.topics.includes?(topic))
print(ES.emit(topic, "user-1", {"id": 1, "total": 12.5}, {"partition": 0}))
print(ES.produce(topic, ["plain", {"key": "k", "value": "v", "partition": 0}]))
page = ES.consume(topic, 0, 0)
print([page["next_offset"], page["high_watermark"], page["records"].length])
print(JSON.parse(page["records"][0]["value"])["total"])
print([page["records"][0]["key"], page["records"][1]["key"]])
print(ES.consume(topic, 0, 1, {"max_records": 1})["records"].map { |r| r["value"] })
print(ES.commit("billing", topic, 0, page["next_offset"]))
print(ES.offsets("billing")[topic][0])
print(ES.group_consume("billing", topic, 0)["records"].length)
member = ES.join("billing", [topic])
print(member["assignment"].length)
print(ES.heartbeat("billing", member["member_id"], member["generation"])["status"])
print(ES.leave("billing", member["member_id"]))
first = ES.emit(topic, "k", "once", {"producer_id": "p-#{topic}", "sequence": 0, "partition": 1})
again = ES.emit(topic, "k", "once", {"producer_id": "p-#{topic}", "sequence": 0, "partition": 1})
print([first["duplicate"], again["duplicate"], first["offset"] == again["offset"]])
print(ES.delete_topic(topic)["name"] == topic)
"#;

const EXPECTED: &str = "true
{duplicate => false, offset => 0, partition => 0}
[{duplicate => false, offset => 1, partition => 0}, {duplicate => false, offset => 2, partition => 0}]
[3, 3, 3]
12.5
[user-1, null]
[plain]
true
3
0
2
ok
true
[false, true, true]
true
";

#[test]
fn the_same_script_gives_the_same_answers_over_both_transports() {
    let Some(broker) = start() else { return };
    for binary in [true, false] {
        let label = if binary { "bin" } else { "http" };
        let source = ROUND_TRIP.replace(
            "getenv(\"ES_TRANSPORT_LABEL\") ?? \"x\"",
            &format!("\"{label}\""),
        );
        let (stdout, stderr) = run(&broker, binary, &source);
        assert_eq!(stdout, EXPECTED, "over {label}; stderr:\n{stderr}");
    }
}

#[test]
fn the_transport_is_reported_and_errors_name_the_call() {
    let Some(broker) = start() else { return };
    let source = r##"
print(ES.config()["transport"])
print(ES.ping)
try
  ES.consume("missing", 0, 0)
catch e
  print("#{e}")
end
try
  ES.consume("missing", 0, 0, {"bogus": 1})
catch e
  print("#{e}")
end
try
  ES.produce("missing", [{"value": "x"}], {"producer_id": "p"})
catch e
  print("#{e}")
end
"##;
    for binary in [true, false] {
        let (stdout, stderr) = run(&broker, binary, source);
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines.len(), 5, "{stdout}\n{stderr}");
        assert_eq!(lines[0], if binary { "binary" } else { "http" });
        assert_eq!(lines[1], "true");
        assert!(lines[2].starts_with("ES.consume: "), "{}", lines[2]);
        assert!(
            lines[2].contains("topic 'missing' not found"),
            "{}",
            lines[2]
        );
        assert!(
            lines[3].contains("unknown option \"bogus\""),
            "{}",
            lines[3]
        );
        assert!(
            lines[4].contains("every record needs a \"sequence\""),
            "{}",
            lines[4]
        );
    }
}

#[test]
fn a_broker_that_is_not_there_is_named() {
    // No broker needed: nothing listens on these ports.
    let (http, binary) = (free_port(), free_port());
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("s.sl"),
        "try\n  ES.emit(\"t\", nil, \"v\")\ncatch e\n  print(\"#{e}\")\nend\n",
    )
    .expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg(dir.path().join("s.sl"))
        .env("ES_BROKER", format!("http://127.0.0.1:{http}"))
        .env("ES_BINARY", format!("127.0.0.1:{binary}"))
        .output()
        .expect("run soli");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(&format!(
            "ES.emit: cannot connect to the es binary listener at 127.0.0.1:{binary}"
        )),
        "{stdout}"
    );
}
