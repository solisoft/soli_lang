//! Validation types and execution logic.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::executor::{ControlFlow, Interpreter};
use crate::interpreter::value::{Class, Function, HashKey, HashPairs, Instance, Value};

/// Persistence operation names used for `validates ... on:` matching.
/// `run_validations` derives the operation from `exclude_key`: every update
/// path (instance.update / save-with-_key) passes the record's key so
/// uniqueness can exclude self, and every create path passes `None`.
const OP_CREATE: &str = "create";
const OP_UPDATE: &str = "update";

use super::core::{class_name_to_collection, MODEL_REGISTRY};
use super::crud::exec_with_auto_collection;

/// `if:` / `unless:` condition closures attached to one `validates(...)` call.
/// Stored thread-local: closures are `Rc<Function>` (`!Send`), which the
/// process-wide `MODEL_REGISTRY` cannot hold, and each worker registers its
/// own copies when it loads the model files.
#[derive(Clone, Default, Debug)]
pub struct RuleConditions {
    pub if_fn: Option<Rc<Function>>,
    pub unless_fn: Option<Rc<Function>>,
    /// `custom: fn(value, record) { ... }` — the closure form of `custom:`.
    pub custom_fn: Option<Rc<Function>>,
}

impl RuleConditions {
    /// No `if:`/`unless:` gate (the custom closure is not a condition).
    pub fn is_empty(&self) -> bool {
        self.if_fn.is_none() && self.unless_fn.is_none()
    }
}

thread_local! {
    static RULE_CONDITIONS: RefCell<HashMap<String, RuleConditions>> =
        RefCell::new(HashMap::new());
}

/// Identity of a rule inside the global registry, used to key its thread-local
/// conditions. The Debug rendering of the rule's static content is stable
/// across the N+1 model-file loads (boot + per worker), so every thread maps
/// the same `validates(...)` call to the same key. Insertion overwrites, which
/// also keeps dev-mode hot reloads (same thread re-runs the class body) from
/// stacking duplicates.
fn rule_condition_key(class_name: &str, rule: &ValidationRule) -> String {
    format!("{}::{:?}", class_name, rule)
}

fn conditions_for(class_name: &str, rule: &ValidationRule) -> RuleConditions {
    RULE_CONDITIONS.with(|c| {
        c.borrow()
            .get(&rule_condition_key(class_name, rule))
            .cloned()
            .unwrap_or_default()
    })
}

/// STI copy-down: rule conditions are keyed by `class::rule` identity, so
/// the parent's rules copied into the child's metadata need their `if:` /
/// `unless:` closures mirrored under the child's key.
pub fn copy_rule_conditions(parent: &str, child: &str, rules: &[ValidationRule]) {
    RULE_CONDITIONS.with(|c| {
        let mut map = c.borrow_mut();
        for rule in rules {
            if let Some(conditions) = map.get(&rule_condition_key(parent, rule)).cloned() {
                map.insert(rule_condition_key(child, rule), conditions);
            }
        }
    });
}

/// Run a validator closure — it receives the field value and the full record
/// hash — and return what it returned. An error inside the closure
/// short-circuits validation with that error message.
fn invoke_validator_value(
    func: &Function,
    field_value: &Value,
    record: &Value,
) -> Result<Value, String> {
    let mut env_inner = Environment::with_enclosing(func.closure.clone());
    // Bind positional params: (value), (value, record), or fewer.
    let mut params = func.params.iter();
    if let Some(p) = params.next() {
        env_inner.define(p.name.clone(), field_value.clone());
    }
    if let Some(p) = params.next() {
        env_inner.define(p.name.clone(), record.clone());
    }
    run_validator_body(func, env_inner, "custom validator")
}

/// Execute a validator's body in `env` on a fresh interpreter.
fn run_validator_body(func: &Function, env: Environment, what: &str) -> Result<Value, String> {
    let mut interp = Interpreter::default();
    // COUVERTURE : reporter le fichier QUI DECLARE la fonction.
    // L'interpreteur cree ici est neuf — pile vide, `current_source_path`
    // a `None` — si bien qu'aucune ligne de ce corps n'etait imputee a un
    // fichier, quel que soit le nombre d'appels.
    if let Some(ref declaring_file) = func.source_path {
        interp.current_source_path = Some(std::path::PathBuf::from(declaring_file));
    }
    match interp.execute_block(&func.body, env) {
        Ok(ControlFlow::Return(v)) | Ok(ControlFlow::Normal(v)) => Ok(v),
        Ok(ControlFlow::Continue) | Ok(ControlFlow::Break) => Ok(Value::Bool(true)),
        Ok(ControlFlow::Throw(e)) => Err(format!("{} threw: {}", what, e)),
        Err(e) => Err(format!("{} error: {}", what, e)),
    }
}

/// Turn what a custom validator returned into errors: `false` is "is
/// invalid", a String is the message, anything else passes. (`nil` passes
/// too — a method that only pushes onto `_errors` usually ends on nothing.)
fn push_custom_outcome(errors: &mut Vec<ValidationError>, field: &str, outcome: &Value) {
    match outcome {
        Value::Bool(false) => errors.push(ValidationError::new(field, "is invalid")),
        Value::String(message) => errors.push(ValidationError::new(field, message.to_string())),
        _ => {}
    }
}

