//! Memo of the document tables the SQL adapters have already created.
//!
//! Every document write (`insert`, `insert_many`, `update`) used to run a
//! `CREATE TABLE IF NOT EXISTS` first — a full extra round-trip per write for a
//! statement that is a no-op after the first one. This remembers, process-wide,
//! which `(connection, url, table)` triples have been ensured and skips the DDL
//! for them.
//!
//! Staying correct when the memo is wrong:
//! - **Inside a transaction the memo is bypassed** (neither read nor written).
//!   Postgres and SQLite DDL is transactional, so a table created by a write
//!   that is later rolled back does not exist afterwards; remembering it would
//!   be a lie.
//! - **`drop_table`, `execute_ddl` and `execute_raw` forget everything**, so a
//!   table dropped through the adapter is recreated on the next write.
//! - **A table dropped behind our back** (another process, a raw query) shows up
//!   as a "missing table" error on a remembered table. The entry is forgotten,
//!   the table ensured and the write retried once — the outcome the unmemoised
//!   code gave.

use std::collections::HashSet;
use std::sync::{OnceLock, RwLock};

fn ensured() -> &'static RwLock<HashSet<String>> {
    static ENSURED: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();
    ENSURED.get_or_init(|| RwLock::new(HashSet::new()))
}

/// Keyed by connection name AND url (a connection repointed at another
/// database must ensure its tables there too) and the table.
fn memo_key(table: &str) -> Option<String> {
    super::registry::with_active_spec(|spec| {
        format!(
            "{}\u{1f}{}\u{1f}{table}",
            spec.name,
            spec.url.as_deref().unwrap_or("")
        )
    })
    .ok()
}

fn is_known(key: &str) -> bool {
    ensured()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .contains(key)
}

fn remember(key: String) {
    ensured()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key);
}

fn forget(key: &str) {
    ensured()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(key);
}

/// Forget every ensured table — after a drop or arbitrary DDL.
pub(crate) fn forget_all() {
    ensured().write().unwrap_or_else(|e| e.into_inner()).clear();
}

/// The driver messages for "that table does not exist": Postgres (SQLSTATE
/// 42P01 `relation "x" does not exist`), MySQL (1146 `Table 'db.x' doesn't
/// exist`) and SQLite (`no such table: x`). A false positive only costs one
/// extra ensure and retry.
fn is_missing_table_error(message: &str) -> bool {
    message.contains("does not exist")
        || message.contains("doesn't exist")
        || message.contains("no such table")
}

/// Run a document write against `table`, creating the table first unless it is
/// already known to exist.
///
/// `op` receives `true` when it may be called a second time (the memo skipped
/// the DDL, so a stale entry means a retry) — it must then not consume anything
/// it would need again; with `false` it is the last call.
pub(crate) fn write_with_table<T>(
    table: &str,
    in_transaction: bool,
    ensure_table: impl Fn(&str) -> Result<(), String>,
    mut op: impl FnMut(bool) -> Result<T, String>,
) -> Result<T, String> {
    let key = if in_transaction {
        None
    } else {
        memo_key(table)
    };
    let Some(key) = key else {
        ensure_table(table)?;
        return op(false);
    };
    if is_known(&key) {
        match op(true) {
            Err(e) if is_missing_table_error(&e) => {
                forget(&key);
                ensure_table(table)?;
                remember(key);
                op(false)
            }
            other => other,
        }
    } else {
        ensure_table(table)?;
        remember(key);
        op(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_table_messages_are_recognised() {
        assert!(is_missing_table_error(
            "postgres insert: relation \"posts\" does not exist"
        ));
        assert!(is_missing_table_error(
            "mysql insert: MySqlError { ERROR 1146 (42S02): Table 'app.posts' doesn't exist }"
        ));
        assert!(is_missing_table_error(
            "sqlite insert: no such table: posts"
        ));
        assert!(!is_missing_table_error(
            "sqlite insert: UNIQUE constraint failed"
        ));
    }
}
