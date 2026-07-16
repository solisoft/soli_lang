//! Minimal, dependency-free progress bar for long-running CLI phases (the
//! `soli graph build` parse / embed / write passes, which can take a while on
//! big projects).
//!
//! Renders on **stderr** so stdout stays clean for piped output. On a TTY it
//! draws an in-place bar (throttled to avoid flicker); on a non-TTY it emits
//! sparse percentage milestones so CI logs show a heartbeat without spam.

use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

const BAR_WIDTH: usize = 24;
const MIN_REDRAW: Duration = Duration::from_millis(60);

pub struct ProgressBar {
    label: String,
    total: usize,
    done: usize,
    tty: bool,
    last_draw: Instant,
    started: bool,
    /// Last 25% bucket announced on a non-TTY.
    last_milestone: usize,
    /// Set once the line has been terminated (a newline emitted) so later
    /// output can't jam into the bar and `finish` is idempotent.
    finished: bool,
}

impl ProgressBar {
    pub fn new(label: &str) -> Self {
        ProgressBar {
            label: label.to_string(),
            total: 0,
            done: 0,
            tty: std::io::stderr().is_terminal(),
            last_draw: Instant::now(),
            started: false,
            last_milestone: 0,
            finished: false,
        }
    }

    /// Report progress. `total` may grow as it becomes known; a `total` of 0 is
    /// treated as "unknown" and draws nothing.
    pub fn set(&mut self, done: usize, total: usize) {
        if self.finished {
            return;
        }
        self.done = done;
        self.total = total;
        if total == 0 {
            return;
        }
        if self.tty {
            // Always draw the first and the final update; throttle the rest.
            let at_end = done >= total;
            if self.started && !at_end && self.last_draw.elapsed() < MIN_REDRAW {
                return;
            }
            self.last_draw = Instant::now();
            self.started = true;
            self.draw();
            // Terminate the line as soon as we hit 100% so any later phase
            // output (route loading, warnings) starts fresh below the bar
            // rather than jamming into it.
            if at_end {
                eprintln!();
                self.finished = true;
            }
        } else {
            let bucket = (done * 100 / total) / 25;
            if bucket > self.last_milestone {
                self.last_milestone = bucket;
                eprintln!("  {}… {}%", self.label, done * 100 / total);
            }
        }
    }

    fn draw(&self) {
        let filled = BAR_WIDTH * self.done / self.total;
        let pct = self.done * 100 / self.total;
        let bar: String = "█".repeat(filled) + &"░".repeat(BAR_WIDTH.saturating_sub(filled));
        eprint!(
            "\r  \x1b[1m{}\x1b[0m  [{}]  {}/{}  {:>3}%",
            self.label, bar, self.done, self.total, pct
        );
        let _ = std::io::stderr().flush();
    }

    /// Complete the bar and move to a fresh line. Idempotent, and a no-op on
    /// output when the bar already self-terminated at 100% or nothing was ever
    /// drawn (e.g. an empty graph).
    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        if self.tty {
            if self.started {
                self.draw();
                eprintln!();
            }
        } else if self.total > 0 {
            eprintln!("  {}: done ({})", self.label, self.total.max(self.done));
        }
        self.finished = true;
    }
}
