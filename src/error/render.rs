//! Text code frames for a [`Diagnostic`]:
//!
//! ```text
//!   --> app/report.sl:3:7
//!    |
//!  3 | print(nme)
//!    |       ^^^
//!    = help: did you mean 'name'?
//!    = note: error[E0301] undefined-variable — `soli explain E0301`
//! ```
//!
//! Callers print their usual first line (`Error: …`, `file: …`) and the
//! frame below it, so a tool reading the first line sees what it always saw.
//!
//! The underline covers the token at the span's column — an identifier
//! (with its `?` / `!`), else one character — and a column of 0 draws no
//! caret. It is not taken from the span's byte range on purpose: a VM span
//! has none, and a frame must read the same on both engines (`soli x.sl`
//! and `soli --tree x.sl` print byte-identical errors), so both measure the
//! token in the source. Built in-crate, with no dependency: it runs on every
//! target the interpreter builds for.

use std::fmt::Write as _;

use super::diagnostic::{Diagnostic, Label, Severity};
use super::source_map::{self, SourceSet};

#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    /// ANSI colours (see [`color_for`]).
    pub color: bool,
}

/// Whether to colour output written to a stream: only a terminal, and never
/// under `NO_COLOR` (https://no-color.org) or `TERM=dumb`.
pub fn color_for(is_terminal: bool) -> bool {
    is_terminal
        && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
        && std::env::var("TERM").map_or(true, |t| t != "dumb")
}

/// Whether stderr should be coloured.
pub fn stderr_color() -> bool {
    use std::io::IsTerminal;
    color_for(std::io::stderr().is_terminal())
}

/// Whether stdout should be coloured.
pub fn stdout_color() -> bool {
    use std::io::IsTerminal;
    color_for(std::io::stdout().is_terminal())
}

struct Paint {
    on: bool,
}

