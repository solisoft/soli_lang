//! Error types for all compilation phases.

use crate::span::Span;
use thiserror::Error;

/// Lexer errors.
#[derive(Debug, Error)]
pub enum LexerError {
    #[error("Unexpected character '{0}' at {1}")]
    UnexpectedChar(char, Span),

    #[error("Unterminated string at {0}")]
    UnterminatedString(Span),

    #[error("Invalid escape sequence '\\{0}' at {1}")]
    InvalidEscape(char, Span),

    #[error("Invalid number '{0}' at {1}")]
    InvalidNumber(String, Span),
}

impl LexerError {
    pub fn unexpected_char(c: char, span: Span) -> Self {
        Self::UnexpectedChar(c, span)
    }

    pub fn unterminated_string(span: Span) -> Self {
        Self::UnterminatedString(span)
    }

    pub fn invalid_escape(c: char, span: Span) -> Self {
        Self::InvalidEscape(c, span)
    }

    pub fn invalid_number(s: String, span: Span) -> Self {
        Self::InvalidNumber(s, span)
    }

    pub fn span(&self) -> Span {
        match self {
            Self::UnexpectedChar(_, span) => *span,
            Self::UnterminatedString(span) => *span,
            Self::InvalidEscape(_, span) => *span,
            Self::InvalidNumber(_, span) => *span,
        }
    }
}

/// Parser errors.
#[derive(Debug, Error)]
pub enum ParserError {
    #[error("Unexpected token '{found}', expected {expected} at {span}")]
    UnexpectedToken {
        expected: String,
        found: String,
        span: Span,
    },

    #[error("Unexpected end of file at {0}")]
    UnexpectedEof(Span),

    #[error("Invalid assignment target at {0}")]
    InvalidAssignmentTarget(Span),

    #[error("{message} at {span}")]
    General { message: String, span: Span },
}

impl ParserError {
    pub fn unexpected_token(
        expected: impl Into<String>,
        found: impl Into<String>,
        span: Span,
    ) -> Self {
        Self::UnexpectedToken {
            expected: expected.into(),
            found: found.into(),
            span,
        }
    }

    pub fn unexpected_eof(span: Span) -> Self {
        Self::UnexpectedEof(span)
    }

    pub fn invalid_assignment_target(span: Span) -> Self {
        Self::InvalidAssignmentTarget(span)
    }

    pub fn general(message: impl Into<String>, span: Span) -> Self {
        Self::General {
            message: message.into(),
            span,
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Self::UnexpectedToken { span, .. } => *span,
            Self::UnexpectedEof(span) => *span,
            Self::InvalidAssignmentTarget(span) => *span,
            Self::General { span, .. } => *span,
        }
    }
}

impl From<LexerError> for ParserError {
    fn from(err: LexerError) -> Self {
        Self::General {
            message: err.to_string(),
            span: err.span(),
        }
    }
}

/// Type checking errors.
#[derive(Debug, Error)]
pub enum TypeError {
    #[error("Type mismatch: expected {expected}, found {found} at {span}")]
    Mismatch {
        expected: String,
        found: String,
        span: Span,
    },

    #[error("Undefined variable '{0}' at {1}")]
    UndefinedVariable(String, Span),

    #[error("Undefined type '{0}' at {1}")]
    UndefinedType(String, Span),

    #[error("Undefined function '{0}' at {1}")]
    UndefinedFunction(String, Span),

    #[error("Cannot call non-function type '{0}' at {1}")]
    NotCallable(String, Span),

    #[error("Wrong number of arguments: expected {expected}, got {got} at {span}")]
    WrongArity {
        expected: usize,
        got: usize,
        span: Span,
    },

    #[error("Cannot access member '{member}' on type '{type_name}' at {span}")]
    NoSuchMember {
        type_name: String,
        member: String,
        span: Span,
    },

    #[error("Class '{0}' has no superclass at {1}")]
    NoSuperclass(String, Span),

    #[error("Cannot use 'this' outside of a class at {0}")]
    ThisOutsideClass(Span),

