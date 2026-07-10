//! Graph (edge-collection) support for the Model layer.
//!
//! An edge model declares its vertex collections in the class body:
//!
//! ```soli
//! class Follow < Model
//!   edge from: "users", to: "users"
//! end
//! ```
//!
//! Edges are plain documents carrying `_from`/`_to` ("coll/key" refs). This
//! module owns endpoint coercion (`Follow.create(from: alice, to: bob)`),
//! traversal-option parsing for `instance.traverse(...)`, and the
//! `SHORTEST_PATH` query builder. The traversal FOR-head itself is emitted by
//! `QueryBuilder::for_head` so every existing FILTER/SORT/LIMIT path applies
//! to traversal results unchanged.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::value::{HashKey, Instance, Value};

use super::query::QueryBuilder;
use super::registry::{get_edge_spec, EdgeSpec};

/// Traversal direction. `Out` follows `_from → _to`, `In` the reverse,
/// `Any` both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalDirection {
    Out,
    In,
    Any,
}

impl TraversalDirection {
    pub fn sdbql(&self) -> &'static str {
        match self {
            TraversalDirection::Out => "OUTBOUND",
            TraversalDirection::In => "INBOUND",
            TraversalDirection::Any => "ANY",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "out" | "outbound" => Ok(TraversalDirection::Out),
            "in" | "inbound" => Ok(TraversalDirection::In),
            "any" => Ok(TraversalDirection::Any),
            other => Err(format!(
                "invalid traversal direction '{}': expected \"out\", \"in\", or \"any\"",
                other
            )),
        }
    }
}

/// A graph-traversal FOR-head on a QueryBuilder. The start vertex travels as
/// the `@__soli_traverse_start` bind var (inserted when the QB is built);
/// depth and edge collection must be literals per the DB grammar, so they are
/// validated here and inlined.
#[derive(Debug, Clone)]
pub struct TraversalClause {
    pub edge_collection: String,
    pub direction: TraversalDirection,
    pub min_depth: usize,
    pub max_depth: usize,
}

/// Bind-var name carrying the traversal start vertex.
pub const TRAVERSE_START_BIND: &str = "__soli_traverse_start";

/// Validate a collection identifier that gets inlined into SDBQL text (edge
/// collections can't be bind vars in the DB grammar).
pub fn validate_collection_ident(name: &str, context: &str) -> Result<(), String> {
    let ok = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{}: '{}' is not a valid collection name",
            context, name
        ))
    }
}

/// Normalize an `edge from:`/`to:` declaration value to a collection name.
/// Accepts a collection name ("users"), a model class name ("User"), or a
/// class value.
pub fn endpoint_to_collection(value: &Value) -> Result<String, String> {
    let name = match value {
        Value::String(s) => s.to_string(),
        Value::Symbol(s) => s.to_string(),
        Value::Class(c) => c.name.to_string(),
        other => {
            return Err(format!(
                "edge endpoints must be collection names or model classes, got {}",
                other.type_name()
            ))
        }
    };
    // Class names are PascalCase; collection names are snake_case. Convert
    // when the value looks like a class name.
    let collection = if name.starts_with(|c: char| c.is_ascii_uppercase()) {
        super::core::class_name_to_collection(&name)
    } else {
        name
    };
    validate_collection_ident(&collection, "edge")?;
    Ok(collection)
}

/// Strip a database qualifier from a document id: SolidB `_id`s come back
/// as `"db:coll/key"`, but the traversal executor resolves vertices by the
/// plain `"coll/key"` form (and matches `_from`/`_to` by raw string
/// equality), so the canonical edge-ref format is unqualified.
pub fn strip_db_qualifier(id: &str) -> &str {
    match (id.find(':'), id.find('/')) {
        (Some(colon), Some(slash)) if colon < slash => &id[colon + 1..],
        _ => id,
    }
}

