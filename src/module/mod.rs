//! Module system for Solilang.
//!
//! This module provides:
//! - Import/export resolution
//! - Package file (soli.toml) parsing
//! - Module dependency graph building
//! - Circular dependency detection

pub mod credentials;
pub mod installer;
pub mod lockfile;
mod package;
pub mod registry;
mod resolver;

pub use package::{Dependency, Package};
pub use resolver::{ModuleResolver, ResolvedModule};
