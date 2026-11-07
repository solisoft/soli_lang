//! A DateTime reads in its own zone, not the server's (#673).
//!
//! `to_iso` printed UTC while `format`/`hour`/`day` read the process zone, so
//! on a Paris host `DateTime.from_unix(0)` was `…T00:00:00+00:00` yet
//! `format("%H:%M")` said `01:00`, and in New York `DateTime.parse("2026-03-02")`
//! formatted as the 1st. `$TZ` is process-wide, so each case runs the binary
//! as a subprocess with its own `TZ` rather than mutating this test's
//! environment — and on both engines.

use std::process::Command;

fn run(source: &str, tz: &str, vm: bool, tag: &str) -> String {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "soli_dt_zone_{}_{}_{}.sl",
        std::process::id(),
        tag,
        if vm { "vm" } else { "tree" }
    ));
    std::fs::write(&path, source).expect("write temp source");
    let output = Command::new(env!("CARGO_BIN_EXE_soli"))
        .arg(&path)
        .arg(if vm { "--vm" } else { "--tree" })
        .env("TZ", tz)
        .output()
        .expect("run soli");
    let _ = std::fs::remove_file(&path);
    assert!(
        output.status.success(),
        "TZ={tz} vm={vm}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Everything here is pinned to UTC or to an offset written in the text, so
/// the output must be identical whatever the server's zone.
const ZONE_INDEPENDENT: &str = r#"d = DateTime.from_unix(0)
print(d.to_iso + " " + d.format("%H:%M") + " " + d.hour.to_s + " " + d.day.to_s)
p = DateTime.parse("2026-03-02")
print(p.format("%Y-%m-%d") + " " + p.weekday + " " + p.to_iso)
q = DateTime.parse("2026-03-02 23:30:00")
print(q.day.to_s + " " + q.to_s + " " + q.beginning_of_day.to_iso)
o = DateTime.parse("2026-03-02T10:15:00+02:00")
print(o.to_iso + " " + o.hour.to_s + " " + o.format("%H:%M %z") + " " + o.to_s)
print(o.utc.to_iso + " " + o.utc.hour.to_s)
print(o.beginning_of_day.to_iso + " " + o.end_of_month.day.to_s)
print(o.add_hours(20).to_iso)
n = DateTime.parse("2026-01-01T10:00:00-05:00")
print(n.to_iso + " " + n.hour.to_s)
print(DateTime.parse("Mon, 02 Mar 2026 10:00:00 +0100").to_iso)
print(DateTime.epoch.to_iso + " " + DateTime.epoch.year.to_s)
print(o == o.utc)
print({"at": o}.to_json)
"#;

const EXPECTED: &str = "1970-01-01T00:00:00+00:00 00:00 0 1
2026-03-02 Monday 2026-03-02T00:00:00+00:00
2 2026-03-02 23:30:00 2026-03-02T00:00:00+00:00
2026-03-02T10:15:00+02:00 10 10:15 +0200 2026-03-02 10:15:00
2026-03-02T08:15:00+00:00 8
2026-03-02T00:00:00+02:00 31
2026-03-03T06:15:00+02:00
2026-01-01T10:00:00-05:00 10
2026-03-02T10:00:00+01:00
1970-01-01T00:00:00+00:00 1970
true
{\"at\":\"2026-03-02T08:15:00+00:00\"}
";

#[test]
fn values_read_in_their_own_offset_whatever_the_server_zone() {
    for tz in ["UTC", "Europe/Paris", "America/New_York", "Asia/Kolkata"] {
        for vm in [false, true] {
            let tag = tz.replace('/', "_");
            assert_eq!(
                run(ZONE_INDEPENDENT, tz, vm, &tag),
                EXPECTED,
                "TZ={tz} vm={vm}"
            );
        }
    }
}

/// `.local` (and `DateTime.now`) is the one view that reads the server's
/// zone — and then `to_iso` says so, agreeing with `format` and `hour`.
#[test]
fn the_local_view_agrees_with_itself() {
    let source = r#"l = DateTime.from_unix(0).local
print(l.to_iso + " " + l.format("%H:%M") + " " + l.hour.to_s + " " + l.to_s)
print(l == DateTime.from_unix(0))
w = DateTime.parse("2026-07-01T12:00:00Z").local
print(w.to_iso + " " + w.format("%H %z"))
"#;
    for vm in [false, true] {
        assert_eq!(
            run(source, "Europe/Paris", vm, "local_paris"),
            "1970-01-01T01:00:00+01:00 01:00 1 1970-01-01 01:00:00\ntrue\n\
             2026-07-01T14:00:00+02:00 14 +0200\n",
            "vm={vm}"
        );
        assert_eq!(
            run(source, "America/New_York", vm, "local_ny"),
            "1969-12-31T19:00:00-05:00 19:00 19 1969-12-31 19:00:00\ntrue\n\
             2026-07-01T08:00:00-04:00 08 -0400\n",
            "vm={vm}"
        );
    }
}
