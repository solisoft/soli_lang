//! `soli cloud` — deploy an app and move its alias.
//!
//! The chain, end to end:
//!
//! ```text
//! build (Builder)  →  ship (ssh)  →  release dir  →  repoint symlink
//!                  →  proxy deploy  →  health gate  →  alias
//! ```
//!
//! # What makes this a deployment primitive rather than a script
//!
//! **The deployment is immutable and the alias moves.** A build lands in
//! `releases/<app>/<id>/` and is never touched again; `sites/<app>` is a symlink
//! that points at one of them. A rollback is therefore repointing a symlink —
//! constant time, no rebuild, and the bytes it goes back to are provably the
//! bytes that were serving before.
//!
//! # The ordering that is not negotiable
//!
//! The proxy is asked to deploy **before** the alias moves, and the health gate
//! sits between them. Moving the alias first would route real traffic at a
//! release that has not started yet; the whole point of blue/green is that the
//! old slot keeps serving until the new one answers.

pub mod plan;
pub mod proxy;
pub mod release;
pub mod run;

#[allow(unused_imports)]
pub use release::{Layout, ReleaseId};
