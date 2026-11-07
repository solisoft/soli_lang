//! Error types for all compilation phases.

use crate::span::Span;
use thiserror::Error;

pub mod suggest;
pub use suggest::Hint;

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

    /// The message, without the ` at <span>` suffix `Display` appends.
    pub fn message_without_span(&self) -> String {
        match self {
            Self::UnexpectedToken {
                expected, found, ..
            } => format!("Unexpected token '{}', expected {}", found, expected),
            Self::UnexpectedEof(_) => "Unexpected end of file".to_string(),
            Self::InvalidAssignmentTarget(_) => "Invalid assignment target".to_string(),
            Self::General { message, .. } => message.clone(),
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

    /// The third field is a "did you mean" suggestion.
    #[error("Undefined variable '{0}'{} at {1}", suggest::suffix(.2.as_deref()))]
    UndefinedVariable(String, Span, Option<String>),

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

    #[error("Cannot access member '{member}' on type '{type_name}'{} at {span}", suggest::suffix(.suggestion.as_deref()))]
    NoSuchMember {
        type_name: String,
        member: String,
        span: Span,
        /// A "did you mean" suggestion.
        suggestion: Option<String>,
    },

    #[error("Class '{0}' has no superclass at {1}")]
    NoSuperclass(String, Span),

    #[error("Cannot use 'this' outside of a class at {0}")]
    ThisOutsideClass(Span),

    #[error("Cannot use 'super' outside of a class at {0}")]
    SuperOutsideClass(Span),

    #[error("{message} at {span}")]
    General { message: String, span: Span },

    /// Non-blocking warning: a `match` on a known enum doesn't cover every
    /// variant and has no `_` catch-all. Surfaced by `soli check`; never fails
    /// the check or blocks `soli run`.
    #[error(
        "warning: match on enum '{enum_name}' is not exhaustive at {span} — missing: {missing} (add them, or a `_ =>` arm)"
    )]
    ExhaustivenessWarning {
        enum_name: String,
        missing: String,
        span: Span,
    },
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
            Self::UndefinedVariable(_, span, _) => *span,
            Self::UndefinedType(_, span) => *span,
            Self::UndefinedFunction(_, span) => *span,
            Self::NotCallable(_, span) => *span,
            Self::WrongArity { span, .. } => *span,
            Self::NoSuchMember { span, .. } => *span,
            Self::NoSuperclass(_, span) => *span,
            Self::ThisOutsideClass(span) => *span,
            Self::SuperOutsideClass(span) => *span,
            Self::General { span, .. } => *span,
            Self::ExhaustivenessWarning { span, .. } => *span,
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

/// How an uncaught thrown value is written into an error message.
///
/// A class used as an exception carries its text in a `message` field, so
/// rendering the instance itself would print `<AppError instance>` and lose
/// the only part anyone wants to read. The VM did this extraction inline
/// before thrown values were preserved end-to-end; it lives here now so both
/// engines get it from the one place that formats a `Thrown`.
pub(crate) fn render_thrown(value: &crate::interpreter::value::Value) -> String {
    use crate::interpreter::value::Value;
    match value {
        Value::String(s) => s.to_string(),
        Value::Instance(inst) => {
            let inst = inst.borrow();
            match inst.fields.get("message") {
                Some(msg) => format!("{msg}"),
                None => format!("<{} instance>", inst.class.name),
            }
        }
        other => format!("{other}"),
    }
}

/// The error class an error *message* reports as, for the paths that only
/// kept the text (a middleware's `Error(String)`, a handler that failed to
/// resolve). Mirrors [`RuntimeError::exception_type`] on the prefixes its
/// `Display` writes; anything else is a `RuntimeError`.
pub fn classify_error_message(message: &str) -> &'static str {
    let m = message.trim_start();
    if m.starts_with("Unhandled exception:") {
        "Error"
    } else if m.starts_with("Type error:") || m.starts_with("Cannot call non-function") {
        "TypeError"
    } else if m.starts_with("Division by zero") {
        "ZeroDivisionError"
    } else if m.starts_with("Undefined variable") {
        "NameError"
    } else if m.starts_with("Wrong number of arguments") {
        "ArgumentError"
    } else if m.starts_with("Index out of bounds") {
        "IndexError"
    } else if m.starts_with("Cannot access property") {
        "NoMethodError"
    } else {
        "RuntimeError"
    }
}