/// `custom: "method"`: call the model's instance method on a record built
/// from `data`, with `this` bound — `@field` reads work, and the method can
/// push `{"field": …, "message": …}` onto `@_errors` or return `false` / a
/// message String. A method with a parameter receives the field's value.
fn run_custom_method(
    class_name: &str,
    class: Option<&Rc<Class>>,
    rule: &ValidationRule,
    method: &str,
    data: &Value,
    errors: &mut Vec<ValidationError>,
) -> Result<(), String> {
    let class = class.ok_or_else(|| {
        format!(
            "validates(\"{}\", {{\"custom\": \"{}\"}}) needs a record of {} to call it on",
            rule.field, method, class_name
        )
    })?;
    let func = class.find_method(method).ok_or_else(|| {
        format!(
            "validates(\"{}\", {{\"custom\": \"{}\"}}): {} has no method `{}`",
            rule.field, method, class_name, method
        )
    })?;
    let mut record = Instance::new(class.clone());
    if let Value::Hash(hash) = data {
        for (key, value) in hash.borrow().iter() {
            if let HashKey::String(name) = key {
                record.set(name.clone(), value.clone());
            }
        }
    }
    let record = Rc::new(RefCell::new(record));
    let mut env = Environment::with_enclosing(func.closure.clone());
    env.define("this".to_string(), Value::Instance(record.clone()));
    if let Some(p) = func.params.first() {
        env.define(
            p.name.clone(),
            lookup_field(data, &rule.field).unwrap_or(Value::Null),
        );
    }
    let outcome = run_validator_body(&func, env, "custom validation method")?;

    // Whatever the method pushed onto `@_errors`.
    let pushed = record.borrow().get("_errors");
    if let Some(Value::Array(entries)) = pushed {
        for entry in entries.borrow().iter() {
            let (field, message) = match entry {
                Value::Hash(h) => {
                    let h = h.borrow();
                    let text = |k: &str| match h.get(&HashKey::String(k.into())) {
                        Some(Value::String(s)) => Some(s.to_string()),
                        Some(Value::Null) | None => None,
                        Some(other) => Some(other.to_string()),
                    };
                    (
                        text("field").unwrap_or_else(|| rule.field.clone()),
                        text("message").unwrap_or_else(|| "is invalid".to_string()),
                    )
                }
                Value::String(message) => (rule.field.clone(), message.to_string()),
                _ => continue,
            };
            errors.push(ValidationError::new(field, message));
        }
    }
    push_custom_outcome(errors, &rule.field, &outcome);
    Ok(())
}

/// Does `value` have the `type:` a rule asks for?
fn value_has_type(value: &Value, expected: &str) -> bool {
    match expected {
        "string" => matches!(value, Value::String(_)),
        "int" | "integer" => matches!(value, Value::Int(_)),
        "float" => matches!(value, Value::Float(_)),
        "number" => matches!(value, Value::Int(_) | Value::Float(_) | Value::Decimal(_)),
        "bool" | "boolean" => matches!(value, Value::Bool(_)),
        "array" => matches!(value, Value::Array(_)),
        "hash" => matches!(value, Value::Hash(_)),
        _ => true,
    }
}

/// A single validation rule for a field.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidationRule {
    pub field: String,
    pub presence: bool,
    pub uniqueness: bool,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub format: Option<String>, // regex pattern
    pub numericality: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// `custom: "method"`: an instance method called on the record.
    pub custom: Option<String>,
    /// `custom: fn(value, record) { ... }` is attached (the closure lives in
    /// the thread-local [`RULE_CONDITIONS`] registry, beside `if:`/`unless:`).
    pub has_custom_fn: bool,
    /// `inclusion: [...]` / `one_of: [...]`: the allowed values, compared
    /// with `==` on their JSON form (so `1` does not match `"1"`).
    pub inclusion: Option<Vec<serde_json::Value>>,
    /// `allow_nil: true` / `allow_null: true`: skip the whole rule when the
    /// value is nil or absent.
    pub allow_nil: bool,
    /// `type: "string" | "int" | "float" | "number" | "bool" | "array" |
    /// "hash"`: the value must be of that type (nil/absent skipped).
    pub value_type: Option<String>,
    /// Restrict the rule to one operation: `"create"` or `"update"`.
    /// `None` runs on both.
    pub on: Option<String>,
    /// True when an `if:`/`unless:` closure is attached (the closures
    /// themselves live in the thread-local [`RULE_CONDITIONS`] registry).
    pub has_condition: bool,
}

impl ValidationRule {
    pub fn new(field: String) -> Self {
        Self {
            field,
            ..Default::default()
        }
    }
}

/// A validation error.
#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }

    pub fn to_value(&self) -> Value {
        let mut pairs: HashPairs = HashPairs::default();
        pairs.insert(
            HashKey::String("field".into()),
            Value::String(self.field.clone().into()),
        );
        pairs.insert(
            HashKey::String("message".into()),
            Value::String(self.message.clone().into()),
        );
        Value::Hash(Rc::new(RefCell::new(pairs)))
    }
}

