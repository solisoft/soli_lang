//! Sampling profiler for `soli run --profile` and `soli test --profile`.
//!
//! A sampler thread advances a process-wide clock, `TICK`, `hz` times a
//! second. The engine threads do not get interrupted: they *poll* — on every
//! function entry and exit, on every tree-walker statement, and at the
//! handler-deadline checks the VM already makes on its backward jumps — and
//! when the clock moved since their last poll, they charge the elapsed ticks
//! to their current Soli stack (the one [`super`] keeps).
//!
//! Weighting a sample by the ticks it covers keeps the totals honest even when
//! polls are sparse: a thread that spent 40 ms in one native call reports
//! 40 ms, charged to the stack that made the call. The price is the usual
//! safepoint bias — time lands on the frame that was running at the next poll,
//! not at the instant the tick fired — and the clock is wall time, so a script
//! waiting on the database shows that wait.
//!
//! Function calls a native kernel answers never open a frame (on either
//! engine), so their time is charged to the caller.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use web_time::{Duration, Instant};

/// Samples per second when `--profile-hz` is not given.
pub const DEFAULT_HZ: u32 = 1000;
/// The highest `--profile-hz` accepted: past it the sampler thread's sleeps
/// are shorter than the scheduler's resolution and the clock only gets noisier.
pub const MAX_HZ: u32 = 10_000;

/// The name of the frame every stack starts from: the top level of a script
/// or spec file, outside any function.
pub const ROOT_NAME: &str = "<main>";

static RUNNING: AtomicBool = AtomicBool::new(false);
/// The running profile weighs bytes allocated, not time (`--profile=alloc`).
static ALLOC_MODE: AtomicBool = AtomicBool::new(false);
/// Read by the process's global allocator (`note_alloc`): count the bytes
/// each thread allocates. Off unless an allocation profile runs.
static COUNTING: AtomicBool = AtomicBool::new(false);
/// Ticks since an arbitrary origin; only ever grows, across runs too.
static TICK: AtomicU64 = AtomicU64::new(0);
/// `TICK` when the current run started. A thread whose last poll is older
/// belongs to a previous run and starts afresh instead of charging the gap.
static START_TICK: AtomicU64 = AtomicU64::new(0);
static DATA: Mutex<Option<Collected>> = Mutex::new(None);

/// What a thread's `LAST` holds before its first poll.
const UNSEEDED: u64 = u64::MAX;

thread_local! {
    /// Bytes this thread allocated since its last poll, while counting. A
    /// `const` cell with no destructor, so the allocator may touch it at any
    /// point of a thread's life.
    static ALLOCATED: Cell<u64> = const { Cell::new(0) };
    /// `TICK` at this thread's last poll.
    static LAST: Cell<u64> = const { Cell::new(UNSEEDED) };
    /// The file this thread is running, when it runs one of several (a
    /// `soli test` worker): its stacks start from `<main> (file)`.
    static THREAD_ENTRY: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Stop charging the calling thread's time until it next runs Soli code: for
/// a thread that only coordinates others (the `soli test` runner's), whose
/// wait would otherwise land on `<main>` when the profile finishes.
pub fn detach_thread() {
    LAST.with(|l| l.set(UNSEEDED));
}

/// Name the file the calling thread runs from now on (`None`: none in
/// particular). `soli test` calls it per spec file, so a profile tells the
/// files' top levels apart. The time so far is charged first, to the file
/// it was spent in.
pub fn set_thread_entry(file: Option<&str>) {
    if !is_running() {
        return;
    }
    poll();
    THREAD_ENTRY.with(|entry| *entry.borrow_mut() = file.map(str::to_string));
}

/// One distinct frame of the profile.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FrameKey {
    /// The name shown: `<anonymous>` for a block or lambda, [`ROOT_NAME`] for
    /// the top level.
    pub name: String,
    pub file: Option<String>,
    pub line: u32,
}

impl FrameKey {
    fn root() -> Self {
        FrameKey {
            name: ROOT_NAME.to_string(),
            file: None,
            line: 0,
        }
    }

    /// `name (file:line)`, the file relative to the working directory.
    pub fn label(&self) -> String {
        match &self.file {
            Some(file) if self.line > 0 => {
                format!("{} ({}:{})", self.name, display_path(file), self.line)
            }
            Some(file) => format!("{} ({})", self.name, display_path(file)),
            None => self.name.clone(),
        }
    }
}