/// Runtime errors.
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("Division by zero at {0}")]
    DivisionByZero(Span),

    /// The third field is a "did you mean" suggestion, usually lazy.
    #[error("Undefined variable '{0}'{} at {1}", .2.suffix())]
    UndefinedVariable(String, Span, Hint),

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

    /// Boxed: two `String`s and a `Span` would make this the second-widest
    /// variant (64 bytes) and set the size of every `Result<_, RuntimeError>`.
    /// Build it with [`RuntimeError::no_such_property`].
    #[error("{0}")]
    NoSuchProperty(Box<NoSuchPropertyError>),

    #[error("Cannot instantiate '{0}' at {1}")]
    NotAClass(String, Span),

    /// The VM deliberately refuses this operation so the serve-mode
    /// interpreter fallback handles it (e.g. Model instance mutators, whose
    /// lifecycle callbacks only fire in the tree-walker). NOT routed through
    /// `try`/`rescue` handlers — a user-level catch must never swallow it,
    /// or the fallback (and the callbacks) would be skipped.
    #[error("{0} requires the interpreter at {1}")]
    EngineFallback(String, Span),

    /// A user-level `throw` that escaped the function it was thrown in.
    ///
    /// Within one function body a throw travels as `ControlFlow::Throw(value)`
    /// and reaches `catch` with the value intact. Crossing a call boundary
    /// means leaving Soli's control-flow enum for Rust's `Result`, so the
    /// thrown value needs somewhere to ride — hence this variant. Formatting
    /// it stringifies the payload, which is what an *uncaught* throw should
    /// print, but `try`/`catch` unwraps `value` instead of the message, so a
    /// thrown hash is still a hash once it is caught.
    ///
    /// Before this existed the value was stringified at the call boundary and
    /// `catch` re-wrapped that message as a `String`, so `throw {"code": 404}`
    /// arrived as the *text* `Unhandled exception: {code => 404} at 0:0` and
    /// `e["code"]` failed — but only in the interpreter. The VM has always
    /// preserved the value, so this is the interpreter matching production.
    #[error("Unhandled exception: {} at {span}", render_thrown(.value))]
    Thrown {
        value: crate::interpreter::value::Value,
        span: Span,
    },

    #[error("{message} at {span}")]
    General { message: String, span: Span },

    /// A `debug()` breakpoint. Boxed with its captured environment: see
    /// [`DebugCapture`]. Build it with [`RuntimeError::breakpoint`].
    #[error("Breakpoint hit at {}", .0.span)]
    Breakpoint(Box<DebugCapture>),

    /// Error with captured environment for debugging
    /// This allows accessing local variables in the dev error page REPL.
    /// Note: callers wrap an inner error via `e.to_string()`, which already
    /// includes its own " at {span}" suffix — so we display only `{message}`
    /// here to avoid producing "... at 92:20 at 92:20".
    ///
    /// Boxed: see [`DebugCapture`]. Build it with [`RuntimeError::with_env`].
    #[error("{}", .0.message)]
    WithEnv(Box<DebugCapture>),
}

/// The payload of [`RuntimeError::NoSuchProperty`].
///
/// It lives behind a `Box` so that `RuntimeError` stays small: every VM and
/// interpreter helper returns `Result<_, RuntimeError>`, so the widest variant
/// sets the size of each of those return values (they are returned through
/// memory) and of the temporaries in `run_dispatch`'s stack frame. Building
/// the error already allocates its two strings; the box is one more
/// allocation on a path that is an error by definition.
#[derive(Debug)]
pub struct NoSuchPropertyError {
    pub value_type: String,
    pub property: String,
    pub span: Span,
    /// A "did you mean" suggestion. When it is empty and the receiver is a
    /// primitive, the built-in method table is searched instead.
    pub hint: Hint,
}

