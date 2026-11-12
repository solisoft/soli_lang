//! The VM's side of the step debugger (`crate::debug::session`).
//!
//! The compiler emits `Op::DebugStmt` at each statement (and `Op::DebugBreak`
//! for `debug()`) only when a debugger was attached; on the debuggee thread
//! they land here. Deciding whether to stop is shared with the tree-walker
//! (`executor::debugger::vm_statement`); when the VM stops, it reads each
//! frame off its stack — the function, the line, the live slots named by the
//! chunk's `debug_locals` table — and hands them to the shared pause loop.

use std::collections::HashSet;

use super::vm::Vm;
use crate::debug::session::{normalize_path, Location};
use crate::interpreter::executor::debugger::{self, FrameScope, FrameState};
use crate::interpreter::value::Value;

impl Vm {
    /// Before a statement on `line` of the running frame.
    pub(crate) fn debugger_statement(&mut self, line: u32) {
        let Some(file) = self.frame_file(self.frames.len().saturating_sub(1)) else {
            return;
        };
        let here = Location {
            file,
            line,
            depth: self.frames.len().saturating_sub(1),
        };
        let builtins = || self.globals.keys().cloned().collect::<HashSet<String>>();
        if let Some((reason, description)) = debugger::vm_statement(here, builtins) {
            self.debugger_stop(line, reason, description);
        }
    }

    /// `debug()` reached under a debugger.
    pub(crate) fn debugger_break(&mut self) {
        let line = self.current_span().line;
        self.debugger_stop(line, "breakpoint", debugger::breakpoint_description());
    }

    fn debugger_stop(&mut self, line: u32, reason: &'static str, description: String) {
        let frames = self.debugger_frames(line);
        let globals = self.debugger_globals();
        debugger::vm_pause(frames, globals, reason, description);
    }

    /// The source file of frame `index`, as the debugger compares paths.
    fn frame_file(&self, index: usize) -> Option<String> {
        self.frames
            .get(index)?
            .closure
            .proto
            .source_path
            .as_ref()
            .map(|path| normalize_path(&path.to_string_lossy()))
    }

    /// Every frame, outermost first, with its live named slots.
    fn debugger_frames(&self, top_line: u32) -> Vec<FrameState> {
        let count = self.frames.len();
        (0..count)
            .map(|index| {
                let frame = &self.frames[index];
                let proto = &frame.closure.proto;
                let chunk = &proto.chunk;
                // The frame's own ip points past the instruction it is in
                // (for a caller, past its call).
                let ip = frame.ip.saturating_sub(1);
                let line = if index + 1 == count {
                    top_line
                } else {
                    chunk.lines.get(ip).copied().unwrap_or(0) as u32
                };
                let name = if index == 0 {
                    "<main>".to_string()
                } else if proto.name.is_empty() {
                    "<anonymous>".to_string()
                } else {
                    proto.name.clone()
                };
                // Slots live at `ip`: per slot, the latest local that starts
                // at or before it and has not ended.
                let limit = self
                    .frames
                    .get(index + 1)
                    .map(|next| next.stack_base)
                    .unwrap_or(self.stack.len());
                let mut by_slot: std::collections::BTreeMap<u16, (u32, &str)> =
                    std::collections::BTreeMap::new();
                for local in &chunk.debug_locals {
                    let ip = ip as u32;
                    if local.start <= ip && ip < local.end {
                        let entry = by_slot
                            .entry(local.slot)
                            .or_insert((local.start, &local.name));
                        if local.start >= entry.0 {
                            *entry = (local.start, &local.name);
                        }
                    }
                }
                let mut values: Vec<(String, Value)> = by_slot
                    .into_iter()
                    .filter_map(|(slot, (_, name))| {
                        let at = frame.stack_base + slot as usize;
                        (at < limit)
                            .then(|| self.stack.get(at).cloned())
                            .flatten()
                            .map(|value| (name.to_string(), value))
                    })
                    .collect();
                values.sort_by(|a, b| a.0.cmp(&b.0));
                FrameState {
                    name,
                    file: self.frame_file(index).unwrap_or_default(),
                    line,
                    scope: FrameScope::Values(values),
                }
            })
            .collect()
    }

    /// The program's globals, without what was defined before it ran.
    fn debugger_globals(&self) -> Vec<(String, Value)> {
        let builtins = debugger::builtin_names();
        let mut globals: Vec<(String, Value)> = self
            .globals
            .iter()
            .filter(|(name, value)| {
                !builtins.contains(*name) && !matches!(value, Value::NativeFunction(_))
            })
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        globals.sort_by(|a, b| a.0.cmp(&b.0));
        globals
    }
}
