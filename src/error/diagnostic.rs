//! A renderer-neutral description of one error or warning: what went wrong,
//! where, and what to try. Built from any error the toolchain reports, then
//! drawn as a code frame (`render`) or written as JSON.
//!
//! The error's `Display` text is not touched: specs match on it with
//! `assert_raises`, user code reads it in `catch`, and the dev page and the
//! test runner parse it. The diagnostic is an extra view, never a
//! replacement — `soli run` still prints `Error: <Display>` first, and the
//! frame below it.

use std::path::PathBuf;

use super::codes::{self, ErrorCode};
use super::source_map::SourceSet;
use super::{SolilangError, TypeError};
use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// A place in a file, with an optional note drawn next to the underline.
#[derive(Debug, Clone)]
pub struct Label {
    /// `None` is the entry source of the [`SourceSet`].
    pub file: Option<PathBuf>,
    pub span: Span,
    pub text: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// The registered code (`E0301`), when the error kind has one.
    pub code: Option<&'static ErrorCode>,
    /// A lint rule id, for a lint diagnostic (it has no `E` code).
    pub rule: Option<&'static str>,
    pub severity: Severity,
    /// The message, without its ` at L:C` suffix and its "did you mean".
    pub message: String,
    /// Where it points; `None` when the location is unknown or uncertain.
    pub primary: Option<Label>,
    /// Related places (the first definition of a duplicate, …).
    pub secondary: Vec<Label>,
    /// The "did you mean" suggestion, as `did you mean 'x'?`.
    pub hint: Option<String>,
    pub notes: Vec<String>,
}

impl Diagnostic {
    /// The diagnostic for an error `sources` produced.
    pub fn from_error(error: &SolilangError, sources: &SourceSet) -> Self {
        let origin_code = error
            .span()
            .and_then(|span| sources.origin().filter(|o| o.error_span == span))
            .and_then(|o| o.code);
        let code = error.code().or(origin_code).and_then(codes::by_code);
        let (message, hint) = split_message(&error.inner_message());
        let primary = error
            .span()
            .and_then(|span| sources.locate(span))
            .map(|(file, span)| Label {
                file,
                span,
                text: None,
            });
        Self {
            code,
            rule: None,
            severity: Severity::Error,
            message,
            primary,
            secondary: Vec::new(),
            hint,
            notes: Vec::new(),
        }
    }

    /// The diagnostic for a type-checker warning (`W0401`).
    pub fn from_type_warning(warning: &TypeError, sources: &SourceSet) -> Self {
        let message = match warning {
            TypeError::ExhaustivenessWarning {
                enum_name, missing, ..
            } => format!(
                "match on enum '{enum_name}' is not exhaustive — missing: {missing} (add them, or a `_ =>` arm)"
            ),
            other => split_message(&other.to_string()).0,
        };
        let primary = sources.locate(warning.span()).map(|(file, span)| Label {
            file,
            span,
            text: None,
        });
        Self {
            code: warning.code().and_then(codes::by_code),
            rule: None,
            severity: if warning.is_warning() {
                Severity::Warning
            } else {
                Severity::Error
            },
            message,
            primary,
            secondary: Vec::new(),
            hint: None,
            notes: Vec::new(),
        }
    }

    /// The diagnostic for a lint finding in the entry file of `sources`.
    pub fn from_lint(finding: &crate::lint::LintDiagnostic, sources: &SourceSet) -> Self {
        let (message, hint) = split_message(&finding.message);
        let primary = sources.locate(finding.span).map(|(file, span)| Label {
            file,
            span,
            text: None,
        });
        Self {
            code: None,
            rule: Some(finding.rule),
            severity: Severity::Warning,
            message,
            primary,
            secondary: Vec::new(),
            hint,
            notes: Vec::new(),
        }
    }

