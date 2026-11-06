//! Streaming keeps memory bounded: a 200 MiB document is read with a peak
//! RSS growth well under 50 MiB, by the core's `Pull` and by `Xml.each` /
//! `Xml.reader` on both engines.
//!
//! Lives in its own test binary on purpose, with a single `#[test]`: the
//! peak RSS (`VmHWM`) is process-wide, so nothing else may run beside it.
//! Linux only (it reads `/proc/self/status`). Run with `--nocapture` to see
//! the numbers.

#![cfg(target_os = "linux")]

use std::io::{self, BufReader, Read, Write};

use solilang::xml::{Matcher, ParsePolicy, Pull};
use solilang::{run_script, Engine};

const MIB: u64 = 1024 * 1024;
const TOTAL: u64 = 200 * MIB;
const LIMIT: u64 = 50 * MIB;

const ITEM: &str = "<item id=\"42\" status=\"active\"><name>Product name</name>\
<price currency=\"EUR\">12.50</price><description>Lorem ipsum dolor sit amet, \
consectetur adipiscing elit, sed do eiusmod tempor.</description></item>\n";

/// A `/proc/self/status` field, in bytes.
fn status(field: &str) -> u64 {
    let text = std::fs::read_to_string("/proc/self/status").expect("read /proc/self/status");
    let line = text
        .lines()
        .find(|l| l.starts_with(field))
        .unwrap_or_else(|| panic!("no {field} in /proc/self/status"));
    let kb: u64 = line
        .split_whitespace()
        .nth(1)
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("bad {field} line: {line}"));
    kb * 1024
}

/// Reset the peak RSS (`VmHWM`) to the current RSS, where the kernel allows
/// it; otherwise the growth below is measured against the earlier peak.
fn reset_peak() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

/// Run `f` and return how much the peak RSS rose above the RSS before it.
fn peak_growth(what: &str, f: impl FnOnce()) -> u64 {
    reset_peak();
    let before = status("VmRSS:");
    let start = std::time::Instant::now();
    f();
    let growth = status("VmHWM:").saturating_sub(before);
    eprintln!(
        "{what}: peak RSS growth {:.1} MiB (RSS before {:.1} MiB), {:.2} s",
        growth as f64 / MIB as f64,
        before as f64 / MIB as f64,
        start.elapsed().as_secs_f64()
    );
    growth
}

/// `<catalog>`, then `ITEM` until about `total` bytes, then `</catalog>`,
/// produced on the fly.
struct Generated {
    head: &'static [u8],
    items_left: u64,
    pos: usize,
    tail_done: bool,
}

impl Generated {
    fn new(total: u64) -> Self {
        Generated {
            head: b"<?xml version=\"1.0\"?>\n<catalog>\n",
            items_left: total / ITEM.len() as u64,
            pos: 0,
            tail_done: false,
        }
    }

    fn items(total: u64) -> u64 {
        total / ITEM.len() as u64
    }
}

impl Read for Generated {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let current: &[u8] = if !self.head.is_empty() {
            self.head
        } else if self.items_left > 0 {
            ITEM.as_bytes()
        } else if !self.tail_done {
            b"</catalog>\n"
        } else {
            return Ok(0);
        };
        let rest = &current[self.pos..];
        let n = rest.len().min(out.len());
        out[..n].copy_from_slice(&rest[..n]);
        self.pos += n;
        if self.pos == current.len() {
            self.pos = 0;
            if !self.head.is_empty() {
                self.head = b"";
            } else if self.items_left > 0 {
                self.items_left -= 1;
            } else {
                self.tail_done = true;
            }
        }
        Ok(n)
    }
}

fn run(source: &str, engine: Engine) {
    if let Err(e) = run_script(source, None, false, engine) {
        panic!("{engine:?}: {e}");
    }
}

#[test]
fn streaming_200_mib_keeps_the_peak_rss_growth_under_50_mib() {
    std::env::set_var("SOLI_VM_OPTIONAL_LET", "1");
    let items = Generated::items(TOTAL);

    // The core, over a generated input (no disk).
    let growth = peak_growth("core Pull, 200 MiB generated", || {
        let matcher = Matcher::compile("item[@status='active']", &[]).expect("compile");
        let mut pull = Pull::new(
            BufReader::new(Generated::new(TOTAL)),
            matcher,
            ParsePolicy::xml(),
        );
        let mut count = 0u64;
        while let Some((tree, id)) = pull.next().expect("stream") {
            assert_eq!(tree.element(id).map(|e| e.local_name), Some("item"));
            count += 1;
        }
        assert_eq!(count, items);
    });
    assert!(growth < LIMIT, "core: peak RSS grew by {growth} bytes");

    // The binding, from a file, on both engines.
    let dir = std::env::temp_dir().join(format!("soli_xml_mem_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("catalog.xml");
    {
        let mut file = io::BufWriter::new(std::fs::File::create(&path).expect("create"));
        io::copy(&mut Generated::new(TOTAL), &mut file).expect("write the catalog");
        file.flush().expect("flush");
    }
    let path_str = path.display().to_string();
    for engine in [Engine::Vm, Engine::Tree] {
        let each = format!(
            r#"
total = 0
n = Xml.each({path_str:?}, "item[@status='active']") do |node|
  total = total + node["id"].to_i
end
throw "each yielded #{{n}}" unless n == {items}
throw "total #{{total}}" unless total == {items} * 42
"#
        );
        let growth = peak_growth(&format!("Xml.each on {engine:?}, 200 MiB file"), || {
            run(&each, engine)
        });
        assert!(growth < LIMIT, "{engine:?} Xml.each: peak RSS grew by {growth} bytes");

        let reader = format!(
            r#"
reader = Xml.reader({path_str:?}, "item")
k = 0
while (node = reader.next)
  k = k + 1
end
throw "reader gave #{{k}}" unless k == {items}
"#
        );
        let growth = peak_growth(&format!("Xml.reader on {engine:?}, 200 MiB file"), || {
            run(&reader, engine)
        });
        assert!(growth < LIMIT, "{engine:?} Xml.reader: peak RSS grew by {growth} bytes");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
