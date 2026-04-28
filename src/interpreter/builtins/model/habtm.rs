//! has_and_belongs_to_many mutators.
//!
//! Provides `add_<singular>` / `remove_<singular>` instance methods that
//! insert/delete rows in the join table. Method names are derived from
//! `to_snake_case(relation.class_name)`. For instance, on a `Post` with
//! `has_and_belongs_to_many("tags")`, callers get `post.add_tag(tag)` and
//! `post.remove_tag(tag)`.

use crate::interpreter::value::{Instance, Value};

use super::crud::{exec_insert, exec_with_auto_collection};
use super::relations::{get_relations, singularize, RelationDef, RelationType};

/// Build a candidate HABTM method name like `"add_tag"` from a relation
/// (plural) name like `"tags"`.
pub fn to_singular_method_name(prefix: &str, relation_name: &str) -> String {
    format!("{}_{}", prefix, singularize(relation_name))
}

/// Parsed match for `add_<x>` / `remove_<x>` against a HABTM relation.
pub struct HabtmMethodMatch {
    pub action: HabtmAction,
    pub relation: RelationDef,
}

#[derive(Debug, Clone, Copy)]
pub enum HabtmAction {
    Add,
    Remove,
}

/// Resolve a method name on a model instance to a HABTM mutator. Returns
/// `Some` only when the method name exactly matches `add_<singular>` or
/// `remove_<singular>` for a declared HABTM relation on the instance's class.
pub fn match_habtm_method(class_name: &str, method_name: &str) -> Option<HabtmMethodMatch> {
    let (action, suffix) = if let Some(s) = method_name.strip_prefix("add_") {
        (HabtmAction::Add, s)
    } else if let Some(s) = method_name.strip_prefix("remove_") {
        (HabtmAction::Remove, s)
    } else {
        return None;
    };

    for rel in get_relations(class_name) {
        if rel.relation_type != RelationType::HasAndBelongsToMany {
            continue;
        }
        if singularize(&rel.name) == suffix {
            return Some(HabtmMethodMatch {
                action,
                relation: rel,
            });
        }
    }
    None
}

/// Extract a `_key` from an argument that may be a Soli model instance, a
/// raw String key, or an Int key.
fn extract_key(arg: &Value) -> Result<String, String> {
    match arg {
        Value::String(s) => Ok(s.clone()),
        Value::Int(n) => Ok(n.to_string()),
        Value::Instance(inst) => match inst.borrow().get("_key") {
            Some(Value::String(s)) => Ok(s),
            Some(Value::Int(n)) => Ok(n.to_string()),
            Some(other) => Err(format!(
                "instance _key has unsupported type {}",
                other.type_name()
            )),
            None => Err("instance has no _key (record was not saved?)".to_string()),
        },
        other => Err(format!(
            "expected model instance or _key, got {}",
            other.type_name()
        )),
    }
}

/// Flatten arguments into a list of related-record keys. Each argument may be
/// a single instance/key or an array of those.
fn collect_keys(args: &[Value]) -> Result<Vec<String>, String> {
    let mut keys = Vec::new();
    for arg in args {
        match arg {
            Value::Array(arr) => {
                for item in arr.borrow().iter() {
                    keys.push(extract_key(item)?);
                }
            }
            other => keys.push(extract_key(other)?),
        }
    }
    Ok(keys)
}

/// Insert join-table rows linking the owner to each provided related key.
/// Returns the number of rows inserted.
pub fn habtm_add(
    inst: &std::rc::Rc<std::cell::RefCell<Instance>>,
    rel: &RelationDef,
    args: &[Value],
) -> Result<Value, String> {
    let owner_key = match inst.borrow().get("_key") {
        Some(Value::String(s)) => s,
        Some(Value::Int(n)) => n.to_string(),
        _ => return Err("owner instance has no _key (save the record first)".to_string()),
    };

    let join_table = rel
        .join_table
        .as_deref()
        .ok_or("HABTM relation missing join_table")?;
    let assoc_fk = rel
        .association_foreign_key
        .as_deref()
        .ok_or("HABTM relation missing association_foreign_key")?;

    let related_keys = collect_keys(args)?;
    let mut inserted = 0i64;
    for related_key in related_keys {
        let doc = serde_json::json!({
            rel.foreign_key.clone(): owner_key,
            assoc_fk.to_string(): related_key,
        });
        exec_insert(join_table, None, doc).map_err(|e| format!("habtm add failed: {}", e))?;
        inserted += 1;
    }
    Ok(Value::Int(inserted))
}

/// Delete join-table rows linking the owner to each provided related key.
/// Returns the number of rows deleted.
pub fn habtm_remove(
    inst: &std::rc::Rc<std::cell::RefCell<Instance>>,
    rel: &RelationDef,
    args: &[Value],
) -> Result<Value, String> {
    let owner_key = match inst.borrow().get("_key") {
        Some(Value::String(s)) => s,
        Some(Value::Int(n)) => n.to_string(),
        _ => return Err("owner instance has no _key".to_string()),
    };

    let join_table = rel
        .join_table
        .as_deref()
        .ok_or("HABTM relation missing join_table")?;
    let assoc_fk = rel
        .association_foreign_key
        .as_deref()
        .ok_or("HABTM relation missing association_foreign_key")?;

    let related_keys = collect_keys(args)?;
    if related_keys.is_empty() {
        return Ok(Value::Int(0));
    }

    let placeholders: Vec<String> = (0..related_keys.len())
        .map(|i| format!("@k{}", i))
        .collect();
    let sdbql = format!(
        "FOR doc IN {jt} FILTER doc.{owner_fk} == @owner AND doc.{assoc_fk} IN [{ph}] REMOVE doc IN {jt} RETURN OLD",
        jt = join_table,
        owner_fk = rel.foreign_key,
        assoc_fk = assoc_fk,
        ph = placeholders.join(", "),
    );
    let mut bind_vars = std::collections::HashMap::new();
    bind_vars.insert("owner".to_string(), serde_json::Value::String(owner_key));
    for (i, key) in related_keys.iter().enumerate() {
        bind_vars.insert(format!("k{}", i), serde_json::Value::String(key.clone()));
    }

    let removed = exec_with_auto_collection(sdbql, Some(bind_vars), join_table)
        .map_err(|e| format!("habtm remove failed: {}", e))?;
    Ok(Value::Int(removed.len() as i64))
}
