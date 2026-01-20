//! Type system module for Solilang.

pub mod checker;
pub mod environment;
pub mod type_repr;

pub use checker::TypeChecker;
pub use type_repr::Type;
