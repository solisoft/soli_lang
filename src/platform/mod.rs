//! Platform-specific primitives, isolated behind cross-platform APIs.
//!
//! Everything here exists because the operation genuinely differs per OS —
//! advisory file locking, process liveness, private directory creation. Keeping
//! the `cfg` branches in one place stops them from spreading through call
//! sites, and gives the Windows port a single set of holes to fill rather than
//! an audit of every module.

pub mod lock;
pub mod process;
