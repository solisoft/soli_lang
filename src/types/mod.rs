//! Type system module for Solilang.

pub mod checker;
pub mod environment;
pub mod generics;
pub mod runtime_globals;
pub mod type_repr;
pub mod xml_classes;

pub use checker::TypeChecker;
pub use type_repr::Type;