impl Paint {
    fn wrap(&self, code: &str, text: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
    fn gutter(&self, text: &str) -> String {
        self.wrap("1;34", text)
    }
    fn mark(&self, severity: Severity, text: &str) -> String {
        match severity {
            Severity::Error => self.wrap("1;31", text),
            Severity::Warning => self.wrap("1;33", text),
        }
    }
    fn bold(&self, text: &str) -> String {
        self.wrap("1", text)
    }
}

/// `error[E0301]: message` — the header a frame has when it stands alone.
pub fn header(diagnostic: &Diagnostic, options: RenderOptions) -> String {
    let paint = Paint { on: options.color };
    let kind = match diagnostic.explain_id() {
        Some(id) => format!("{}[{}]", diagnostic.severity.as_str(), id),
        None => diagnostic.severity.as_str().to_string(),
    };
    format!(
        "{}{}",
        paint.mark(diagnostic.severity, &kind),
        paint.bold(&format!(": {}", diagnostic.message))
    )
}

/// The frame below an error's first line: the source line with an underline,
/// then `= help:` and `= note:` lines. Empty when there is nothing to add
/// (no location, no hint, no code), so the output is then what it was
/// before frames existed. Ends with a newline when not empty.
pub fn frame(diagnostic: &Diagnostic, sources: &SourceSet, options: RenderOptions) -> String {
    let paint = Paint { on: options.color };
    let mut labels: Vec<(&Label, bool)> = Vec::new();
    if let Some(primary) = &diagnostic.primary {
        labels.push((primary, true));
    }
    labels.extend(diagnostic.secondary.iter().map(|l| (l, false)));
    let width = labels
        .iter()
        .map(|(l, _)| l.span.line.to_string().len())
        .max()
        .unwrap_or(1);
    let pad = " ".repeat(width);
    let mut out = String::new();

    for (index, (label, primary)) in labels.iter().enumerate() {
        let Some(text) = sources.text(label.file.as_deref()) else {
            continue;
        };
        let Some(line) = source_map::line_text(&text, label.span.line_usize()) else {
            continue;
        };
        let arrow = if index == 0 { "-->" } else { ":::" };
        let _ = writeln!(
            out,
            "{} {}:{}:{}",
            paint.gutter(&format!("{pad}{arrow}")),
            sources.display_name(label.file.as_deref()),
            label.span.line,
            label.span.column
        );
        let _ = writeln!(out, "{}", paint.gutter(&format!("{pad} |")));
        let _ = writeln!(
            out,
            "{} {}",
            paint.gutter(&format!("{:>width$} |", label.span.line)),
            line
        );
        if let Some((indent, len)) = underline(line, label) {
            let ch = if *primary { "^" } else { "-" };
            let marks = ch.repeat(len);
            let note = label
                .text
                .as_deref()
                .map(|t| format!(" {t}"))
                .unwrap_or_default();
            let severity = if *primary {
                diagnostic.severity
            } else {
                Severity::Warning
            };
            let _ = writeln!(
                out,
                "{} {}{}",
                paint.gutter(&format!("{pad} |")),
                indent,
                paint.mark(severity, &format!("{marks}{note}"))
            );
        }
    }

    let mut tail = Vec::new();
    if let Some(hint) = &diagnostic.hint {
        tail.push(format!("{}: {}", paint.bold("help"), hint));
    }
    for note in &diagnostic.notes {
        tail.push(format!("{}: {}", paint.bold("note"), note));
    }
    if let Some(code) = diagnostic.code {
        tail.push(format!(
            "{}: {}[{}] {} — `soli explain {}`",
            paint.bold("note"),
            diagnostic.severity.as_str(),
            code.code,
            code.name,
            code.code
        ));
    } else if let Some(rule) = diagnostic.rule {
        tail.push(format!("{}: `soli explain {}`", paint.bold("note"), rule));
    }
    if !out.is_empty() && !tail.is_empty() {
        let _ = writeln!(out, "{}", paint.gutter(&format!("{pad} |")));
    }
    for line in tail {
        let _ = writeln!(out, "{} {}", paint.gutter(&format!("{pad} =")), line);
    }
    out
}

/// The indent (tabs kept, so it lines up under a tab-indented line) and the
/// width of the underline for `label` on `line`, or `None` to draw none.
fn underline(line: &str, label: &Label) -> Option<(String, usize)> {
    let column = label.span.column_usize();
    if column == 0 {
        return None;
    }
    let chars: Vec<char> = line.chars().collect();
    let at = (column - 1).min(chars.len());
    let indent: String = chars[..at]
        .iter()
        .map(|&c| if c == '\t' { '\t' } else { ' ' })
        .collect();
    let len = token_width(&chars[at..]);
    Some((indent, len.max(1)))
}

/// Width of the token starting `rest`: an identifier (with a trailing `?`
/// or `!`), else one character.
fn token_width(rest: &[char]) -> usize {
    let is_ident = |c: &char| c.is_alphanumeric() || *c == '_';
    match rest.first() {
        Some(c) if is_ident(c) || *c == '@' => {
            let mut n = 1 + rest[1..].iter().take_while(|c| is_ident(c)).count();
            if matches!(rest.get(n), Some('?') | Some('!')) {
                n += 1;
            }
            n
        }
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{codes, RuntimeError, SolilangError};
    use crate::span::Span;
    use std::path::Path;

    fn diag(span: Span, hint: Option<&str>) -> Diagnostic {
        Diagnostic {
            code: codes::by_code("E0301"),
            rule: None,
            severity: Severity::Error,
            message: "Undefined variable 'nme'".into(),
            primary: Some(Label {
                file: None,
                span,
                text: None,
            }),
            secondary: Vec::new(),
            hint: hint.map(String::from),
            notes: Vec::new(),
        }
    }

    fn plain() -> RenderOptions {
        RenderOptions { color: false }
    }

    #[test]
    fn a_caret_under_the_token_at_the_column() {
        let src = "name = 1\nprint(nme)\n";
        let sources = SourceSet::new(src, Some(Path::new("s.sl")));
        let out = frame(
            &diag(Span::new(15, 18, 2, 7), Some("did you mean 'name'?")),
            &sources,
            plain(),
        );
        assert_eq!(
            out,
            " --> s.sl:2:7\n  |\n2 | print(nme)\n  |       ^^^\n  |\n  = help: did you mean 'name'?\n  = note: error[E0301] undefined-variable — `soli explain E0301`\n"
        );
        assert!(!out.contains('\x1b'));
    }

    #[test]
    fn a_tab_indented_line_keeps_its_tabs_under_it() {
        let src = "def f\n\tprint(nme)\nend\n";
        let sources = SourceSet::new(src, None);
        let out = frame(&diag(Span::new(13, 16, 2, 8), None), &sources, plain());
        assert!(out.contains("2 | \tprint(nme)\n  | \t      ^^^\n"), "{out}");
        assert!(out.contains("<eval>:2:8"), "{out}");
    }

    #[test]
    fn multibyte_chars_before_the_column() {
        let src = "café = 1\nprint(\"é\", nme)";
        let at = src.find("nme").unwrap();
        let sources = SourceSet::new(src, None);
        let out = frame(&diag(Span::new(at, at + 3, 2, 12), None), &sources, plain());
        assert!(out.contains("\n  |            ^^^\n"), "{out}");
    }

    #[test]
    fn the_last_line_without_a_newline() {
        let src = "x = 1\nprint(nme)";
        let sources = SourceSet::new(src, None);
        let out = frame(&diag(Span::new(12, 15, 2, 7), None), &sources, plain());
        assert!(out.contains("2 | print(nme)\n  |       ^^^\n"), "{out}");
    }

    #[test]
    fn a_vm_span_measures_the_identifier() {
        let src = "x = 1\nprint(user.admin?)";
        let sources = SourceSet::new(src, None);
        let out = frame(&diag(Span::new(0, 0, 2, 7), None), &sources, plain());
        assert!(out.contains("  |       ^^^^\n"), "{out}");
        let out = frame(&diag(Span::new(0, 0, 2, 12), None), &sources, plain());
        assert!(out.contains("  |            ^^^^^^\n"), "{out}");
    }

    #[test]
    fn a_vm_span_with_column_zero_draws_the_line_and_no_caret() {
        let src = "x = 1\nprint(nme)";
        let sources = SourceSet::new(src, None);
        let out = frame(&diag(Span::new(0, 0, 2, 0), None), &sources, plain());
        assert!(out.contains("2 | print(nme)\n  |\n  = note"), "{out}");
        assert!(!out.contains('^'), "{out}");
    }

    #[test]
    fn a_placeholder_span_prints_no_frame() {
        let sources = SourceSet::new("print(1)\n", Some(Path::new("s.sl")));
        let error: SolilangError = RuntimeError::new("boom", Span::new(0, 0, 1, 1)).into();
        let diagnostic = Diagnostic::from_error(&error, &sources);
        assert_eq!(frame(&diagnostic, &sources, plain()), "");
    }

    #[test]
    fn secondary_labels_get_their_own_line() {
        let src = "def a\nend\ndef a\nend\n";
        let sources = SourceSet::new(src, None);
        let mut d = diag(Span::new(10, 15, 3, 1), None);
        d.code = None;
        d.secondary.push(Label {
            file: None,
            span: Span::new(0, 5, 1, 1),
            text: Some("first defined here".into()),
        });
        let out = frame(&d, &sources, plain());
        assert!(out.contains(" ::: <eval>:1:1\n"), "{out}");
        assert!(out.contains("  | --- first defined here\n"), "{out}");
    }

    #[test]
    fn colour_only_when_asked() {
        let src = "print(nme)";
        let sources = SourceSet::new(src, None);
        let out = frame(
            &diag(Span::new(6, 9, 1, 7), None),
            &sources,
            RenderOptions { color: true },
        );
        assert!(out.contains("\x1b[1;31m^^^"), "{out}");
        assert!(!color_for(false));
    }

    #[test]
    fn header_names_the_code() {
        let d = diag(Span::new(0, 0, 1, 1), None);
        assert_eq!(
            header(&d, plain()),
            "error[E0301]: Undefined variable 'nme'"
        );
    }
}