/// `path` relative to the working directory when it lies under it.
fn display_path(path: &str) -> String {
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(rest) = std::path::Path::new(path).strip_prefix(&cwd) {
            return rest.to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

#[derive(Default)]
struct Collected {
    frames: Vec<FrameKey>,
    index: HashMap<FrameKey, u32>,
    /// Frame ids, outermost first → ticks charged to that exact stack.
    stacks: HashMap<Vec<u32>, u64>,
    /// The file of a frame that names none: the script being profiled. The
    /// tree-walker leaves a script's own functions without a path.
    entry_file: Option<String>,
}

impl Collected {
    fn intern(&mut self, key: FrameKey) -> u32 {
        if let Some(&id) = self.index.get(&key) {
            return id;
        }
        let id = self.frames.len() as u32;
        self.frames.push(key.clone());
        self.index.insert(key, id);
        id
    }
}

/// Called by the global allocator for every allocation (and every growing
/// reallocation) of `size` bytes. One relaxed load when no allocation
/// profile runs; it records nothing itself, it only adds to the thread's
/// count, which the next poll charges to the Soli stack.
#[inline(always)]
pub fn note_alloc(size: usize) {
    if COUNTING.load(Ordering::Relaxed) {
        let _ = ALLOCATED.try_with(|count| count.set(count.get().wrapping_add(size as u64)));
    }
}

/// Is a profile being collected?
#[inline(always)]
pub fn is_running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// Charge the ticks since this thread's last poll to its current stack.
/// One relaxed load when no profile runs.
#[inline(always)]
pub fn poll() {
    if is_running() {
        poll_slow();
    }
}

#[cold]
#[inline(never)]
fn poll_slow() {
    if ALLOC_MODE.load(Ordering::Relaxed) {
        let bytes = ALLOCATED.with(|count| count.replace(0));
        if bytes > 0 {
            record(bytes);
            // What recording itself allocated is the profiler's, not the
            // program's.
            ALLOCATED.with(|count| count.set(0));
        }
        return;
    }
    let now = TICK.load(Ordering::Relaxed);
    let last = LAST.with(|l| l.replace(now));
    if last == now || last == UNSEEDED || last > now || last < START_TICK.load(Ordering::Relaxed) {
        return;
    }
    record(now - last);
}

/// Add `weight` ticks to the calling thread's current stack.
fn record(weight: u64) {
    super::with_stack(|frames| {
        let Ok(mut guard) = DATA.lock() else {
            return;
        };
        let Some(data) = guard.as_mut() else {
            return;
        };
        let thread_entry = THREAD_ENTRY.with(|entry| entry.borrow().clone());
        let fallback_file = thread_entry.clone().or_else(|| data.entry_file.clone());
        let mut ids = Vec::with_capacity(frames.len() + 1);
        ids.push(data.intern(FrameKey {
            file: thread_entry,
            ..FrameKey::root()
        }));
        for frame in frames {
            ids.push(data.intern(FrameKey {
                name: frame.display_name().to_string(),
                file: frame.file.clone().or_else(|| fallback_file.clone()),
                line: frame.line,
            }));
        }
        *data.stacks.entry(ids).or_insert(0) += weight;
    });
}

/// What a profile's weights count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// Sampling ticks: time.
    Ticks,
    /// Bytes allocated (`--profile=alloc`).
    Bytes,
}

/// A running profile. [`Profiler::finish`] stops it and returns the report.
pub struct Profiler {
    hz: u32,
    unit: Unit,
    started: Instant,
    #[cfg(not(target_arch = "wasm32"))]
    sampler: Option<std::thread::JoinHandle<()>>,
}

/// Start collecting at `hz` samples a second, on every engine thread of the
/// process. `None` when a profile is already running.
pub fn start(hz: u32) -> Option<Profiler> {
    start_with_entry(hz, None)
}

/// [`start`], charging frames that carry no file to `entry_file` — the
/// script being profiled, whose own functions the tree-walker leaves without
/// a path while the VM records it.
pub fn start_with_entry(hz: u32, entry_file: Option<&str>) -> Option<Profiler> {
    let hz = hz.clamp(1, MAX_HZ);
    {
        let mut data = DATA.lock().ok()?;
        if data.is_some() {
            return None;
        }
        *data = Some(Collected {
            entry_file: entry_file.map(str::to_string),
            ..Collected::default()
        });
    }
    let base = TICK.load(Ordering::Relaxed);
    START_TICK.store(base, Ordering::Relaxed);
    // The starting thread charges from now, not from its first poll.
    LAST.with(|l| l.set(base));
    super::mark_ever_on();
    RUNNING.store(true, Ordering::Relaxed);
    let started = Instant::now();

    #[cfg(not(target_arch = "wasm32"))]
    let sampler = {
        let period_ns = 1_000_000_000u64 / u64::from(hz);
        std::thread::Builder::new()
            .name("soli-profiler".to_string())
            .spawn(move || {
                let origin = std::time::Instant::now();
                while RUNNING.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_nanos(period_ns));
                    // From the elapsed time, not one tick per wake-up: a
                    // late wake-up (the scheduler's, not the program's) still
                    // advances the clock by the time that really passed.
                    let elapsed = origin.elapsed().as_nanos() as u64;
                    TICK.store(base + elapsed / period_ns, Ordering::Relaxed);
                }
            })
            .ok()
    };

    Some(Profiler {
        hz,
        unit: Unit::Ticks,
        started,
        #[cfg(not(target_arch = "wasm32"))]
        sampler,
    })
}

