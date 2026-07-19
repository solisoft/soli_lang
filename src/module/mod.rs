//! Module system for Solilang.
//!
//! This module provides:
//! - Import/export resolution
//! - Package file (soli.toml) parsing
//! - Module dependency graph building
//! - Circular dependency detection

pub mod credentials;
// `soli deploy` is built on ssh2, which is a Unix-only dependency (see the
// note in Cargo.toml). Deploying to a remote server is a server-ops feature; a
// Windows desktop build has no use for it and must not fail to compile over it.
#[cfg(unix)]
pub mod deploy;
pub mod installer;
pub mod lockfile;
mod package;
pub mod registry;
mod resolver;
mod tar_extract;

pub use package::{compare_versions, enforce_min_soli_version, Dependency, Package};
pub use resolver::{ModuleResolver, ResolvedModule};
