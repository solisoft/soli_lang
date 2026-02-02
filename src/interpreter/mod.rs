//! Interpreter module for Solilang.

pub mod builtins;
pub mod environment;
pub mod executor;
pub mod hidden_class;
pub mod inline_cache;
pub mod symbol;
pub mod value;

pub use environment::Environment;
pub use executor::Interpreter;
pub use hidden_class::{
    HiddenClass, HiddenClassObject, HiddenClassRegistry, HIDDEN_CLASS_REGISTRY,
};
pub use inline_cache::{HiddenClassId, MethodInlineCache, PropertyInlineCache, INLINE_CACHE};
pub use symbol::{get_symbol, symbol_string, SymbolId};
pub use value::{unwrap_value, Value};
