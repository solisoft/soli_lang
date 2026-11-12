//! Engine hooks shared by the tree-walker and the VM.
//!
//! Both engines report the same three events here: a function frame was
//! entered ([`enter`]), frames were left ([`return_to`]), and a statement is
//! about to run ([`on_line`]). What listens to them:
//!
//! * the dev-bar flamegraph (`serve::span_log`), which gets one `fn` span per
//!   Soli call — from either engine, so a VM-run action shows its functions;
//! * the sampling profiler ([`profiler`]), which reads the logical Soli stack
//!   kept here whenever a sample is due.
//!
//! A future step debugger hangs off the same points.
//!
//! # Cost when nothing listens
//!
//! Every call site checks [`active`] first. It reads one process-wide atomic
//! (`EVER_ON`) that only becomes true once some thread turned a listener on, so
//! a production worker — where nothing ever does — pays one relaxed load and a
//! never-taken branch per call and per statement. This is the pattern
//! `span_log::is_enabled` already uses.
//!
//! # Pairing
//!
//! [`enter`] returns a mark (the logical stack's length before the push), and
//! [`return_to`] truncates back to a mark. An engine keeps the mark with its
//! frame and hands it back when the frame goes, so a frame an error unwound
//! without its own exit hook is closed by the next exit below it, rather than
//! staying on the stack forever.

#[cfg(test)]
mod engine_tests;
pub mod profiler;
pub mod session;

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};

/// Bit of the per-thread listener mask: the flamegraph span log is on.
pub const FLAME: u8 = 1;
/// Bit of the per-thread listener mask: this thread runs the program a step
/// debugger is attached to ([`session`]). Only that thread ever stops.
pub const DEBUGGEE: u8 = 2;

// Process-wide: has any listener ever been switched on, on any thread? Read
// first by `active`, so an uninstrumented process answers with one load.
static EVER_ON: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Which listeners are on for this thread (`FLAME`, …). Set once per
    /// worker at boot, like `span_log`'s flag. The profiler is process-wide
    /// and does not use it: it samples every engine thread.
    static MASK: Cell<u8> = const { Cell::new(0) };
}

crate::request_local! {
    /// The logical Soli call stack, innermost last. Only kept while a
    /// listener is on. Request state: a worker interleaving requests keeps
    /// one stack per request, as `span_log`'s span stack it drives does.
    static STACK: RefCell<Vec<Frame>> = const { RefCell::new(Vec::new()) };
}

/// One Soli function frame on the logical stack.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Frame {
    /// The function's name; empty for a lambda or block.
    pub name: String,
    /// The file the function is defined in, when known.
    pub file: Option<String>,
    /// The line of its definition (0 when unknown).
    pub line: u32,
    /// `span_log::push_fn` was called for it, so leaving it pops a span.
    flame: bool,
}

impl Frame {
    /// The name to show: `<anonymous>` for a lambda or block.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            "<anonymous>"
        } else {
            &self.name
        }
    }
}

/// Record that a process-wide or thread listener is on. Never cleared: a
/// listener that stops leaves `active` to the cheaper checks behind it.
pub(crate) fn mark_ever_on() {
    EVER_ON.store(true, Ordering::Relaxed);
}

/// Switch one listener bit on or off for the calling thread.
pub fn set_thread_listener(bit: u8, on: bool) {
    if on {
        mark_ever_on();
    }
    MASK.with(|m| {
        let mask = m.get();
        m.set(if on { mask | bit } else { mask & !bit });
    });
}

/// Is anything listening to the engine hooks on this thread?
///
/// The check every call and statement makes; the first load is enough in a
/// process that never turned a listener on.
#[inline(always)]
pub fn active() -> bool {
    EVER_ON.load(Ordering::Relaxed) && active_slow()
}

#[inline(never)]
fn active_slow() -> bool {
    profiler::is_running() || MASK.with(|m| m.get()) != 0
}

/// A Soli function frame was entered. Returns the mark to hand to
/// [`return_to`] when it is left. Callers check [`active`] first.
pub fn enter(name: &str, file: Option<&str>, line: usize) -> usize {
    profiler::poll();
    let flame = crate::serve::span_log::is_enabled();
    if flame {
        let meta = file.map(|path| {
            if line > 0 {
                format!("{}:{}", path, line)
            } else {
                path.to_string()
            }
        });
        crate::serve::span_log::push_fn(name, meta);
    }
    STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        let mark = stack.len();
        stack.push(Frame {
            name: name.to_string(),
            file: file.map(str::to_string),
            line: u32::try_from(line).unwrap_or(u32::MAX),
            flame,
        });
        mark
    })
}

/// Leave every frame entered since `mark` was returned, innermost first.
pub fn return_to(mark: usize) {
    profiler::poll();
    loop {
        let popped = STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.len() > mark {
                stack.pop()
            } else {
                None
            }
        });
        match popped {
            Some(frame) => {
                if frame.flame {
                    crate::serve::span_log::pop_fn();
                }
            }
            None => break,
        }
    }
}

/// A statement is about to run. Callers check [`active`] first.
#[inline]
pub fn on_line() {
    profiler::poll();
}

/// Is the calling thread the one a step debugger stops? Callers check
/// [`active`] first.
#[inline]
pub fn is_debuggee() -> bool {
    session::attached() && MASK.with(|m| m.get()) & DEBUGGEE != 0
}

/// The depth of the calling thread's logical stack.
pub fn depth() -> usize {
    STACK.with(|stack| stack.borrow().len())
}

/// A copy of the calling thread's logical stack, outermost first.
pub fn stack_snapshot() -> Vec<Frame> {
    STACK.with(|stack| stack.borrow().clone())
}

/// Run `f` over the calling thread's logical stack, outermost first.
pub(crate) fn with_stack<R>(f: impl FnOnce(&[Frame]) -> R) -> R {
    STACK.with(|stack| f(&stack.borrow()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_until_a_listener_is_on() {
        // No profile runs while this holds the lock, and a fresh thread has
        // no listener bit.
        let _serial = profiler::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::thread::spawn(|| {
            assert!(!active_slow());
            set_thread_listener(FLAME, true);
            assert!(active());
            set_thread_listener(FLAME, false);
            assert!(!active_slow());
        })
        .join()
        .unwrap();
    }

    #[test]
    fn return_to_closes_frames_an_unwind_skipped() {
        std::thread::spawn(|| {
            let outer = enter("outer", Some("a.sl"), 1);
            let _inner = enter("inner", Some("a.sl"), 5);
            let _lost = enter("", None, 0);
            assert_eq!(depth(), 3);
            assert_eq!(stack_snapshot()[2].display_name(), "<anonymous>");
            // The outer frame's exit closes the two an error left open.
            return_to(outer);
            assert_eq!(depth(), 0);
        })
        .join()
        .unwrap();
    }
}
