//! Packaging a Soli app as a self-contained desktop application.
//!
//! The artifact is the existing standalone executable shape — soli runtime plus
//! an appended bundle — carrying additionally its own database binary, its
//! encrypted app payload, and any read-only reference data. At launch it
//! resolves an encryption key from a key server, starts a private database on
//! loopback, and serves the app to a browser.
//!
//! Built up in phases; this module currently provides the directory layout that
//! the database and its state live in.

pub mod container;
pub mod db;
pub mod manifest;
pub mod paths;
pub mod seed;
pub mod shell;
pub mod token;
