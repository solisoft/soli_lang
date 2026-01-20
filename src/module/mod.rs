//! Module system for Solilang.
//!
//! This module provides:
//! - Import/export resolution
//! - Package file (soli.toml) parsing
//! - Module dependency graph building
//! - Circular dependency detection

mod package;
mod resolver;

pub use package::Package;
pub use resolver::{ModuleResolver, ResolvedModule};
