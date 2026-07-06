//! Module system for Solilang.
//!
//! This module provides:
//! - Import/export resolution
//! - Package file (soli.toml) parsing
//! - Module dependency graph building
//! - Circular dependency detection

pub mod credentials;
pub mod deploy;
pub mod installer;
pub mod lockfile;
mod package;
pub mod registry;
mod resolver;
mod tar_extract;

pub use package::{compare_versions, enforce_min_soli_version, Dependency, Package};
pub use resolver::{ModuleResolver, ResolvedModule};
