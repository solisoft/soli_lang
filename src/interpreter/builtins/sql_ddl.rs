//! Schema builtins for SQL migrations — the bridge from the migration DSL to
//! [`crate::db::ddl`].
//!
//! These are the private helpers `MigrationDb` calls (see
//! `crate::migration::execute_migration_sql`); apps use the `db.*` methods, not
//! these names. Each one compiles portable DDL for the active connection's
//! dialect and runs it, so one migration file works on Postgres, MySQL, and
//! SQLite.

use crate::db::ddl;
use crate::interpreter::value::{value_to_json, HashKey, NativeFunction, Value};
use crate::interpreter::Environment;

/// Reject the call early on a non-SQL connection, naming the operation.
fn require_sql(op: &str) -> Result<crate::db::sql_compile::Dialect, String> {
    if !crate::db::is_sql() {
        return Err(format!(
            "{op} needs a SQL connection (postgres, mysql, or sqlite). Column \
             tables have no meaning on SoliDB, which is schemaless — use \
             create_collection there."
        ));
    }
    crate::db::sql::active_dialect()
}

fn arg_string(args: &[Value], index: usize, op: &str, what: &str) -> Result<String, String> {
    match args.get(index) {
        Some(Value::String(s)) => Ok(s.to_string()),
        _ => Err(format!("{op}: {what} must be a string")),
    }
}

/// A hash as **ordered** `(key, json)` pairs.
///
/// Order is the point: Soli hashes are insertion-ordered, and a table's columns
/// should be created in the order the migration lists them.
fn ordered_pairs(value: &Value, op: &str) -> Result<Vec<(String, serde_json::Value)>, String> {
    let Value::Hash(hash) = value else {
        return Err(format!("{op}: expected a hash of columns"));
    };
    let mut out = Vec::new();
    for (key, val) in hash.borrow().iter() {
        let HashKey::String(name) = key else {
            return Err(format!("{op}: column names must be strings"));
        };
        out.push((name.to_string(), value_to_json(val)?));
    }
    Ok(out)
}

/// Parse a column argument: a type string (`"string"`) or an options hash.
fn column_spec(name: &str, value: &Value, op: &str) -> Result<ddl::ColumnSpec, String> {
    let json = value_to_json(value).map_err(|e| format!("{op}: {e}"))?;
    ddl::parse_column(name, &json)
}

fn string_list(value: Option<&Value>, op: &str) -> Result<Vec<String>, String> {
    match value {
        Some(Value::String(s)) => Ok(vec![s.to_string()]),
        Some(Value::Array(items)) => items
            .borrow()
            .iter()
            .map(|item| match item {
                Value::String(s) => Ok(s.to_string()),
                other => Err(format!(
                    "{op}: column names must be strings, got {}",
                    other.type_name()
                )),
            })
            .collect(),
        _ => Err(format!("{op}: expected a column name or an array of them")),
    }
}

/// Look up one option in a hash argument.
fn option_bool(args: &[Value], index: usize, key: &str) -> Result<bool, String> {
    let Some(Value::Hash(hash)) = args.get(index) else {
        return Ok(false);
    };
    for (k, v) in hash.borrow().iter() {
        if let HashKey::String(name) = k {
            if name.as_ref() == key {
                return match v {
                    Value::Bool(b) => Ok(*b),
                    Value::Null => Ok(false),
                    other => Err(format!(
                        "option {key:?} expects true or false, got {}",
                        other.type_name()
                    )),
                };
            }
        }
    }
    Ok(false)
}

