//! The step debugger's shared state: breakpoints, the pause request, and the
//! channels between the engine thread and the adapter (`soli dap`).
//!
//! The engine thread runs the program. At every statement it asks
//! [`should_stop`] whether to stop there (a breakpoint, a step that
//! finished, a pause request, or `debug()`); when it stops, it sends
//! [`Event::Stopped`] and then serves [`Command`]s until one resumes it.
//! Everything that looks at the paused program — the stack, scopes,
//! variables, `evaluate` — arrives as a [`Command::Inspect`] closure and runs
//! on the engine thread, which is the only one that may touch its values
//! (they are `Rc`s).
//!
//! The adapter thread owns the protocol. It edits breakpoints and requests a
//! pause directly here (both are thread-safe), and sends commands only while
//! the engine is stopped.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};

/// What the engine runs while paused.
pub type InspectFn =
    Box<dyn FnOnce(&mut dyn crate::interpreter::executor::debugger::Paused) + Send>;

/// Sent by the adapter to a stopped engine.
pub enum Command {
    Continue,
    /// Step over: stop at the next statement of this frame or a caller.
    Next,
    /// Step in: stop at the very next statement.
    StepIn,
    /// Step out: stop at the next statement of a caller.
    StepOut,
    /// Look at the paused program, on the engine thread.
    Inspect(InspectFn),
}

/// Sent by the engine to the adapter.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Stopped; `reason` is a DAP stop reason (`breakpoint`, `step`, `pause`,
    /// `entry`), `description` what to show.
    Stopped {
        reason: &'static str,
        description: String,
    },
    /// The program finished; `error` is the message of an uncaught error.
    Finished { error: Option<String> },
}

/// How the engine should behave between two stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepMode {
    /// Run until a breakpoint.
    Run,
    /// Stop at the next statement anywhere.
    In,
    /// Stop at the next statement at this depth or shallower.
    Over(usize),
    /// Stop at the next statement shallower than this depth.
    Out(usize),
}

/// One debugging session: there is at most one per process.
pub struct Session {
    /// Breakpoint lines by file, the file as the engine reports it
    /// (absolute, see [`normalize_path`]).
    breakpoints: Mutex<HashMap<String, BTreeSet<u32>>>,
    pause: AtomicBool,
    commands: Mutex<Receiver<Command>>,
    events: Sender<Event>,
}

static SESSION: OnceLock<Arc<Session>> = OnceLock::new();
static ATTACHED: AtomicBool = AtomicBool::new(false);
/// An editor is connected and authorized. `soli dap` has one from the start;
/// `soli serve --inspect` only while an editor is attached. Without one,
/// nothing stops.
static CLIENT: AtomicBool = AtomicBool::new(false);

/// One stopped thread at a time: a second worker reaching a breakpoint
/// waits here until the first resumes.
static STOP_LOCK: Mutex<()> = Mutex::new(());

/// Note whether an editor is connected.
pub fn set_client_connected(connected: bool) {
    CLIENT.store(connected, Ordering::Release);
}

/// Is an editor connected?
pub fn client_connected() -> bool {
    CLIENT.load(Ordering::Acquire)
}

/// Taken by a thread for as long as it is stopped.
pub(crate) fn stop_lock() -> std::sync::MutexGuard<'static, ()> {
    STOP_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Forget every breakpoint (the editor went away).
pub fn clear_breakpoints() {
    if let Some(session) = session() {
        session
            .breakpoints
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

/// Open the process's debugging session. The adapter keeps the command
/// sender and the event receiver. Fails if one is already open.
pub fn open() -> Result<(Sender<Command>, Receiver<Event>), String> {
    let (command_tx, command_rx) = std::sync::mpsc::channel();
    let (event_tx, event_rx) = std::sync::mpsc::channel();
    let session = Arc::new(Session {
        breakpoints: Mutex::new(HashMap::new()),
        pause: AtomicBool::new(false),
        commands: Mutex::new(command_rx),
        events: event_tx,
    });
    SESSION
        .set(session)
        .map_err(|_| "a debugging session is already open".to_string())?;
    ATTACHED.store(true, Ordering::Release);
    Ok((command_tx, event_rx))
}

/// Is a debugger attached to this process? One relaxed load.
#[inline(always)]
pub fn attached() -> bool {
    ATTACHED.load(Ordering::Relaxed)
}

fn session() -> Option<&'static Arc<Session>> {
    SESSION.get()
}

/// A path as both sides compare it: absolute, `.` and `..` resolved
/// lexically, symlinks kept (what the engine records for a file).
pub fn normalize_path(path: &str) -> String {
    let path = std::path::Path::new(path);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|dir| dir.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let mut out = std::path::PathBuf::new();
    for part in absolute.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out.to_string_lossy().into_owned()
}

/// Replace the breakpoints of `file` (the DAP `setBreakpoints` semantics).
pub fn set_breakpoints(file: &str, lines: impl IntoIterator<Item = u32>) {
    if let Some(session) = session() {
        let mut all = session
            .breakpoints
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let lines: BTreeSet<u32> = lines.into_iter().collect();
        let file = normalize_path(file);
        if lines.is_empty() {
            all.remove(&file);
        } else {
            all.insert(file, lines);
        }
    }
}

/// Ask the running program to stop at its next statement.
pub fn request_pause() {
    if let Some(session) = session() {
        session.pause.store(true, Ordering::Release);
    }
}

/// Where the engine is, for [`should_stop`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub file: String,
    pub line: u32,
    pub depth: usize,
}

