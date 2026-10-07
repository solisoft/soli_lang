//! Memo of the document tables the SQL adapters have already created.
//!
//! Every document write (`insert`, `insert_many`, `update`) used to run a
//! `CREATE TABLE IF NOT EXISTS` first — a full extra round-trip per write for a
//! statement that is a no-op after the first one. This remembers, process-wide,
//! which `(connection, url, table)` triples have been ensured and skips the DDL
//! for them.
//!
//! Reads use it the same way (`read_with_table`): a read on a table that may
//! not exist used to ask `information_schema` / `sqlite_master` first — another
//! round trip on every `find`, `where`, `count`. A table known to exist (ensured
//! by a write, or seen by an earlier read) is now read directly.
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
//!   code gave. A read gets the empty answer a missing table always gave.

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

/// "That table does not exist", naming `table` itself — so a read on a
/// remembered table that was dropped behind our back answers empty, while a
/// missing column, function or other table still surfaces as an error.
fn is_missing_this_table_error(message: &str, table: &str) -> bool {
    message.contains(&format!("relation \"{table}\" does not exist"))
        || message.contains(&format!(".{table}' doesn't exist"))
        || message.contains(&format!("no such table: {table}"))
}

/// Run a document read against `table`, answering `missing()` when the table
/// does not exist. A table known to exist is read without asking the database
/// first; otherwise `table_exists` asks, and a yes is remembered.
pub(crate) fn read_with_table<T>(
    table: &str,
    in_transaction: bool,
    table_exists: impl FnOnce(&str) -> Result<bool, String>,
    missing: impl FnOnce() -> T,
    op: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let key = if in_transaction {
        None
    } else {
        memo_key(table)
    };
    if let Some(key) = key.as_deref().filter(|key| is_known(key)) {
        return match op() {
            Err(e) if is_missing_this_table_error(&e, table) => {
                forget(key);
                Ok(missing())
            }
            other => other,
        };
    }
    if !table_exists(table)? {
        return Ok(missing());
    }
    if let Some(key) = key {
        remember(key);
    }
    op()
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

    #[test]
    fn a_read_only_treats_its_own_table_as_missing() {
        let pg = "postgres get: relation \"posts\" does not exist";
        assert!(is_missing_this_table_error(pg, "posts"));
        assert!(!is_missing_this_table_error(pg, "post"));
        assert!(!is_missing_this_table_error(
            "postgres list: column \"titel\" does not exist",
            "posts"
        ));
        assert!(is_missing_this_table_error(
            "mysql get: MySqlError { ERROR 1146 (42S02): Table 'app.posts' doesn't exist }",
            "posts"
        ));
        assert!(is_missing_this_table_error(
            "sqlite get: no such table: posts",
            "posts"
        ));
    }
}