/// Start an allocation profile: every byte the program's threads allocate
/// is charged to the Soli stack that allocated it (as of the next call,
/// return or statement). `None` when a profile is already running.
pub fn start_alloc(entry_file: Option<&str>) -> Option<Profiler> {
    {
        let mut data = DATA.lock().ok()?;
        if data.is_some() {
            return None;
        }
        *data = Some(Collected {
            entry_file: entry_file.map(str::to_string),
            ..Collected::default()
        });
    }
    ALLOCATED.with(|count| count.set(0));
    ALLOC_MODE.store(true, Ordering::Relaxed);
    super::mark_ever_on();
    RUNNING.store(true, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    Some(Profiler {
        hz: DEFAULT_HZ,
        unit: Unit::Bytes,
        started: Instant::now(),
        #[cfg(not(target_arch = "wasm32"))]
        sampler: None,
    })
}

impl Profiler {
    /// Stop sampling and return what was collected. The calling thread's
    /// last stretch is charged first.
    pub fn finish(self) -> Report {
        poll();
        COUNTING.store(false, Ordering::Relaxed);
        RUNNING.store(false, Ordering::Relaxed);
        ALLOC_MODE.store(false, Ordering::Relaxed);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(handle) = self.sampler {
            let _ = handle.join();
        }
        let collected = DATA
            .lock()
            .ok()
            .and_then(|mut data| data.take())
            .unwrap_or_default();
        let mut stacks: Vec<(Vec<u32>, u64)> = collected.stacks.into_iter().collect();
        stacks.sort();
        Report {
            unit: self.unit,
            frames: collected.frames,
            stacks,
            period: Duration::from_nanos(1_000_000_000u64 / u64::from(self.hz)),
            elapsed: self.started.elapsed(),
        }
    }
}

/// A finished profile.
#[derive(Debug, Clone)]
pub struct Report {
    /// What the weights count: time ticks, or bytes allocated.
    pub unit: Unit,
    /// Every distinct frame; a stack refers to them by index.
    pub frames: Vec<FrameKey>,
    /// Each distinct stack (frame ids, outermost first) and the ticks it got.
    pub stacks: Vec<(Vec<u32>, u64)>,
    /// What one tick stands for.
    pub period: Duration,
    /// Wall time from start to finish.
    pub elapsed: Duration,
}

/// Self and total time of one function, in ticks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionTime {
    pub frame: u32,
    pub self_ticks: u64,
    pub total_ticks: u64,
}

impl Report {
    /// Ticks recorded across every stack.
    pub fn total_ticks(&self) -> u64 {
        self.stacks.iter().map(|(_, ticks)| ticks).sum()
    }

    fn ms(&self, ticks: u64) -> f64 {
        self.period.as_secs_f64() * 1000.0 * ticks as f64
    }

    /// Brendan Gregg's folded-stack format, one `a;b;c ticks` line per
    /// stack — the input of `flamegraph.pl`, inferno, and speedscope.
    pub fn to_folded(&self) -> String {
        let mut out = String::new();
        for (ids, ticks) in &self.stacks {
            let labels: Vec<String> = ids
                .iter()
                .map(|&id| self.frames[id as usize].label().replace(';', ","))
                .collect();
            out.push_str(&labels.join(";"));
            out.push(' ');
            out.push_str(&ticks.to_string());
            out.push('\n');
        }
        out
    }