/// Coerce an endpoint value to a full `"coll/key"` document id.
///
/// Accepts a model instance (uses `_id`, or `collection/_key`), a full
/// `"coll/key"` string (collection must match the declared one), or a bare
/// key (prefixed with the declared collection). Database-qualified ids
/// (`"db:coll/key"`) are normalized to the plain form.
pub fn edge_ref(value: &Value, expected_collection: &str, field: &str) -> Result<String, String> {
    match value {
        Value::Instance(inst) => {
            let inst_ref = inst.borrow();
            if let Some(Value::String(id)) = inst_ref.get("_id") {
                return Ok(strip_db_qualifier(&id).to_string());
            }
            match inst_ref.get("_key") {
                Some(Value::String(key)) => Ok(format!("{}/{}", expected_collection, key)),
                _ => Err(format!(
                    "{}: expected a saved record ({} instance has no _key)",
                    field, inst_ref.class.name
                )),
            }
        }
        Value::String(s) => {
            let s = strip_db_qualifier(s);
            if let Some((coll, key)) = s.split_once('/') {
                if coll != expected_collection {
                    return Err(format!(
                        "{}: '{}' does not belong to the declared '{}' collection",
                        field, s, expected_collection
                    ));
                }
                if key.is_empty() {
                    return Err(format!("{}: '{}' is missing a document key", field, s));
                }
                Ok(s.to_string())
            } else if s.is_empty() {
                Err(format!("{} is required", field))
            } else {
                Ok(format!("{}/{}", expected_collection, s))
            }
        }
        Value::Symbol(s) => Ok(format!("{}/{}", expected_collection, s)),
        Value::Null => Err(format!("{} is required", field)),
        other => Err(format!(
            "{}: expected a model instance, \"coll/key\" id, or key string, got {}",
            field,
            other.type_name()
        )),
    }
}

/// Pull `from`/`to` (or `_from`/`_to`) out of a create/save hash and coerce
/// them to `_from`/`_to` document ids. Returns the pair on success, or a list
/// of `{field, message}` validation-error hashes matching the `_errors`
/// convention.
pub fn transform_edge_data(
    data: &Value,
    spec: &EdgeSpec,
) -> Result<(String, String), Vec<(String, String)>> {
    let (from_val, to_val) = match data {
        Value::Hash(hash) => {
            let h = hash.borrow();
            let get = |names: [&str; 2]| {
                names.iter().find_map(|n| {
                    h.iter().find_map(|(k, v)| match k {
                        HashKey::String(s) if s.as_str() == *n => Some(v.clone()),
                        _ => None,
                    })
                })
            };
            (get(["from", "_from"]), get(["to", "_to"]))
        }
        _ => (None, None),
    };

    let mut errors: Vec<(String, String)> = Vec::new();
    let from_ref = match from_val {
        Some(v) => match edge_ref(&v, &spec.from_collection, "from") {
            Ok(r) => Some(r),
            Err(e) => {
                errors.push(("from".to_string(), e));
                None
            }
        },
        None => {
            errors.push(("from".to_string(), "from is required".to_string()));
            None
        }
    };
    let to_ref = match to_val {
        Some(v) => match edge_ref(&v, &spec.to_collection, "to") {
            Ok(r) => Some(r),
            Err(e) => {
                errors.push(("to".to_string(), e));
                None
            }
        },
        None => {
            errors.push(("to".to_string(), "to is required".to_string()));
            None
        }
    };

    match (from_ref, to_ref) {
        (Some(f), Some(t)) if errors.is_empty() => Ok((f, t)),
        _ => Err(errors),
    }
}

/// Coerce a vertex reference without enforcing a collection: full
/// `"coll/key"` ids pass through as-is (shortest_path targets may live in
/// either endpoint collection), instances use `_id`, bare keys get the
/// fallback collection prefix.
pub fn any_vertex_ref(
    value: &Value,
    fallback_collection: &str,
    context: &str,
) -> Result<String, String> {
    match value {
        Value::String(s) if s.contains('/') => {
            let s = strip_db_qualifier(s);
            let (coll, key) = s.split_once('/').unwrap_or(("", ""));
            if coll.is_empty() || key.is_empty() {
                return Err(format!("{}: '{}' is not a valid document id", context, s));
            }
            Ok(s.to_string())
        }
        other => edge_ref(other, fallback_collection, context),
    }
}

