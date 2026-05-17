//! Soli source formatter (`soli fmt`).
//!
//! Parses the source into an AST and re-emits it with canonical formatting:
//!   - 2-space indent
//!   - Ruby-style `class X < Y ... end`, `if cond ... end`, `def name ... end`
//!   - Operator spacing normalized (`a + b`, `a == b`)
//!   - Comments preserved at their original line positions
//!
//! **Coverage**: this is Phase 1. Common nodes (let/const/fn/class/if/while/
//! for/return/throw/try/import/export/literals/binary/unary/call/member/
//! array/hash/lambda/match/pipeline/assign/grouping) are canonically
//! formatted. For nodes the printer hasn't learned yet (SDBQL blocks,
//! comprehensions, complex match patterns), we fall back to copying the
//! original source bytes via the AST span — semantics preserved, formatting
//! left untouched on those specific nodes.

use crate::lexer::Scanner;
use crate::parser::Parser;

mod comments;
mod expressions;
mod printer;
mod statements;

#[cfg(test)]
mod tests;

pub use printer::Printer;

#[derive(Debug)]
pub enum FmtError {
    Lex(String),
    Parse(String),
}

impl std::fmt::Display for FmtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FmtError::Lex(s) => write!(f, "lex error: {}", s),
            FmtError::Parse(s) => write!(f, "parse error: {}", s),
        }
    }
}

impl std::error::Error for FmtError {}

/// Format a Soli source string. Returns the canonical formatting.
pub fn format_source(source: &str) -> Result<String, FmtError> {
    let tokens = Scanner::new(source)
        .scan_tokens()
        .map_err(|e| FmtError::Lex(format!("{:?}", e)))?;
    let program = Parser::new(tokens)
        .parse()
        .map_err(|e| FmtError::Parse(format!("{:?}", e)))?;
    let comments = comments::extract_comments(source);
    let mut printer = Printer::new(source, comments);
    printer.print_program(&program);
    Ok(printer.finish())
}