    /// The id `soli explain` takes for this diagnostic: its code or its rule.
    pub fn explain_id(&self) -> Option<&'static str> {
        self.code.map(|c| c.code).or(self.rule)
    }

    /// The diagnostic as a JSON object (`--format json`).
    pub fn to_json(&self, sources: &SourceSet) -> serde_json::Value {
        let location = self.primary.as_ref().map(|label| {
            serde_json::json!({
                "file": sources.display_name(label.file.as_deref()),
                "line": label.span.line,
                "column": label.span.column,
            })
        });
        let secondary: Vec<serde_json::Value> = self
            .secondary
            .iter()
            .map(|label| {
                serde_json::json!({
                    "file": sources.display_name(label.file.as_deref()),
                    "line": label.span.line,
                    "column": label.span.column,
                    "label": label.text,
                })
            })
            .collect();
        serde_json::json!({
            "severity": self.severity.as_str(),
            "code": self.code.map(|c| c.code).or(self.rule),
            "name": self.code.map(|c| c.name),
            "message": self.message,
            "file": location.as_ref().map(|l| l["file"].clone()),
            "line": location.as_ref().map(|l| l["line"].clone()),
            "column": location.as_ref().map(|l| l["column"].clone()),
            "hint": self.hint,
            "secondary": secondary,
            "notes": self.notes,
        })
    }
}

/// Split an error's text into its message and its "did you mean" hint, with
/// the trailing ` at L:C` location(s) removed. Works on the text rather than
/// on the variants because the suggestion also travels inside wrapped
/// messages (`WithEnv`, lint findings) that only kept the string.
pub fn split_message(text: &str) -> (String, Option<String>) {
    let mut message = text.trim_end();
    while let Some(rest) = strip_location(message) {
        message = rest;
    }
    let mut hint = None;
    if let Some(at) = message.rfind(" (did you mean '") {
        if message.ends_with("'?)") {
            hint = Some(message[at + 2..message.len() - 1].to_string());
            message = &message[..at];
        }
    }
    (message.to_string(), hint)
}

/// `text` without a trailing ` at <line>:<col>`, if it has one.
fn strip_location(text: &str) -> Option<&str> {
    let at = text.rfind(" at ")?;
    let (line, col) = text[at + 4..].split_once(':')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(line) && digits(col)).then_some(&text[..at])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RuntimeError;
    use std::path::Path;

    #[test]
    fn split_message_takes_the_location_and_the_hint_off() {
        assert_eq!(
            split_message("Undefined variable 'nme' (did you mean 'name'?) at 3:7"),
            (
                "Undefined variable 'nme'".to_string(),
                Some("did you mean 'name'?".to_string())
            )
        );
        assert_eq!(
            split_message("Unexpected character '@' at 1:5 at 1:5").0,
            "Unexpected character '@'"
        );
        assert_eq!(split_message("meet at noon").0, "meet at noon");
        assert_eq!(split_message("Division by zero at 2:3").1, None);
    }

    #[test]
    fn a_runtime_error_gets_its_code_location_and_hint() {
        let sources = SourceSet::new("name = 1\nprint(nme)\n", Some(Path::new("s.sl")));
        let error: SolilangError = RuntimeError::undefined_variable_hinted(
            "nme",
            Span::new(15, 18, 2, 7),
            crate::error::Hint::ready(Some("name".into())),
        )
        .into();
        let diagnostic = Diagnostic::from_error(&error, &sources);
        assert_eq!(diagnostic.code.unwrap().code, "E0301");
        assert_eq!(diagnostic.message, "Undefined variable 'nme'");
        assert_eq!(diagnostic.hint.as_deref(), Some("did you mean 'name'?"));
        assert_eq!(diagnostic.primary.unwrap().span.line, 2);
    }

    #[test]
    fn a_general_error_has_no_code_and_a_placeholder_no_location() {
        let sources = SourceSet::new("x\n", None);
        let error: SolilangError = RuntimeError::new("boom", Span::new(0, 0, 1, 1)).into();
        let diagnostic = Diagnostic::from_error(&error, &sources);
        assert!(diagnostic.code.is_none());
        assert!(diagnostic.primary.is_none());
        let json = diagnostic.to_json(&sources);
        assert_eq!(json["message"], "boom");
        assert!(json["line"].is_null());
    }
}