/// Build a fresh data hash for an edge write: the input minus any
/// `from`/`to`/`_from`/`_to` keys, plus the coerced `_from`/`_to` ids.
/// A new hash is returned so the caller's hash isn't mutated.
pub fn rebuild_edge_data(data: &Value, from_ref: &str, to_ref: &str) -> Value {
    let mut pairs = crate::interpreter::value::HashPairs::default();
    if let Value::Hash(hash) = data {
        for (k, v) in hash.borrow().iter() {
            if let HashKey::String(s) = k {
                if matches!(s.as_str(), "from" | "to" | "_from" | "_to") {
                    continue;
                }
            }
            pairs.insert(k.clone(), v.clone());
        }
    }
    pairs.insert(
        HashKey::String("_from".into()),
        Value::String(from_ref.into()),
    );
    pairs.insert(HashKey::String("_to".into()), Value::String(to_ref.into()));
    Value::Hash(std::rc::Rc::new(std::cell::RefCell::new(pairs)))
}

/// Parsed options for `instance.traverse(EdgeModel, {...})`.
pub struct TraverseSpec {
    pub edge_collection: String,
    pub edge_spec: Option<EdgeSpec>,
    pub direction: TraversalDirection,
    pub min_depth: usize,
    pub max_depth: usize,
}

/// Resolve the edge argument (an edge model class or a raw edge-collection
/// name) plus the options hash (`direction:`, `depth:`) of a traverse() call.
///
/// `depth:` accepts an Int (meaning 1..N) or a `[min, max]` array. Note Soli
/// ranges materialize to exclusive-end arrays (`1..3` == `[1, 2]`), so an
/// array longer than 2 is treated as [min(a), max(a)].
pub fn parse_traverse_options(args: &[Value]) -> Result<TraverseSpec, String> {
    let edge_arg = args
        .first()
        .ok_or_else(|| "traverse() requires an edge model (or edge collection name)".to_string())?;

    let (edge_collection, edge_spec) = match edge_arg {
        Value::Class(c) => {
            let class_name = c.name.to_string();
            let spec = get_edge_spec(&class_name).ok_or_else(|| {
                format!(
                    "{} has no `edge` declaration — declare `edge from: \"...\", to: \"...\"` \
                     in its class body (or pass an edge collection name)",
                    class_name
                )
            })?;
            (
                super::core::class_name_to_collection(&class_name),
                Some(spec),
            )
        }
        Value::String(s) => (s.to_string(), None),
        other => {
            return Err(format!(
                "traverse() expects an edge model class or collection name, got {}",
                other.type_name()
            ))
        }
    };
    validate_collection_ident(&edge_collection, "traverse")?;

    let mut direction = TraversalDirection::Out;
    let mut min_depth: usize = 1;
    let mut max_depth: usize = 1;

    if let Some(opts) = args.get(1) {
        let hash = match opts {
            Value::Hash(h) => h,
            other => {
                return Err(format!(
                    "traverse() options must be a hash, got {}",
                    other.type_name()
                ))
            }
        };
        for (k, v) in hash.borrow().iter() {
            let key = match k {
                HashKey::String(s) => s.to_string(),
                other => format!("{:?}", other),
            };
            match key.as_str() {
                "direction" => {
                    let dir = match v {
                        Value::String(s) => s.to_string(),
                        Value::Symbol(s) => s.to_string(),
                        other => {
                            return Err(format!(
                                "traverse() direction must be a string, got {}",
                                other.type_name()
                            ))
                        }
                    };
                    direction = TraversalDirection::parse(&dir)?;
                }
                "depth" => match v {
                    Value::Int(n) => {
                        if *n < 1 {
                            return Err("traverse() depth must be >= 1".to_string());
                        }
                        min_depth = 1;
                        max_depth = *n as usize;
                    }
                    Value::Array(arr) => {
                        let arr = arr.borrow();
                        let ints: Vec<i64> = arr
                            .iter()
                            .map(|v| match v {
                                Value::Int(n) => Ok(*n),
                                other => Err(format!(
                                    "traverse() depth array must hold ints, got {}",
                                    other.type_name()
                                )),
                            })
                            .collect::<Result<_, _>>()?;
                        // A Soli range literal (1..3) materializes to an
                        // exclusive-end array [1, 2]; min/max keeps both the
                        // [min, max] pair form and range form sensible.
                        let (min, max) = match (ints.iter().min(), ints.iter().max()) {
                            (Some(&min), Some(&max)) => (min, max),
                            _ => return Err("traverse() depth array is empty".to_string()),
                        };
                        if min < 1 {
                            return Err("traverse() depth must be >= 1".to_string());
                        }
                        min_depth = min as usize;
                        max_depth = max as usize;
                    }
                    other => {
                        return Err(format!(
                            "traverse() depth must be an Int or [min, max] array, got {}",
                            other.type_name()
                        ))
                    }
                },
                other => {
                    return Err(format!(
                        "traverse() unknown option '{}': expected direction: or depth:",
                        other
                    ))
                }
            }
        }
    }

    Ok(TraverseSpec {
        edge_collection,
        edge_spec,
        direction,
        min_depth,
        max_depth,
    })
}

