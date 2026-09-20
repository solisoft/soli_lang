//! `Model`'s aggregation: `pluck`, `aggregate` and `group_by`.
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
use crate::interpreter::value::{NativeFunction, Value};

pub(super) fn register(native_static_methods: &mut HashMap<String, Rc<NativeFunction>>) {
    // Model.pluck(field, ...) - Convenience: creates QB and sets pluck fields
    native_static_methods.insert(
        "pluck".to_string(),
        Rc::new(NativeFunction::new("Model.pluck", None, |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);
            let mut fields = Vec::new();
            for arg in args.iter().skip(1) {
                match arg {
                    Value::String(s) | Value::Symbol(s) => {
                        validate_field_name(s, "pluck")?;
                        fields.push(s.to_string());
                    }
                    _ => return Err("pluck() expects a field name (string or symbol)s".to_string()),
                }
            }
            if fields.is_empty() {
                return Err("pluck() requires at least one field name".to_string());
            }
            let mut qb = super::query::QueryBuilder::new_with_class(class_name, collection, class);
            qb.set_pluck(fields);
            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // Model.sum(field), Model.avg(field), ... - Aggregations. The stats
    // funcs (median/stddev/variance) are emitted via COLLECT_LIST — see
    // build_aggregation_query. PERCENTILE is not a SolidB function (yet),
    // so it is deliberately absent.
    for (name, func) in &[
        ("sum", super::AggregationFunc::Sum),
        ("avg", super::AggregationFunc::Avg),
        ("min", super::AggregationFunc::Min),
        ("max", super::AggregationFunc::Max),
        ("median", super::AggregationFunc::Median),
        ("stddev", super::AggregationFunc::Stddev),
        ("variance", super::AggregationFunc::Variance),
        ("count_distinct", super::AggregationFunc::CountDistinct),
    ] {
        let method_name = name.to_string();
        let func = func.clone();
        native_static_methods.insert(
            method_name.clone(),
            Rc::new(NativeFunction::new(
                Box::leak(format!("Model.{}", method_name).into_boxed_str()),
                Some(2),
                move |args| {
                    let class = get_class_rc_from_args(args)?;
                    let class_name = class.name.clone();
                    let collection = class_name_to_collection(&class_name);
                    let field = match args.get(1) {
                        Some(Value::String(s) | Value::Symbol(s)) => s.clone(),
                        _ => {
                            return Err(format!(
                                "{}() expects a field name (string or symbol)",
                                method_name
                            ))
                        }
                    };
                    validate_field_name(&field, &method_name)?;
                    let mut qb =
                        super::query::QueryBuilder::new_with_class(class_name, collection, class);
                    qb.aggregation = Some((func.clone(), field.to_string()));
                    Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
                },
            )),
        );
    }

    // Model.aggregate(...) — ONE static, two model kinds:
    //   Document models: aggregate({alias: [func, field], ...}) →
    //     QueryBuilder in grouped mode (combine with .group_by/.having;
    //     chain .all / .first).
    //   Columnar models: aggregate(column, op[, {"group_by": [...]}]) →
    //     executes immediately via the columnar engine (scalar, or rows
    //     of {group cols..., "value"} when grouped).
    native_static_methods.insert(
        "aggregate".to_string(),
        Rc::new(NativeFunction::new("Model.aggregate", None, |args| {
            let class = get_class_rc_from_args(args)?;
            let class_name = class.name.clone();
            let collection = class_name_to_collection(&class_name);

            if super::registry::is_columnar_model(&class_name) {
                let column = match args.get(1) {
                    Some(Value::String(s)) => s.to_string(),
                    _ => {
                        return Err(format!(
                            "{}.aggregate(column, op[, options]) expects a column name \
                                 string (columnar form)",
                            class_name
                        ))
                    }
                };
                validate_field_name(&column, "aggregate")?;
                let op = match args.get(2) {
                    Some(Value::String(s)) => s.to_lowercase(),
                    _ => {
                        return Err(format!(
                            "{}.aggregate(column, op[, options]) expects an operation \
                                 string ({})",
                            class_name,
                            super::columnar::COLUMN_AGG_OPS.join(", ")
                        ))
                    }
                };
                if !super::columnar::COLUMN_AGG_OPS.contains(&op.as_str()) {
                    return Err(format!(
                        "{}.aggregate unknown operation '{}': expected one of {}",
                        class_name,
                        op,
                        super::columnar::COLUMN_AGG_OPS.join(", ")
                    ));
                }
                let mut group_by: Option<Vec<String>> = None;
                if let Some(Value::Hash(opts)) = args.get(3) {
                    use crate::interpreter::value::HashKey;
                    for (k, v) in opts.borrow().iter() {
                        match k {
                            HashKey::String(s) if s.as_str() == "group_by" => {
                                let cols = match v {
                                    Value::Array(arr) => {
                                        let arr = arr.borrow();
                                        let mut out = Vec::with_capacity(arr.len());
                                        for c in arr.iter() {
                                            match c {
                                                Value::String(s) => {
                                                    validate_field_name(s, "aggregate")?;
                                                    out.push(s.to_string());
                                                }
                                                other => {
                                                    return Err(format!(
                                                        "aggregate() group_by entries must \
                                                             be strings, got {}",
                                                        other.type_name()
                                                    ))
                                                }
                                            }
                                        }
                                        out
                                    }
                                    Value::String(s) => {
                                        validate_field_name(s, "aggregate")?;
                                        vec![s.to_string()]
                                    }
                                    other => {
                                        return Err(format!(
                                            "aggregate() group_by must be an array of \
                                                 columns, got {}",
                                            other.type_name()
                                        ))
                                    }
                                };
                                group_by = Some(cols);
                            }
                            HashKey::String(s) => {
                                return Err(format!(
                                    "aggregate() unknown option '{}': expected group_by",
                                    s
                                ))
                            }
                            _ => {}
                        }
                    }
                }
                return super::columnar::aggregate(&collection, &column, &op, group_by);
            }

            let spec_arg = args.get(1).ok_or_else(|| {
                "aggregate() requires a spec hash, e.g. aggregate({ \"total\": [\"sum\", \
                     \"amount\"] })"
                    .to_string()
            })?;
            let specs = super::query::parse_aggregate_spec_hash(spec_arg)?;
            let mut qb = super::query::QueryBuilder::new_with_class(class_name, collection, class);
            qb.aggregate_specs = specs;
            Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
        })),
    );

    // Columnar-model statics. Each requires a `columnar` declaration;
    // document models get a clear pointer at the document API instead.
    {
        fn require_columnar(args: &[Value], method: &str) -> Result<(String, String), String> {
            let class_name = get_class_name_from_class(args)?;
            if !super::registry::is_columnar_model(&class_name) {
                return Err(format!(
                    "{}.{} requires a `columnar` declaration in the class body (regular \
                         models use the document API)",
                    class_name, method
                ));
            }
            let collection = class_name_to_collection(&class_name);
            Ok((class_name, collection))
        }

        // Model.insert_rows([{...}, ...]) — bulk row insert; auto-creates
        // the store from the declared schema in dev.
        native_static_methods.insert(
            "insert_rows".to_string(),
            Rc::new(NativeFunction::new("Model.insert_rows", Some(2), |args| {
                let (class_name, collection) = require_columnar(args, "insert_rows")?;
                let rows = match args.get(1) {
                    Some(rows @ Value::Array(_)) => crate::interpreter::value::value_to_json(rows)?,
                    _ => {
                        return Err(format!(
                            "{}.insert_rows expects an array of row hashes",
                            class_name
                        ))
                    }
                };
                let schema = super::registry::get_columnar_schema(&class_name).unwrap_or_default();
                super::columnar::insert_rows(&collection, &schema, rows)
            })),
        );

        // Model.query({"columns": [...], "filter": {...}, "limit": n}) —
        // projection scan with the engine's single optional filter.
        native_static_methods.insert(
            "query".to_string(),
            Rc::new(NativeFunction::new("Model.query", Some(2), |args| {
                let (class_name, collection) = require_columnar(args, "query")?;
                let options = args.get(1).ok_or_else(|| {
                    format!(
                        "{}.query expects an options hash with a \"columns\" array",
                        class_name
                    )
                })?;
                let (columns, filter, limit) = super::columnar::parse_query_options(options)?;
                super::columnar::query(&collection, columns, filter, limit)
            })),
        );

        // Model.add_column_index(column[, type]) — sorted (default),
        // hash, bitmap, minmax, or bloom.
        native_static_methods.insert(
            "add_column_index".to_string(),
            Rc::new(NativeFunction::new(
                "Model.add_column_index",
                None,
                |args| {
                    let (class_name, collection) = require_columnar(args, "add_column_index")?;
                    let column = match args.get(1) {
                        Some(Value::String(s)) => s.to_string(),
                        _ => {
                            return Err(format!(
                                "{}.add_column_index expects a column name",
                                class_name
                            ))
                        }
                    };
                    validate_field_name(&column, "add_column_index")?;
                    let index_type = match args.get(2) {
                        Some(Value::String(s)) => {
                            let t = s.to_lowercase();
                            if !super::columnar::COLUMN_INDEX_TYPES.contains(&t.as_str()) {
                                return Err(format!(
                                    "{}.add_column_index unknown type '{}': expected one \
                                         of {}",
                                    class_name,
                                    t,
                                    super::columnar::COLUMN_INDEX_TYPES.join(", ")
                                ));
                            }
                            Some(t)
                        }
                        None => None,
                        Some(other) => {
                            return Err(format!(
                                "{}.add_column_index type must be a string, got {}",
                                class_name,
                                other.type_name()
                            ))
                        }
                    };
                    super::columnar::create_index(&collection, &column, index_type.as_deref())
                },
            )),
        );

        native_static_methods.insert(
            "column_indexes".to_string(),
            Rc::new(NativeFunction::new_auto_invocable(
                "Model.column_indexes",
                Some(1),
                |args| {
                    let (_, collection) = require_columnar(args, "column_indexes")?;
                    super::columnar::list_indexes(&collection)
                },
            )),
        );

        native_static_methods.insert(
            "drop_column_index".to_string(),
            Rc::new(NativeFunction::new(
                "Model.drop_column_index",
                Some(2),
                |args| {
                    let (class_name, collection) = require_columnar(args, "drop_column_index")?;
                    let column = match args.get(1) {
                        Some(Value::String(s)) => s.to_string(),
                        _ => {
                            return Err(format!(
                                "{}.drop_column_index expects a column name",
                                class_name
                            ))
                        }
                    };
                    validate_field_name(&column, "drop_column_index")?;
                    super::columnar::drop_index(&collection, &column)
                },
            )),
        );

        native_static_methods.insert(
            "columnar_stats".to_string(),
            Rc::new(NativeFunction::new_auto_invocable(
                "Model.columnar_stats",
                Some(1),
                |args| {
                    let (_, collection) = require_columnar(args, "columnar_stats")?;
                    super::columnar::stats(&collection)
                },
            )),
        );
    }

    // Model.group_by(...) — two forms:
    //   group_by("country") / group_by(["country", "plan"]) — multi-key
    //   grouping (combine with .aggregate/.having; implicit count).
    //   group_by(field, func, agg_field) — legacy single-aggregate form,
    //   returns [{group, result}] rows (unchanged).
    native_static_methods.insert(
            "group_by".to_string(),
            Rc::new(NativeFunction::new("Model.group_by", None, |args| {
                let class = get_class_rc_from_args(args)?;
                let class_name = class.name.clone();
                let collection = class_name_to_collection(&class_name);

                if args.len() == 2 {
                    let fields: Vec<String> = match args.get(1) {
                        Some(Value::String(s)) => vec![s.to_string()],
                        Some(Value::Array(arr)) => {
                            let arr = arr.borrow();
                            let mut out = Vec::with_capacity(arr.len());
                            for v in arr.iter() {
                                match v {
                                    Value::String(s) | Value::Symbol(s) => out.push(s.to_string()),
                                    other => {
                                        return Err(format!(
                                            "group_by() expects a field name (string or symbol)s, got {} in \
                                             array",
                                            other.type_name()
                                        ))
                                    }
                                }
                            }
                            out
                        }
                        _ => {
                            return Err("group_by() expects a field name or array of field names"
                                .to_string())
                        }
                    };
                    if fields.is_empty() {
                        return Err("group_by() requires at least one field".to_string());
                    }
                    for f in &fields {
                        validate_field_name(f, "group_by")?;
                    }
                    let mut qb =
                        super::query::QueryBuilder::new_with_class(class_name, collection, class);
                    qb.group_fields = fields;
                    return Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))));
                }

                let group_field = match args.get(1) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("group_by() expects string group field".to_string()),
                };
                validate_field_name(&group_field, "group_by")?;
                let func_name = match args.get(2) {
                    Some(Value::String(s)) => s.clone().to_lowercase(),
                    _ => return Err("group_by() expects string function name".to_string()),
                };
                let agg_field = match args.get(3) {
                    Some(Value::String(s)) => s.clone(),
                    _ => return Err("group_by() expects string aggregate field".to_string()),
                };
                validate_field_name(&agg_field, "group_by")?;
                let func = match func_name.as_str() {
                    "sum" => super::AggregationFunc::Sum,
                    "avg" => super::AggregationFunc::Avg,
                    "min" => super::AggregationFunc::Min,
                    "max" => super::AggregationFunc::Max,
                    _ => {
                        return Err(
                            "group_by() function must be one of: sum, avg, min, max".to_string()
                        )
                    }
                };
                let mut qb =
                    super::query::QueryBuilder::new_with_class(class_name, collection, class);
                qb.group_by_info = Some((group_field.to_string(), func, agg_field.to_string()));
                Ok(Value::QueryBuilder(Rc::new(RefCell::new(qb))))
            })),
        );
}
