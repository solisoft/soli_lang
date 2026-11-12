//! `soli serve --dev --inspect`: the debug adapter on a loopback TCP port, so
//! an editor can attach to a running server, set breakpoints in its
//! controllers, models and views' code, and stop a request there.
//!
//! One editor at a time. It must `attach` with the dev token (the secret the
//! dev error page's REPL already requires), compared in constant time; until
//! then nothing stops. When it disconnects, its breakpoints go, a stopped
//! request resumes, and the server carries on.

use std::io::BufReader;
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use super::{read_message, send_stopped, Adapter, Wire};
use crate::debug::session::{self, Command, Event};

/// Compare two secrets without leaking where they differ.
pub(super) fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() || right.is_empty() {
        return false;
    }
    left.bytes()
        .zip(right.bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}

/// Open the debugging session and listen for an editor on `addr` (a
/// loopback address). Returns the address bound. Call before the workers
/// start, so they compile their code with the debugger's stop markers.
pub fn start_inspector(addr: &str, token: String) -> Result<SocketAddr, String> {
    let wanted: SocketAddr = addr
        .parse()
        .map_err(|_| format!("--inspect: {addr} is not an address (127.0.0.1:9229)"))?;
    if !wanted.ip().is_loopback() {
        return Err(format!(
            "--inspect binds a loopback address only, not {}: attaching is running code in the server",
            wanted.ip()
        ));
    }
    let listener = TcpListener::bind(wanted)
        .map_err(|e| format!("--inspect: cannot listen on {addr}: {e}"))?;
    let bound = listener.local_addr().map_err(|e| e.to_string())?;
    let (commands, events) = session::open()?;

    let current: Arc<Mutex<Option<Wire>>> = Arc::new(Mutex::new(None));
    let stopped = Arc::new(AtomicBool::new(false));
    {
        let current = current.clone();
        let stopped = stopped.clone();
        std::thread::Builder::new()
            .name("soli-inspect-events".into())
            .spawn(move || {
                for event in events {
                    let wire = current.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    match event {
                        Event::Stopped {
                            reason,
                            description,
                        } => {
                            stopped.store(true, Ordering::Release);
                            if let Some(wire) = wire {
                                send_stopped(&wire, reason, &description);
                            }
                        }
                        Event::Output { text } => {
                            if let Some(wire) = wire {
                                wire.event(
                                    "output",
                                    serde_json::json!({"category": "console", "output": text}),
                                );
                            }
                        }
                        Event::Finished { .. } => {}
                    }
                }
            })
            .map_err(|e| e.to_string())?;
    }

    std::thread::Builder::new()
        .name("soli-inspect".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let Ok(writer) = stream.try_clone() else {
                    continue;
                };
                let wire = Wire {
                    out: Arc::new(Mutex::new(Box::new(writer))),
                    seq: Arc::new(AtomicI64::new(1)),
                };
                *current.lock().unwrap_or_else(|e| e.into_inner()) = Some(wire.clone());
                let mut adapter = Adapter::new(wire.clone(), commands.clone(), stopped.clone());
                adapter.inspector_token = Some(token.clone());
                let mut input = BufReader::new(stream);
                while let Some(request) = read_message(&mut input) {
                    if request["type"] != "request" {
                        continue;
                    }
                    if let Some(result) = adapter.handle(&request) {
                        wire.respond(&request, result);
                    }
                    if adapter.closing {
                        break;
                    }
                }
                // The editor is gone: nothing stops any more, and a request
                // it left stopped goes on.
                *current.lock().unwrap_or_else(|e| e.into_inner()) = None;
                session::set_client_connected(false);
                session::clear_breakpoints();
                if stopped.swap(false, Ordering::AcqRel) {
                    let _ = commands.send(Command::Continue);
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_compare_whole_and_never_empty() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abd", "abc"));
        assert!(!constant_time_eq("ab", "abc"));
        assert!(!constant_time_eq("", ""));
    }
}