/// Build a chainable [`QueryBuilder`] for graph traversal from a saved vertex.
/// Mirrors `instance.traverse(EdgeModel, options?)` without going through the
/// interpreter call path (used by `graph_rag` expansion).
pub fn build_traverse_qb_from_seed(
    seed: Rc<RefCell<Instance>>,
    edge_arg: &Value,
    direction: TraversalDirection,
    min_depth: usize,
    max_depth: usize,
) -> Result<QueryBuilder, String> {
    let own_class_name = seed.borrow().class.name.clone();
    let own_collection = super::core::class_name_to_collection(&own_class_name);
    let start_id = edge_ref(
        &Value::Instance(seed.clone()),
        &own_collection,
        "traverse()",
    )
    .map_err(|_| {
        format!(
            "traverse() requires a saved record ({} instance has no _key)",
            own_class_name
        )
    })?;

    let (edge_collection, edge_spec) = match edge_arg {
        Value::Class(c) => {
            let class_name = c.name.to_string();
            let spec = get_edge_spec(&class_name).ok_or_else(|| {
                format!(
                    "{} has no `edge` declaration — declare `edge from: \"...\", to: \"...\"` \
                     in its class body (or pass an edge collection name)",
                    class_name
                )
            })?;
            (
                super::core::class_name_to_collection(&class_name),
                Some(spec),
            )
        }
        Value::String(s) => (s.to_string(), None),
        other => {
            return Err(format!(
                "traverse() expects an edge model class or collection name, got {}",
                other.type_name()
            ))
        }
    };
    validate_collection_ident(&edge_collection, "traverse")?;

    let target_collection = match (&edge_spec, direction) {
        (Some(es), TraversalDirection::Out) => es.to_collection.clone(),
        (Some(es), TraversalDirection::In) => es.from_collection.clone(),
        _ => own_collection.clone(),
    };
    let target_class_name = super::relations::classify(&target_collection);
    let target_class = super::registry::get_model_class(&target_class_name)
        .or_else(|| super::registry::get_model_class(&own_class_name));

    let mut qb = match target_class {
        Some(class) => {
            QueryBuilder::new_with_class(target_class_name, edge_collection.clone(), class)
        }
        None => QueryBuilder::new(target_class_name, edge_collection.clone()),
    };
    qb.bind_vars.insert(
        crate::interpreter::get_symbol(TRAVERSE_START_BIND),
        serde_json::Value::String(start_id),
    );
    qb.traversal = Some(TraversalClause {
        edge_collection,
        direction,
        min_depth,
        max_depth,
    });
    Ok(qb)
}

