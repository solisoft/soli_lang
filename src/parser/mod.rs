//! Parser module for Solilang.

mod core;
mod declarations;
mod expressions;
mod precedence;
mod statements;
mod types;

#[cfg(test)]
mod tests;

pub use self::core::Parser;
