//! Soli language reference docs (www/docs/*.md), embedded into the binary so
//! `soli new` can copy them into every scaffolded app under `docs/`.
//!
//! The bundled tree is the *language* reference (controllers, models, views,
//! middleware, testing, migrations, configuration, ...). Per-directory
//! CLAUDE.md files inside a scaffolded app point at `docs/<topic>.md` —
//! always resolvable inside the app, no network round-trip required.

use include_dir::{include_dir, Dir};

/// The `www/docs/` tree, embedded at compile time.
///
/// `www/` is mostly excluded from the published crate (see `Cargo.toml`),
/// but `www/docs/` is intentionally kept so this `include_dir!` resolves
/// both from a source checkout and from `cargo install solilang`.
pub static DOCS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/www/docs");

/// Files to skip when copying the bundled docs into a user's app. These are
/// Soli-repo-internal — irrelevant in a generated app.
const SKIP: &[&str] = &["CLAUDE.md"];

/// Whether `path` (relative to the docs root) should be copied.
pub fn should_copy(relative_path: &str) -> bool {
    !SKIP
        .iter()
        .any(|skip| relative_path == *skip || relative_path.ends_with(&format!("/{skip}")))
}
