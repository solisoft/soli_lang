//! Module system for Solilang.
//!
//! This module provides:
//! - Import/export resolution
//! - Package file (soli.toml) parsing
//! - Module dependency graph building
//! - Circular dependency detection

pub mod builder;
pub mod credentials;
// `soli deploy` is built on ssh2, which is a Unix-only dependency and, since
// it compiles OpenSSL from source, the most expensive one here (see the note
// in Cargo.toml). Deploying to a remote server is a server-ops feature; a
// Windows desktop build has no use for it and must not fail to compile over
// it, and an offline build drops it with the `ssh` feature.
#[cfg(all(unix, feature = "ssh"))]
pub mod deploy;
// Reading deploy.toml is not: `soli cloud` and `soli env` take their target
// server from it and build everywhere.
pub mod deploy_config;
pub mod installer;
pub mod lockfile;
mod package;
pub mod preview;
pub mod registry;
mod resolver;
mod tar_extract;

pub use package::{
    compare_versions, enforce_min_soli_version, is_valid_version, pinned_soli_version, Dependency,
    Package,
};
pub use resolver::{ModuleResolver, ResolvedModule};