    #[error("Cannot use 'super' outside of a class at {0}")]
    SuperOutsideClass(Span),

    #[error("{message} at {span}")]
    General { message: String, span: Span },
}

impl TypeError {
    pub fn mismatch(expected: impl Into<String>, found: impl Into<String>, span: Span) -> Self {
        Self::Mismatch {
            expected: expected.into(),
            found: found.into(),
            span,
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Self::Mismatch { span, .. } => *span,
            Self::UndefinedVariable(_, span) => *span,
            Self::UndefinedType(_, span) => *span,
            Self::UndefinedFunction(_, span) => *span,
            Self::NotCallable(_, span) => *span,
            Self::WrongArity { span, .. } => *span,
            Self::NoSuchMember { span, .. } => *span,
            Self::NoSuperclass(_, span) => *span,
            Self::ThisOutsideClass(span) => *span,
            Self::SuperOutsideClass(span) => *span,
            Self::General { span, .. } => *span,
        }
    }
}

/// Bytecode compilation errors.
#[derive(Debug, Error)]
pub enum CompileError {
    #[error("{message} at {span}")]
    General { message: String, span: Span },
}

impl CompileError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self::General {
            message: message.into(),
            span,
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Self::General { span, .. } => *span,
        }
    }
}

/// Runtime errors.
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("Division by zero at {0}")]
    DivisionByZero(Span),

    #[error("Undefined variable '{0}' at {1}")]
    UndefinedVariable(String, Span),

    #[error("Cannot call non-function value at {0}")]
    NotCallable(Span),

    #[error("Wrong number of arguments: expected {expected}, got {got} at {span}")]
    WrongArity {
        expected: usize,
        got: usize,
        span: Span,
    },

    #[error("Type error: {message} at {span}")]
    TypeError { message: String, span: Span },

    #[error("Index out of bounds: {index} (length {length}) at {span}")]
    IndexOutOfBounds {
        index: i64,
        length: usize,
        span: Span,
    },

    #[error("Cannot access property '{property}' on {value_type} at {span}")]
    NoSuchProperty {
        value_type: String,
        property: String,
        span: Span,
    },

    #[error("Cannot instantiate '{0}' at {1}")]
    NotAClass(String, Span),

    #[error("{message} at {span}")]
    General { message: String, span: Span },

    #[error("Breakpoint hit at {span}")]
    Breakpoint {
        span: Span,
        /// JSON-serialized environment variables for debugging
        env_json: String,
        /// Stack trace captured at the moment of the breakpoint
        stack_trace: Vec<String>,
    },

    /// Error with captured environment for debugging
    /// This allows accessing local variables in the dev error page REPL.
    /// Note: callers wrap an inner error via `e.to_string()`, which already
    /// includes its own " at {span}" suffix — so we display only `{message}`
    /// here to avoid producing "... at 92:20 at 92:20".
    #[error("{message}")]
    WithEnv {
        message: String,
        span: Span,
        /// JSON-serialized environment variables for debugging
        env_json: String,
        /// Stack trace captured at the moment of the error
        stack_trace: Vec<String>,
    },
}

impl RuntimeError {
    /// Sentinel embedded in the error message when `Model.find` (or any
    /// future "record not found" path) fails to locate a record. The HTTP
    /// request handler looks for this marker and converts the error into
    /// a 404 response instead of the default 500.
    pub const RECORD_NOT_FOUND_MARKER: &'static str = "__RecordNotFound__:";

    /// Construct a Model.find-style "record not found" runtime error.
    /// `message` is surfaced as-is after the marker prefix.
    pub fn record_not_found(message: impl Into<String>, span: Span) -> Self {
        Self::General {
            message: format!("{}{}", Self::RECORD_NOT_FOUND_MARKER, message.into()),
            span,
        }
    }

