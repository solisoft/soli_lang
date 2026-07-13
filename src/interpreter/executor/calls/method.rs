//! Method call evaluation for Array, Hash, QueryBuilder, and String.

use std::cell::RefCell;
use std::rc::Rc;

use crate::error::RuntimeError;
use crate::interpreter::environment::Environment;
use crate::interpreter::executor::{ControlFlow, Interpreter, RuntimeResult};
use crate::interpreter::value::{
    hash_contains_value, hash_get_value, HashKey, HashPairs, Value, ValueMethod,
};
use crate::span::Span;

impl Interpreter {
    pub(crate) fn call_hash_method_on_rc(
        &mut self,
        hash: &Rc<RefCell<HashPairs>>,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "set" | "delete" | "clear" | "shift" => match method_name {
                "set" => {
                    if arguments.len() != 2 {
                        return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
                    }
                    let key = &arguments[0];
                    let value = arguments[1].clone();
                    match key {
                        Value::String(s) => {
                            let mut hash_ref = hash.borrow_mut();
                            if let Some((_, _, existing)) =
                                hash_ref.get_full_mut(&crate::interpreter::value::StrKey(s))
                            {
                                *existing = value.clone();
                            } else {
                                hash_ref.insert(HashKey::String(s.clone()), value.clone());
                            }
                        }
                        _ => {
                            let hash_key = key.to_hash_key().ok_or_else(|| {
                                RuntimeError::type_error(
                                    format!("{} cannot be used as a hash key", key.type_name()),
                                    span,
                                )
                            })?;
                            hash.borrow_mut().insert(hash_key, value.clone());
                        }
                    }
                    Ok(value)
                }
                "delete" => {
                    if arguments.len() != 1 {
                        return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                    }
                    let key = &arguments[0];
                    let deleted_value = match key {
                        Value::String(s) => hash
                            .borrow_mut()
                            .shift_remove(&crate::interpreter::value::StrKey(s)),
                        _ => {
                            let hash_key = match key.to_hash_key() {
                                Some(k) => k,
                                None => return Ok(Value::Null),
                            };
                            hash.borrow_mut().shift_remove(&hash_key)
                        }
                    };
                    Ok(deleted_value.unwrap_or(Value::Null))
                }
                "clear" => {
                    if !arguments.is_empty() {
                        return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                    }
                    hash.borrow_mut().clear();
                    Ok(Value::Null)
                }
                "shift" => {
                    if !arguments.is_empty() {
                        return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                    }
                    let mut hash_ref = hash.borrow_mut();
                    if hash_ref.is_empty() {
                        return Ok(Value::Null);
                    }
                    let (key, value) =
                        hash_ref
                            .swap_remove_index(0)
                            .ok_or_else(|| RuntimeError::General {
                                message: "unexpected error in hash shift".to_string(),
                                span,
                            })?;
                    Ok(Value::Array(Rc::new(RefCell::new(vec![
                        key.to_value(),
                        value,
                    ]))))
                }
                _ => unreachable!(),
            },
            _ => {
                {
                    let entries = hash.borrow();
                    if let Some(result) =
                        self.call_hash_method_borrowed(&entries, method_name, &arguments, span)
                    {
                        return result;
                    }
                }
                let entries: Vec<(HashKey, Value)> = hash
                    .borrow()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                self.call_hash_method(&entries, method_name, arguments, span)
            }
        }
    }

    /// Call an array method on the receiver's `Rc<RefCell<Vec<Value>>>`:
    /// mutating methods operate on the cell, the borrowed/pure tiers run on
    /// a live borrow, and only closure-taking iterators pay the snapshot
    /// clone. Shared by `call_method` (Value::Method dispatch) and the
    /// direct `arr.method(args)` fast path in `evaluate_call`.
    pub(crate) fn call_array_method_on_rc(
        &mut self,
        arr: &Rc<RefCell<Vec<Value>>>,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "push" | "pop" | "clear" | "concat" => {
                // Mutating methods need the original Rc<RefCell>
                match method_name {
                    "push" => {
                        if arguments.len() != 1 {
                            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                        }
                        arr.borrow_mut().push(arguments[0].clone());
                        Ok(Value::Null)
                    }
                    "pop" => {
                        if !arguments.is_empty() {
                            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                        }
                        arr.borrow_mut()
                            .pop()
                            .ok_or_else(|| RuntimeError::type_error("pop on empty array", span))
                    }
                    "clear" => {
                        if !arguments.is_empty() {
                            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
                        }
                        arr.borrow_mut().clear();
                        Ok(Value::Null)
                    }
                    "concat" => {
                        // Validate every argument before mutating so a
                        // bad arg (or `arr.concat(arr)`) doesn't leave
                        // the receiver in a half-extended state.
                        let mut to_append: Vec<Value> = Vec::new();
                        for arg in arguments.iter() {
                            let other = match arg {
                                Value::Array(other_arr) => other_arr.borrow().clone(),
                                Value::Instance(inst) => {
                                    match inst.borrow().fields.get("__value").cloned() {
                                        Some(Value::Array(other_arr)) => other_arr.borrow().clone(),
                                        _ => {
                                            return Err(RuntimeError::type_error(
                                                "Array.concat() argument must be an Array",
                                                span,
                                            ))
                                        }
                                    }
                                }
                                _ => {
                                    return Err(RuntimeError::type_error(
                                        "Array.concat() argument must be an Array",
                                        span,
                                    ))
                                }
                            };
                            to_append.extend(other);
                        }
                        arr.borrow_mut().extend(to_append);
                        Ok(Value::Array(arr.clone()))
                    }
                    _ => unreachable!(),
                }
            }
            _ => {
                {
                    let items = arr.borrow();
                    if let Some(result) =
                        self.call_array_method_borrowed(&items, method_name, &arguments, span)
                    {
                        return result;
                    }
                    // Methods that never run user code can work
                    // directly on the live borrow — no re-entrant
                    // mutation of `arr` is possible, so the O(n)
                    // snapshot clone below is only needed for the
                    // closure-taking iterators.
                    if !Self::array_method_runs_user_code(method_name) {
                        return self.call_array_method(&items, method_name, arguments, span);
                    }
                }
                // Closure-taking methods iterate over a snapshot so a
                // user closure mutating the receiver mid-iteration
                // (`arr.push(...)` inside `map`) stays well-defined
                // instead of panicking on a RefCell double-borrow.
                let items = arr.borrow().clone();
                self.call_array_method(&items, method_name, arguments, span)
            }
        }
    }

    /// Call a method on a Value.
    pub(crate) fn call_method(
        &mut self,
        method: ValueMethod,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match *method.receiver {
            Value::Array(ref arr) => {
                self.call_array_method_on_rc(arr, &method.method_name, arguments, span)
            }
            Value::Hash(ref hash) => {
                self.call_hash_method_on_rc(hash, &method.method_name, arguments, span)
            }
            Value::QueryBuilder(qb) => {
                self.call_query_builder_method(qb, &method.method_name, arguments, span)
            }
            Value::String(s) => {
                if let Some(result) =
                    self.call_string_method_borrowed(&s, &method.method_name, &arguments, span)
                {
                    result
                } else {
                    self.call_string_method(&s, &method.method_name, arguments, span)
                }
            }
            Value::Int(n) => self.call_int_method(n, &method.method_name, arguments, span),
            Value::Float(n) => self.call_float_method(n, &method.method_name, arguments, span),
            Value::Bool(b) => self.call_bool_method(b, &method.method_name, arguments, span),
            Value::Null => self.call_null_method(&method.method_name, arguments, span),
            Value::Decimal(d) => self.call_decimal_method(d, &method.method_name, arguments, span),
            Value::Class(ref class) => match (class.name.as_str(), method.method_name.as_str()) {
                ("Cache", "fetch") => self.cache_fetch(arguments, span),
                _ => Err(RuntimeError::type_error(
                    format!("{}.{}() is not supported", class.name, method.method_name),
                    span,
                )),
            },
            Value::Instance(ref inst) => {
                let class_name = inst.borrow().class.name.clone();
                if let Some(matched) =
                    crate::interpreter::builtins::model::habtm::match_habtm_method(
                        &class_name,
                        &method.method_name,
                    )
                {
                    use crate::interpreter::builtins::model::habtm::{
                        habtm_add, habtm_remove, HabtmAction,
                    };
                    let result = match matched.action {
                        HabtmAction::Add => habtm_add(inst, &matched.relation, &arguments),
                        HabtmAction::Remove => habtm_remove(inst, &matched.relation, &arguments),
                    };
                    return result.map_err(|e| RuntimeError::new(e, span));
                }
                self.call_uploader_method(inst.clone(), &method.method_name, arguments, span)
            }
            _ => Err(RuntimeError::type_error(
                format!("{} does not support methods", method.receiver.type_name()),
                span,
            )),
        }
    }

    /// Dispatch an auto-generated uploader method on a model instance:
    /// `attach_<field>(file)` → `attach_upload(self, "<field>", file)`,
    /// `detach_<field>(blob_id?)` → `detach_upload(self, "<field>", blob_id)`,
    /// `<field>_url(blob_id?)` → `upload_url(self, "<field>", blob_id)`.
    /// The Soli helpers live in the app's `support.sl`; this function looks
    /// them up in the global environment and forwards through `call_value`.
    fn call_uploader_method(
        &mut self,
        inst: Rc<RefCell<crate::interpreter::value::Instance>>,
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        use crate::interpreter::executor::access::member::{
            parse_uploader_method_name, UploaderMethod,
        };

        let (kind, field) = parse_uploader_method_name(method_name).ok_or_else(|| {
            RuntimeError::type_error(
                format!("'{}' is not a recognized uploader method", method_name),
                span,
            )
        })?;

        let helper_name = match kind {
            UploaderMethod::Attach => "attach_upload",
            UploaderMethod::Detach => "detach_upload",
            UploaderMethod::Url | UploaderMethod::Urls => "upload_url",
        };

        let helper = self.environment.borrow().get(helper_name).ok_or_else(|| {
            RuntimeError::type_error(
                format!(
                    "uploader method '{}' requires the '{}' helper to be defined \
                     (see app/controllers/support.sl)",
                    method_name, helper_name
                ),
                span,
            )
        })?;

        let mut forwarded: Vec<Value> = Vec::with_capacity(arguments.len() + 2);
        forwarded.push(Value::Instance(inst));
        forwarded.push(Value::String(field.to_string().into()));
        match kind {
            UploaderMethod::Attach => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                forwarded.extend(arguments);
            }
            UploaderMethod::Detach | UploaderMethod::Url | UploaderMethod::Urls => {
                if arguments.len() > 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                if let Some(arg) = arguments.into_iter().next() {
                    forwarded.push(arg);
                } else {
                    forwarded.push(Value::Null);
                }
            }
        }

        self.call_value(helper, forwarded, span)
    }

    /// Array methods that invoke a user-supplied closure while iterating.
    /// These must operate on a SNAPSHOT of the array: the closure can
    /// re-entrantly mutate the receiver (`arr.push(...)` inside `map`),
    /// which would panic on a RefCell double-borrow under a live borrow.
    /// Everything else in `call_array_method` is pure Rust and is invoked
    /// on the live borrow, skipping the O(n) snapshot clone. Keep this
    /// list in sync with `call_array_method`: when adding a new
    /// closure-taking method there, add it here too — omitting one is a
    /// runtime panic when a closure mutates the receiver, not a perf bug.
    fn array_method_runs_user_code(name: &str) -> bool {
        matches!(
            name,
            "map"
                | "filter"
                | "select"
                | "each"
                | "each_with_index"
                | "reduce"
                | "fold"
                | "find"
                | "any?"
                | "all?"
                | "none?"
                | "one?"
                | "count"
                | "reject"
                | "sort"
                | "sort_by"
        )
    }

    fn call_array_method_borrowed(
        &self,
        items: &[Value],
        method_name: &str,
        arguments: &[Value],
        span: Span,
    ) -> Option<RuntimeResult<Value>> {
        match method_name {
            "reverse" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let mut result = items.to_vec();
                result.reverse();
                Some(Ok(Value::Array(Rc::new(RefCell::new(result)))))
            }
            "uniq" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let mut result = Vec::with_capacity(items.len());
                for item in items {
                    if !result.contains(item) {
                        result.push(item.clone());
                    }
                }
                Some(Ok(Value::Array(Rc::new(RefCell::new(result)))))
            }
            "compact" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let result: Vec<Value> = items
                    .iter()
                    .filter(|v| !matches!(v, Value::Null))
                    .cloned()
                    .collect();
                Some(Ok(Value::Array(Rc::new(RefCell::new(result)))))
            }
            "first" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(items.first().cloned().unwrap_or(Value::Null)))
            }
            "last" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(items.last().cloned().unwrap_or(Value::Null)))
            }
            "empty?" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::Bool(items.is_empty())))
            }
            "includes?" | "include?" | "contains" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                Some(Ok(Value::Bool(items.contains(&arguments[0]))))
            }
            "index_of" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let idx = items
                    .iter()
                    .position(|v| v == &arguments[0])
                    .map(|i| i as i64)
                    .unwrap_or(-1);
                Some(Ok(Value::Int(idx)))
            }
            "get" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let idx = match &arguments[0] {
                    Value::Int(n) => *n,
                    _ => {
                        return Some(Err(RuntimeError::type_error(
                            "get expects an integer index",
                            span,
                        )))
                    }
                };
                let idx_usize = if idx < 0 {
                    (items.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                Some(
                    items
                        .get(idx_usize)
                        .cloned()
                        .ok_or(RuntimeError::IndexOutOfBounds {
                            index: idx,
                            length: items.len(),
                            span,
                        }),
                )
            }
            "length" | "len" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::Int(items.len() as i64)))
            }
            "to_string" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let mut total_len = 2;
                for (i, value) in items.iter().enumerate() {
                    total_len += value.display_len();
                    if i > 0 {
                        total_len += 2;
                    }
                }
                let mut result = String::with_capacity(total_len);
                result.push('[');
                for (i, value) in items.iter().enumerate() {
                    if i > 0 {
                        result.push_str(", ");
                    }
                    value.write_to_string(&mut result);
                }
                result.push(']');
                Some(Ok(Value::String(result.into())))
            }
            "join" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let delim = match &arguments[0] {
                    Value::String(d) => d.as_ref(),
                    _ => {
                        return Some(Err(RuntimeError::type_error(
                            "join expects a string delimiter",
                            span,
                        )))
                    }
                };
                let mut total_len = delim.len().saturating_mul(items.len().saturating_sub(1));
                for value in items {
                    total_len += value.display_len();
                }
                let mut result = String::with_capacity(total_len);
                for (i, value) in items.iter().enumerate() {
                    if i > 0 {
                        result.push_str(delim);
                    }
                    value.write_to_string(&mut result);
                }
                Some(Ok(Value::String(result.into())))
            }
            _ => None,
        }
    }

    fn call_hash_method_borrowed(
        &self,
        entries: &HashPairs,
        method_name: &str,
        arguments: &[Value],
        span: Span,
    ) -> Option<RuntimeResult<Value>> {
        match method_name {
            "get" => {
                if arguments.is_empty() || arguments.len() > 2 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let found = hash_get_value(entries, &arguments[0]).cloned();
                Some(Ok(match found {
                    Some(v) => v,
                    None => arguments.get(1).cloned().unwrap_or(Value::Null),
                }))
            }
            "fetch" => {
                if arguments.is_empty() || arguments.len() > 2 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                if let Some(value) = hash_get_value(entries, &arguments[0]) {
                    Some(Ok(value.clone()))
                } else if let Some(default) = arguments.get(1) {
                    Some(Ok(default.clone()))
                } else {
                    Some(Err(RuntimeError::type_error(
                        format!("key not found: {:?}", arguments[0]),
                        span,
                    )))
                }
            }
            "length" | "len" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::Int(entries.len() as i64)))
            }
            "keys" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let keys: Vec<Value> = entries.keys().map(HashKey::to_value).collect();
                Some(Ok(Value::Array(Rc::new(RefCell::new(keys)))))
            }
            "values" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let values: Vec<Value> = entries.values().cloned().collect();
                Some(Ok(Value::Array(Rc::new(RefCell::new(values)))))
            }
            "entries" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let pairs: Vec<Value> = entries
                    .iter()
                    .map(|(k, v)| {
                        Value::Array(Rc::new(RefCell::new(vec![k.to_value(), v.clone()])))
                    })
                    .collect();
                Some(Ok(Value::Array(Rc::new(RefCell::new(pairs)))))
            }
            "merge" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                match &arguments[0] {
                    Value::Hash(other) => {
                        let mut merged = entries.clone();
                        for (k, v) in other.borrow().iter() {
                            merged.insert(k.clone(), v.clone());
                        }
                        Some(Ok(Value::Hash(Rc::new(RefCell::new(merged)))))
                    }
                    _ => Some(Err(RuntimeError::type_error(
                        "merge expects a hash argument",
                        span,
                    ))),
                }
            }
            "compact" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let mut compacted = HashPairs::with_capacity_and_hasher(
                    entries.len(),
                    ahash::RandomState::default(),
                );
                for (k, v) in entries.iter() {
                    if !matches!(v, Value::Null) {
                        compacted.insert(k.clone(), v.clone());
                    }
                }
                Some(Ok(Value::Hash(Rc::new(RefCell::new(compacted)))))
            }
            "invert" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let mut inverted = HashPairs::with_capacity_and_hasher(
                    entries.len(),
                    ahash::RandomState::default(),
                );
                for (k, v) in entries.iter() {
                    let new_key = match v.to_hash_key() {
                        Some(key) => key,
                        None => {
                            return Some(Err(RuntimeError::type_error(
                                format!("{} cannot be used as a hash key", v.type_name()),
                                span,
                            )))
                        }
                    };
                    inverted.insert(new_key, k.to_value());
                }
                Some(Ok(Value::Hash(Rc::new(RefCell::new(inverted)))))
            }
            "has_key" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                Some(Ok(Value::Bool(hash_contains_value(entries, &arguments[0]))))
            }
            "empty?" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(Ok(Value::Bool(entries.is_empty())))
            }
            "to_string" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let mut total_len = 2;
                for (i, (k, v)) in entries.iter().enumerate() {
                    total_len += k.display_len();
                    total_len += 4 + v.display_len();
                    if i > 0 {
                        total_len += 2;
                    }
                }
                let mut result = String::with_capacity(total_len);
                result.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        result.push_str(", ");
                    }
                    k.write_key_to_string(&mut result);
                    result.push_str(" => ");
                    v.write_to_string(&mut result);
                }
                result.push('}');
                Some(Ok(Value::String(result.into())))
            }
            "flatten" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let pairs: Vec<Value> = entries
                    .iter()
                    .map(|(k, v)| {
                        Value::Array(Rc::new(RefCell::new(vec![k.to_value(), v.clone()])))
                    })
                    .collect();
                Some(Ok(Value::Array(Rc::new(RefCell::new(pairs)))))
            }
            "values_at" => {
                if arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let mut values = Vec::with_capacity(arguments.len());
                for arg in arguments {
                    let v = hash_get_value(entries, arg).cloned().unwrap_or(Value::Null);
                    values.push(v);
                }
                Some(Ok(Value::Array(Rc::new(RefCell::new(values)))))
            }
            "key" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let needle = &arguments[0];
                for (k, v) in entries.iter() {
                    if v == needle {
                        return Some(Ok(k.to_value()));
                    }
                }
                Some(Ok(Value::Null))
            }
            "has_value?" | "value?" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let needle = &arguments[0];
                let found = entries.values().any(|v| v == needle);
                Some(Ok(Value::Bool(found)))
            }
            "to_h" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                let mut new_hash = HashPairs::with_capacity_and_hasher(
                    entries.len(),
                    ahash::RandomState::default(),
                );
                for (k, v) in entries.iter() {
                    new_hash.insert(k.clone(), v.clone());
                }
                Some(Ok(Value::Hash(Rc::new(RefCell::new(new_hash)))))
            }
            "update" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                match &arguments[0] {
                    Value::Hash(other) => {
                        let mut merged = entries.clone();
                        for (k, v) in other.borrow().iter() {
                            merged.insert(k.clone(), v.clone());
                        }
                        Some(Ok(Value::Hash(Rc::new(RefCell::new(merged)))))
                    }
                    _ => Some(Err(RuntimeError::type_error(
                        "update expects a hash argument",
                        span,
                    ))),
                }
            }
            "assoc" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let value = hash_get_value(entries, &arguments[0]).cloned();
                match value {
                    Some(v) => Some(Ok(Value::Array(Rc::new(RefCell::new(vec![
                        arguments[0].clone(),
                        v,
                    ]))))),
                    None => Some(Ok(Value::Null)),
                }
            }
            "rassoc" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let needle = &arguments[0];
                for (k, v) in entries.iter() {
                    if v == needle {
                        return Some(Ok(Value::Array(Rc::new(RefCell::new(vec![
                            k.to_value(),
                            v.clone(),
                        ])))));
                    }
                }
                Some(Ok(Value::Null))
            }
            "fetch_values" => {
                if arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let mut values = Vec::with_capacity(arguments.len());
                for arg in arguments {
                    match hash_get_value(entries, arg) {
                        Some(v) => values.push(v.clone()),
                        None => {
                            return Some(Err(RuntimeError::type_error(
                                format!("key not found: {:?}", arg),
                                span,
                            )));
                        }
                    }
                }
                Some(Ok(Value::Array(Rc::new(RefCell::new(values)))))
            }
            "to_json" => {
                if !arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(0, arguments.len(), span)));
                }
                Some(
                    match crate::interpreter::value_stringify::stringify_hash_map_to_string(entries)
                    {
                        Ok(json) => Ok(Value::String(json.into())),
                        Err(e) => Err(RuntimeError::General { message: e, span }),
                    },
                )
            }
            "is_a?" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let class_name = match &arguments[0] {
                    Value::String(s) => s.as_ref(),
                    _ => {
                        return Some(Err(RuntimeError::type_error(
                            "is_a? expects a string argument",
                            span,
                        )))
                    }
                };
                Some(Ok(Value::Bool(
                    class_name == "hash" || class_name == "object",
                )))
            }
            "slice" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let Value::Array(keys) = &arguments[0] else {
                    return Some(Err(RuntimeError::type_error(
                        "slice expects an array of keys",
                        span,
                    )));
                };
                let keys = keys.borrow();
                let mut result =
                    HashPairs::with_capacity_and_hasher(keys.len(), ahash::RandomState::default());
                for key in keys.iter() {
                    let Some(hash_key) = key.to_hash_key() else {
                        return Some(Err(RuntimeError::type_error(
                            format!("{} cannot be used as a hash key", key.type_name()),
                            span,
                        )));
                    };
                    if let Some(v) = hash_get_value(entries, key) {
                        result.insert(hash_key, v.clone());
                    }
                }
                Some(Ok(Value::Hash(Rc::new(RefCell::new(result)))))
            }
            "except" => {
                if arguments.len() != 1 {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                let Value::Array(keys) = &arguments[0] else {
                    return Some(Err(RuntimeError::type_error(
                        "except expects an array of keys",
                        span,
                    )));
                };
                let exclude: std::collections::HashSet<HashKey> = keys
                    .borrow()
                    .iter()
                    .filter_map(|k| k.to_hash_key())
                    .collect();
                let mut result = HashPairs::with_capacity_and_hasher(
                    entries.len(),
                    ahash::RandomState::default(),
                );
                for (k, v) in entries.iter() {
                    if !exclude.contains(k) {
                        result.insert(k.clone(), v.clone());
                    }
                }
                Some(Ok(Value::Hash(Rc::new(RefCell::new(result)))))
            }
            "dig" => {
                if arguments.is_empty() {
                    return Some(Err(RuntimeError::wrong_arity(1, arguments.len(), span)));
                }
                // First level: look up directly in the borrowed map (no clone of
                // the whole hash). Subsequent levels descend into nested Hash/Array
                // values, which are independent Rc<RefCell<_>> and borrowed lazily.
                let mut current = hash_get_value(entries, &arguments[0]).cloned();
                for key in &arguments[1..] {
                    current = match current.take() {
                        Some(Value::Hash(hash)) => hash_get_value(&hash.borrow(), key).cloned(),
                        Some(Value::Array(arr)) => {
                            if let Value::Int(idx) = key {
                                let arr_ref = arr.borrow();
                                let idx = if *idx < 0 {
                                    arr_ref.len() as i64 + idx
                                } else {
                                    *idx
                                };
                                usize::try_from(idx)
                                    .ok()
                                    .and_then(|i| arr_ref.get(i).cloned())
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    if current.is_none() {
                        return Some(Ok(Value::Null));
                    }
                }
                Some(Ok(current.unwrap_or(Value::Null)))
            }
            _ => None,
        }
    }

    /// Handle array methods.
    pub(crate) fn call_array_method(
        &mut self,
        items: &[Value],
        method_name: &str,
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        match method_name {
            "map" => self.array_map(items, arguments, span),
            "filter" | "select" => self.array_filter(items, arguments, span),
            "each" => self.array_each(items, arguments, span),
            "each_with_index" => self.array_each_with_index(items, arguments, span),
            "index_of" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let idx = items
                    .iter()
                    .position(|v| v == &arguments[0])
                    .map(|i| i as i64)
                    .unwrap_or(-1);
                Ok(Value::Int(idx))
            }
            "reduce" | "fold" => self.array_reduce(items, arguments, span),
            "find" => self.array_find(items, arguments, span),
            "any?" => self.array_any(items, arguments, span),
            "all?" => self.array_all(items, arguments, span),
            "sort" => self.array_sort(items, arguments, span),
            "sort_by" => self.array_sort_by(items, arguments, span),
            "reverse" => self.array_reverse(items, arguments, span),
            "uniq" => self.array_uniq(items, arguments, span),
            "intersection" => self.array_intersection(items, arguments, span),
            "union" => self.array_union(items, arguments, span),
            "difference" => self.array_difference(items, arguments, span),
            "compact" => self.array_compact(items, arguments, span),
            "compact_blank" => self.array_compact_blank(items, arguments, span),
            "flatten" => self.array_flatten(items, arguments, span),
            "first" => self.array_first(items, arguments, span),
            "last" => self.array_last(items, arguments, span),
            "empty?" => self.array_empty(items, arguments, span),
            "includes?" | "include?" | "contains" => self.array_include(items, arguments, span),
            "sample" => self.array_sample(items, arguments, span),
            "shuffle" => self.array_shuffle(items, arguments, span),
            "take" => self.array_take(items, arguments, span),
            "drop" => self.array_drop(items, arguments, span),
            "slice" => self.array_slice(items, arguments, span),
            "zip" => self.array_zip(items, arguments, span),
            "sum" => self.array_sum(items, arguments, span),
            "min" => self.array_min(items, arguments, span),
            "max" => self.array_max(items, arguments, span),
            "push" => self.array_push(items, arguments, span),
            "pop" => self.array_pop(items, arguments, span),
            "clear" => self.array_clear(items, arguments, span),
            "delete" => self.array_delete(items, arguments, span),
            "delete_at" => self.array_delete_at(items, arguments, span),
            "shift" => self.array_shift(items, arguments, span),
            "unshift" => self.array_unshift(items, arguments, span),
            "insert" => self.array_insert(items, arguments, span),
            "rotate" => self.array_rotate(items, arguments, span),
            "reject" => self.array_reject(items, arguments, span),
            "none?" => self.array_none(items, arguments, span),
            "one?" => self.array_one(items, arguments, span),
            "values_at" => self.array_values_at(items, arguments, span),
            "count" => self.array_count(items, arguments, span),
            "get" => self.array_get(items, arguments, span),
            "dig" => self.array_dig(items, arguments, span),
            "pluck" => Self::array_pluck(items, arguments, span),
            "pick" => Self::array_pick(items, arguments, span),
            "length" | "len" | "size" => self.array_length(items, arguments, span),
            "to_string" => self.array_to_string(items, arguments, span),
            "to_json" => match crate::interpreter::value::stringify_array_to_string(items) {
                Ok(json) => Ok(Value::String(json.into())),
                Err(e) => Err(RuntimeError::General { message: e, span }),
            },
            "join" => self.array_join(items, arguments, span),
            // ActiveRecord-style chainable query methods that also work on a
            // materialized array (has_many/has_one accessors return arrays).
            // Controllers written with Rails habits do
            // `org.contacts.order(...).all()`, which should succeed on the
            // preloaded array rather than error.
            "all" => Ok(Value::Array(Rc::new(RefCell::new(items.to_vec())))),
            "includes" => Ok(Value::Array(Rc::new(RefCell::new(items.to_vec())))),
            "order" => {
                if arguments.is_empty() {
                    return Ok(Value::Array(Rc::new(RefCell::new(items.to_vec()))));
                }
                let field = match &arguments[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "order() expects a string field name",
                            span,
                        ))
                    }
                };
                let ascending = match arguments.get(1) {
                    Some(Value::String(d)) => {
                        let d = d.to_lowercase();
                        !(d == "desc" || d == "descending")
                    }
                    _ => true,
                };
                let mut sorted = items.to_vec();
                sorted.sort_by(|a, b| {
                    let av = extract_field_for_sort(a, &field);
                    let bv = extract_field_for_sort(b, &field);
                    let ord = cmp_sort_values(&av, &bv);
                    if ascending {
                        ord
                    } else {
                        ord.reverse()
                    }
                });
                Ok(Value::Array(Rc::new(RefCell::new(sorted))))
            }
            "is_a?" => {
                if arguments.len() != 1 {
                    return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
                }
                let class_name = match &arguments[0] {
                    Value::String(s) => s.as_ref(),
                    _ => {
                        return Err(RuntimeError::type_error(
                            "is_a? expects a string argument",
                            span,
                        ))
                    }
                };
                Ok(Value::Bool(class_name == "array" || class_name == "object"))
            }
            _ => Err(RuntimeError::NoSuchProperty {
                value_type: "Array".to_string(),
                property: method_name.to_string(),
                span,
            }),
        }
    }

    fn array_map(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "map expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());

        let call_env = Environment::with_enclosing(func.closure.clone());
        let call_env_rc = Rc::new(RefCell::new(call_env));
        call_env_rc
            .borrow_mut()
            .define(param_name.clone(), Value::Null);

        let mut result = Vec::with_capacity(items.len());
        for item in items {
            call_env_rc
                .borrow_mut()
                .define_or_update(&param_name, item.clone());

            match self.execute_block_in(&func.body, call_env_rc.clone())? {
                ControlFlow::Return(v) => result.push(v),
                ControlFlow::Normal(v) => result.push(v),
                ControlFlow::Continue => {}
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array method", span));
                }
            }
        }

        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_filter(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "filter expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());

        let call_env = Environment::with_enclosing(func.closure.clone());
        let call_env_rc = Rc::new(RefCell::new(call_env));
        call_env_rc
            .borrow_mut()
            .define(param_name.clone(), Value::Null);

        let mut result = Vec::new();
        for item in items {
            call_env_rc
                .borrow_mut()
                .define_or_update(&param_name, item.clone());

            let result_value = match self.execute_block_in(&func.body, call_env_rc.clone())? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Continue => Value::Null,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array filter", span));
                }
            };

            if result_value.is_truthy() {
                result.push(item.clone());
            }
        }

        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_each(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "each expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());

        let call_env = Environment::with_enclosing(func.closure.clone());
        let call_env_rc = Rc::new(RefCell::new(call_env));
        call_env_rc
            .borrow_mut()
            .define(param_name.clone(), Value::Null);

        for item in items {
            call_env_rc
                .borrow_mut()
                .define_or_update(&param_name, item.clone());

            match self.execute_block_in(&func.body, call_env_rc.clone())? {
                ControlFlow::Return(_) | ControlFlow::Normal(_) | ControlFlow::Continue => {}
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array each", span));
                }
            }
        }

        Ok(Value::Array(Rc::new(RefCell::new(items.to_vec()))))
    }

    fn array_each_with_index(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "each_with_index expects a function argument",
                    span,
                ))
            }
        };

        let call_env = Environment::with_enclosing(func.closure.clone());
        let call_env_rc = Rc::new(RefCell::new(call_env));
        let param0_name = func.params.first().map(|p| p.name.clone());
        let param1_name = func.params.get(1).map(|p| p.name.clone());
        if let Some(ref n) = param0_name {
            call_env_rc.borrow_mut().define(n.clone(), Value::Null);
        }
        if let Some(ref n) = param1_name {
            call_env_rc.borrow_mut().define(n.clone(), Value::Null);
        }

        for (i, item) in items.iter().enumerate() {
            match (&param0_name, &param1_name) {
                (Some(n0), Some(n1)) => {
                    let mut env = call_env_rc.borrow_mut();
                    env.define_or_update(n0, item.clone());
                    env.define_or_update(n1, Value::Int(i as i64));
                }
                (Some(n0), None) => {
                    call_env_rc.borrow_mut().define_or_update(n0, item.clone());
                }
                _ => {}
            }

            match self.execute_block_in(&func.body, call_env_rc.clone())? {
                ControlFlow::Return(_) | ControlFlow::Normal(_) | ControlFlow::Continue => {}
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new(
                        "Exception in array each_with_index",
                        span,
                    ));
                }
            }
        }

        Ok(Value::Array(Rc::new(RefCell::new(items.to_vec()))))
    }

    fn array_reduce(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() || arguments.len() > 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "reduce expects a function argument",
                    span,
                ))
            }
        };

        let mut acc = if arguments.len() == 2 {
            arguments[1].clone()
        } else if !items.is_empty() {
            items[0].clone()
        } else {
            return Err(RuntimeError::type_error(
                "reduce on empty array requires initial value",
                span,
            ));
        };

        let start_idx = if arguments.len() == 2 { 0 } else { 1 };

        let call_env = Environment::with_enclosing(func.closure.clone());
        let call_env_rc = Rc::new(RefCell::new(call_env));
        let param0_name = func.params.first().map(|p| p.name.clone());
        let param1_name = func.params.get(1).map(|p| p.name.clone());
        if let Some(ref n) = param0_name {
            call_env_rc.borrow_mut().define(n.clone(), Value::Null);
        }
        if let Some(ref n) = param1_name {
            call_env_rc.borrow_mut().define(n.clone(), Value::Null);
        }

        for item in items.iter().skip(start_idx) {
            match (&param0_name, &param1_name) {
                (Some(n0), Some(n1)) => {
                    let mut env = call_env_rc.borrow_mut();
                    env.define_or_update(n0, acc.clone());
                    env.define_or_update(n1, item.clone());
                }
                (Some(n0), None) => {
                    let pair = Value::Array(Rc::new(RefCell::new(vec![acc.clone(), item.clone()])));
                    call_env_rc.borrow_mut().define_or_update(n0, pair);
                }
                _ => {}
            }

            acc = match self.execute_block_in(&func.body, call_env_rc.clone())? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Continue => Value::Null,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array reduce", span));
                }
            };
        }

        Ok(acc)
    }

    fn array_find(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "find expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());
        let call_env_rc = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        call_env_rc
            .borrow_mut()
            .define(param_name.clone(), Value::Null);

        for item in items {
            call_env_rc
                .borrow_mut()
                .define_or_update(&param_name, item.clone());

            let result_value = match self.execute_block_in(&func.body, call_env_rc.clone())? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Continue => Value::Null,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array find", span));
                }
            };

            if result_value.is_truthy() {
                return Ok(item.clone());
            }
        }

        Ok(Value::Null)
    }

    fn array_any(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "any? expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());
        let call_env_rc = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        call_env_rc
            .borrow_mut()
            .define(param_name.clone(), Value::Null);

        for item in items {
            call_env_rc
                .borrow_mut()
                .define_or_update(&param_name, item.clone());

            let result_value = match self.execute_block_in(&func.body, call_env_rc.clone())? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Continue => Value::Null,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array any?", span));
                }
            };

            if result_value.is_truthy() {
                return Ok(Value::Bool(true));
            }
        }

        Ok(Value::Bool(false))
    }

    fn array_all(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "all? expects a function argument",
                    span,
                ))
            }
        };

        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());
        let call_env_rc = Rc::new(RefCell::new(Environment::with_enclosing(
            func.closure.clone(),
        )));
        call_env_rc
            .borrow_mut()
            .define(param_name.clone(), Value::Null);

        for item in items {
            call_env_rc
                .borrow_mut()
                .define_or_update(&param_name, item.clone());

            let result_value = match self.execute_block_in(&func.body, call_env_rc.clone())? {
                ControlFlow::Return(v) => v,
                ControlFlow::Normal(v) => v,
                ControlFlow::Continue => Value::Null,
                ControlFlow::Throw(_) => {
                    return Err(RuntimeError::new("Exception in array all?", span));
                }
            };

            if !result_value.is_truthy() {
                return Ok(Value::Bool(false));
            }
        }

        Ok(Value::Bool(true))
    }

    fn array_sort(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        let mut result = items.to_vec();
        if arguments.len() > 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }

        if let Some(func_val) = arguments.first() {
            let func = match func_val {
                Value::Function(f) => f.clone(),
                _ => {
                    return Err(RuntimeError::type_error(
                        "sort expects a function argument",
                        span,
                    ))
                }
            };

            result.sort_by(|a, b| {
                let mut call_env = Environment::with_enclosing(func.closure.clone());

                if func.params.len() >= 2 {
                    call_env.define(func.params[0].name.clone(), a.clone());
                    call_env.define(func.params[1].name.clone(), b.clone());
                }

                match self.execute_block(&func.body, call_env) {
                    Ok(ControlFlow::Return(Value::Int(n)))
                    | Ok(ControlFlow::Normal(Value::Int(n))) => n.cmp(&0),
                    Ok(ControlFlow::Return(Value::Float(n)))
                    | Ok(ControlFlow::Normal(Value::Float(n))) => {
                        if n < 0.0 {
                            std::cmp::Ordering::Less
                        } else if n > 0.0 {
                            std::cmp::Ordering::Greater
                        } else {
                            std::cmp::Ordering::Equal
                        }
                    }
                    _ => std::cmp::Ordering::Equal,
                }
            });
        } else {
            result.sort_by(|a, b| match (a, b) {
                (Value::Int(a), Value::Int(b)) => a.cmp(b),
                (Value::Float(a), Value::Float(b)) => {
                    a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                }
                (Value::String(a), Value::String(b)) => a.cmp(b),
                (Value::Int(a), Value::Float(b)) => (*a as f64)
                    .partial_cmp(b)
                    .unwrap_or(std::cmp::Ordering::Equal),
                (Value::Float(a), Value::Int(b)) => a
                    .partial_cmp(&(*b as f64))
                    .unwrap_or(std::cmp::Ordering::Equal),
                _ => std::cmp::Ordering::Equal,
            });
        }

        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_sort_by(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }

        let mut result = items.to_vec();

        match &arguments[0] {
            Value::String(key) => {
                let hash_key = HashKey::String(key.clone());
                result.sort_by(|a, b| {
                    let val_a = Self::extract_hash_value(a, &hash_key);
                    let val_b = Self::extract_hash_value(b, &hash_key);
                    Self::compare_sort_values(&val_a, &val_b)
                });
            }
            Value::Function(func) => {
                let func = func.clone();
                let param_name = func
                    .params
                    .first()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "it".to_string());

                // Extract key values for each item using the function.
                // Reuse a single lambda env across iterations to avoid re-allocating
                // HashMaps and the parameter's String per item.
                let call_env_rc = Rc::new(RefCell::new(Environment::with_enclosing(
                    func.closure.clone(),
                )));
                call_env_rc
                    .borrow_mut()
                    .define(param_name.clone(), Value::Null);

                let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(result.len());
                for item in &result {
                    call_env_rc
                        .borrow_mut()
                        .define_or_update(&param_name, item.clone());

                    let key_val = match self.execute_block_in(&func.body, call_env_rc.clone()) {
                        Ok(ControlFlow::Return(v)) | Ok(ControlFlow::Normal(v)) => v,
                        _ => Value::Null,
                    };
                    keyed.push((item.clone(), key_val));
                }

                keyed.sort_by(|a, b| Self::compare_sort_values(&a.1, &b.1));
                result = keyed.into_iter().map(|(item, _)| item).collect();
            }
            _ => {
                return Err(RuntimeError::type_error(
                    "sort_by expects a string key or a function argument",
                    span,
                ))
            }
        }

        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn extract_hash_value(value: &Value, key: &HashKey) -> Value {
        match value {
            Value::Hash(hash) => hash.borrow().get(key).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }

    fn compare_sort_values(a: &Value, b: &Value) -> std::cmp::Ordering {
        match (a, b) {
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Float(a), Value::Float(b)) => {
                a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
            }
            (Value::String(a), Value::String(b)) => a.cmp(b),
            (Value::Int(a), Value::Float(b)) => (*a as f64)
                .partial_cmp(b)
                .unwrap_or(std::cmp::Ordering::Equal),
            (Value::Float(a), Value::Int(b)) => a
                .partial_cmp(&(*b as f64))
                .unwrap_or(std::cmp::Ordering::Equal),
            _ => std::cmp::Ordering::Equal,
        }
    }

    fn array_reverse(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let mut result = items.to_vec();
        result.reverse();
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_uniq(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let result = super::array_ops::uniq_values(items);
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_intersection(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let other = extract_array_arg(&arguments[0], "intersection", span)?;
        let mut result: Vec<Value> = Vec::new();
        for item in items {
            if other.contains(item) && !result.contains(item) {
                result.push(item.clone());
            }
        }
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_union(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let other = extract_array_arg(&arguments[0], "union", span)?;
        let mut result: Vec<Value> = Vec::new();
        for item in items {
            if !result.contains(item) {
                result.push(item.clone());
            }
        }
        for item in &other {
            if !result.contains(item) {
                result.push(item.clone());
            }
        }
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_difference(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let other = extract_array_arg(&arguments[0], "difference", span)?;
        let mut result: Vec<Value> = Vec::new();
        for item in items {
            if !other.contains(item) && !result.contains(item) {
                result.push(item.clone());
            }
        }
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_compact(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let result = super::array_ops::compact_values(items);
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_compact_blank(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let result: Vec<Value> = items
            .iter()
            .filter(|v| !Self::is_blank(v))
            .cloned()
            .collect();
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn is_blank(value: &Value) -> bool {
        match value {
            Value::Null => true,
            Value::String(s) => s.is_empty(),
            Value::Array(a) => a.borrow().is_empty(),
            Value::Hash(h) => h.borrow().is_empty(),
            _ => false,
        }
    }

    fn array_flatten(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        let depth = match arguments.len() {
            0 => None,
            1 => match &arguments[0] {
                Value::Int(n) if *n >= 0 => Some(*n as usize),
                _ => {
                    return Err(RuntimeError::type_error(
                        "flatten expects a non-negative integer",
                        span,
                    ))
                }
            },
            _ => return Err(RuntimeError::wrong_arity(1, arguments.len(), span)),
        };
        let result = super::array_ops::flatten_values(items, depth);
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_first(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(items.first().cloned().unwrap_or(Value::Null))
    }

    fn array_last(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(items.last().cloned().unwrap_or(Value::Null))
    }

    fn array_empty(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::Bool(items.is_empty()))
    }

    fn array_include(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        Ok(Value::Bool(items.contains(&arguments[0])))
    }

    fn array_sample(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        if items.is_empty() {
            return Ok(Value::Null);
        }
        use rand::seq::SliceRandom;
        use rand::thread_rng;
        let mut rng = thread_rng();
        Ok(items.choose(&mut rng).cloned().unwrap_or(Value::Null))
    }

    fn array_shuffle(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        use rand::seq::SliceRandom;
        use rand::thread_rng;
        let mut result = items.to_vec();
        let mut rng = thread_rng();
        result.shuffle(&mut rng);
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_take(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let n = match &arguments[0] {
            Value::Int(n) if *n >= 0 => *n as usize,
            _ => {
                return Err(RuntimeError::type_error(
                    "take expects a non-negative integer",
                    span,
                ))
            }
        };
        let result: Vec<Value> = items.iter().take(n).cloned().collect();
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_drop(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let n = match &arguments[0] {
            Value::Int(n) if *n >= 0 => *n as usize,
            _ => {
                return Err(RuntimeError::type_error(
                    "drop expects a non-negative integer",
                    span,
                ))
            }
        };
        let result: Vec<Value> = items.iter().skip(n).cloned().collect();
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_slice(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        _span: Span,
    ) -> RuntimeResult<Value> {
        let start = if !arguments.is_empty() {
            match &arguments[0] {
                Value::Int(n) => Some(*n),
                _ => None,
            }
        } else {
            None
        };
        let end = if arguments.len() >= 2 {
            match &arguments[1] {
                Value::Int(n) => Some(*n),
                _ => None,
            }
        } else {
            None
        };
        let len = items.len() as i64;
        let start_idx = match start {
            Some(s) if s < 0 => (len + s).max(0) as usize,
            Some(s) => (s as usize).min(len as usize),
            None => 0,
        };
        let end_idx = match end {
            Some(e) if e < 0 => (len + e).max(0) as usize,
            Some(e) => (e as usize).min(len as usize),
            None => len as usize,
        };
        let result: Vec<Value> = items
            .iter()
            .skip(start_idx)
            .take(end_idx.saturating_sub(start_idx))
            .cloned()
            .collect();
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_zip(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let other = match &arguments[0] {
            Value::Array(arr) => arr.borrow().clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "zip expects an array argument",
                    span,
                ))
            }
        };

        let result: Vec<Value> = items
            .iter()
            .zip(other.iter())
            .map(|(a, b)| Value::Array(Rc::new(RefCell::new(vec![a.clone(), b.clone()]))))
            .collect();
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_sum(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let mut total = 0.0;
        for item in items {
            match item {
                Value::Int(n) => total += *n as f64,
                Value::Float(n) => total += *n,
                Value::Decimal(d) => total += d.to_f64(),
                _ => return Err(RuntimeError::type_error("sum expects numeric array", span)),
            }
        }
        Ok(Value::Float(total))
    }

    fn array_min(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        if items.is_empty() {
            return Ok(Value::Null);
        }
        let mut min = &items[0];
        for item in items.iter().skip(1) {
            match (min, item) {
                (Value::Int(a), Value::Int(b)) if b < a => min = item,
                (Value::Float(a), Value::Float(b)) if b < a => min = item,
                (Value::String(a), Value::String(b)) if b < a => min = item,
                (Value::Int(a), Value::Float(b)) if *b < *a as f64 => min = item,
                (Value::Float(a), Value::Int(b)) if (*b as f64) < *a => min = item,
                (Value::Decimal(a), Value::Decimal(b)) if b.to_f64() < a.to_f64() => min = item,
                (Value::Int(a), Value::Decimal(b)) if b.to_f64() < *a as f64 => min = item,
                (Value::Decimal(a), Value::Int(b)) if (*b as f64) < a.to_f64() => min = item,
                (Value::Float(a), Value::Decimal(b)) if b.to_f64() < *a => min = item,
                (Value::Decimal(a), Value::Float(b)) if *b < a.to_f64() => min = item,
                _ => {}
            }
        }
        Ok(min.clone())
    }

    fn array_max(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        if items.is_empty() {
            return Ok(Value::Null);
        }
        let mut max = &items[0];
        for item in items.iter().skip(1) {
            match (max, item) {
                (Value::Int(a), Value::Int(b)) if b > a => max = item,
                (Value::Float(a), Value::Float(b)) if b > a => max = item,
                (Value::String(a), Value::String(b)) if b > a => max = item,
                (Value::Int(a), Value::Float(b)) if *b > *a as f64 => max = item,
                (Value::Float(a), Value::Int(b)) if (*b as f64) > *a => max = item,
                (Value::Decimal(a), Value::Decimal(b)) if b.to_f64() > a.to_f64() => max = item,
                (Value::Int(a), Value::Decimal(b)) if b.to_f64() > *a as f64 => max = item,
                (Value::Decimal(a), Value::Int(b)) if (*b as f64) > a.to_f64() => max = item,
                (Value::Float(a), Value::Decimal(b)) if b.to_f64() > *a => max = item,
                (Value::Decimal(a), Value::Float(b)) if *b > a.to_f64() => max = item,
                _ => {}
            }
        }
        Ok(max.clone())
    }

    fn array_push(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let mut new_arr = items.to_vec();
        new_arr.push(arguments[0].clone());
        Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
    }

    fn array_pop(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let mut new_arr = items.to_vec();
        new_arr.pop();
        Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
    }

    fn array_clear(
        &mut self,
        _items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))))
    }

    fn array_get(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let idx = match &arguments[0] {
            Value::Int(n) => *n,
            _ => {
                return Err(RuntimeError::type_error(
                    "get expects an integer index",
                    span,
                ))
            }
        };
        let idx_usize = if idx < 0 {
            (items.len() as i64 + idx) as usize
        } else {
            idx as usize
        };
        items
            .get(idx_usize)
            .cloned()
            .ok_or(RuntimeError::IndexOutOfBounds {
                index: idx,
                length: items.len(),
                span,
            })
    }

    fn array_dig(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }

        let mut current: Option<Value> = Some(Value::Array(Rc::new(RefCell::new(items.to_vec()))));
        for key in arguments {
            current = match current.take() {
                Some(Value::Hash(hash)) => {
                    let hash_key = key.to_hash_key();
                    if let Some(hash_key) = hash_key {
                        let hash_ref = hash.borrow();
                        hash_ref.get(&hash_key).cloned()
                    } else {
                        None
                    }
                }
                Some(Value::Array(arr)) => {
                    if let Value::Int(idx) = key {
                        let arr_ref = arr.borrow();
                        let idx = if idx < 0 {
                            arr_ref.len() as i64 + idx
                        } else {
                            idx
                        };
                        let idx = idx as usize;
                        if idx < arr_ref.len() {
                            Some(arr_ref[idx].clone())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if current.is_none() {
                return Ok(Value::Null);
            }
        }
        Ok(current.unwrap_or(Value::Null))
    }

    fn array_pluck(items: &[Value], arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.is_empty() {
            return Err(RuntimeError::new(
                "pluck() requires at least one field name or index",
                span,
            ));
        }

        let mut result = Vec::with_capacity(items.len());

        for item in items {
            if arguments.len() == 1 {
                let v = Self::extract_pluck_field(item, &arguments[0]);
                result.push(v);
            } else {
                let row: Vec<Value> = arguments
                    .iter()
                    .map(|k| Self::extract_pluck_field(item, k))
                    .collect();
                result.push(Value::Array(Rc::new(RefCell::new(row))));
            }
        }

        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_pick(items: &[Value], arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        if arguments.is_empty() {
            return Err(RuntimeError::new(
                "pick() requires at least one field name or index",
                span,
            ));
        }

        if items.is_empty() {
            return Ok(Value::Null);
        }

        let first = &items[0];

        if arguments.len() == 1 {
            return Ok(Self::extract_pluck_field(first, &arguments[0]));
        }

        let row: Vec<Value> = arguments
            .iter()
            .map(|k| Self::extract_pluck_field(first, k))
            .collect();

        Ok(Value::Array(Rc::new(RefCell::new(row))))
    }

    fn extract_pluck_field(value: &Value, key: &Value) -> Value {
        match (value, key) {
            (Value::Hash(h), Value::String(s)) => {
                let hk = HashKey::String(s.clone());
                h.borrow().get(&hk).cloned().unwrap_or(Value::Null)
            }
            (Value::Array(a), Value::Int(i)) => {
                let arr = a.borrow();
                let idx = if *i < 0 {
                    (arr.len() as i64 + *i) as usize
                } else {
                    *i as usize
                };
                if idx < arr.len() {
                    arr[idx].clone()
                } else {
                    Value::Null
                }
            }
            _ => Value::Null,
        }
    }

    fn array_length(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        Ok(Value::Int(items.len() as i64))
    }

    fn array_to_string(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        let parts: Vec<String> = items.iter().map(|v| format!("{}", v)).collect();
        Ok(Value::String(format!("[{}]", parts.join(", ")).into()))
    }

    fn array_join(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let delim = match &arguments[0] {
            Value::String(d) => d.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "join expects a string delimiter",
                    span,
                ))
            }
        };
        let parts: Vec<String> = items.iter().map(|v| format!("{}", v)).collect();
        Ok(Value::String(parts.join(&delim).into()))
    }

    fn array_delete(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let val = &arguments[0];
        if items.contains(val) {
            let new_arr: Vec<Value> = items.iter().filter(|v| *v != val).cloned().collect();
            Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
        } else {
            Ok(Value::Null)
        }
    }

    fn array_delete_at(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let idx = match &arguments[0] {
            Value::Int(i) => {
                if *i >= 0 {
                    *i as usize
                } else {
                    items.len().saturating_sub((-*i) as usize)
                }
            }
            _ => {
                return Err(RuntimeError::type_error(
                    "delete_at expects an integer index",
                    span,
                ))
            }
        };
        if idx < items.len() {
            let _removed = items[idx].clone();
            let mut new_arr = items.to_vec();
            new_arr.remove(idx);
            Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
        } else {
            Ok(Value::Null)
        }
    }

    fn array_shift(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if !arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(0, arguments.len(), span));
        }
        if items.is_empty() {
            return Ok(Value::Null);
        }
        let mut new_arr = items.to_vec();
        new_arr.remove(0);
        Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
    }

    fn array_unshift(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        _span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() {
            return Ok(Value::Array(Rc::new(RefCell::new(items.to_vec()))));
        }
        let mut new_arr = arguments.clone();
        new_arr.extend(items.iter().cloned());
        Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
    }

    fn array_insert(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() < 2 {
            return Err(RuntimeError::wrong_arity(2, arguments.len(), span));
        }
        let idx = match &arguments[0] {
            Value::Int(i) => {
                if *i >= 0 {
                    *i as usize
                } else {
                    items.len().saturating_sub((-*i) as usize)
                }
            }
            _ => {
                return Err(RuntimeError::type_error(
                    "insert expects an integer index",
                    span,
                ))
            }
        };
        let mut new_arr = items.to_vec();
        let vals = &arguments[1..];
        let insert_at = idx.min(new_arr.len());
        let mut tail = new_arr.split_off(insert_at);
        new_arr.extend(vals.iter().cloned());
        new_arr.append(&mut tail);
        Ok(Value::Array(Rc::new(RefCell::new(new_arr))))
    }

    fn array_rotate(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() > 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let count = match arguments.first() {
            Some(Value::Int(n)) => *n,
            None => 1,
            _ => return Err(RuntimeError::type_error("rotate expects an integer", span)),
        };
        if items.is_empty() {
            return Ok(Value::Array(Rc::new(RefCell::new(Vec::new()))));
        }
        let len = items.len() as i64;
        let normalized = ((count % len) + len) % len;
        let split_at = normalized as usize;
        let rotated: Vec<Value> = items[split_at..]
            .iter()
            .chain(items[..split_at].iter())
            .cloned()
            .collect();
        Ok(Value::Array(Rc::new(RefCell::new(rotated))))
    }

    fn array_reject(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => return Err(RuntimeError::type_error("reject expects a function", span)),
        };
        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());
        let call_env = Environment::with_enclosing(func.closure.clone());
        let call_env_rc = Rc::new(RefCell::new(call_env));
        call_env_rc
            .borrow_mut()
            .define(param_name.clone(), Value::Null);
        let mut result = Vec::new();
        for item in items {
            call_env_rc
                .borrow_mut()
                .define_or_update(&param_name, item.clone());
            let val = match self.execute_block_in(&func.body, call_env_rc.clone())? {
                ControlFlow::Return(v) | ControlFlow::Normal(v) => v,
                _ => Value::Null,
            };
            if !val.is_truthy() {
                result.push(item.clone());
            }
        }
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_none(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => return Err(RuntimeError::type_error("none? expects a function", span)),
        };
        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());
        let call_env = Environment::with_enclosing(func.closure.clone());
        let call_env_rc = Rc::new(RefCell::new(call_env));
        call_env_rc
            .borrow_mut()
            .define(param_name.clone(), Value::Null);
        for item in items {
            call_env_rc
                .borrow_mut()
                .define_or_update(&param_name, item.clone());
            let val = match self.execute_block_in(&func.body, call_env_rc.clone())? {
                ControlFlow::Return(v) | ControlFlow::Normal(v) => v,
                _ => Value::Null,
            };
            if val.is_truthy() {
                return Ok(Value::Bool(false));
            }
        }
        Ok(Value::Bool(true))
    }

    fn array_one(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.len() != 1 {
            return Err(RuntimeError::wrong_arity(1, arguments.len(), span));
        }
        let func = match &arguments[0] {
            Value::Function(f) => f.clone(),
            _ => return Err(RuntimeError::type_error("one? expects a function", span)),
        };
        let param_name = func
            .params
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "it".to_string());
        let call_env = Environment::with_enclosing(func.closure.clone());
        let call_env_rc = Rc::new(RefCell::new(call_env));
        call_env_rc
            .borrow_mut()
            .define(param_name.clone(), Value::Null);
        let mut found = false;
        for item in items {
            call_env_rc
                .borrow_mut()
                .define_or_update(&param_name, item.clone());
            let val = match self.execute_block_in(&func.body, call_env_rc.clone())? {
                ControlFlow::Return(v) | ControlFlow::Normal(v) => v,
                _ => Value::Null,
            };
            if val.is_truthy() {
                if found {
                    return Ok(Value::Bool(false));
                }
                found = true;
            }
        }
        Ok(Value::Bool(found))
    }

    fn array_values_at(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        _span: Span,
    ) -> RuntimeResult<Value> {
        let mut result = Vec::new();
        for arg in &arguments {
            match arg {
                Value::Int(i) => {
                    let idx = if *i >= 0 {
                        *i as usize
                    } else {
                        items.len().saturating_sub((-*i) as usize)
                    };
                    if idx < items.len() {
                        result.push(items[idx].clone());
                    } else {
                        result.push(Value::Null);
                    }
                }
                Value::Array(indices) => {
                    for i in indices.borrow().iter() {
                        if let Value::Int(n) = i {
                            let idx = if *n >= 0 {
                                *n as usize
                            } else {
                                items.len().saturating_sub((-*n) as usize)
                            };
                            if idx < items.len() {
                                result.push(items[idx].clone());
                            } else {
                                result.push(Value::Null);
                            }
                        }
                    }
                }
                _ => {
                    result.push(Value::Null);
                }
            }
        }
        Ok(Value::Array(Rc::new(RefCell::new(result))))
    }

    fn array_count(
        &mut self,
        items: &[Value],
        arguments: Vec<Value>,
        span: Span,
    ) -> RuntimeResult<Value> {
        if arguments.is_empty() {
            return Ok(Value::Int(items.len() as i64));
        }
        if arguments.len() == 1 {
            if let Value::Function(func) = &arguments[0] {
                let func = func.clone();
                let param_name = func
                    .params
                    .first()
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| "it".to_string());
                let call_env = Environment::with_enclosing(func.closure.clone());
                let call_env_rc = Rc::new(RefCell::new(call_env));
                call_env_rc
                    .borrow_mut()
                    .define(param_name.clone(), Value::Null);
                let mut count = 0i64;
                for item in items {
                    call_env_rc
                        .borrow_mut()
                        .define_or_update(&param_name, item.clone());
                    let val = match self.execute_block_in(&func.body, call_env_rc.clone())? {
                        ControlFlow::Return(v) | ControlFlow::Normal(v) => v,
                        _ => Value::Null,
                    };
                    if val.is_truthy() {
                        count += 1;
                    }
                }
                return Ok(Value::Int(count));
            }
            let c = items.iter().filter(|v| *v == &arguments[0]).count() as i64;
            return Ok(Value::Int(c));
        }
        Err(RuntimeError::wrong_arity(1, arguments.len(), span))
    }

    fn cache_fetch(&mut self, arguments: Vec<Value>, span: Span) -> RuntimeResult<Value> {
        use crate::interpreter::builtins::cache::{cache_get_impl, cache_set_impl};

        if arguments.is_empty() {
            return Err(RuntimeError::wrong_arity(1, 0, span));
        }
        let key = match &arguments[0] {
            Value::String(s) => s.clone(),
            _ => {
                return Err(RuntimeError::type_error(
                    "Cache.fetch() expects string key",
                    span,
                ))
            }
        };

        // Parse optional TTL and block from remaining args
        let mut ttl: Option<u64> = None;
        let mut block = None;
        for arg in arguments.iter().skip(1) {
            match arg {
                Value::Int(i) => ttl = Some(*i as u64),
                Value::Function(f) => block = Some(f.clone()),
                _ => {}
            }
        }

        // Check cache
        let cached =
            cache_get_impl(&key).map_err(|e| RuntimeError::General { message: e, span })?;
        if !matches!(cached, Value::Null) {
            return Ok(cached);
        }

        // Cache miss — no block means return null
        let func = match block {
            Some(f) => f,
            None => return Ok(Value::Null),
        };

        // Execute block
        let call_env = Environment::with_enclosing(func.closure.clone());
        let result = match self.execute_block(&func.body, call_env)? {
            ControlFlow::Return(v) | ControlFlow::Normal(v) => v,
            ControlFlow::Continue => Value::Null,
            ControlFlow::Throw(_) => {
                return Err(RuntimeError::new("Exception in Cache.fetch block", span))
            }
        };

        // Store in cache
        cache_set_impl(&key, &result, ttl)
            .map_err(|e| RuntimeError::General { message: e, span })?;

        Ok(result)
    }
}