/// Register a validation rule for a model class. Idempotent: if an
/// equivalent rule is already registered, the call is a no-op. Required
/// because model files are loaded once at server boot and again in every
/// per-worker interpreter (see serve/mod.rs `load_models` calls), so each
/// `validates(...)` line in user code fires N+1 times. Without the dedup,
/// uniqueness checks issue N+1 identical SDBQL queries per save —
/// dominant cost in `soli test` for app-style controller suites.
pub fn register_validation(class_name: &str, rule: ValidationRule) {
    MODEL_REGISTRY.write(|registry| {
        let metadata = registry.entry(class_name.to_string()).or_default();
        if metadata.validations.iter().any(|r| r == &rule) {
            return;
        }
        metadata.validations.push(rule);
    })
}

/// Register a rule together with its `if:`/`unless:` closures. The rule goes
/// into the global (deduped) registry; the closures go into this thread's
/// condition registry, keyed by the rule's identity.
pub fn register_validation_with_conditions(
    class_name: &str,
    rule: ValidationRule,
    conditions: RuleConditions,
) {
    if rule.has_condition || rule.has_custom_fn {
        RULE_CONDITIONS.with(|c| {
            c.borrow_mut()
                .insert(rule_condition_key(class_name, &rule), conditions);
        });
    }
    register_validation(class_name, rule);
}

/// Every key `validates(field, {...})` understands, for the unknown-key error.
const VALIDATES_KEYS: &str = "presence, uniqueness, min_length, max_length, format, \
     numericality, min, max, inclusion (or one_of), type, allow_nil (or allow_null), custom, \
     on, if, unless";

const VALUE_TYPES: &[&str] = &[
    "string", "int", "integer", "float", "number", "bool", "boolean", "array", "hash",
];

fn wrong_type(key: &str, expected: &str, got: &Value) -> String {
    format!(
        "validates() `{}:` expects {}, got {}",
        key,
        expected,
        got.type_name()
    )
}

/// Parse the options hash of a `validates(field, {...})` call into a rule plus
/// its optional condition closures. Shared by the class static method and the
/// class-body DSL registration paths.
///
/// Strict: an unknown key, or a known key with a value of the wrong type,
/// raises at class-load time. Both used to be ignored, so a rule that was
/// never enforced read like protection — `"one_of": [...]` (before it
/// existed) let any value through, `"min_length": "3"` checked nothing.
pub fn parse_validates_options(
    field: &str,
    options: &HashPairs,
) -> Result<(ValidationRule, RuleConditions), String> {
    let mut rule = ValidationRule::new(field.to_string());
    let mut conditions = RuleConditions::default();

    for (key, value) in options.iter() {
        let key_str = match key {
            HashKey::String(s) | HashKey::Symbol(s) => s.as_ref(),
            _ => return Err("validates() option names must be strings".to_string()),
        };
        match key_str {
            "presence" | "uniqueness" | "numericality" | "allow_nil" | "allow_null" => {
                let Value::Bool(b) = value else {
                    return Err(wrong_type(key_str, "true or false", value));
                };
                match key_str {
                    "presence" => rule.presence = *b,
                    "uniqueness" => rule.uniqueness = *b,
                    "numericality" => rule.numericality = *b,
                    _ => rule.allow_nil = *b,
                }
            }
            "min_length" | "max_length" => {
                let n = match value {
                    Value::Int(n) if *n >= 0 => *n as usize,
                    other => return Err(wrong_type(key_str, "a non-negative Int", other)),
                };
                if key_str == "min_length" {
                    rule.min_length = Some(n);
                } else {
                    rule.max_length = Some(n);
                }
            }
            "format" => match value {
                Value::String(s) => rule.format = Some(s.to_string()),
                other => return Err(wrong_type(key_str, "a regex String", other)),
            },
            "min" | "max" => {
                let n = match value {
                    Value::Int(n) => *n as f64,
                    Value::Float(n) => *n,
                    other => return Err(wrong_type(key_str, "a number", other)),
                };
                if key_str == "min" {
                    rule.min = Some(n);
                } else {
                    rule.max = Some(n);
                }
            }
            "inclusion" | "one_of" => match value {
                Value::Array(items) => {
                    let allowed = items
                        .borrow()
                        .iter()
                        .map(crate::interpreter::value::value_to_json)
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| format!("validates() `{}:` {}", key_str, e))?;
                    rule.inclusion = Some(allowed);
                }
                other => return Err(wrong_type(key_str, "an Array of allowed values", other)),
            },
            "type" => match value {
                Value::String(s) | Value::Symbol(s) if VALUE_TYPES.contains(&s.as_str()) => {
                    rule.value_type = Some(s.to_string());
                }
                other => {
                    return Err(format!(
                        "validates() `type:` must be one of {}, got {}",
                        VALUE_TYPES.join(", "),
                        match other {
                            Value::String(s) | Value::Symbol(s) => format!("\"{}\"", s),
                            v => v.type_name(),
                        }
                    ))
                }
            },
            "custom" => match value {
                Value::String(s) | Value::Symbol(s) => rule.custom = Some(s.to_string()),
                Value::Function(f) => {
                    conditions.custom_fn = Some(f.clone());
                    rule.has_custom_fn = true;
                }
                other => {
                    return Err(wrong_type(
                        key_str,
                        "a method name or a fn(value, record) closure",
                        other,
                    ))
                }
            },
            "on" => match value {
                Value::String(s) | Value::Symbol(s)
                    if s.as_str() == OP_CREATE || s.as_str() == OP_UPDATE =>
                {
                    rule.on = Some(s.to_string());
                }
                other => {
                    return Err(format!(
                        "validates() `on:` must be \"create\" or \"update\", got {}",
                        match other {
                            Value::String(s) | Value::Symbol(s) => format!("\"{}\"", s),
                            v => v.type_name(),
                        }
                    ))
                }
            },
            "if" => match value {
                Value::Function(f) => conditions.if_fn = Some(f.clone()),
                other => {
                    return Err(format!(
                        "validates() `if:` expects a function, got {}",
                        other.type_name()
                    ))
                }
            },
            "unless" => match value {
                Value::Function(f) => conditions.unless_fn = Some(f.clone()),
                other => {
                    return Err(format!(
                        "validates() `unless:` expects a function, got {}",
                        other.type_name()
                    ))
                }
            },
            unknown => {
                return Err(format!(
                    "validates(\"{}\") has an unknown option `{}` — known options: {}",
                    field, unknown, VALIDATES_KEYS
                ))
            }
        }
    }

    rule.has_condition = !conditions.is_empty();
    Ok((rule, conditions))
}

