//! Lexer module for Solilang.

pub mod scanner;
pub mod token;

pub use scanner::Scanner;
pub use token::{Token, TokenKind};