/// Build the SHORTEST_PATH query. Start/end travel as bind vars; direction
/// and edge collection are validated literals.
pub fn shortest_path_query(
    edge_collection: &str,
    direction: TraversalDirection,
    start_id: &str,
    end_id: &str,
) -> (String, HashMap<String, serde_json::Value>) {
    let query = format!(
        "FOR doc IN SHORTEST_PATH @__soli_sp_start TO @__soli_sp_end {} {} RETURN doc",
        direction.sdbql(),
        edge_collection
    );
    let mut binds = HashMap::new();
    binds.insert(
        "__soli_sp_start".to_string(),
        serde_json::Value::String(start_id.to_string()),
    );
    binds.insert(
        "__soli_sp_end".to_string(),
        serde_json::Value::String(end_id.to_string()),
    );
    (query, binds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::HashPairs;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn hash(pairs: &[(&str, Value)]) -> Value {
        let mut h = HashPairs::default();
        for (k, v) in pairs {
            h.insert(HashKey::String((*k).into()), v.clone());
        }
        Value::Hash(Rc::new(RefCell::new(h)))
    }

    fn spec() -> EdgeSpec {
        EdgeSpec {
            from_collection: "users".to_string(),
            to_collection: "posts".to_string(),
        }
    }

    #[test]
    fn edge_ref_accepts_full_id_bare_key_and_rejects_wrong_collection() {
        assert_eq!(
            edge_ref(&Value::String("users/abc".into()), "users", "from").unwrap(),
            "users/abc"
        );
        assert_eq!(
            edge_ref(&Value::String("abc".into()), "users", "from").unwrap(),
            "users/abc"
        );
        let err = edge_ref(&Value::String("posts/abc".into()), "users", "from").unwrap_err();
        assert!(err.contains("does not belong"), "{}", err);
        let err = edge_ref(&Value::Null, "users", "from").unwrap_err();
        assert!(err.contains("required"), "{}", err);
        let err = edge_ref(&Value::String("users/".into()), "users", "from").unwrap_err();
        assert!(err.contains("missing a document key"), "{}", err);
    }

    #[test]
    fn transform_edge_data_coerces_and_collects_errors() {
        let data = hash(&[
            ("from", Value::String("users/a".into())),
            ("to", Value::String("b".into())),
            ("since", Value::Int(2024)),
        ]);
        let (f, t) = transform_edge_data(&data, &spec()).unwrap();
        assert_eq!(f, "users/a");
        assert_eq!(t, "posts/b");

        let missing = hash(&[("since", Value::Int(1))]);
        let errs = transform_edge_data(&missing, &spec()).unwrap_err();
        assert_eq!(errs.len(), 2);
        assert_eq!(errs[0].0, "from");
        assert_eq!(errs[1].0, "to");
    }

    #[test]
    fn parse_traverse_options_defaults_and_depth_forms() {
        // Raw collection-name form, defaults.
        let spec = parse_traverse_options(&[Value::String("follows".into())]).unwrap();
        assert_eq!(spec.edge_collection, "follows");
        assert_eq!(spec.direction, TraversalDirection::Out);
        assert_eq!((spec.min_depth, spec.max_depth), (1, 1));

        // depth: Int means 1..N.
        let opts = hash(&[("depth", Value::Int(3))]);
        let spec = parse_traverse_options(&[Value::String("follows".into()), opts]).unwrap();
        assert_eq!((spec.min_depth, spec.max_depth), (1, 3));

        // depth: [min, max] + direction.
        let opts = hash(&[
            (
                "depth",
                Value::Array(Rc::new(RefCell::new(vec![Value::Int(2), Value::Int(3)]))),
            ),
            ("direction", Value::String("any".into())),
        ]);
        let spec = parse_traverse_options(&[Value::String("follows".into()), opts]).unwrap();
        assert_eq!((spec.min_depth, spec.max_depth), (2, 3));
        assert_eq!(spec.direction, TraversalDirection::Any);

        // Unknown option errors.
        let opts = hash(&[("dir", Value::String("in".into()))]);
        assert!(parse_traverse_options(&[Value::String("follows".into()), opts]).is_err());
    }

    #[test]
    fn shortest_path_query_shape() {
        let (q, binds) =
            shortest_path_query("follows", TraversalDirection::Any, "users/a", "users/b");
        assert_eq!(
            q,
            "FOR doc IN SHORTEST_PATH @__soli_sp_start TO @__soli_sp_end ANY follows RETURN doc"
        );
        assert_eq!(binds["__soli_sp_start"], "users/a");
        assert_eq!(binds["__soli_sp_end"], "users/b");
    }
}