    /// True when this error originated from a record-not-found path.
    /// Uses `contains` rather than `starts_with` so the marker survives
    /// error-wrapping layers (e.g. "Error calling method: __RecordNotFound__:...").
    pub fn is_record_not_found(&self) -> bool {
        let msg = match self {
            Self::General { message, .. } | Self::WithEnv { message, .. } => message.as_str(),
            _ => return false,
        };
        msg.contains(Self::RECORD_NOT_FOUND_MARKER)
    }

    /// The user-facing message from a record-not-found error (marker stripped).
    pub fn record_not_found_message(&self) -> Option<String> {
        let msg = match self {
            Self::General { message, .. } | Self::WithEnv { message, .. } => message.as_str(),
            _ => return None,
        };
        msg.find(Self::RECORD_NOT_FOUND_MARKER)
            .map(|idx| msg[idx + Self::RECORD_NOT_FOUND_MARKER.len()..].to_string())
    }

    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self::General {
            message: message.into(),
            span,
        }
    }

    pub fn division_by_zero(span: Span) -> Self {
        Self::DivisionByZero(span)
    }

    pub fn undefined_variable(name: impl Into<String>, span: Span) -> Self {
        Self::UndefinedVariable(name.into(), span)
    }

    pub fn not_callable(span: Span) -> Self {
        Self::NotCallable(span)
    }

    pub fn wrong_arity(expected: usize, got: usize, span: Span) -> Self {
        Self::WrongArity {
            expected,
            got,
            span,
        }
    }

    pub fn type_error(message: impl Into<String>, span: Span) -> Self {
        Self::TypeError {
            message: message.into(),
            span,
        }
    }

    pub fn span(&self) -> Span {
        match self {
            Self::DivisionByZero(span) => *span,
            Self::UndefinedVariable(_, span) => *span,
            Self::NotCallable(span) => *span,
            Self::WrongArity { span, .. } => *span,
            Self::TypeError { span, .. } => *span,
            Self::IndexOutOfBounds { span, .. } => *span,
            Self::NoSuchProperty { span, .. } => *span,
            Self::NotAClass(_, span) => *span,
            Self::General { span, .. } => *span,
            Self::Breakpoint { span, .. } => *span,
            Self::WithEnv { span, .. } => *span,
        }
    }

    /// Check if this is a breakpoint error.
    pub fn is_breakpoint(&self) -> bool {
        matches!(self, Self::Breakpoint { .. })
    }

    /// Get the environment JSON from a breakpoint or WithEnv error.
    pub fn breakpoint_env_json(&self) -> Option<&str> {
        match self {
            Self::Breakpoint { env_json, .. } => Some(env_json),
            Self::WithEnv { env_json, .. } => Some(env_json),
            _ => None,
        }
    }

    /// Create a WithEnv error with captured environment and stack trace
    pub fn with_env(
        message: impl Into<String>,
        span: Span,
        env_json: impl Into<String>,
        stack_trace: Vec<String>,
    ) -> Self {
        Self::WithEnv {
            message: message.into(),
            span,
            env_json: env_json.into(),
            stack_trace,
        }
    }

    /// Check if this error has captured environment
    pub fn has_captured_env(&self) -> bool {
        matches!(self, Self::WithEnv { .. })
    }

    /// Get the stack trace from a breakpoint or WithEnv error.
    pub fn breakpoint_stack_trace(&self) -> Option<&[String]> {
        match self {
            Self::Breakpoint { stack_trace, .. } => Some(stack_trace),
            Self::WithEnv { stack_trace, .. } => Some(stack_trace),
            _ => None,
        }
    }
}