fn option_string(args: &[Value], index: usize, key: &str) -> Option<String> {
    let Some(Value::Hash(hash)) = args.get(index) else {
        return None;
    };
    for (k, v) in hash.borrow().iter() {
        if let HashKey::String(name) = k {
            if name.as_ref() == key {
                if let Value::String(s) = v {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

pub fn register_sql_ddl_builtins(env: &mut Environment) {
    // create_table(name, columns) — a real column table.
    env.define(
        "__soli_sql_create_columns".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__soli_sql_create_columns",
            Some(2),
            |args| {
                let op = "create_table(name, columns)";
                let dialect = require_sql(op)?;
                let table = arg_string(args, 0, op, "the table name")?;
                let pairs = ordered_pairs(args.get(1).unwrap_or(&Value::Null), op)?;
                let spec = ddl::parse_table_spec(&table, &pairs)?;
                let sql = ddl::create_table_sql(dialect, &spec)?;
                crate::db::sql::execute_ddl(&sql)?;
                Ok(Value::Bool(true))
            },
        )),
    );

    env.define(
        "__soli_sql_add_column".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__soli_sql_add_column",
            Some(3),
            |args| {
                let op = "add_column(table, name, type)";
                let dialect = require_sql(op)?;
                let table = arg_string(args, 0, op, "the table name")?;
                let name = arg_string(args, 1, op, "the column name")?;
                let col = column_spec(&name, args.get(2).unwrap_or(&Value::Null), op)?;
                let sql = ddl::add_column_sql(dialect, &table, &col)?;
                crate::db::sql::execute_ddl(&sql)?;
                Ok(Value::Bool(true))
            },
        )),
    );

    env.define(
        "__soli_sql_drop_column".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__soli_sql_drop_column",
            Some(2),
            |args| {
                let op = "drop_column(table, name)";
                let dialect = require_sql(op)?;
                let table = arg_string(args, 0, op, "the table name")?;
                let name = arg_string(args, 1, op, "the column name")?;
                let sql = ddl::drop_column_sql(dialect, &table, &name)?;
                crate::db::sql::execute_ddl(&sql)?;
                Ok(Value::Bool(true))
            },
        )),
    );

    env.define(
        "__soli_sql_rename_column".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__soli_sql_rename_column",
            Some(3),
            |args| {
                let op = "rename_column(table, from, to)";
                let dialect = require_sql(op)?;
                let table = arg_string(args, 0, op, "the table name")?;
                let from = arg_string(args, 1, op, "the current column name")?;
                let to = arg_string(args, 2, op, "the new column name")?;
                let sql = ddl::rename_column_sql(dialect, &table, &from, &to)?;
                crate::db::sql::execute_ddl(&sql)?;
                Ok(Value::Bool(true))
            },
        )),
    );

    env.define(
        "__soli_sql_rename_table".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__soli_sql_rename_table",
            Some(2),
            |args| {
                let op = "rename_table(from, to)";
                let dialect = require_sql(op)?;
                let from = arg_string(args, 0, op, "the current table name")?;
                let to = arg_string(args, 1, op, "the new table name")?;
                let sql = ddl::rename_table_sql(dialect, &from, &to)?;
                crate::db::sql::execute_ddl(&sql)?;
                Ok(Value::Bool(true))
            },
        )),
    );

    env.define(
        "__soli_sql_add_index".to_string(),
        Value::NativeFunction(NativeFunction::new("__soli_sql_add_index", None, |args| {
            let op = "add_index(table, columns, options?)";
            let dialect = require_sql(op)?;
            let table = arg_string(args, 0, op, "the table name")?;
            let columns = string_list(args.get(1), op)?;
            let unique = option_bool(args, 2, "unique")?;
            let name = option_string(args, 2, "name");
            let sql = ddl::add_index_sql(dialect, &table, &columns, name.as_deref(), unique)?;
            crate::db::sql::execute_ddl(&sql)?;
            Ok(Value::Bool(true))
        })),
    );

    env.define(
        "__soli_sql_drop_index".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "__soli_sql_drop_index",
            Some(2),
            |args| {
                let op = "drop_index(table, name)";
                let dialect = require_sql(op)?;
                let table = arg_string(args, 0, op, "the table name")?;
                let name = arg_string(args, 1, op, "the index name")?;
                let sql = ddl::drop_index_sql(dialect, &table, &name)?;
                crate::db::sql::execute_ddl(&sql)?;
                Ok(Value::Bool(true))
            },
        )),
    );

    // The escape hatch: whatever the portable DSL cannot express (a CHECK
    // constraint, a partial index, `ALTER COLUMN TYPE`). Engine-specific by
    // definition, so a migration using it targets one backend.
    env.define(
        "__soli_sql_execute".to_string(),
        Value::NativeFunction(NativeFunction::new("__soli_sql_execute", Some(1), |args| {
            let op = "execute(sql)";
            require_sql(op)?;
            let sql = arg_string(args, 0, op, "the statement")?;
            crate::db::sql::execute_ddl(&sql)?;
            Ok(Value::Bool(true))
        })),
    );
}