/// The decision at one statement: `None` to run on, or the stop reason.
///
/// `last` is where the previous statement was: a line holding several
/// statements (or a statement the engine reports twice) stops once.
pub fn should_stop(
    here: &Location,
    last: Option<&Location>,
    mode: StepMode,
) -> Option<&'static str> {
    let session = session()?;
    if !client_connected() {
        return None;
    }
    if session.pause.swap(false, Ordering::AcqRel) {
        return Some("pause");
    }
    let moved = last != Some(here);
    if !moved {
        return None;
    }
    let stepped = match mode {
        StepMode::Run => false,
        StepMode::In => true,
        StepMode::Over(depth) => here.depth <= depth,
        StepMode::Out(depth) => here.depth < depth,
    };
    if stepped {
        return Some("step");
    }
    let on_breakpoint = session
        .breakpoints
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&here.file)
        .is_some_and(|lines| lines.contains(&here.line));
    if on_breakpoint {
        // A breakpoint fires when the line is reached, not again for each
        // statement on it.
        let same_line = last.is_some_and(|l| l.file == here.file && l.line == here.line);
        if !same_line {
            return Some("breakpoint");
        }
    }
    None
}

/// Tell the adapter something happened.
pub fn send(event: Event) {
    if let Some(session) = session() {
        let _ = session.events.send(event);
    }
}

/// End the program with `code`. With a debugger attached, the process is
/// the adapter's: the program's thread reports the end (so the editor gets
/// `exited` and `terminated`) and waits for the editor to disconnect, which
/// ends the process. Without one, `std::process::exit`.
///
/// For code that would otherwise exit in the middle of a debugged run —
/// `soli test`'s runner, launched by `soli dap`.
pub fn exit(code: i32) -> ! {
    if attached() && session().is_some() {
        use std::io::Write;
        let _ = std::io::stdout().flush();
        send(Event::Finished {
            error: (code != 0).then(|| format!("exited with status {code}")),
        });
        loop {
            std::thread::park();
        }
    }
    std::process::exit(code)
}

/// Wait for the adapter's next command. `None` when the adapter is gone: the
/// program then runs to its end.
pub fn next_command() -> Option<Command> {
    session()?
        .commands
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .recv()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: u32, depth: usize) -> Location {
        Location {
            file: "/app/main.sl".into(),
            line,
            depth,
        }
    }

    #[test]
    fn paths_are_compared_absolute_and_lexically_resolved() {
        assert_eq!(normalize_path("/a/./b/../c.sl"), "/a/c.sl");
        assert!(normalize_path("x.sl").ends_with("/x.sl"));
    }

    #[test]
    fn step_modes_stop_where_they_should() {
        // One session for the whole test binary; these checks do not need
        // breakpoints, and an open session is harmless to other tests (it
        // only answers `attached()` for code that asks).
        let _ = open();
        set_client_connected(true);
        let last = at(3, 2);
        assert_eq!(should_stop(&at(4, 2), Some(&last), StepMode::Run), None);
        assert_eq!(
            should_stop(&at(9, 3), Some(&last), StepMode::In),
            Some("step")
        );
        assert_eq!(should_stop(&at(9, 3), Some(&last), StepMode::Over(2)), None);
        assert_eq!(
            should_stop(&at(4, 2), Some(&last), StepMode::Over(2)),
            Some("step")
        );
        assert_eq!(
            should_stop(&at(7, 1), Some(&last), StepMode::Over(2)),
            Some("step")
        );
        assert_eq!(should_stop(&at(4, 2), Some(&last), StepMode::Out(2)), None);
        assert_eq!(
            should_stop(&at(7, 1), Some(&last), StepMode::Out(2)),
            Some("step")
        );
        // The same place twice never stops on a step.
        assert_eq!(should_stop(&last, Some(&last), StepMode::In), None);

        set_breakpoints("/app/main.sl", [12]);
        assert_eq!(
            should_stop(&at(12, 1), Some(&at(11, 1)), StepMode::Run),
            Some("breakpoint")
        );
        // A second statement on the breakpoint's line does not stop again.
        assert_eq!(
            should_stop(&at(12, 1), Some(&at(12, 2)), StepMode::Run),
            None
        );
        set_breakpoints("/app/main.sl", []);
        assert_eq!(
            should_stop(&at(12, 1), Some(&at(11, 1)), StepMode::Run),
            None
        );

        request_pause();
        assert_eq!(
            should_stop(&last, Some(&last), StepMode::Run),
            Some("pause")
        );
        assert_eq!(should_stop(&last, Some(&last), StepMode::Run), None);
    }
}