/// A unified error type for all phases.
#[derive(Debug, Error)]
pub enum SolilangError {
    #[error("Lexer error: {0}")]
    Lexer(#[from] LexerError),

    #[error("Parser error: {0}")]
    Parser(#[from] ParserError),

    #[error("Type error: {0}")]
    Type(#[from] TypeError),

    #[error("Compile error: {0}")]
    Compile(#[from] CompileError),

    #[error("Runtime error: {0}")]
    Runtime(#[from] RuntimeError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(line: usize, col: usize) -> Span {
        Span::new(0, 1, line, col)
    }

    // =====================================================================
    // LexerError
    // =====================================================================

    #[test]
    fn lexer_unexpected_char_carries_span_and_displays() {
        let e = LexerError::unexpected_char('@', span(3, 5));
        assert_eq!(e.span(), span(3, 5));
        let msg = e.to_string();
        assert!(msg.contains("Unexpected character"), "{msg}");
        assert!(msg.contains("@"), "{msg}");
        assert!(msg.contains("3:5"), "{msg}");
    }

    #[test]
    fn lexer_unterminated_string_carries_span() {
        let e = LexerError::unterminated_string(span(7, 1));
        assert_eq!(e.span(), span(7, 1));
        assert!(e.to_string().contains("Unterminated string"));
    }

    #[test]
    fn lexer_invalid_escape_carries_span_and_char() {
        let e = LexerError::invalid_escape('z', span(1, 9));
        assert_eq!(e.span(), span(1, 9));
        let msg = e.to_string();
        assert!(msg.contains("Invalid escape"));
        assert!(msg.contains("z"));
    }

    #[test]
    fn lexer_invalid_number_carries_span_and_text() {
        let e = LexerError::invalid_number("12abc".to_string(), span(2, 3));
        assert_eq!(e.span(), span(2, 3));
        assert!(e.to_string().contains("12abc"));
    }

    // =====================================================================
    // ParserError
    // =====================================================================

    #[test]
    fn parser_unexpected_token_includes_expected_and_found() {
        let e = ParserError::unexpected_token(";", "}", span(1, 1));
        let msg = e.to_string();
        assert!(msg.contains("expected ;"), "{msg}");
        assert!(msg.contains("}"), "{msg}");
        assert_eq!(e.span(), span(1, 1));
    }

    #[test]
    fn parser_unexpected_eof_span() {
        let e = ParserError::unexpected_eof(span(99, 0));
        assert_eq!(e.span(), span(99, 0));
        assert!(e.to_string().contains("Unexpected end of file"));
    }

    #[test]
    fn parser_invalid_assignment_target_span() {
        let e = ParserError::invalid_assignment_target(span(2, 2));
        assert_eq!(e.span(), span(2, 2));
        assert!(e.to_string().contains("Invalid assignment target"));
    }

    #[test]
    fn parser_general_carries_message() {
        let e = ParserError::general("oh no", span(1, 1));
        assert!(e.to_string().contains("oh no"));
    }

    #[test]
    fn parser_from_lexer_error_preserves_span_and_message() {
        let lex = LexerError::unexpected_char('?', span(4, 8));
        let parsed: ParserError = lex.into();
        assert_eq!(parsed.span(), span(4, 8));
        // Wrapped LexerError display gets embedded in the General message.
        assert!(parsed.to_string().contains("Unexpected character"));
    }

    // =====================================================================
    // TypeError
    // =====================================================================

    #[test]
    fn type_mismatch_carries_expected_found_span() {
        let e = TypeError::mismatch("Int", "String", span(1, 1));
        assert_eq!(e.span(), span(1, 1));
        let msg = e.to_string();
        assert!(msg.contains("expected Int"), "{msg}");
        assert!(msg.contains("found String"), "{msg}");
    }

    #[test]
    fn type_error_span_accessor_covers_all_variants() {
        let s = span(2, 2);
        let cases = vec![
            TypeError::Mismatch {
                expected: "Int".into(),
                found: "Str".into(),
                span: s,
            },
            TypeError::UndefinedVariable("x".into(), s),
            TypeError::UndefinedType("Foo".into(), s),
            TypeError::UndefinedFunction("f".into(), s),
            TypeError::NotCallable("Int".into(), s),
            TypeError::WrongArity {
                expected: 1,
                got: 2,
                span: s,
            },
            TypeError::NoSuchMember {
                type_name: "Foo".into(),
                member: "bar".into(),
                span: s,
            },
            TypeError::NoSuperclass("A".into(), s),
            TypeError::ThisOutsideClass(s),
            TypeError::SuperOutsideClass(s),
            TypeError::General {
                message: "boom".into(),
                span: s,
            },
        ];
        for e in &cases {
            assert_eq!(e.span(), s, "span() mismatch for {e:?}");
        }
    }

    #[test]
    fn type_no_such_member_message_includes_type_and_member() {
        let e = TypeError::NoSuchMember {
            type_name: "Array".into(),
            member: "frobnicate".into(),
            span: span(1, 1),
        };
        let msg = e.to_string();
        assert!(msg.contains("Array"));
        assert!(msg.contains("frobnicate"));
    }

    // =====================================================================
    // CompileError
    // =====================================================================

    #[test]
    fn compile_error_span_and_display() {
        let e = CompileError::new("bad", span(5, 5));
        assert_eq!(e.span(), span(5, 5));
        assert!(e.to_string().contains("bad"));
    }

    // =====================================================================
    // RuntimeError
    // =====================================================================

    #[test]
    fn runtime_error_span_accessor_covers_all_variants() {
        let s = span(2, 2);
        let cases = vec![
            RuntimeError::DivisionByZero(s),
            RuntimeError::UndefinedVariable("x".into(), s),
            RuntimeError::NotCallable(s),
            RuntimeError::WrongArity {
                expected: 1,
                got: 2,
                span: s,
            },
            RuntimeError::TypeError {
                message: "m".into(),
                span: s,
            },
            RuntimeError::IndexOutOfBounds {
                index: 1,
                length: 0,
                span: s,
            },
            RuntimeError::NoSuchProperty {
                value_type: "v".into(),
                property: "p".into(),
                span: s,
            },
            RuntimeError::NotAClass("X".into(), s),
            RuntimeError::General {
                message: "m".into(),
                span: s,
            },
            RuntimeError::Breakpoint {
                span: s,
                env_json: "{}".into(),
                stack_trace: vec![],
            },
            RuntimeError::WithEnv {
                message: "m".into(),
                span: s,
                env_json: "{}".into(),
                stack_trace: vec![],
            },
        ];
        for e in &cases {
            assert_eq!(e.span(), s, "span() mismatch for {e:?}");
        }
    }

    #[test]
    fn runtime_division_by_zero_displays_with_span() {
        let e = RuntimeError::division_by_zero(span(1, 1));
        assert!(e.to_string().contains("Division by zero"));
        assert!(e.to_string().contains("1:1"));
    }

    #[test]
    fn runtime_index_out_of_bounds_includes_index_and_length() {
        let e = RuntimeError::IndexOutOfBounds {
            index: 5,
            length: 3,
            span: span(1, 1),
        };
        let msg = e.to_string();
        assert!(msg.contains("5"));
        assert!(msg.contains("3"));
    }

    #[test]
    fn runtime_with_env_does_not_double_append_span_in_display() {
        // WithEnv display shows only `{message}` so wrapping an inner
        // already-spanned message doesn't produce "... at 1:1 at 1:1".
        let e = RuntimeError::with_env("inner at 1:1", span(2, 2), "{}".to_string(), vec![]);
        assert_eq!(e.to_string(), "inner at 1:1");
    }

    // ---------- record-not-found marker round trip ----------

    #[test]
    fn record_not_found_message_round_trips() {
        let e = RuntimeError::record_not_found("user 42", span(1, 1));
        assert!(e.is_record_not_found());
        assert_eq!(e.record_not_found_message().as_deref(), Some("user 42"));
    }

    #[test]
    fn record_not_found_marker_survives_wrapping_prefix() {
        // The doc comment guarantees the marker is found via `contains`,
        // not `starts_with`, so prefixed wrappers still classify correctly.
        let e = RuntimeError::General {
            message: format!(
                "Error calling method: {}{}",
                RuntimeError::RECORD_NOT_FOUND_MARKER,
                "user 7"
            ),
            span: span(1, 1),
        };
        assert!(e.is_record_not_found());
        assert_eq!(e.record_not_found_message().as_deref(), Some("user 7"));
    }

    #[test]
    fn record_not_found_false_for_unrelated_errors() {
        assert!(!RuntimeError::division_by_zero(span(1, 1)).is_record_not_found());
        assert!(RuntimeError::division_by_zero(span(1, 1))
            .record_not_found_message()
            .is_none());

        let plain = RuntimeError::new("just a regular error", span(1, 1));
        assert!(!plain.is_record_not_found());
        assert!(plain.record_not_found_message().is_none());
    }

    #[test]
    fn record_not_found_works_for_with_env_variant() {
        let msg = format!("{}lost record", RuntimeError::RECORD_NOT_FOUND_MARKER);
        let e = RuntimeError::with_env(msg, span(1, 1), "{}".to_string(), vec![]);
        assert!(e.is_record_not_found());
        assert_eq!(e.record_not_found_message().as_deref(), Some("lost record"));
    }

    // ---------- breakpoint helpers ----------

    #[test]
    fn breakpoint_is_breakpoint_and_exposes_env_and_trace() {
        let trace = vec!["frame_a".to_string(), "frame_b".to_string()];
        let e = RuntimeError::Breakpoint {
            span: span(1, 1),
            env_json: r#"{"x":1}"#.to_string(),
            stack_trace: trace.clone(),
        };
        assert!(e.is_breakpoint());
        assert_eq!(e.breakpoint_env_json(), Some(r#"{"x":1}"#));
        assert_eq!(e.breakpoint_stack_trace(), Some(trace.as_slice()));
    }

    #[test]
    fn with_env_exposes_env_and_trace_but_is_not_breakpoint() {
        let trace = vec!["frame_z".to_string()];
        let e = RuntimeError::with_env("boom", span(1, 1), r#"{"y":2}"#.to_string(), trace.clone());
        assert!(!e.is_breakpoint());
        assert!(e.has_captured_env());
        assert_eq!(e.breakpoint_env_json(), Some(r#"{"y":2}"#));
        assert_eq!(e.breakpoint_stack_trace(), Some(trace.as_slice()));
    }

    #[test]
    fn non_breakpoint_errors_have_no_env_or_trace() {
        let e = RuntimeError::division_by_zero(span(1, 1));
        assert!(!e.is_breakpoint());
        assert!(!e.has_captured_env());
        assert!(e.breakpoint_env_json().is_none());
        assert!(e.breakpoint_stack_trace().is_none());
    }

    // =====================================================================
    // SolilangError From conversions + Display prefix
    // =====================================================================

    #[test]
    fn solilang_error_from_lexer_uses_lexer_prefix() {
        let inner = LexerError::unexpected_char('?', span(1, 1));
        let outer: SolilangError = inner.into();
        let msg = outer.to_string();
        assert!(msg.starts_with("Lexer error:"), "{msg}");
    }

    #[test]
    fn solilang_error_from_parser_uses_parser_prefix() {
        let inner = ParserError::general("nope", span(1, 1));
        let outer: SolilangError = inner.into();
        assert!(outer.to_string().starts_with("Parser error:"));
    }

    #[test]
    fn solilang_error_from_type_uses_type_prefix() {
        let inner = TypeError::mismatch("a", "b", span(1, 1));
        let outer: SolilangError = inner.into();
        assert!(outer.to_string().starts_with("Type error:"));
    }

    #[test]
    fn solilang_error_from_compile_uses_compile_prefix() {
        let inner = CompileError::new("nope", span(1, 1));
        let outer: SolilangError = inner.into();
        assert!(outer.to_string().starts_with("Compile error:"));
    }

    #[test]
    fn solilang_error_from_runtime_uses_runtime_prefix() {
        let inner = RuntimeError::division_by_zero(span(1, 1));
        let outer: SolilangError = inner.into();
        assert!(outer.to_string().starts_with("Runtime error:"));
    }

    #[test]
    fn solilang_error_from_io_uses_io_prefix() {
        let inner = std::io::Error::new(std::io::ErrorKind::NotFound, "nope");
        let outer: SolilangError = inner.into();
        assert!(outer.to_string().starts_with("IO error:"));
    }
}
