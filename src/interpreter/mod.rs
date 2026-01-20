//! Interpreter module for Solilang.

pub mod builtins;
pub mod environment;
pub mod executor;
pub mod value;

pub use environment::Environment;
pub use executor::Interpreter;
pub use value::Value;
