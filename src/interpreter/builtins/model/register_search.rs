//! `Model`'s declared-index surface: vector (`similar`), fulltext
//! (`search`), the hybrid and RAG combinations, geo (`near`, `within`), and
//! the timeseries verbs `time_bucket` and `prune`.
//!
//! Split out of `register_model_class`, which registered the whole ORM surface
//! as inline closures in one 4,329-line function — the largest in the
//! repository, and why `core.rs` was over seven thousand lines. Every ORM
//! change touched that function, so every parallel change conflicted inside it.
//!
//! These files are siblings of `core.rs` rather than a `register/`
//! subdirectory on purpose: the moved code carries 224 `super::` paths to
//! sixteen sibling modules, and a directory would have changed what every one
//! of them means.
//!
//! Nothing here is new. Each `register` fills the same map the one function
//! filled, in the same order — though the order does not matter, since every
//! entry is an independent closure keyed by its own name.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::core::*;
use crate::interpreter::value::{Class, NativeFunction, Value};

pub(super) fn register(native_static_methods: &mut HashMap<String, Rc<NativeFunction>>) {
    // Model.similar(query[, field][, k][, options]) — start a similarity
    // chain without a where(): returns a QueryBuilder with the similar
    // spec set (same argument shapes as the chainable .similar()).
    native_static_methods.insert(
        "similar".to_string(),
        Rc::new(NativeFunction::new("Model.similar", None, |args| {
            use super::query::{SimilarInput, SimilarSpec};
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let input = match args.get(1) {
                Some(Value::String(s)) => SimilarInput::Text(s.to_string()),
                Some(Value::Array(arr)) => {
                    let vec: Vec<f64> = arr
                        .borrow()
                        .iter()
                        .map(|v| match v {
                            Value::Int(n) => Ok(*n as f64),
                            Value::Float(f) => Ok(*f),
                            other => Err(format!(
                                "similar() vector entries must be numbers, got {}",
                                other.type_name()
                            )),
                        })
                        .collect::<Result<_, _>>()?;
                    if vec.is_empty() {
                        return Err("similar() vector is empty".to_string());
                    }
                    SimilarInput::Vector(vec)
                }
                _ => return Err("similar() expects query text or a numeric vector".to_string()),
            };
            let field = match args.get(2) {
                Some(Value::String(s)) => s.to_string(),
                _ => "embedding".to_string(),
            };
            validate_field_name(&field, "similar")?;
            let top_k = match args.get(3) {
                // Clamped like `paginate`'s `per`: a request-supplied
                // neighbour count must not scan the whole index.
                Some(Value::Int(n)) if *n > 0 => {
                    crate::interpreter::limits::clamp_page_size(*n as usize)
                }
                _ => 10,
            };
            let mut exact = false;
            let mut ef_search: Option<usize> = None;
            if let Some(Value::Hash(opts)) = args.get(4) {
                use crate::interpreter::value::HashKey;
                for (k, v) in opts.borrow().iter() {
                    let key = match k {
                        HashKey::String(s) => s.to_string(),
                        _ => continue,
                    };
                    match (key.as_str(), v) {
                        ("exact", Value::Bool(b)) => exact = *b,
                        ("ef_search", Value::Int(n)) if *n > 0 => ef_search = Some(*n as usize),
                        (other, _) => {
                            return Err(format!("similar() unknown/invalid option '{}'", other))
                        }
                    }
                }
            }

            let mut qb = super::query::QueryBuilder::new_with_class(class_name, collection, class);
            qb.similar_query = Some(SimilarSpec {
                input,
                field,
                top_k,
                exact,
                ef_search,
            });
            qb.limit_val = Some(top_k);
            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // Model.search("query"[, options]) — fulltext search over the fields
    // of the declared `fulltext_index`. Options: field:, distance:,
    // limit:, highlight:. Returns ranked instances with _search_score.
    native_static_methods.insert(
        "search".to_string(),
        Rc::new(NativeFunction::new("Model.search", None, |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let indexes = super::registry::get_fulltext_indexes(&class_name);
            let declared = indexes.first().ok_or_else(|| {
                format!(
                    "{}.search requires a `fulltext_index` declaration in the class body, \
                         e.g. fulltext_index \"title\", \"body\"",
                    class_name
                )
            })?;

            let query_text = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                _ => return Err(format!("{}.search expects a query string", class_name)),
            };

            let mut field = declared
                .fields
                .first()
                .cloned()
                .unwrap_or_else(|| "".to_string());
            let mut distance: usize = 2;
            let mut limit: usize = 10;
            let mut highlight = false;
            if let Some(Value::Hash(opts)) = args.get(2) {
                use crate::interpreter::value::HashKey;
                for (k, v) in opts.borrow().iter() {
                    let key = match k {
                        HashKey::String(s) => s.to_string(),
                        _ => continue,
                    };
                    match (key.as_str(), v) {
                        ("field", Value::String(s)) => {
                            let s = s.to_string();
                            if !declared.fields.iter().any(|f| f == &s) {
                                return Err(format!(
                                    "{}.search field '{}' is not covered by the \
                                         fulltext_index (declared: {})",
                                    class_name,
                                    s,
                                    declared.fields.join(", ")
                                ));
                            }
                            field = s;
                        }
                        ("distance", Value::Int(n)) if *n >= 0 => distance = *n as usize,
                        ("limit", Value::Int(n)) if *n > 0 => limit = *n as usize,
                        ("highlight", Value::Bool(b)) => highlight = *b,
                        (other, _) => {
                            return Err(format!(
                                "search() unknown/invalid option '{}': expected field:, \
                                     distance:, limit:, or highlight:",
                                other
                            ))
                        }
                    }
                }
            }

            super::search::exec_fulltext_search(
                &collection,
                &class,
                &field,
                &query_text,
                distance,
                limit,
                highlight,
                is_soft_delete(&class_name),
            )
        })),
    );

    // Model.hybrid("query"[, options]) — combined vector + fulltext
    // search over the declared `vector_index` and `fulltext_index`.
    // The query text is embedded client-side for the vector leg (pass
    // vector: to skip) and used raw for the fulltext leg. Eager, like
    // search(): returns ranked instances carrying _hybrid_score,
    // _vector_score, _text_score and _sources. Options: vector:,
    // vector_field:, field:, vector_weight:, text_weight:, fusion:
    // ("weighted" | "rrf"), limit:.
    native_static_methods.insert(
        "hybrid".to_string(),
        Rc::new(NativeFunction::new("Model.hybrid", None, |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let query_text = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                _ => return Err(format!("{}.hybrid expects a query string", class_name)),
            };

            let mut explicit_vector: Option<Vec<f64>> = None;
            let mut vector_field: Option<String> = None;
            let mut fulltext_field_opt: Option<String> = None;
            let mut vector_weight: Option<f64> = None;
            let mut text_weight: Option<f64> = None;
            let mut fusion: Option<String> = None;
            let mut limit: Option<usize> = None;
            if let Some(Value::Hash(opts)) = args.get(2) {
                use crate::interpreter::value::HashKey;
                for (k, v) in opts.borrow().iter() {
                    let key = match k {
                        HashKey::String(s) => s.to_string(),
                        _ => continue,
                    };
                    match (key.as_str(), v) {
                        ("vector", Value::Array(arr)) => {
                            let vec: Vec<f64> = arr
                                .borrow()
                                .iter()
                                .map(|v| match v {
                                    Value::Int(n) => Ok(*n as f64),
                                    Value::Float(f) => Ok(*f),
                                    other => Err(format!(
                                        "hybrid() vector entries must be numbers, got {}",
                                        other.type_name()
                                    )),
                                })
                                .collect::<Result<_, _>>()?;
                            if vec.is_empty() {
                                return Err("hybrid() vector is empty".to_string());
                            }
                            explicit_vector = Some(vec);
                        }
                        ("vector_field", Value::String(s)) => vector_field = Some(s.to_string()),
                        ("field", Value::String(s)) => fulltext_field_opt = Some(s.to_string()),
                        ("vector_weight", Value::Float(f)) => vector_weight = Some(*f),
                        ("vector_weight", Value::Int(n)) => vector_weight = Some(*n as f64),
                        ("text_weight", Value::Float(f)) => text_weight = Some(*f),
                        ("text_weight", Value::Int(n)) => text_weight = Some(*n as f64),
                        ("fusion", Value::String(s)) => {
                            let s = s.to_string();
                            if s != "weighted" && s != "rrf" {
                                return Err(format!(
                                    "hybrid() fusion must be \"weighted\" or \"rrf\", \
                                         got \"{}\"",
                                    s
                                ));
                            }
                            fusion = Some(s);
                        }
                        ("limit", Value::Int(n)) if *n > 0 => limit = Some(*n as usize),
                        (other, _) => {
                            return Err(format!(
                                "hybrid() unknown/invalid option '{}': expected vector:, \
                                     vector_field:, field:, vector_weight:, text_weight:, \
                                     fusion:, or limit:",
                                other
                            ))
                        }
                    }
                }
            }

            // Resolve the vector index from the class declarations.
            let vindex = match &vector_field {
                Some(f) => super::registry::get_vector_index_for_field(&class_name, f).ok_or_else(
                    || {
                        format!(
                            "{}.hybrid: no vector_index declared on field '{}'",
                            class_name, f
                        )
                    },
                )?,
                None => {
                    let mut declared = super::registry::get_vector_indexes(&class_name);
                    match declared.len() {
                        0 => {
                            return Err(format!(
                                "{}.hybrid requires a `vector_index` declaration in the \
                                     class body, e.g. vector_index \"embedding\", dimension: 1536",
                                class_name
                            ))
                        }
                        1 => declared.remove(0),
                        _ => {
                            return Err(format!(
                                "{}.hybrid: several vector indexes declared; pass \
                                     vector_field: to pick one",
                                class_name
                            ))
                        }
                    }
                }
            };

            // Resolve the fulltext field from the class declarations.
            let ft_indexes = super::registry::get_fulltext_indexes(&class_name);
            let declared_ft = ft_indexes.first().ok_or_else(|| {
                format!(
                    "{}.hybrid requires a `fulltext_index` declaration in the class body, \
                         e.g. fulltext_index \"title\", \"body\"",
                    class_name
                )
            })?;
            let fulltext_field = match fulltext_field_opt {
                Some(f) => {
                    if !ft_indexes
                        .iter()
                        .any(|idx| idx.fields.iter().any(|x| x == &f))
                    {
                        return Err(format!(
                            "{}.hybrid field '{}' is not covered by a fulltext_index \
                                 (declared: {})",
                            class_name,
                            f,
                            ft_indexes
                                .iter()
                                .flat_map(|i| i.fields.iter().cloned())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    f
                }
                None => declared_ft.fields.first().cloned().unwrap_or_default(),
            };

            // Resolve the query vector: explicit literal, or embed the
            // query text client-side (same path as similar()).
            let query_vector = match explicit_vector {
                Some(v) => v,
                None => crate::embedding::generate_embedding(&query_text).ok_or_else(|| {
                    format!(
                        "{}.hybrid could not embed the query text: set \
                             SOLI_EMBEDDING_API_KEY (and optionally SOLI_EMBEDDING_URL / \
                             SOLI_EMBEDDING_MODEL) or pass vector: with a query vector",
                        class_name
                    )
                })?,
            };

            super::search::exec_hybrid_search(
                &collection,
                &class,
                &vindex.name,
                &fulltext_field,
                &query_vector,
                &query_text,
                vector_weight,
                text_weight,
                fusion,
                limit,
                is_soft_delete(&class_name),
            )
        })),
    );

    // Model.graph_rag(query, { via:, ... }) — graph-augmented retrieval:
    // ANN seeds on the declared vector_index, expand each seed through the
    // edge model's traversal, re-rank the union by cosine similarity.
    // Options: via: (required), direction:, depth:, field:, seed_k:,
    // limit:, vector:.
    native_static_methods.insert(
        "graph_rag".to_string(),
        Rc::new(NativeFunction::new("Model.graph_rag", None, |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let query_text = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                _ => return Err(format!("{}.graph_rag expects a query string", class_name)),
            };

            let opts = super::graph_rag::parse_graph_rag_options(&class_name, args.get(2))?;

            super::graph_rag::exec_graph_rag(&class, &class_name, &collection, &query_text, opts)
        })),
    );

    // Model.rag(question[, {field:, text_field:, k:, system:}]) — one-call
    // retrieval-augmented generation: embed the question, ANN-search the
    // vector index for the top-k rows, build a context from each row's
    // text_field, and answer with the LLM. Returns { answer, sources }.
    native_static_methods.insert(
        "rag".to_string(),
        Rc::new(NativeFunction::new("Model.rag", None, |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            let question = match args.get(1) {
                Some(Value::String(s)) => s.to_string(),
                _ => return Err(format!("{}.rag expects a question string", class_name)),
            };

            let mut opts = super::search::RagOptions::default();
            if let Some(Value::Hash(h)) = args.get(2) {
                use crate::interpreter::value::HashKey;
                for (k, v) in h.borrow().iter() {
                    let key = match k {
                        HashKey::String(s) => s.to_string(),
                        _ => continue,
                    };
                    match (key.as_str(), v) {
                        ("field", Value::String(s)) => opts.field = s.to_string(),
                        ("text_field", Value::String(s)) => opts.text_field = s.to_string(),
                        ("k", Value::Int(n)) if *n > 0 => opts.k = *n as usize,
                        ("system", Value::String(s)) => opts.system = s.to_string(),
                        (other, _) => {
                            return Err(format!(
                                "rag() unknown/invalid option '{}': expected field:, \
                                     text_field:, k:, or system:",
                                other
                            ))
                        }
                    }
                }
            }

            super::search::exec_rag(&class, &class_name, &collection, &question, &opts)
        })),
    );

    // Model.near(lat, lon[, options]) / Model.within(lat, lon, radius) —
    // geo queries over the declared `geo_index` field. Results carry
    // `_distance` (meters).
    fn geo_args(
        args: &[Value],
        method: &str,
    ) -> Result<(String, String, Rc<Class>, f64, f64), String> {
        let class = get_class_rc_from_args(args)?;
        let class_name = class.name.clone();
        let collection = class_name_to_collection(&class_name);
        let geo = super::registry::get_geo_indexes(&class_name);
        let field = geo.first().map(|g| g.field.clone()).ok_or_else(|| {
            format!(
                "{}.{} requires a `geo_index` declaration in the class body, e.g. \
                         geo_index \"location\"",
                class_name, method
            )
        })?;
        let num = |v: Option<&Value>, what: &str| -> Result<f64, String> {
            match v {
                Some(Value::Int(n)) => Ok(*n as f64),
                Some(Value::Float(f)) => Ok(*f),
                _ => Err(format!(
                    "{}.{} expects numeric {}",
                    class_name, method, what
                )),
            }
        };
        let lat = num(args.get(1), "lat")?;
        let lon = num(args.get(2), "lon")?;
        Ok((collection, field, class, lat, lon))
    }

    native_static_methods.insert(
        "near".to_string(),
        Rc::new(NativeFunction::new("Model.near", None, |args| {
            let class_name = get_class_name_from_class(args)?;
            let (collection, field, class, lat, lon) = geo_args(args, "near")?;
            let mut limit: f64 = 10.0;
            if let Some(Value::Hash(opts)) = args.get(3) {
                use crate::interpreter::value::HashKey;
                for (k, v) in opts.borrow().iter() {
                    match (k, v) {
                        (HashKey::String(key), Value::Int(n))
                            if key.as_str() == "limit" && *n > 0 =>
                        {
                            limit = *n as f64
                        }
                        (HashKey::String(key), _) => {
                            return Err(format!(
                                "near() unknown/invalid option '{}': expected limit:",
                                key
                            ))
                        }
                        _ => {}
                    }
                }
            }
            super::search::exec_geo_query(
                &collection,
                &class,
                &field,
                "near",
                lat,
                lon,
                ("limit", limit),
                is_soft_delete(&class_name),
            )
        })),
    );

    native_static_methods.insert(
        "within".to_string(),
        Rc::new(NativeFunction::new("Model.within", None, |args| {
            let class_name = get_class_name_from_class(args)?;
            let (collection, field, class, lat, lon) = geo_args(args, "within")?;
            let radius = match args.get(3) {
                Some(Value::Int(n)) if *n > 0 => *n as f64,
                Some(Value::Float(f)) if *f > 0.0 => *f,
                _ => {
                    return Err(format!(
                        "{}.within expects a radius in meters as the third argument",
                        class_name
                    ))
                }
            };
            super::search::exec_geo_query(
                &collection,
                &class,
                &field,
                "within",
                lat,
                lon,
                ("radius", radius),
                is_soft_delete(&class_name),
            )
        })),
    );

    // Model.time_bucket(interval[, aggregates]) — bucketed aggregation for
    // timeseries models. Returns a QueryBuilder (chain .all to execute).
    native_static_methods.insert(
        "time_bucket".to_string(),
        Rc::new(NativeFunction::new("Model.time_bucket", None, |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            if !super::registry::is_timeseries_model(&class_name) {
                return Err(format!(
                    "{}.time_bucket() requires a `timeseries` declaration in the class \
                         body — for regular models use group_by()",
                    class_name
                ));
            }

            let interval_arg = args.get(1).ok_or_else(|| {
                "time_bucket(interval, {aggregates}) requires an interval string, \
                         e.g. time_bucket(\"1h\", { \"avg\": \"value\" })"
                    .to_string()
            })?;
            let spec =
                super::query::parse_time_bucket_args(interval_arg, args.get(2), &class_name)?;

            let mut qb = super::query::QueryBuilder::new_with_class(class_name, collection, class);
            qb.time_bucket_info = Some(spec);
            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // Model.prune([older_than]) — timeseries retention. Deletes documents
    // older than the cutoff: a duration ("30d" → now - 30d), an RFC3339
    // timestamp, or (no argument) the declared `retention:`. Returns the
    // number of deleted documents.
    native_static_methods.insert(
        "prune".to_string(),
        Rc::new(NativeFunction::new_auto_invocable(
            "Model.prune",
            None,
            |args| {
                let class = get_class_rc_from_args(args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                let ts_spec =
                    super::registry::get_timeseries_spec(&class_name).ok_or_else(|| {
                        format!(
                            "{}.prune() requires a `timeseries` declaration in the class \
                                 body",
                            class_name
                        )
                    })?;

                let cutoff_iso = match args.get(1) {
                    Some(Value::String(s)) => {
                        let s = s.to_string();
                        if validate_retention_duration(&s).is_ok() {
                            duration_to_cutoff_rfc3339(&s)?
                        } else if chrono::DateTime::parse_from_rfc3339(&s).is_ok() {
                            s
                        } else {
                            return Err(format!(
                                "{}.prune() expects a duration (\"30d\") or an RFC3339 \
                                     timestamp, got {:?}",
                                class_name, s
                            ));
                        }
                    }
                    Some(other) => {
                        return Err(format!(
                            "{}.prune() expects a string argument, got {}",
                            class_name,
                            other.type_name()
                        ))
                    }
                    None => match &ts_spec.retention {
                        Some(retention) => duration_to_cutoff_rfc3339(retention)?,
                        None => {
                            return Err(format!(
                                "{}.prune requires an argument or a retention: declaration \
                                     (e.g. timeseries retention: \"30d\")",
                                class_name
                            ))
                        }
                    },
                };

                let deleted = super::crud::exec_prune(&collection, &cutoff_iso)?;
                Ok(Value::Int(deleted))
            },
        )),
    );

    // ====================================================================
}