fn extract_array_arg(
    value: &crate::interpreter::value::Value,
    method_name: &str,
    span: Span,
) -> RuntimeResult<Vec<crate::interpreter::value::Value>> {
    use crate::interpreter::value::Value;
    match value {
        Value::Array(arr) => Ok(arr.borrow().clone()),
        Value::Instance(inst) => match inst.borrow().fields.get("__value").cloned() {
            Some(Value::Array(arr)) => Ok(arr.borrow().clone()),
            _ => Err(RuntimeError::type_error(
                format!("Array.{}() argument must be an Array", method_name),
                span,
            )),
        },
        _ => Err(RuntimeError::type_error(
            format!("Array.{}() argument must be an Array", method_name),
            span,
        )),
    }
}

fn extract_field_for_sort(
    v: &crate::interpreter::value::Value,
    field: &str,
) -> crate::interpreter::value::Value {
    use crate::interpreter::value::{HashKey, Value};
    match v {
        Value::Instance(inst) => inst.borrow().get(field).unwrap_or(Value::Null),
        Value::Hash(h) => h
            .borrow()
            .get(&HashKey::String(field.to_string().into()))
            .cloned()
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn cmp_sort_values(
    a: &crate::interpreter::value::Value,
    b: &crate::interpreter::value::Value,
) -> std::cmp::Ordering {
    use crate::interpreter::value::Value;
    use std::cmp::Ordering;
    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        (Value::String(a), Value::String(b)) => a.cmp(b),
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        (Value::Float(a), Value::Float(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}
