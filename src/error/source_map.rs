//! Which file an error's span points into, and that file's text.
//!
//! A [`Span`] carries a line and a column but no file: it stays 16 bytes
//! because one sits in every AST node. That is enough while a program is one
//! file. Once imports are resolved, the statements of every imported module
//! are spliced into the entry program with their own line numbers, and a span
//! alone no longer says which file it means. A frame drawn from the wrong
//! file misleads more than no frame, so [`SourceSet::locate`] names a file
//! only when it is sure:
//!
//! 1. an engine or the resolver *said* where the error came from (an
//!    [`Origin`], noted on the error path — the VM names the proto's file,
//!    the resolver the import or the module that failed to parse);
//! 2. the program is a single file, so every span is in it;
//! 3. the span's byte offset agrees with its line and column in exactly one
//!    of the program's files (tree-walker and type-checker spans keep their
//!    offsets; VM spans do not).
//!
//! Otherwise the error prints as it always did, with no frame.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::span::Span;

/// Where an error came from, as noted by the code that raised it.
#[derive(Debug, Clone)]
pub struct Origin {
    /// The span the error itself carries: the origin applies to that error
    /// only (a stale origin from an earlier, caught error never matches).
    pub error_span: Span,
    /// The file; `None` is the entry source.
    pub file: Option<PathBuf>,
    /// Where to point in that file (often `error_span` itself).
    pub span: Span,
    /// A code the error's own type cannot give (module resolution errors
    /// travel as a `RuntimeError::General`).
    pub code: Option<&'static str>,
}

thread_local! {
    static ORIGIN: RefCell<Option<Origin>> = const { RefCell::new(None) };
}

/// Note where the error being raised came from.
pub fn note_origin(origin: Origin) {
    ORIGIN.with(|o| *o.borrow_mut() = Some(origin));
}

/// Note the file of a VM error leaving its frame. The innermost activation
/// notes it first; an outer activation handing the same error on (same span)
/// keeps that note. A frame with no known file (`None`) notes nothing: the
/// span then has to name its file on its own.
pub fn note_vm_origin(span: Span, file: Option<&Arc<PathBuf>>) {
    ORIGIN.with(|o| {
        let mut slot = o.borrow_mut();
        if slot.as_ref().is_some_and(|prev| prev.error_span == span) {
            return;
        }
        *slot = file.map(|f| Origin {
            error_span: span,
            file: Some(f.as_ref().clone()),
            span,
            code: None,
        });
    });
}

/// Forget any noted origin (before running something whose errors will be
/// reported).
pub fn clear_origin() {
    ORIGIN.with(|o| *o.borrow_mut() = None);
}

/// The noted origin, taken.
pub fn take_origin() -> Option<Origin> {
    ORIGIN.with(|o| o.borrow_mut().take())
}

/// The placeholder span errors without a location carry (`1:1`, no width).
pub fn is_placeholder(span: Span) -> bool {
    span.line == 0 || (span.start == 0 && span.end == 0 && span.line == 1 && span.column == 1)
}

/// The sources one run or one check was built from.
#[derive(Debug)]
pub struct SourceSet {
    /// The entry file, `None` for `soli -e` and other in-memory sources.
    entry_path: Option<PathBuf>,
    /// The entry text, exactly as it was compiled.
    entry_text: Arc<str>,
    /// The files imports brought in.
    imported: Vec<PathBuf>,
    /// What the engine or resolver noted about the error, if anything.
    origin: Option<Origin>,
    cache: RefCell<HashMap<PathBuf, Option<Arc<str>>>>,
}