impl std::fmt::Display for NoSuchPropertyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hint = if self.hint.is_none() {
            suggest::suffix(
                suggest::primitive_method_suggestion(&self.value_type, &self.property).as_deref(),
            )
        } else {
            self.hint.suffix()
        };
        write!(
            f,
            "Cannot access property '{}' on {}{} at {}",
            self.property, self.value_type, hint, self.span
        )
    }
}

/// The payload of [`RuntimeError::Breakpoint`] and [`RuntimeError::WithEnv`]:
/// the environment and stack captured for the dev error page and its REPL.
///
/// Boxed for the same reason as [`NoSuchPropertyError`]: unboxed, these two
/// variants were 88 bytes wide and made every `Result<_, RuntimeError>` 88
/// bytes. They are built only when an error is about to reach the dev error
/// page (or on a `debug()` breakpoint), never on a path a program takes
/// while it runs normally. `message` is empty for a breakpoint.
#[derive(Debug)]
pub struct DebugCapture {
    pub message: String,
    pub span: Span,
    /// JSON-serialized environment variables for debugging
    pub env_json: String,
    /// Stack trace captured at the moment of the error or breakpoint
    pub stack_trace: Vec<String>,
    /// The error class of the error this one wraps (see
    /// [`RuntimeError::with_env_from`]), which `message` alone has lost:
    /// a thrown `PaymentError` renders as its message only.
    pub exception_type: Option<String>,
}

