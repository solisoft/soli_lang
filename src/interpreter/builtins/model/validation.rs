//! Validation types and execution logic.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::executor::{ControlFlow, Interpreter};
use crate::interpreter::value::{Function, HashKey, HashPairs, Value};

/// Persistence operation names used for `validates ... on:` matching.
/// `run_validations` derives the operation from `exclude_key`: every update
/// path (instance.update / save-with-_key) passes the record's key so
/// uniqueness can exclude self, and every create path passes `None`.
const OP_CREATE: &str = "create";
const OP_UPDATE: &str = "update";

use super::core::{class_name_to_collection, MODEL_REGISTRY};
use super::crud::exec_with_auto_collection;

/// A closure-based custom validator. Keyed `(class_name, field_name)`. Stored
/// thread-local because `Rc<Function>` is `!Send` and the global
/// `MODEL_REGISTRY` (a process-wide `RwLock`) requires `Send + Sync` contents.
/// Each worker thread populates this registry independently when it loads the
/// model files; the same closures are functionally equivalent across threads.
#[derive(Clone)]
pub struct CustomValidator {
    pub field: String,
    pub func: Rc<Function>,
    /// Human-readable error message used when the closure returns false.
    pub message: String,
}

thread_local! {
    static CUSTOM_VALIDATORS: RefCell<HashMap<String, Vec<CustomValidator>>> =
        RefCell::new(HashMap::new());
}

pub fn register_custom_validator(class_name: &str, validator: CustomValidator) {
    CUSTOM_VALIDATORS.with(|c| {
        c.borrow_mut()
            .entry(class_name.to_string())
            .or_default()
            .push(validator);
    });
}

fn custom_validators_for(class_name: &str) -> Vec<CustomValidator> {
    CUSTOM_VALIDATORS.with(|c| c.borrow().get(class_name).cloned().unwrap_or_default())
}

/// STI copy-down: seed the child's closure validators with the parent's
/// (replacing any previous copy, so hot reloads don't stack). The child's
/// own `validate(fn...)` calls register afterward and append.
pub fn copy_custom_validators(parent: &str, child: &str) {
    CUSTOM_VALIDATORS.with(|c| {
        let mut map = c.borrow_mut();
        let inherited = map.get(parent).cloned().unwrap_or_default();
        if inherited.is_empty() {
            map.remove(child);
        } else {
            map.insert(child.to_string(), inherited);
        }
    });
}

/// `if:` / `unless:` condition closures attached to one `validates(...)` call.
/// Stored thread-local for the same reason as [`CustomValidator`]: closures
/// are `Rc<Function>` (`!Send`) and each worker registers its own copies when
/// it loads the model files.
#[derive(Clone, Default, Debug)]
pub struct RuleConditions {
    pub if_fn: Option<Rc<Function>>,
    pub unless_fn: Option<Rc<Function>>,
}