/// Heuristic detection of a unique-index conflict in an error returned by
/// `exec_insert`/`exec_update`. SoliDB stringifies failures as
/// `"HTTP {status} {url}: {body}"` (see `crud.rs::exec_document_request`),
/// so we anchor on the `HTTP 409` status and require a body keyword that
/// names a uniqueness conflict. The collection-already-exists case (also a
/// 409, body says `collection ... already exists`) is filtered out so callers
/// don't mistake an auto-create race for a uniqueness failure.
///
/// SEC-039: the `validates uniqueness:` SELECT-then-INSERT path is racy by
/// construction; this helper lets `Model.create`/`save`/`update`/`upsert`/
/// `find_or_create_by` translate the atomic DB-side error into a normal
/// validation failure when a unique index is in place. Earlier versions
/// matched any error containing `"duplicate"`, `"conflict"`, etc., which
/// would silently convert an unrelated 5xx that happened to mention those
/// words into a validation error and mask the real fault — the tighter
/// anchor on `HTTP 409` plus a body keyword keeps the false-positive rate
/// near zero.
pub fn is_unique_violation(err: &str) -> bool {
    // A SQL adapter classifies its own driver error and marks it, so there is
    // nothing to infer from prose there (see `db::error::Constraint`).
    if let Some(constraint) = crate::db::error::Constraint::parse(err) {
        return constraint.kind == crate::db::error::ConstraintKind::Unique;
    }
    let lower = err.to_lowercase();
    if !lower.contains("http 409") {
        return false;
    }
    let has_keyword =
        lower.contains("conflict") || lower.contains("duplicate") || lower.contains("unique");
    if !has_keyword {
        return false;
    }
    !(lower.contains("collection") && lower.contains("already"))
}

/// Is `value` already used in `field` by a row other than `exclude_key`?
///
/// The portable equivalent of the SDBQL pre-check: a hash filter plus a
/// key comparison done in Rust, since the SQL compiler takes equality filters
/// only. Column-aware models answer through their own path.
fn unique_taken_on_sql(
    collection: &str,
    field: &str,
    value: &str,
    exclude_key: Option<&str>,
) -> Result<bool, String> {
    use std::collections::BTreeMap;

    let mut eq_filters = BTreeMap::new();
    eq_filters.insert(
        field.to_string(),
        serde_json::Value::String(value.to_string()),
    );

    if let Some(schema) = super::column_mode::schema_for_collection(collection) {
        let mut query = crate::db::sql_columns_compile::ColumnQuery::new(schema.clone());
        query.eq_filters = eq_filters;
        let rows = crate::db::columns::select_rows(&query)?;
        return Ok(rows
            .iter()
            .any(|row| !is_same_row(row, &schema.pk, exclude_key)));
    }

    let query = crate::db::ListQuery {
        table: collection.to_string(),
        eq_filters,
        hash_filter: None,
        filter_sdbql: Some(format!("doc.{field} == @{field}")),
        having: None,
        exists_filters: Vec::new(),
        soft_delete: crate::db::SqlSoftDeleteMode::Default,
        is_soft_delete_model: false,
        order_field: None,
        order_desc: false,
        limit: None,
        offset: None,
    };
    let rows = crate::db::sql::select(&query)?;
    Ok(rows
        .iter()
        .any(|row| !is_same_row(row, "_key", exclude_key)))
}

