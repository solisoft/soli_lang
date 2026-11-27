//! Queries on translated fields (`translate "title"`): compare or sort by the
//! value a reader in the current locale sees — the translation along the
//! locale's regional chain (`fr-CA`, its declared fallbacks, `fr`), then the
//! column — rather than by the column alone.
//!
//! SoliDB only: the filter is SDBQL (`??`, `doc.translated_fields.title[…]`),
//! which the SQL adapters refuse like any raw string `where`.

use std::collections::HashMap;

use crate::interpreter::builtins::i18n::helpers::{get_locale, regional_chain};
use crate::interpreter::value::{HashKey, HashPairs, Value};

/// A field name safe to write into SDBQL: an identifier.
fn checked_field(field: &str, method: &str) -> Result<(), String> {
    let mut chars = field.chars();
    let starts_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if starts_ok && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(())
    } else {
        Err(format!("{method}: \"{field}\" is not a field name"))
    }
}

/// The locales of the current request's regional chain, each safe to write
/// inside a quoted SDBQL attribute name.
fn chain_locales() -> Vec<String> {
    regional_chain(&get_locale())
        .into_iter()
        .filter(|l| {
            l.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .collect()
}

/// `doc.translated_fields.title["fr-CA"] ?? doc.translated_fields.title["fr"]
/// ?? doc.title` for a translated field, `doc.title` for any other.
fn reader_expression(class_name: &str, field: &str) -> String {
    if !super::is_translated_field(class_name, field) {
        return format!("doc.{field}");
    }
    let mut parts: Vec<String> = chain_locales()
        .iter()
        .map(|l| format!("doc.translated_fields.{field}[\"{l}\"]"))
        .collect();
    parts.push(format!("doc.{field}"));
    format!("({})", parts.join(" ?? "))
}

/// `where_translated({"title": "Bonjour", "status": "draft"})` as a string
/// filter and its binds: each translated field compared as a reader in the
/// current locale sees it, any other field as `where` would. `namespace`
/// keeps the bind names apart from those already on a builder.
pub fn translated_filter(
    class_name: &str,
    conditions: &HashPairs,
    namespace: usize,
) -> Result<(String, HashMap<String, serde_json::Value>), String> {
    let mut clauses = Vec::new();
    let mut binds = HashMap::new();
    for (i, (key, value)) in conditions.iter().enumerate() {
        let field = match key {
            HashKey::String(s) | HashKey::Symbol(s) => s.to_string(),
            _ => return Err("where_translated() expects field names as keys".to_string()),
        };
        checked_field(&field, "where_translated()")?;
        let bind = format!("__tr{namespace}_{i}");
        let json = crate::interpreter::value::value_to_json(value)
            .map_err(|e| format!("where_translated(): {e}"))?;
        let expression = reader_expression(class_name, &field);
        if json.is_null() {
            clauses.push(format!("{expression} == null"));
        } else {
            clauses.push(format!("{expression} == @{bind}"));
            binds.insert(bind, json);
        }
    }
    if clauses.is_empty() {
        return Err("where_translated() needs at least one condition".to_string());
    }
    Ok((clauses.join(" AND "), binds))
}

/// The sort criterion `order_translated("title")` adds, in the builder's
/// `doc.<field>` form: the reader's value of a translated field. Refused on
/// the SQL adapters, which cannot run it.
pub fn translated_order_field(class_name: &str, field: &str) -> Result<String, String> {
    checked_field(field, "order_translated()")?;
    if crate::db::is_sql() {
        return Err(
            "order_translated() is SoliDB-only: the SQL adapters cannot sort by a translation"
                .to_string(),
        );
    }
    let expression = reader_expression(class_name, field);
    // The builder renders `doc.<criterion>`: hand it the expression minus
    // that leading `doc.`, unparenthesized so the prefix lands on its first
    // operand.
    let bare = expression
        .strip_prefix('(')
        .and_then(|e| e.strip_suffix(')'))
        .unwrap_or(&expression);
    Ok(bare.strip_prefix("doc.").unwrap_or(bare).to_string())
}

/// `order_translated(field, direction?)`'s arguments: the field and
/// `"ASC"` / `"DESC"`.
pub fn order_translated_args(args: &[Value]) -> Result<(String, String), String> {
    let field = match args.first() {
        Some(Value::String(s)) | Some(Value::Symbol(s)) => s.to_string(),
        _ => return Err("order_translated(field, direction?) expects a field name".to_string()),
    };
    let direction = match args.get(1) {
        None | Some(Value::Null) => "ASC".to_string(),
        Some(Value::String(s)) | Some(Value::Symbol(s)) => match s.to_ascii_lowercase().as_str() {
            "asc" => "ASC".to_string(),
            "desc" => "DESC".to_string(),
            _ => {
                return Err(format!(
                    "order_translated(): direction must be asc or desc, got {s}"
                ))
            }
        },
        Some(other) => {
            return Err(format!(
                "order_translated(): direction must be asc or desc, got {}",
                other.type_name()
            ))
        }
    };
    Ok((field, direction))
}

/// `missing_translations(locale)`'s filter: a record lacking `locale` for at
/// least one of the model's translated fields.
pub fn missing_filter(
    class_name: &str,
    locale: &str,
) -> Result<(String, HashMap<String, serde_json::Value>), String> {
    let fields = super::get_translated_fields(class_name);
    if fields.is_empty() {
        return Err(format!(
            "missing_translations(): {class_name} declares no translated field"
        ));
    }
    let mut clauses = Vec::with_capacity(fields.len());
    for field in &fields {
        checked_field(field, "missing_translations()")?;
        clauses.push(format!(
            "doc.translated_fields.{field}[@__tr_locale] == null"
        ));
    }
    let mut binds = HashMap::new();
    binds.insert(
        "__tr_locale".to_string(),
        serde_json::Value::String(locale.to_string()),
    );
    Ok((format!("({})", clauses.join(" OR ")), binds))
}

/// Stage `value` as `field`'s translation in `locale`
/// (`_pending_translations.{field}.{locale}`); `save` / `update` fold it
/// into `translated_fields`.
pub fn stage_translation(
    inst: &mut crate::interpreter::value::Instance,
    field: &str,
    locale: &str,
    value: Value,
) {
    use std::cell::RefCell;
    use std::rc::Rc;
    let child = |parent: &Rc<RefCell<HashPairs>>, key: &str| {
        let key = HashKey::String(key.into());
        let existing = parent.borrow().get(&key).cloned();
        match existing {
            Some(Value::Hash(hash)) => hash,
            _ => {
                let hash = Rc::new(RefCell::new(HashPairs::default()));
                parent.borrow_mut().insert(key, Value::Hash(hash.clone()));
                hash
            }
        }
    };
    let pending = match inst.get("_pending_translations") {
        Some(Value::Hash(hash)) => hash,
        _ => {
            let hash = Rc::new(RefCell::new(HashPairs::default()));
            inst.fields
                .insert("_pending_translations".into(), Value::Hash(hash.clone()));
            hash
        }
    };
    child(&pending, field)
        .borrow_mut()
        .insert(HashKey::String(locale.into()), value);
}

/// `{"title": {"fr": "Bonjour"}, …}` — a `translated_fields` hash given to
/// `update` / `save` (a form's `translated_fields[title][fr]` fields) —
/// staged locale by locale for the model's translated fields, so the
/// locales it does not name are kept. Anything else in it is ignored.
pub fn stage_translations_hash(
    inst: &mut crate::interpreter::value::Instance,
    class_name: &str,
    given: &Value,
) {
    let Value::Hash(by_field) = given else {
        return;
    };
    let entries: Vec<(HashKey, Value)> = by_field
        .borrow()
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (key, by_locale) in entries {
        let (HashKey::String(field) | HashKey::Symbol(field)) = &key else {
            continue;
        };
        if !super::is_translated_field(class_name, field) {
            continue;
        }
        let Value::Hash(by_locale) = by_locale else {
            continue;
        };
        let values: Vec<(HashKey, Value)> = by_locale
            .borrow()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (locale, value) in values {
            if let HashKey::String(locale) | HashKey::Symbol(locale) = locale {
                stage_translation(inst, field, &locale, value);
            }
        }
    }
}

/// The value `field` holds in `locale` (no chain), for `title_in("fr")`.
pub fn translation_in(
    translated_fields: Option<&Value>,
    field: &str,
    locale: &str,
) -> Option<Value> {
    let Some(Value::Hash(by_field)) = translated_fields else {
        return None;
    };
    let by_locale = by_field
        .borrow()
        .get(&HashKey::String(field.into()))
        .cloned()?;
    let Value::Hash(by_locale) = by_locale else {
        return None;
    };
    let found = by_locale
        .borrow()
        .get(&HashKey::String(locale.into()))
        .cloned();
    found
}
