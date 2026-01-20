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
}

impl RuntimeError {
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