    /// A speedscope "sampled" profile (https://www.speedscope.app), weights
    /// in milliseconds.
    pub fn to_speedscope(&self, name: &str) -> String {
        let frames: Vec<serde_json::Value> = self
            .frames
            .iter()
            .map(|frame| {
                let mut entry = serde_json::json!({ "name": frame.name });
                if let Some(file) = &frame.file {
                    entry["file"] = serde_json::Value::String(display_path(file));
                    if frame.line > 0 {
                        entry["line"] = serde_json::Value::from(frame.line);
                    }
                }
                entry
            })
            .collect();
        let samples: Vec<&Vec<u32>> = self.stacks.iter().map(|(ids, _)| ids).collect();
        let weight = |ticks: u64| match self.unit {
            Unit::Ticks => self.ms(ticks),
            Unit::Bytes => ticks as f64,
        };
        let weights: Vec<f64> = self
            .stacks
            .iter()
            .map(|(_, ticks)| weight(*ticks))
            .collect();
        let unit = match self.unit {
            Unit::Ticks => "milliseconds",
            Unit::Bytes => "bytes",
        };
        let doc = serde_json::json!({
            "$schema": "https://www.speedscope.app/file-format-schema.json",
            "exporter": format!("soli {}", env!("CARGO_PKG_VERSION")),
            "name": name,
            "activeProfileIndex": 0,
            "shared": { "frames": frames },
            "profiles": [{
                "type": "sampled",
                "name": name,
                "unit": unit,
                "startValue": 0,
                "endValue": weight(self.total_ticks()),
                "samples": samples,
                "weights": weights,
            }],
        });
        doc.to_string()
    }

    /// Per-function self and total ticks, heaviest self time first. A
    /// recursive function's total counts each stack once.
    pub fn functions(&self) -> Vec<FunctionTime> {
        let mut by_frame: HashMap<u32, FunctionTime> = HashMap::new();
        for (ids, ticks) in &self.stacks {
            let mut seen: Vec<u32> = Vec::with_capacity(ids.len());
            for &id in ids {
                if seen.contains(&id) {
                    continue;
                }
                seen.push(id);
                by_frame
                    .entry(id)
                    .or_insert(FunctionTime {
                        frame: id,
                        self_ticks: 0,
                        total_ticks: 0,
                    })
                    .total_ticks += ticks;
            }
            if let Some(&leaf) = ids.last() {
                if let Some(entry) = by_frame.get_mut(&leaf) {
                    entry.self_ticks += ticks;
                }
            }
        }
        let mut functions: Vec<FunctionTime> = by_frame.into_values().collect();
        functions.sort_by(|a, b| {
            b.self_ticks
                .cmp(&a.self_ticks)
                .then(b.total_ticks.cmp(&a.total_ticks))
                .then(a.frame.cmp(&b.frame))
        });
        functions
    }

    /// The `limit` heaviest functions by self time, as a table.
    pub fn top_table(&self, limit: usize) -> String {
        let total = self.total_ticks().max(1);
        let (self_head, total_head) = match self.unit {
            Unit::Ticks => ("self ms", "total ms"),
            Unit::Bytes => ("self KB", "total KB"),
        };
        let amount = |ticks: u64| match self.unit {
            Unit::Ticks => self.ms(ticks),
            Unit::Bytes => ticks as f64 / 1024.0,
        };
        let mut out = format!(
            "{:>10} {:>6} {:>10} {:>6}  function\n",
            self_head, "self%", total_head, "total%"
        );
        for function in self.functions().into_iter().take(limit) {
            out.push_str(&format!(
                "{:>10.1} {:>5.1}% {:>10.1} {:>5.1}%  {}\n",
                amount(function.self_ticks),
                100.0 * function.self_ticks as f64 / total as f64,
                amount(function.total_ticks),
                100.0 * function.total_ticks as f64 / total as f64,
                self.frames[function.frame as usize].label(),
            ));
        }
        out
    }
}

