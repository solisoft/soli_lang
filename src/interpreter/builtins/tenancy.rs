//! `switch_tenant`, `with_tenant` and `current_tenant`: which tenant's database
//! the connections with one database per tenant answer from
//! (see [`crate::db::tenancy`]).
//!
//! ```soli
//! site = Site.find_by("host", req["host"])
//! switch_tenant(site.db_key)                       # the template's database
//! switch_tenant(site.db_key, {"url": site.db_url}) # a database of its own
//! with_tenant("acme") do
//!   Post.count                                     # acme's posts
//! end
//! current_tenant()                                 # "acme", or nil
//! ```
//!
//! `with_tenant` runs a block, so it is intercepted by both engines (it needs
//! the engine to call the block); the native here only reports misuse.

use crate::db::tenancy::{self, Tenant};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{HashKey, NativeFunction, Value};

/// The tenant `args` name: a key (or `nil` for none), then an optional hash
/// with the tenant's own `url` (SQL, D1) or `database` (SoliDB).
pub fn tenant_from_args(args: &[Value], what: &str) -> Result<Option<Tenant>, String> {
    let key = match args.first() {
        Some(Value::String(key)) => key.to_string(),
        Some(Value::Null) => return Ok(None),
        Some(other) => {
            return Err(format!(
                "{what}(key) expects the tenant's key as a string, got {}",
                other.type_name()
            ))
        }
        None => return Err(format!("{what}(key) needs the tenant's key")),
    };
    let mut tenant = Tenant::new(&key).map_err(|e| format!("{what}: {e}"))?;
    match args.get(1) {
        None | Some(Value::Null) => {}
        Some(Value::Hash(options)) => {
            for (name, value) in options.borrow().iter() {
                let HashKey::String(name) = name else {
                    return Err(format!("{what}: option names are strings"));
                };
                let text = match value {
                    Value::String(s) if !s.trim().is_empty() => s.to_string(),
                    Value::Null => continue,
                    _ => return Err(format!("{what}: {name:?} must be a non-empty string")),
                };
                match name.as_ref() {
                    "url" => tenant.url = Some(text),
                    "database" => {
                        // It goes into the SoliDB URL path as it is: a name,
                        // not `x/../other` or `x?…` from a site record a
                        // tenant can edit.
                        let named = !text.is_empty()
                            && text.len() <= 64
                            && text
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
                        if !named {
                            return Err(format!(
                                "{what}: \"database\" must be 1-64 letters, digits, `_` or `-`"
                            ));
                        }
                        tenant.database = Some(text)
                    }
                    other => {
                        return Err(format!(
                            "{what}: unknown option {other:?} (use \"url\" or \"database\")"
                        ))
                    }
                }
            }
        }
        Some(other) => {
            return Err(format!(
                "{what}(key, options) expects options as a hash, got {}",
                other.type_name()
            ))
        }
    }
    if args.len() > 2 {
        return Err(format!(
            "{what} takes a key and an optional hash of options"
        ));
    }
    Ok(Some(tenant))
}

/// What has to be true before the tenant changes: no transaction is open (it
/// holds a connection to the old tenant's database), and the reads a
/// `grouped` block deferred have run against the tenant they were made under.
fn before_switch(what: &str) -> Result<(), String> {
    if super::model::crud::has_active_tx() {
        return Err(format!(
            "{what} inside a transaction: the transaction holds the current tenant's \
             database; switch before it begins"
        ));
    }
    super::model::batch::flush()
}

/// `switch_tenant(key, options?)`: the tenant for the rest of the request.
pub fn switch(args: &[Value]) -> Result<Value, String> {
    let tenant = tenant_from_args(args, "switch_tenant")?;
    before_switch("switch_tenant")?;
    tenancy::set(tenant);
    Ok(Value::Null)
}

/// Run `block` under `tenant`, then go back to the tenant before. The block's
/// deferred reads run before it ends, under its tenant.
pub fn with_tenant<T, E>(
    tenant: Option<Tenant>,
    block: impl FnOnce() -> Result<T, E>,
    error: impl Fn(String) -> E,
) -> Result<T, E> {
    before_switch("with_tenant").map_err(&error)?;
    tenancy::scoped(tenant, || {
        let value = block()?;
        super::model::batch::flush().map_err(&error)?;
        Ok(value)
    })
}

pub fn register_tenancy_builtins(env: &mut Environment) {
    env.define(
        "switch_tenant".to_string(),
        Value::NativeFunction(NativeFunction::new("switch_tenant", None, switch)),
    );
    env.define(
        "current_tenant".to_string(),
        Value::NativeFunction(NativeFunction::new("current_tenant", Some(0), |_| {
            Ok(tenancy::current_key()
                .map(|key| Value::String(key.into()))
                .unwrap_or(Value::Null))
        })),
    );
    // The engines run `with_tenant(key) do … end` themselves; reaching this
    // means there was no block.
    env.define(
        "with_tenant".to_string(),
        Value::NativeFunction(NativeFunction::new("with_tenant", None, |_| {
            Err(
                "with_tenant(key, options?) expects a block: with_tenant(\"acme\") do … end"
                    .to_string(),
            )
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn hash(pairs: &[(&str, &str)]) -> Value {
        let mut map = crate::interpreter::value::HashPairs::default();
        for (k, v) in pairs {
            map.insert(HashKey::String((*k).into()), Value::String((*v).into()));
        }
        Value::Hash(Rc::new(RefCell::new(map)))
    }

    #[test]
    fn a_key_and_its_options_become_a_tenant() {
        let t = tenant_from_args(&[Value::String("acme".into())], "switch_tenant")
            .unwrap()
            .unwrap();
        assert_eq!(t.key, "acme");
        assert!(t.url.is_none());
        let t = tenant_from_args(
            &[
                Value::String("acme".into()),
                hash(&[("url", "postgres://h/acme")]),
            ],
            "switch_tenant",
        )
        .unwrap()
        .unwrap();
        assert_eq!(t.url.as_deref(), Some("postgres://h/acme"));
        assert!(tenant_from_args(&[Value::Null], "switch_tenant")
            .unwrap()
            .is_none());
    }

    #[test]
    fn bad_keys_and_options_are_refused() {
        let err = |args: &[Value]| tenant_from_args(args, "switch_tenant").unwrap_err();
        assert!(err(&[Value::String("../x".into())]).contains("tenant key"));
        assert!(err(&[Value::Int(3)]).contains("string"));
        assert!(err(&[]).contains("needs"));
        assert!(err(&[Value::String("a".into()), hash(&[("host", "x")])]).contains("unknown"));
        assert!(err(&[Value::String("a".into()), hash(&[("url", " ")])]).contains("non-empty"));
    }
}