impl SourceSet {
    pub fn new(entry_text: &str, entry_path: Option<&Path>) -> Self {
        Self {
            entry_path: entry_path.map(Path::to_path_buf),
            entry_text: Arc::from(entry_text),
            imported: Vec::new(),
            origin: None,
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// The files a resolved program's imported statements came from.
    pub fn with_program(mut self, program: &crate::ast::Program) -> Self {
        for stmt in &program.statements {
            if let Some(path) = &stmt.source_path {
                if !self.imported.contains(path) && Some(path) != self.entry_path.as_ref() {
                    self.imported.push(path.clone());
                }
            }
        }
        self
    }

    pub fn with_origin(mut self, origin: Option<Origin>) -> Self {
        self.origin = origin;
        self
    }

    pub fn origin(&self) -> Option<&Origin> {
        self.origin.as_ref()
    }

    pub fn has_imports(&self) -> bool {
        !self.imported.is_empty()
    }

    /// The text of `file` (`None` = the entry), read from disk on first use
    /// for an imported file. Best effort: a file that cannot be read gives
    /// `None`, and its errors print with no frame.
    pub fn text(&self, file: Option<&Path>) -> Option<Arc<str>> {
        let Some(path) = file else {
            return Some(self.entry_text.clone());
        };
        if Some(path) == self.entry_path.as_deref() {
            return Some(self.entry_text.clone());
        }
        self.cache
            .borrow_mut()
            .entry(path.to_path_buf())
            .or_insert_with(|| std::fs::read_to_string(path).ok().map(Arc::from))
            .clone()
    }

    /// How to print `file` in a ` --> ` line: the entry as it was given, an
    /// imported file relative to the working directory when it is under it.
    pub fn display_name(&self, file: Option<&Path>) -> String {
        match file {
            None => match &self.entry_path {
                Some(p) => p.display().to_string(),
                None => "<eval>".to_string(),
            },
            Some(path) => {
                if Some(path) == self.entry_path.as_deref() {
                    return path.display().to_string();
                }
                let cwd = std::env::current_dir()
                    .ok()
                    .and_then(|c| c.canonicalize().ok());
                match cwd.as_deref().and_then(|c| path.strip_prefix(c).ok()) {
                    Some(rel) => rel.display().to_string(),
                    None => path.display().to_string(),
                }
            }
        }
    }

    /// The file and span to frame for an error carrying `span`, or `None`
    /// when the file is not known for certain (see the module docs).
    pub fn locate(&self, span: Span) -> Option<(Option<PathBuf>, Span)> {
        if let Some(origin) = self.origin.as_ref().filter(|o| o.error_span == span) {
            if is_placeholder(origin.span) {
                return None;
            }
            let file = origin
                .file
                .clone()
                .filter(|f| Some(f) != self.entry_path.as_ref());
            return Some((file, origin.span));
        }
        if is_placeholder(span) {
            return None;
        }
        if self.imported.is_empty() {
            return line_exists(&self.entry_text, span.line_usize()).then_some((None, span));
        }
        if span.end <= span.start {
            return None;
        }
        let mut found: Option<Option<PathBuf>> = None;
        let candidates = std::iter::once(None).chain(self.imported.iter().map(|p| Some(p.clone())));
        for candidate in candidates {
            let Some(text) = self.text(candidate.as_deref()) else {
                continue;
            };
            if offset_agrees(&text, span) {
                if found.is_some() {
                    return None; // ambiguous
                }
                found = Some(candidate);
            }
        }
        found.map(|file| (file, span))
    }
}

/// Byte offset of the start of 1-based `line`, if the text has that line.
pub(crate) fn line_start(text: &str, line: usize) -> Option<usize> {
    if line == 0 {
        return None;
    }
    if line == 1 {
        return Some(0);
    }
    text.match_indices('\n')
        .nth(line - 2)
        .map(|(at, _)| at + 1)
        .filter(|&at| at <= text.len())
}

fn line_exists(text: &str, line: usize) -> bool {
    match line_start(text, line) {
        Some(at) => at < text.len() || line == 1,
        None => false,
    }
}

/// The text of 1-based `line`, without its line ending.
pub(crate) fn line_text(text: &str, line: usize) -> Option<&str> {
    let start = line_start(text, line)?;
    let rest = &text[start..];
    let end = rest.find('\n').unwrap_or(rest.len());
    Some(rest[..end].trim_end_matches('\r'))
}

/// Whether `span.start` is the byte offset of `span.line`:`span.column`
/// (1-based, counted in chars) in `text` — the lexer's convention, so a span
/// that agrees was measured in this text.
pub(crate) fn offset_agrees(text: &str, span: Span) -> bool {
    let (Some(start), Some(line)) = (
        line_start(text, span.line_usize()),
        line_text(text, span.line_usize()),
    ) else {
        return false;
    };
    if span.column == 0 {
        return false;
    }
    let mut offset = start;
    let mut chars = line.chars();
    for _ in 1..span.column_usize() {
        match chars.next() {
            Some(c) => offset += c.len_utf8(),
            None => return false,
        }
    }
    offset == span.start_usize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_helpers() {
        let text = "a = 1\nbb = 2\r\nccc";
        assert_eq!(line_start(text, 1), Some(0));
        assert_eq!(line_start(text, 2), Some(6));
        assert_eq!(line_text(text, 2), Some("bb = 2"));
        assert_eq!(line_text(text, 3), Some("ccc"));
        assert_eq!(line_text(text, 4), None);
        assert_eq!(line_text(text, 0), None);
    }

    #[test]
    fn offsets_agree_only_in_the_file_they_were_measured_in() {
        let text = "x = 1\nprint(nme)\n";
        // `nme` at 2:7, byte 12
        let span = Span::new(12, 15, 2, 7);
        assert!(offset_agrees(text, span));
        assert!(!offset_agrees("print(nme)\nx = 1\n", span));
        // multibyte chars before the column
        let text = "é = 1\nprint(é, nme)";
        let at = text.find("nme").unwrap();
        assert!(offset_agrees(text, Span::new(at, at + 3, 2, 10)));
    }

    #[test]
    fn a_single_file_locates_any_real_span_and_no_placeholder() {
        let set = SourceSet::new("a\nb\n", Some(Path::new("x.sl")));
        assert_eq!(set.locate(Span::new(0, 0, 2, 1)).unwrap().0, None);
        assert!(set.locate(Span::new(0, 0, 1, 1)).is_none());
        assert!(set.locate(Span::new(0, 0, 0, 0)).is_none());
        assert!(set.locate(Span::new(0, 0, 9, 1)).is_none());
    }

    #[test]
    fn an_origin_wins_only_for_its_own_error() {
        let origin = Origin {
            error_span: Span::new(0, 0, 1, 1),
            file: Some(PathBuf::from("/m.sl")),
            span: Span::new(10, 12, 3, 4),
            code: Some("E0701"),
        };
        let set = SourceSet::new("a\n", Some(Path::new("x.sl"))).with_origin(Some(origin));
        let (file, span) = set.locate(Span::new(0, 0, 1, 1)).unwrap();
        assert_eq!(file, Some(PathBuf::from("/m.sl")));
        assert_eq!(span.line, 3);
        assert!(set.locate(Span::new(0, 0, 1, 1)).is_some());
        assert_eq!(set.locate(Span::new(0, 1, 1, 1)).unwrap().0, None);
    }
}