impl RuleConditions {
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

/// Invoke a user closure as a validator. Receives the field value and the
/// full record hash; returns true on pass, false on fail. Any error inside
/// the closure short-circuits validation with that error message.
fn invoke_validator(func: &Function, field_value: &Value, record: &Value) -> Result<bool, String> {
    let call_env = Environment::with_enclosing(func.closure.clone());
    let mut env_inner = call_env;
    // Bind positional params: (value), (value, record), or fewer.
    let mut params = func.params.iter();
    if let Some(p) = params.next() {
        env_inner.define(p.name.clone(), field_value.clone());
    }
    if let Some(p) = params.next() {
        env_inner.define(p.name.clone(), record.clone());
    }
    let env_rc = Rc::new(RefCell::new(env_inner));
    let env_clone = env_rc.borrow().clone();
    let mut interp = Interpreter::default();
    match interp.execute_block(&func.body, env_clone) {
        Ok(ControlFlow::Return(v)) | Ok(ControlFlow::Normal(v)) => Ok(v.is_truthy()),
        Ok(ControlFlow::Continue) | Ok(ControlFlow::Break) => Ok(true),
        Ok(ControlFlow::Throw(e)) => Err(format!("custom validator threw: {}", e)),
        Err(e) => Err(format!("custom validator error: {}", e)),
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
    pub custom: Option<String>, // method name for custom validation
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
    let mut registry = MODEL_REGISTRY.write().unwrap();
    let metadata = registry.entry(class_name.to_string()).or_default();
    if metadata.validations.iter().any(|r| r == &rule) {
        return;
    }
    metadata.validations.push(rule);
}

/// Register a rule together with its `if:`/`unless:` closures. The rule goes
/// into the global (deduped) registry; the closures go into this thread's
/// condition registry, keyed by the rule's identity.
pub fn register_validation_with_conditions(
    class_name: &str,
    rule: ValidationRule,
    conditions: RuleConditions,
) {
    if rule.has_condition {
        RULE_CONDITIONS.with(|c| {
            c.borrow_mut()
                .insert(rule_condition_key(class_name, &rule), conditions);
        });
    }
    register_validation(class_name, rule);
}

/// Parse the options hash of a `validates(field, {...})` call into a rule plus
/// its optional condition closures. Shared by the class static method and the
/// class-body DSL registration paths. Unknown keys are ignored (back-compat);
/// `on:`/`if:`/`unless:` reject wrong types loudly — silently dropping a
/// condition would make the rule run unconditionally, the opposite of what
/// the author asked for.
pub fn parse_validates_options(
    field: &str,
    options: &HashPairs,
) -> Result<(ValidationRule, RuleConditions), String> {
    let mut rule = ValidationRule::new(field.to_string());
    let mut conditions = RuleConditions::default();

    for (key, value) in options.iter() {
        let HashKey::String(key_str) = key else {
            continue;
        };
        match key_str.as_ref() {
            "presence" => {
                if let Value::Bool(b) = value {
                    rule.presence = *b;
                }
            }
            "uniqueness" => {
                if let Value::Bool(b) = value {
                    rule.uniqueness = *b;
                }
            }
            "min_length" => {
                if let Value::Int(n) = value {
                    rule.min_length = Some(*n as usize);
                }
            }
            "max_length" => {
                if let Value::Int(n) = value {
                    rule.max_length = Some(*n as usize);
                }
            }
            "format" => {
                if let Value::String(s) = value {
                    rule.format = Some(s.to_string());
                }
            }
            "numericality" => {
                if let Value::Bool(b) = value {
                    rule.numericality = *b;
                }
            }
            "min" => match value {
                Value::Int(n) => rule.min = Some(*n as f64),
                Value::Float(n) => rule.min = Some(*n),
                _ => {}
            },
            "max" => match value {
                Value::Int(n) => rule.max = Some(*n as f64),
                Value::Float(n) => rule.max = Some(*n),
                _ => {}
            },
            "custom" => {
                if let Value::String(s) = value {
                    rule.custom = Some(s.to_string());
                }
            }
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
            _ => {}
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

/// Fields with `validates uniqueness: true` registered on `class_name`.
pub fn unique_validation_fields(class_name: &str) -> Vec<String> {
    let registry = MODEL_REGISTRY.read().unwrap();
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
}

/// Build `_errors` entries for a unique-violation insert/update error. We
/// attribute the failure to a specific `validates uniqueness:` field by
/// scanning the error body for its name; if no registered unique field
/// matches we fall back to flagging every uniquely-validated field, and
/// `_base` if none are registered. Silently dropping the error would leave
/// callers thinking the write succeeded, so be loud rather than precise.
pub fn build_unique_violation_errors(class_name: &str, err: &str) -> Vec<ValidationError> {
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
    let call_env = Environment::with_enclosing(func.closure.clone());
    let mut env_inner = call_env;
    if let Some(p) = func.params.first() {
        env_inner.define(p.name.clone(), record.clone());
    }
    let env_rc = Rc::new(RefCell::new(env_inner));
    let env_clone = env_rc.borrow().clone();
    let mut interp = Interpreter::default();
    match interp.execute_block(&func.body, env_clone) {
        Ok(ControlFlow::Return(v)) | Ok(ControlFlow::Normal(v)) => Ok(v.is_truthy()),
        Ok(ControlFlow::Continue) | Ok(ControlFlow::Break) => Ok(true),
        Ok(ControlFlow::Throw(e)) => Err(format!("validation condition threw: {}", e)),
        Err(e) => Err(format!("validation condition error: {}", e)),
    }
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
pub fn run_validations(
    class_name: &str,
    data: &Value,
    exclude_key: Option<&str>,
) -> Result<Vec<ValidationError>, String> {
    // Clone the rules out so no MODEL_REGISTRY lock is held during the run:
    // uniqueness talks to the database and if:/unless: conditions execute
    // user closures, either of which may re-enter the registry.
    let rules: Vec<ValidationRule> = {
        let registry = MODEL_REGISTRY.read().unwrap();
        registry
            .get(class_name)
            .map(|m| m.validations.clone())
            .unwrap_or_default()
    };

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
                    #[allow(unused_variables)]
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
                    let results = exec_with_auto_collection(sdbql, Some(bind_vars), &collection)
                        .map_err(|e| format!("Database error during uniqueness check: {}", e))?;
                    if !results.is_empty() {
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
    }

    // Closure-based custom validators registered via `register_custom_validator`.
    let custom = custom_validators_for(class_name);
    for v in &custom {
        let field_value = lookup_field(data, &v.field).unwrap_or(Value::Null);
        match invoke_validator(&v.func, &field_value, data) {
            Ok(true) => {}
            Ok(false) => errors.push(ValidationError::new(&v.field, v.message.clone())),
            Err(e) => return Err(e),
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
        let errs = run_validations(class, &empty_hash(), None).unwrap();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].field, "email");
        // ...but the rule is skipped on update.
        let errs = run_validations(class, &empty_hash(), Some("k1")).unwrap();
        assert!(errs.is_empty());
    }

    #[test]
    fn on_update_rule_skipped_for_creates() {
        let class = "TestOnUpdateSkip__cond";
        let mut rule = ValidationRule::new("email".to_string());
        rule.presence = true;
        rule.on = Some("update".to_string());
        register_validation(class, rule);

        let errs = run_validations(class, &empty_hash(), None).unwrap();
        assert!(errs.is_empty());
        let errs = run_validations(class, &empty_hash(), Some("k1")).unwrap();
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