// The point of the boxing above. 48 = the widest remaining variants (a
// `String` or a `Value`, plus a `Span`: 40 bytes) and a tag. Widening a
// variant past that should be a decision, not an accident.
const _: () = assert!(std::mem::size_of::<RuntimeError>() <= 48);

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
            Self::General { message, .. } => message.as_str(),
            Self::WithEnv(capture) => capture.message.as_str(),
            _ => return false,
        };
        msg.contains(Self::RECORD_NOT_FOUND_MARKER)
    }

    /// The user-facing message from a record-not-found error (marker stripped).
    pub fn record_not_found_message(&self) -> Option<String> {
        let msg = match self {
            Self::General { message, .. } => message.as_str(),
            Self::WithEnv(capture) => capture.message.as_str(),
            _ => return None,
        };
        msg.find(Self::RECORD_NOT_FOUND_MARKER)
            .map(|idx| msg[idx + Self::RECORD_NOT_FOUND_MARKER.len()..].to_string())
    }

    /// Sentinel embedded in the error message when authorization fails (the
    /// `forbidden()` builtin / the auth Policy layer's `authorize()` helper).
    /// The HTTP request handler looks for this marker and converts the error
    /// into a 403 response instead of the default 500.
    pub const FORBIDDEN_MARKER: &'static str = "__Forbidden__:";

    /// The user-facing message from a forbidden error (marker stripped).
    /// `Some` only when the error carries the forbidden marker.
    pub fn forbidden_message(&self) -> Option<String> {
        let msg = match self {
            Self::General { message, .. } => message.as_str(),
            Self::WithEnv(capture) => capture.message.as_str(),
            _ => return None,
        };
        msg.find(Self::FORBIDDEN_MARKER)
            .map(|idx| msg[idx + Self::FORBIDDEN_MARKER.len()..].to_string())
    }

    /// Sentinel embedded in the error message by `halt(status, message)`. The
    /// request handler turns it into that response (status, `text/plain`
    /// body), wherever it was raised — an action, a `before_action`, a
    /// middleware, a helper several calls deep. Encoded
    /// `<status>:<byte length>:<message>`: the length lets the message be cut
    /// back out exactly, since raising appends the position (` at 3:17`).
    pub const HALT_MARKER: &'static str = "__Halt__:";

    /// The sentinel message `halt(status, message)` raises.
    pub fn halt_sentinel(status: i64, message: &str) -> String {
        format!(
            "{}{}:{}:{}",
            Self::HALT_MARKER,
            status,
            message.len(),
            message
        )
    }

    /// The `(status, message)` a `halt(...)` raised, if this error is one.
    pub fn halt_parts(&self) -> Option<(u16, String)> {
        let msg = match self {
            Self::General { message, .. } => message.as_str(),
            Self::WithEnv(capture) => capture.message.as_str(),
            _ => return None,
        };
        let rest = &msg[msg.find(Self::HALT_MARKER)? + Self::HALT_MARKER.len()..];
        let (status, rest) = rest.split_once(':')?;
        let (len, rest) = rest.split_once(':')?;
        let message = rest.get(..len.parse::<usize>().ok()?)?;
        Some((status.parse().ok()?, message.to_string()))
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
        Self::UndefinedVariable(name.into(), span, Hint::none())
    }

    /// An undefined-variable error whose suggestion is searched for only
    /// when the message is shown.
    pub fn undefined_variable_hinted(name: impl Into<String>, span: Span, hint: Hint) -> Self {
        Self::UndefinedVariable(name.into(), span, hint)
    }

    /// Attach a "did you mean" hint to an undefined-variable or
    /// unknown-property error; any other error is returned unchanged.
    pub fn with_hint(mut self, hint: Hint) -> Self {
        match &mut self {
            Self::UndefinedVariable(_, _, slot) => *slot = hint,
            Self::NoSuchProperty(info) => info.hint = hint,
            _ => {}
        }
        self
    }

    /// Whether this is an undefined-variable error, without formatting it
    /// (formatting would compute its suggestion). Template rendering reads an
    /// undefined local as `nil` through this.
    pub fn is_undefined_variable(&self) -> bool {
        match self {
            Self::UndefinedVariable(..) => true,
            Self::General { message, .. } => message.contains("Undefined variable '"),
            Self::WithEnv(capture) => capture.message.contains("Undefined variable '"),
            _ => false,
        }
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
            Self::UndefinedVariable(_, span, _) => *span,
            Self::NotCallable(span) => *span,
            Self::WrongArity { span, .. } => *span,
            Self::TypeError { span, .. } => *span,
            Self::IndexOutOfBounds { span, .. } => *span,
            Self::NoSuchProperty(info) => info.span,
            Self::NotAClass(_, span) => *span,
            Self::EngineFallback(_, span) => *span,
            Self::Thrown { span, .. } => *span,
            Self::General { span, .. } => *span,
            Self::Breakpoint(capture) | Self::WithEnv(capture) => capture.span,
        }
    }

    /// The text a user's `catch` should bind for this error.
    ///
    /// Errors that the request handler routes by *sentinel* — `Model.find`
    /// missing a record, `forbidden()` / the Policy layer — carry that
    /// sentinel inside their message, because the marker is how the HTTP layer
    /// recognises them (see `RECORD_NOT_FOUND_MARKER`, `FORBIDDEN_MARKER`).
    /// That is an implementation detail of the routing, and it has no business
    /// reaching application code: catching one and rendering it put the literal
    /// text `__Forbidden__:nope at 2:3` on the page.
    ///
    /// Both engines call this at the single point where an error becomes a
    /// catchable value, so they cannot drift apart on it. The sentinel is
    /// untouched on the error itself — only the value handed to `catch` is
    /// cleaned — so the 404/403 routing is unaffected.
    pub fn catchable_message(&self) -> String {
        self.record_not_found_message()
            .or_else(|| self.forbidden_message())
            .or_else(|| self.halt_parts().map(|(_, message)| message))
            .unwrap_or_else(|| self.to_string())
    }

    /// Check if this is a breakpoint error.
    pub fn is_breakpoint(&self) -> bool {
        matches!(self, Self::Breakpoint(_))
    }

    /// A user-level `throw` in flight across a call boundary.
    ///
    /// The error paths that capture locals for the dev error page must leave
    /// these alone: wrapping one in `WithEnv` replaces the payload with its
    /// rendering, and the `catch` that recovers the value is then handed a
    /// string. `is_breakpoint` guards the same paths for the same reason.
    ///
    /// It is also a `throw` that is *about to be caught* in the overwhelming
    /// majority of cases, so serializing the whole environment to JSON for it
    /// is work thrown away — see the sites that call this.
    pub fn is_thrown(&self) -> bool {
        matches!(self, Self::Thrown { .. })
    }

    /// True when the VM deliberately punted this operation to the
    /// tree-walking interpreter. Must bypass `try`/`rescue` routing.
    pub fn is_engine_fallback(&self) -> bool {
        matches!(self, Self::EngineFallback(..))
    }

    /// Get the environment JSON from a breakpoint or WithEnv error.
    pub fn breakpoint_env_json(&self) -> Option<&str> {
        match self {
            Self::Breakpoint(capture) | Self::WithEnv(capture) => Some(&capture.env_json),
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
        Self::WithEnv(Box::new(DebugCapture {
            message: message.into(),
            span,
            env_json: env_json.into(),
            stack_trace,
            exception_type: None,
        }))
    }

    /// [`Self::with_env`] wrapping `err`: its message and span, and its
    /// error class kept for [`Self::exception_type`].
    pub fn with_env_from(
        err: &RuntimeError,
        env_json: impl Into<String>,
        stack_trace: Vec<String>,
    ) -> Self {
        Self::WithEnv(Box::new(DebugCapture {
            message: err.to_string(),
            span: err.span(),
            env_json: env_json.into(),
            stack_trace,
            exception_type: Some(err.exception_type()),
        }))
    }

    /// The Soli error class this error reports as — `exception.type` on a
    /// trace, `error_type` in a JSON error log.
    ///
    /// A thrown instance is its class (`PaymentError`, `DomainError`); any
    /// other thrown value is `Error`, the base class. Built-in failures map
    /// to the class name a Ruby / Python developer would look for.
    pub fn exception_type(&self) -> String {
        match self {
            Self::Thrown {
                value: crate::interpreter::value::Value::Instance(inst),
                ..
            } => inst.borrow().class.name.clone(),
            Self::Thrown { .. } => "Error".into(),
            Self::DivisionByZero(_) => "ZeroDivisionError".into(),
            Self::UndefinedVariable(..) => "NameError".into(),
            Self::NotCallable(_) | Self::NotAClass(..) | Self::TypeError { .. } => {
                "TypeError".into()
            }
            Self::WrongArity { .. } => "ArgumentError".into(),
            Self::IndexOutOfBounds { .. } => "IndexError".into(),
            Self::NoSuchProperty(_) => "NoMethodError".into(),
            Self::Breakpoint(_) => "Breakpoint".into(),
            Self::WithEnv(capture) => capture
                .exception_type
                .clone()
                .unwrap_or_else(|| classify_error_message(&capture.message).to_string()),
            Self::EngineFallback(..) | Self::General { .. } => {
                classify_error_message(&self.to_string()).to_string()
            }
        }
    }

    /// Create a breakpoint error with captured environment and stack trace
    pub fn breakpoint(span: Span, env_json: String, stack_trace: Vec<String>) -> Self {
        Self::Breakpoint(Box::new(DebugCapture {
            message: String::new(),
            span,
            env_json,
            stack_trace,
            exception_type: None,
        }))
    }

    /// Create a "Cannot access property" error.
    /// Out of line on purpose: the box and the field moves stay in one
    /// cold function instead of being inlined into every call site.
    #[cold]
    #[inline(never)]
    pub fn no_such_property(value_type: String, property: String, span: Span) -> Self {
        Self::NoSuchProperty(Box::new(NoSuchPropertyError {
            value_type,
            property,
            span,
            hint: Hint::none(),
        }))
    }

    /// Get the stack trace from a breakpoint or WithEnv error.
    pub fn breakpoint_stack_trace(&self) -> Option<&[String]> {
        match self {
            Self::Breakpoint(capture) | Self::WithEnv(capture) => Some(&capture.stack_trace),
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

    #[error("Template error: {0}")]
    Template(String),
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
            TypeError::UndefinedVariable("x".into(), s, None),
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
                suggestion: None,
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
            suggestion: None,
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
            RuntimeError::UndefinedVariable("x".into(), s, Hint::none()),
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
            RuntimeError::no_such_property("v".into(), "p".into(), s),
            RuntimeError::NotAClass("X".into(), s),
            RuntimeError::General {
                message: "m".into(),
                span: s,
            },
            RuntimeError::breakpoint(s, "{}".into(), vec![]),
            RuntimeError::with_env("m", s, "{}", vec![]),
        ];
        for e in &cases {
            assert_eq!(e.span(), s, "span() mismatch for {e:?}");
        }
    }

    #[test]
    fn boxed_variants_keep_their_display_text() {
        // The payloads of these three moved behind a `Box` to shrink
        // `RuntimeError`; what users and the dev error page read must not move.
        let s = span(3, 7);
        assert_eq!(
            RuntimeError::no_such_property("Int".into(), "nope".into(), s).to_string(),
            "Cannot access property 'nope' on Int at 3:7"
        );
        assert_eq!(
            RuntimeError::breakpoint(s, "{}".into(), vec![]).to_string(),
            "Breakpoint hit at 3:7"
        );
        assert_eq!(
            RuntimeError::with_env("boom at 1:1", s, "{}", vec![]).to_string(),
            "boom at 1:1"
        );
    }

    #[test]
    fn runtime_error_stays_small() {
        // Every `Result<_, RuntimeError>` a VM helper returns is this wide.
        assert!(std::mem::size_of::<RuntimeError>() <= 48);
        assert!(
            std::mem::size_of::<Result<crate::interpreter::value::Value, RuntimeError>>() <= 48
        );
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
    fn forbidden_message_round_trips_and_is_isolated() {
        let e = RuntimeError::General {
            message: format!("{}Not authorized", RuntimeError::FORBIDDEN_MARKER),
            span: span(1, 1),
        };
        assert_eq!(e.forbidden_message().as_deref(), Some("Not authorized"));
        // A forbidden error is not a record-not-found error, and vice versa.
        assert!(!e.is_record_not_found());
        assert!(RuntimeError::record_not_found("x", span(1, 1))
            .forbidden_message()
            .is_none());
        assert!(RuntimeError::new("plain", span(1, 1))
            .forbidden_message()
            .is_none());
    }

    /// The sentinel that routes an error to a 404/403 must never reach the
    /// value a user's `catch` binds — rendering it put `__Forbidden__:nope`
    /// on the page.
    #[test]
    fn catchable_message_strips_routing_sentinels() {
        let forbidden = RuntimeError::General {
            message: format!("{}Not authorized", RuntimeError::FORBIDDEN_MARKER),
            span: span(1, 1),
        };
        assert_eq!(forbidden.catchable_message(), "Not authorized");
        assert!(!forbidden
            .catchable_message()
            .contains(RuntimeError::FORBIDDEN_MARKER));

        let missing = RuntimeError::record_not_found("Post with id 5", span(1, 1));
        assert_eq!(missing.catchable_message(), "Post with id 5");
        assert!(!missing
            .catchable_message()
            .contains(RuntimeError::RECORD_NOT_FOUND_MARKER));

        // halt: the message is cut back out exactly, even with the position
        // raising appends and a message that itself contains `:`.
        let halted = RuntimeError::General {
            message: format!("{} at 3:17", RuntimeError::halt_sentinel(429, "slow: down")),
            span: span(3, 17),
        };
        assert_eq!(halted.halt_parts(), Some((429, "slow: down".to_string())));
        assert_eq!(halted.catchable_message(), "slow: down");

        // An ordinary error is unchanged — it renders as it always did,
        // span suffix included.
        let plain = RuntimeError::new("something broke", span(3, 7));
        assert_eq!(plain.catchable_message(), plain.to_string());
        assert!(plain.catchable_message().contains("something broke"));
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
        let e = RuntimeError::breakpoint(span(1, 1), r#"{"x":1}"#.to_string(), trace.clone());
        assert!(e.is_breakpoint());
        assert_eq!(e.breakpoint_env_json(), Some(r#"{"x":1}"#));
        assert_eq!(e.breakpoint_stack_trace(), Some(trace.as_slice()));
    }

    #[test]
    fn with_env_exposes_env_and_trace_but_is_not_breakpoint() {
        let trace = vec!["frame_z".to_string()];
        let e = RuntimeError::with_env("boom", span(1, 1), r#"{"y":2}"#.to_string(), trace.clone());
        assert!(!e.is_breakpoint());
        assert_eq!(e.breakpoint_env_json(), Some(r#"{"y":2}"#));
        assert_eq!(e.breakpoint_stack_trace(), Some(trace.as_slice()));
    }

    #[test]
    fn non_breakpoint_errors_have_no_env_or_trace() {
        let e = RuntimeError::division_by_zero(span(1, 1));
        assert!(!e.is_breakpoint());
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

    #[test]
    fn exception_type_of_builtin_failures() {
        let s = Span::default();
        assert_eq!(
            RuntimeError::DivisionByZero(s).exception_type(),
            "ZeroDivisionError"
        );
        assert_eq!(
            RuntimeError::UndefinedVariable("x".into(), s, Hint::none()).exception_type(),
            "NameError"
        );
        assert_eq!(
            RuntimeError::TypeError {
                message: "m".into(),
                span: s
            }
            .exception_type(),
            "TypeError"
        );
        assert_eq!(
            RuntimeError::new("boom", s).exception_type(),
            "RuntimeError"
        );
        assert_eq!(
            RuntimeError::Thrown {
                value: crate::interpreter::value::Value::String("bad".into()),
                span: s
            }
            .exception_type(),
            "Error"
        );
    }

    /// The controller boundary flattens an error into `WithEnv`; the class
    /// must survive it, or every trace would say `RuntimeError`.
    #[test]
    fn with_env_from_keeps_the_wrapped_class() {
        let s = Span::default();
        let inner = RuntimeError::DivisionByZero(s);
        let wrapped = RuntimeError::with_env_from(&inner, "{}", vec![]);
        assert_eq!(wrapped.exception_type(), "ZeroDivisionError");
        assert_eq!(wrapped.to_string(), inner.to_string());
        let twice = RuntimeError::with_env_from(&wrapped, "{}", vec![]);
        assert_eq!(twice.exception_type(), "ZeroDivisionError");
        // A bare `with_env` falls back to the message.
        let bare = RuntimeError::with_env("Type error: nope at 1:1", s, "{}", vec![]);
        assert_eq!(bare.exception_type(), "TypeError");
    }

    #[test]
    fn classify_error_message_prefixes() {
        assert_eq!(
            classify_error_message("Unhandled exception: x at 1:1"),
            "Error"
        );
        assert_eq!(
            classify_error_message("Division by zero at 1:1"),
            "ZeroDivisionError"
        );
        assert_eq!(
            classify_error_message("Undefined variable 'y' at 1:1"),
            "NameError"
        );
        assert_eq!(
            classify_error_message("Cannot access property 'a' on Int at 1:1"),
            "NoMethodError"
        );
        assert_eq!(classify_error_message("something else"), "RuntimeError");
    }
}