/// Serializes the tests that start the process-wide profiler.
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(name: &str, line: u32) -> FrameKey {
        FrameKey {
            name: name.to_string(),
            file: Some("/nowhere/app.sl".to_string()),
            line,
        }
    }

    fn sample_report() -> Report {
        Report {
            unit: Unit::Ticks,
            frames: vec![FrameKey::root(), frame("outer", 1), frame("inner", 5)],
            stacks: vec![(vec![0, 1], 2), (vec![0, 1, 2], 6), (vec![0], 2)],
            period: Duration::from_millis(1),
            elapsed: Duration::from_millis(10),
        }
    }

    #[test]
    fn folded_lines_carry_the_stack_and_its_ticks() {
        let folded = sample_report().to_folded();
        assert!(folded.contains("<main>;outer (/nowhere/app.sl:1) 2\n"));
        assert!(folded.contains("<main>;outer (/nowhere/app.sl:1);inner (/nowhere/app.sl:5) 6\n"));
        assert!(folded.contains("<main> 2\n"));
    }

    #[test]
    fn self_and_total_time_per_function() {
        let report = sample_report();
        let functions = report.functions();
        assert_eq!(functions[0].frame, 2, "inner has the most self time");
        assert_eq!(functions[0].self_ticks, 6);
        let outer = functions.iter().find(|f| f.frame == 1).unwrap();
        assert_eq!((outer.self_ticks, outer.total_ticks), (2, 8));
        let root = functions.iter().find(|f| f.frame == 0).unwrap();
        assert_eq!(root.total_ticks, 10);
        assert!(report.top_table(5).contains("inner (/nowhere/app.sl:5)"));
    }

    /// `--profile=alloc`: bytes the allocator reports are charged to the
    /// Soli stack that was running when they were allocated.
    #[test]
    fn an_allocation_profile_charges_bytes_to_the_stack() {
        let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::thread::spawn(|| {
            let profiler = start_alloc(Some("/nowhere/app.sl")).expect("no profile running");
            note_alloc(100);
            let mark = super::super::enter("make_rows", Some("/nowhere/app.sl"), 3);
            note_alloc(4096);
            super::super::return_to(mark);
            let report = profiler.finish();
            assert_eq!(report.unit, Unit::Bytes);
            let functions = report.functions();
            let make = functions
                .iter()
                .find(|f| report.frames[f.frame as usize].name == "make_rows")
                .expect("make_rows is in the profile");
            assert!(make.self_ticks >= 4096, "{}", make.self_ticks);
            assert!(report.top_table(5).contains("self KB"));
            assert!(report.to_speedscope("x").contains("\"unit\":\"bytes\""));
            // Counting stops with the profile.
            note_alloc(1 << 20);
            assert_eq!(ALLOCATED.with(|c| c.get()), 0);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn recursion_counts_a_stack_once_in_the_total() {
        let report = Report {
            unit: Unit::Ticks,
            frames: vec![FrameKey::root(), frame("fib", 1)],
            stacks: vec![(vec![0, 1, 1, 1], 4)],
            period: Duration::from_millis(1),
            elapsed: Duration::from_millis(4),
        };
        let fib = report
            .functions()
            .into_iter()
            .find(|f| f.frame == 1)
            .unwrap();
        assert_eq!((fib.self_ticks, fib.total_ticks), (4, 4));
    }

    #[test]
    fn speedscope_document_is_a_sampled_profile() {
        let doc: serde_json::Value =
            serde_json::from_str(&sample_report().to_speedscope("app.sl")).unwrap();
        let profile = &doc["profiles"][0];
        assert_eq!(profile["type"], "sampled");
        assert_eq!(profile["unit"], "milliseconds");
        assert_eq!(profile["samples"].as_array().unwrap().len(), 3);
        assert_eq!(profile["weights"].as_array().unwrap().len(), 3);
        assert_eq!(doc["shared"]["frames"][2]["name"], "inner");
        assert_eq!(doc["shared"]["frames"][2]["line"], 5);
        assert_eq!(profile["endValue"].as_f64().unwrap(), 10.0);
    }

    #[test]
    fn a_running_profile_charges_ticks_to_the_current_stack() {
        let _serial = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let handle = std::thread::spawn(|| {
            let profiler = start(1000).expect("no other profile runs");
            assert!(start(1000).is_none(), "one profile at a time");
            let mark = super::super::enter("busy", Some("/x/busy.sl"), 3);
            let until = Instant::now() + Duration::from_millis(30);
            while Instant::now() < until {
                poll();
            }
            super::super::return_to(mark);
            profiler.finish()
        });
        let report = handle.join().unwrap();
        assert!(!is_running());
        let busy = report
            .frames
            .iter()
            .position(|f| f.name == "busy")
            .expect("the busy frame was sampled") as u32;
        let function = report
            .functions()
            .into_iter()
            .find(|f| f.frame == busy)
            .unwrap();
        assert!(function.self_ticks >= 10, "{:?}", report);
    }
}