/// True when `row` is the record being updated, which must not conflict with
/// itself.
fn is_same_row(row: &serde_json::Value, key_field: &str, exclude_key: Option<&str>) -> bool {
    let Some(exclude) = exclude_key else {
        return false;
    };
    let stored = row
        .get(key_field)
        .or_else(|| row.get("_key"))
        .map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        });
    stored.as_deref() == Some(exclude)
}

/// The field a marked constraint violation points at, if any.
fn constraint_field(err: &str) -> Option<String> {
    crate::db::error::Constraint::parse(err).and_then(|c| c.field())
}

/// Turn any marked constraint violation into field errors.
///
/// A unique violation reads as "has already been taken", a foreign key as
/// "must reference an existing record", a NOT NULL as "can't be blank" — so a
/// database-level rule the model never declared still surfaces as a normal
/// validation failure instead of driver text. Returns `None` when the error was
/// not a classified violation, leaving it to be reported as-is.
pub fn build_constraint_errors(err: &str) -> Option<Vec<ValidationError>> {
    let constraint = crate::db::error::Constraint::parse(err)?;
    let field = constraint.field().unwrap_or_else(|| "_base".to_string());
    Some(vec![ValidationError::new(
        &field,
        constraint.kind.message(),
    )])
}

/// Fields with `validates uniqueness: true` registered on `class_name`.
pub fn unique_validation_fields(class_name: &str) -> Vec<String> {
    MODEL_REGISTRY.read(|registry| {
        registry
            .get(class_name)
            .map(|m| {
                m.validations
                    .iter()
                    .filter(|r| r.uniqueness)
                    .map(|r| r.field.clone())
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// Build `_errors` entries for a unique-violation insert/update error. We
/// attribute the failure to a specific `validates uniqueness:` field by
/// scanning the error body for its name; if no registered unique field
/// matches we fall back to flagging every uniquely-validated field, and
/// `_base` if none are registered. Silently dropping the error would leave
/// callers thinking the write succeeded, so be loud rather than precise.
pub fn build_unique_violation_errors(class_name: &str, err: &str) -> Vec<ValidationError> {
    // When the database named the column, use it: it is the truth, and it works
    // for a unique index the model never declared a validation for.
    if let Some(field) = constraint_field(err) {
        return vec![ValidationError::new(&field, "has already been taken")];
    }
    let unique = unique_validation_fields(class_name);
    let lower = err.to_lowercase();
    if let Some(field) = unique.iter().find(|f| lower.contains(&f.to_lowercase())) {
        return vec![ValidationError::new(field, "has already been taken")];
    }
    if unique.is_empty() {
        return vec![ValidationError::new("_base", "has already been taken")];
    }
    unique
        .into_iter()
        .map(|f| ValidationError::new(f, "has already been taken"))
        .collect()
}

/// Invoke an `if:`/`unless:` condition closure. The record hash is bound to
/// the closure's first parameter (if it declares one); the closure's
/// truthiness decides.
fn invoke_condition(func: &Function, record: &Value) -> Result<bool, String> {
    let mut env = Environment::with_enclosing(func.closure.clone());
    if let Some(p) = func.params.first() {
        env.define(p.name.clone(), record.clone());
    }
    run_validator_body(func, env, "validation condition").map(|v| v.is_truthy())
}

/// Decide whether a rule applies to this run: `on:` must match the operation
/// and any `if:`/`unless:` closures must agree.
fn rule_should_run(
    class_name: &str,
    rule: &ValidationRule,
    data: &Value,
    op: &str,
) -> Result<bool, String> {
    if let Some(on) = &rule.on {
        if on != op {
            return Ok(false);
        }
    }
    if rule.has_condition {
        let conditions = conditions_for(class_name, rule);
        if let Some(f) = &conditions.if_fn {
            if !invoke_condition(f, data)? {
                return Ok(false);
            }
        }
        if let Some(f) = &conditions.unless_fn {
            if invoke_condition(f, data)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Look up a field in the data hash with a short-lived borrow, so no borrow
/// is held while user closures (conditions, custom validators) run — a
/// closure that mutates the record hash must not panic the validator.
fn lookup_field(data: &Value, field: &str) -> Option<Value> {
    match data {
        Value::Hash(h) => h
            .borrow()
            .iter()
            .find(|(k, _)| matches!(k, HashKey::String(s) if **s == *field))
            .map(|(_, v)| v.clone()),
        _ => None,
    }
}

/// Run validations on data and return any errors.
/// If `exclude_key` is provided (for updates), that record is excluded from
/// uniqueness checks — and the run counts as an `update` for `on:` matching;
/// `None` counts as a `create`.
/// Does this class declare any validation rules?
///
/// Used to decide whether a static `Model.update` needs to pre-read the stored
/// document: validating a partial patch on its own would fail every `presence`
/// rule for a field the patch did not touch, so the record has to be merged
/// first — which costs a read, and is only worth paying when there is something
/// to validate.
pub fn class_has_validations(class_name: &str) -> bool {
    MODEL_REGISTRY.read(|registry| {
        registry
            .get(class_name)
            .map(|m| !m.validations.is_empty())
            .unwrap_or(false)
    })
}

pub fn run_validations(
    class_name: &str,
    class: Option<&Rc<Class>>,
    data: &Value,
    exclude_key: Option<&str>,
) -> Result<Vec<ValidationError>, String> {
    // Clone the rules out so no MODEL_REGISTRY lock is held during the run:
    // uniqueness talks to the database and if:/unless: conditions execute
    // user closures, either of which may re-enter the registry.
    let rules: Vec<ValidationRule> = MODEL_REGISTRY.read(|registry| {
        registry
            .get(class_name)
            .map(|m| m.validations.clone())
            .unwrap_or_default()
    });

    if !matches!(data, Value::Hash(_)) {
        return Ok(vec![ValidationError::new("_base", "Data must be a hash")]);
    }

    let op = if exclude_key.is_some() {
        OP_UPDATE
    } else {
        OP_CREATE
    };

    let mut errors = Vec::new();

    for rule in &rules {
        if !rule_should_run(class_name, rule, data, op)? {
            continue;
        }

        // Find the field value
        let field_value = lookup_field(data, &rule.field);
        let is_nil = matches!(field_value, None | Some(Value::Null));
        if rule.allow_nil && is_nil {
            continue;
        }

        // Presence validation
        if rule.presence {
            match &field_value {
                None => errors.push(ValidationError::new(&rule.field, "can't be blank")),
                Some(Value::Null) => {
                    errors.push(ValidationError::new(&rule.field, "can't be blank"))
                }
                Some(Value::String(s)) if s.is_empty() => {
                    errors.push(ValidationError::new(&rule.field, "can't be blank"))
                }
                _ => {}
            }
        }

        // Min length validation
        if let Some(min_len) = rule.min_length {
            if let Some(Value::String(s)) = &field_value {
                if s.len() < min_len {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("is too short (minimum is {} characters)", min_len),
                    ));
                }
            }
        }

        // Max length validation
        if let Some(max_len) = rule.max_length {
            if let Some(Value::String(s)) = &field_value {
                if s.len() > max_len {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("is too long (maximum is {} characters)", max_len),
                    ));
                }
            }
        }

        // Uniqueness validation (query the database).
        //
        // SEC-039: this SELECT is best-effort — two concurrent writers can
        // both pass it and then both insert. The atomic guarantee comes
        // from a unique DB index on `rule.field`; `Model.create`/`save`/
        // `upsert`/`find_or_create_by` translate the resulting 409 into a
        // `_errors` entry via `build_unique_violation_errors`. Models that
        // declare `validates uniqueness:` should declare the matching index
        // (see `www/docs/models.md` "Atomic uniqueness").
        if rule.uniqueness {
            if let Some(Value::String(val)) = &field_value {
                if !val.is_empty() {
                    let collection = class_name_to_collection(class_name);
                    // SQL adapters have no SDBQL. Ask the portable path instead —
                    // without this, declaring `uniqueness` made every create on
                    // Postgres/MySQL/SQLite raise "Raw SDBQL queries are
                    // SoliDB-only" before any row was written.
                    // (This used to `continue` after the SQL check, which
                    // skipped the rule's format/numericality/min/max checks
                    // on Postgres/MySQL/SQLite.)
                    let taken = if crate::db::is_sql() {
                        unique_taken_on_sql(&collection, &rule.field, val.as_ref(), exclude_key)
                            .map_err(|e| format!("Database error during uniqueness check: {e}"))?
                    } else {
                        let sdbql = if exclude_key.is_some() {
                            format!(
                                "FOR doc IN {} FILTER doc.{} == @val AND doc._key != @key LIMIT 1 RETURN 1",
                                collection, rule.field
                            )
                        } else {
                            format!(
                                "FOR doc IN {} FILTER doc.{} == @val LIMIT 1 RETURN 1",
                                collection, rule.field
                            )
                        };
                        let mut bind_vars = std::collections::HashMap::new();
                        bind_vars.insert(
                            "val".to_string(),
                            serde_json::Value::String(val.clone().to_string()),
                        );
                        if let Some(key) = exclude_key {
                            bind_vars.insert(
                                "key".to_string(),
                                serde_json::Value::String(key.to_string()),
                            );
                        }
                        !exec_with_auto_collection(sdbql, Some(bind_vars), &collection)
                            .map_err(|e| format!("Database error during uniqueness check: {}", e))?
                            .is_empty()
                    };
                    if taken {
                        errors.push(ValidationError::new(&rule.field, "has already been taken"));
                    }
                }
            }
        }

        // Format validation (regex)
        if let Some(pattern) = &rule.format {
            if let Some(Value::String(s)) = &field_value {
                let is_valid = if pattern == "email" {
                    static EMAIL_RE: std::sync::LazyLock<regex::Regex> =
                        std::sync::LazyLock::new(|| {
                            regex::Regex::new(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$")
                                .unwrap()
                        });
                    EMAIL_RE.is_match(s)
                } else if let Ok(re) = crate::regex_cache::get_regex(pattern) {
                    re.is_match(s)
                } else {
                    true
                };
                if !is_valid {
                    errors.push(ValidationError::new(&rule.field, "is invalid"));
                }
            }
        }

        // Numericality validation
        if rule.numericality {
            match &field_value {
                Some(Value::Int(_)) | Some(Value::Float(_)) => {}
                Some(_) => errors.push(ValidationError::new(&rule.field, "is not a number")),
                None => {} // Skip if field is not present (presence handles required)
            }
        }

        // Min value validation
        if let Some(min_val) = rule.min {
            match &field_value {
                Some(Value::Int(n)) if (*n as f64) < min_val => {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("must be greater than or equal to {}", min_val),
                    ));
                }
                Some(Value::Float(n)) if *n < min_val => {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("must be greater than or equal to {}", min_val),
                    ));
                }
                _ => {}
            }
        }

        // Max value validation
        if let Some(max_val) = rule.max {
            match &field_value {
                Some(Value::Int(n)) if (*n as f64) > max_val => {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("must be less than or equal to {}", max_val),
                    ));
                }
                Some(Value::Float(n)) if *n > max_val => {
                    errors.push(ValidationError::new(
                        &rule.field,
                        format!("must be less than or equal to {}", max_val),
                    ));
                }
                _ => {}
            }
        }

        // Type validation
        if let (Some(expected), Some(value)) = (&rule.value_type, &field_value) {
            if !is_nil && !value_has_type(value, expected) {
                errors.push(ValidationError::new(
                    &rule.field,
                    format!(
                        "must be {} {}",
                        if expected.starts_with(['a', 'e', 'i', 'o', 'u']) {
                            "an"
                        } else {
                            "a"
                        },
                        expected
                    ),
                ));
            }
        }

        // Inclusion validation (`inclusion:` / `one_of:`)
        if let (Some(allowed), Some(value)) = (&rule.inclusion, &field_value) {
            if !is_nil {
                let included = crate::interpreter::value::value_to_json(value)
                    .map(|json| allowed.contains(&json))
                    .unwrap_or(false);
                if !included {
                    errors.push(ValidationError::new(
                        &rule.field,
                        "is not included in the list",
                    ));
                }
            }
        }

        // Custom validation: `custom: fn(value, record) { ... }`
        if rule.has_custom_fn {
            if let Some(func) = conditions_for(class_name, rule).custom_fn {
                let value = field_value.clone().unwrap_or(Value::Null);
                let outcome = invoke_validator_value(&func, &value, data)?;
                push_custom_outcome(&mut errors, &rule.field, &outcome);
            }
        }

        // Custom validation: `custom: "method"`, called on the record
        if let Some(method) = &rule.custom {
            run_custom_method(class_name, class, rule, method, data, &mut errors)?;
        }
    }

    Ok(errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_violation_detects_409_conflict() {
        let err = "HTTP 409 Conflict http://localhost/_api/database/db/document/users: \
                   {\"errorMessage\":\"unique constraint violated on email\"}";
        assert!(is_unique_violation(err));
    }

    #[test]
    fn unique_violation_detects_duplicate_keyword() {
        let err = "HTTP 409 Conflict http://x/_api/database/db/document/users: \
                   {\"errorMessage\":\"duplicate key value\"}";
        assert!(is_unique_violation(err));
    }

    #[test]
    fn unique_violation_rejects_keyword_without_409() {
        // Earlier heuristic matched bare "duplicate" / "conflict" anywhere
        // in the error body, which silently turned unrelated 5xx errors that
        // happened to mention those words into validation failures.
        assert!(!is_unique_violation(
            "HTTP 500 duplicate request id rejected"
        ));
        assert!(!is_unique_violation("connection refused: write conflict"));
    }

    #[test]
    fn unique_violation_ignores_collection_already_exists() {
        // Collection auto-create rides on the same 409 status code; we
        // must not mistake it for a unique-key conflict.
        let err = "HTTP 409 Conflict http://x/_api/database/db/collection: \
                   {\"errorMessage\":\"collection 'users' already exists\"}";
        assert!(!is_unique_violation(err));
    }

    #[test]
    fn unique_violation_ignores_unrelated_errors() {
        assert!(!is_unique_violation("HTTP 500 Internal Server Error"));
        assert!(!is_unique_violation("connection refused"));
    }

    #[test]
    fn build_unique_violation_errors_falls_back_to_base_when_no_rule() {
        let errs = build_unique_violation_errors("ModelWithoutUniqueRule__sec039", "duplicate key");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].field, "_base");
        assert_eq!(errs[0].message, "has already been taken");
    }

    #[test]
    fn build_unique_violation_errors_picks_field_from_message() {
        let class = "TestUserSec039MatchField";
        let mut rule = ValidationRule::new("email".to_string());
        rule.uniqueness = true;
        register_validation(class, rule);
        let mut rule2 = ValidationRule::new("username".to_string());
        rule2.uniqueness = true;
        register_validation(class, rule2);

        let err = "HTTP 409 Conflict: unique constraint violated on field email";
        let errs = build_unique_violation_errors(class, err);
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].field, "email");
    }

    fn empty_hash() -> Value {
        Value::Hash(Rc::new(RefCell::new(HashPairs::default())))
    }

    #[test]
    fn on_create_rule_skipped_for_updates() {
        let class = "TestOnCreateSkip__cond";
        let mut rule = ValidationRule::new("email".to_string());
        rule.presence = true;
        rule.on = Some("create".to_string());
        register_validation(class, rule);

        // Missing field: presence fails on create...
        let errs = run_validations(class, None, &empty_hash(), None).unwrap();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].field, "email");
        // ...but the rule is skipped on update.
        let errs = run_validations(class, None, &empty_hash(), Some("k1")).unwrap();
        assert!(errs.is_empty());
    }

    #[test]
    fn on_update_rule_skipped_for_creates() {
        let class = "TestOnUpdateSkip__cond";
        let mut rule = ValidationRule::new("email".to_string());
        rule.presence = true;
        rule.on = Some("update".to_string());
        register_validation(class, rule);

        let errs = run_validations(class, None, &empty_hash(), None).unwrap();
        assert!(errs.is_empty());
        let errs = run_validations(class, None, &empty_hash(), Some("k1")).unwrap();
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn parse_options_reads_on() {
        let mut options = HashPairs::default();
        options.insert(HashKey::String("presence".into()), Value::Bool(true));
        options.insert(HashKey::String("on".into()), Value::String("create".into()));
        let (rule, conditions) = parse_validates_options("email", &options).unwrap();
        assert!(rule.presence);
        assert_eq!(rule.on.as_deref(), Some("create"));
        assert!(!rule.has_condition);
        assert!(conditions.is_empty());
    }

    #[test]
    fn parse_options_rejects_bad_on_value() {
        let mut options = HashPairs::default();
        options.insert(
            HashKey::String("on".into()),
            Value::String("creates".into()),
        );
        let err = parse_validates_options("email", &options).unwrap_err();
        assert!(err.contains("\"creates\""), "unexpected error: {}", err);
    }

    #[test]
    fn parse_options_rejects_unknown_keys_and_wrong_types() {
        let options = |pairs: Vec<(&str, Value)>| {
            let mut o = HashPairs::default();
            for (k, v) in pairs {
                o.insert(HashKey::String(k.into()), v);
            }
            o
        };
        let err =
            parse_validates_options("status", &options(vec![("in", Value::Null)])).unwrap_err();
        assert!(err.contains("unknown option `in`"), "{err}");
        let err = parse_validates_options(
            "name",
            &options(vec![("min_length", Value::String("3".into()))]),
        )
        .unwrap_err();
        assert!(err.contains("min_length"), "{err}");
        assert!(parse_validates_options(
            "active",
            &options(vec![("type", Value::String("bolean".into()))])
        )
        .is_err());
        assert!(
            parse_validates_options("status", &options(vec![("one_of", Value::Int(1))])).is_err()
        );
    }

    #[test]
    fn inclusion_type_and_allow_nil() {
        let class = "TestInclusionRules__v";
        let mut options = HashPairs::default();
        options.insert(
            HashKey::String("one_of".into()),
            Value::Array(Rc::new(RefCell::new(vec![
                Value::String("draft".into()),
                Value::Int(1),
            ]))),
        );
        options.insert(HashKey::String("allow_nil".into()), Value::Bool(true));
        let (rule, _) = parse_validates_options("status", &options).unwrap();
        register_validation(class, rule);

        let record = |v: Value| {
            let mut h = HashPairs::default();
            h.insert(HashKey::String("status".into()), v);
            Value::Hash(Rc::new(RefCell::new(h)))
        };
        let check = |v: Value| run_validations(class, None, &record(v), None).unwrap();
        assert!(check(Value::String("draft".into())).is_empty());
        assert!(check(Value::Int(1)).is_empty());
        assert!(check(Value::Null).is_empty());
        let errs = check(Value::String("1".into()));
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].message, "is not included in the list");
        assert_eq!(check(Value::String("bogus".into())).len(), 1);
    }

    #[test]
    fn parse_options_rejects_non_function_condition() {
        let mut options = HashPairs::default();
        options.insert(HashKey::String("if".into()), Value::Bool(true));
        assert!(parse_validates_options("email", &options).is_err());

        let mut options = HashPairs::default();
        options.insert(HashKey::String("unless".into()), Value::Bool(true));
        assert!(parse_validates_options("email", &options).is_err());
    }

    #[test]
    fn build_unique_violation_errors_flags_all_when_field_unknown() {
        let class = "TestUserSec039AllFields";
        let mut rule_a = ValidationRule::new("alpha".to_string());
        rule_a.uniqueness = true;
        register_validation(class, rule_a);
        let mut rule_b = ValidationRule::new("beta".to_string());
        rule_b.uniqueness = true;
        register_validation(class, rule_b);

        // No registered field name appears in the body.
        let errs = build_unique_violation_errors(class, "duplicate key");
        let fields: Vec<&str> = errs.iter().map(|e| e.field.as_str()).collect();
        assert!(fields.contains(&"alpha"));
        assert!(fields.contains(&"beta"));
    }
}
