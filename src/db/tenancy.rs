//! One database per tenant.
//!
//! An application keeps its accounts and sites in one database and each
//! site's data in a database of its own. The site databases are one named
//! connection whose `url` — or, on SoliDB, `database` — holds `{tenant}`:
//!
//! ```toml
//! [connections.site]
//! adapter = "postgres"
//! url = "postgres://app@db/site_{tenant}"
//! ```
//!
//! Models of the site data declare `connection "site"`; the request picks the
//! tenant with `switch_tenant(key)` (a middleware, usually, from the host) and
//! every query on that connection then goes to `site_<key>`. A site whose data
//! lives elsewhere is switched to with its own URL instead:
//! `switch_tenant(key, {"url": site.db_url})`.
//!
//! The tenant is a thread-local: worker threads serve one request at a time,
//! and each request starts and ends with none (`request_scope`), so a tenant
//! never outlives the request that chose it. A job remembers the tenant it was
//! enqueued under and runs under it.
//!
//! Resolution happens in [`super::registry::with_active_spec`]: the spec a
//! template connection resolves to is the template with the tenant filled in,
//! so the pools, the "table exists" memo and every other per-URL cache keep
//! each tenant apart without knowing tenants exist.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::adapter::Adapter;
use super::registry::{ConnectionRegistry, ConnectionSpec};

/// What a template connection's `url` / `database` holds in place of the key.
pub const PLACEHOLDER: &str = "{tenant}";

/// The tenant a request (or a job) runs under.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tenant {
    /// What `{tenant}` becomes: a database name, a file name, a binding suffix.
    pub key: String,
    /// The connection URL to use instead of the template's (SQL adapters, D1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The SoliDB database to use instead of the template's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
}

impl Tenant {
    /// A tenant whose databases come from the templates.
    pub fn new(key: &str) -> Result<Tenant, String> {
        validate_key(key)?;
        Ok(Tenant {
            key: key.to_string(),
            url: None,
            database: None,
        })
    }
}

/// A key goes into URLs, file paths and database names, so it is held to
/// what is safe in all of them: letters, digits, `_` and `-`, starting with a
/// letter or a digit, at most 63 characters (Postgres's identifier limit). A
/// key taken from a host name or a URL can therefore not reach another
/// directory, another database or another server.
pub fn validate_key(key: &str) -> Result<(), String> {
    let first_ok = key
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric());
    let rest_ok = key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if key.is_empty() || key.len() > 63 || !first_ok || !rest_ok {
        return Err(format!(
            "tenant key {key:?}: use 1-63 letters, digits, `_` or `-`, starting with a \
             letter or a digit"
        ));
    }
    Ok(())
}

thread_local! {
    /// The spec the last template resolution produced on this thread, with
    /// what it was built from: a request makes many queries against one
    /// tenant, and each would otherwise clone and fill the template again.
    static RESOLVED: RefCell<Option<Resolved>> = const { RefCell::new(None) };
}

crate::request_local! {
    static CURRENT: RefCell<Option<Tenant>> = const { RefCell::new(None) };
}

struct Resolved {
    name: String,
    url: Option<String>,
    database: Option<String>,
    tenant: Tenant,
    spec: Arc<ConnectionSpec>,
}

/// The tenant this thread runs under, if any.
pub fn current() -> Option<Tenant> {
    CURRENT.with(|c| c.borrow().clone())
}

/// The current tenant's key, if any.
pub fn current_key() -> Option<String> {
    CURRENT.with(|c| c.borrow().as_ref().map(|t| t.key.clone()))
}

/// Run under `tenant` (none with `None`) from now on, on this thread.
pub fn set(tenant: Option<Tenant>) {
    CURRENT.with(|c| *c.borrow_mut() = tenant);
}

/// Back to no tenant. Each request starts and ends here.
pub fn clear() {
    set(None);
}

/// Run `f` under `tenant`, then put back the tenant that was there before —
/// also when `f` panics.
pub fn scoped<T>(tenant: Option<Tenant>, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Tenant>);
    impl Drop for Restore {
        fn drop(&mut self) {
            set(self.0.take());
        }
    }
    let _restore = Restore(CURRENT.with(|c| c.replace(tenant)));
    f()
}

/// Clears the tenant when dropped: bound for the length of a request.
pub struct ClearOnDrop;

impl Drop for ClearOnDrop {
    fn drop(&mut self) {
        clear();
    }
}

/// Whether `spec` holds one database per tenant.
pub fn is_template(spec: &ConnectionSpec) -> bool {
    spec.url.as_deref().is_some_and(|u| u.contains(PLACEHOLDER))
        || spec
            .solidb_database
            .as_deref()
            .is_some_and(|d| d.contains(PLACEHOLDER))
}

/// The error for a template connection used with no tenant chosen.
pub fn no_tenant_error(connection: &str) -> String {
    format!(
        "connection {connection:?} holds one database per tenant: choose one first with \
         switch_tenant(key) (or with_tenant(key) do … end)"
    )
}

/// The connection `spec` stands for under `tenant`: the template filled in,
/// or the tenant's own URL / database when it brought one.
pub fn materialize(spec: &ConnectionSpec, tenant: &Tenant) -> ConnectionSpec {
    let mut out = spec.clone();
    let fill = |template: &str| template.replace(PLACEHOLDER, &tenant.key);
    if spec.adapter == Adapter::Solidb {
        out.solidb_database = tenant
            .database
            .clone()
            .or_else(|| spec.solidb_database.as_deref().map(fill));
    } else {
        out.url = tenant.url.clone().or_else(|| spec.url.as_deref().map(fill));
    }
    out
}

/// [`materialize`] under the current tenant, reusing this thread's last
/// result when nothing it was built from changed.
pub fn resolve(spec: &ConnectionSpec) -> Result<Arc<ConnectionSpec>, String> {
    let tenant = current().ok_or_else(|| no_tenant_error(&spec.name))?;
    let cached = RESOLVED.with(|r| {
        r.borrow()
            .as_ref()
            .filter(|r| {
                r.name == spec.name
                    && r.url == spec.url
                    && r.database == spec.solidb_database
                    && r.tenant == tenant
            })
            .map(|r| r.spec.clone())
    });
    if let Some(spec) = cached {
        return Ok(spec);
    }
    let resolved = Arc::new(materialize(spec, &tenant));
    RESOLVED.with(|r| {
        *r.borrow_mut() = Some(Resolved {
            name: spec.name.clone(),
            url: spec.url.clone(),
            database: spec.solidb_database.clone(),
            tenant,
            spec: resolved.clone(),
        })
    });
    Ok(resolved)
}

/// Set once a registry with a template connection is installed, in any
/// application of the process. The SoliDB path asks [`active_solidb_database`]
/// on every query; without a template anywhere that is one atomic load.
static ANY_TEMPLATE: AtomicBool = AtomicBool::new(false);

/// Note an installed registry's template connections (see [`ANY_TEMPLATE`]).
pub fn note_registry(registry: &ConnectionRegistry) {
    if registry.connections.values().any(is_template) {
        ANY_TEMPLATE.store(true, Ordering::Relaxed);
    }
}

/// Whether any installed registry has a connection with one database per
/// tenant.
pub fn any_templates() -> bool {
    ANY_TEMPLATE.load(Ordering::Relaxed)
}

/// Whether the active connection is a template one.
pub fn active_is_template() -> bool {
    ANY_TEMPLATE.load(Ordering::Relaxed)
        && super::registry::with_template_spec(is_template).unwrap_or(false)
}

/// The SoliDB database the active connection stands for, when it is a
/// template SoliDB connection; `None` for any other connection, which keeps
/// the database `SOLIDB_DATABASE` names. With no tenant chosen, the template
/// itself comes back, so the request fails on a database named `…{tenant}…`
/// rather than reading another tenant's data.
pub fn active_solidb_database() -> Option<String> {
    if !ANY_TEMPLATE.load(Ordering::Relaxed) {
        return None;
    }
    super::registry::with_template_spec(|spec| {
        if spec.adapter != Adapter::Solidb || !is_template(spec) {
            return None;
        }
        match resolve(spec) {
            Ok(resolved) => resolved.solidb_database.clone(),
            Err(_) => spec.solidb_database.clone(),
        }
    })
    .ok()
    .flatten()
}

/// How many pools an adapter keeps open for tenant databases.
///
/// A pool per database is what keeps tenants apart, but a pool never closed
/// is a server connection (or an open file) per tenant ever served. Past this
/// many, opening a tenant's pool closes the one opened longest ago; that
/// tenant's next request opens it again. `SOLI_TENANT_POOLS` sets it.
pub fn max_tenant_pools() -> usize {
    crate::platform::env::var("SOLI_TENANT_POOLS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(32)
}

/// The tenant pools one adapter has open, oldest first.
pub struct PoolBudget {
    open: Mutex<VecDeque<String>>,
}

impl PoolBudget {
    pub const fn new() -> Self {
        Self {
            open: Mutex::new(VecDeque::new()),
        }
    }

    /// Note that the pool under `key` was just opened; returns the keys of the
    /// pools to close to stay within [`max_tenant_pools`].
    pub fn admit(&self, key: &str) -> Vec<String> {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        open.retain(|k| k != key);
        open.push_back(key.to_string());
        let max = max_tenant_pools();
        let mut closed = Vec::new();
        while open.len() > max {
            if let Some(oldest) = open.pop_front() {
                closed.push(oldest);
            }
        }
        closed
    }
}

impl Default for PoolBudget {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(adapter: Adapter, url: Option<&str>, database: Option<&str>) -> ConnectionSpec {
        ConnectionSpec {
            name: "site".into(),
            adapter,
            url: url.map(str::to_string),
            solidb_host: None,
            solidb_database: database.map(str::to_string),
            solidb_username: None,
            solidb_password: None,
            solidb_api_key: None,
            pool_size: None,
        }
    }

    #[test]
    fn keys_that_could_leave_their_database_are_refused() {
        for good in ["acme", "site_42", "a-b", "7eleven"] {
            assert!(validate_key(good).is_ok(), "{good}");
        }
        for bad in [
            "", "../etc", "a/b", "a.b", "-x", "_x", "a b", "a;b", "é", "a%2fb",
        ] {
            assert!(validate_key(bad).is_err(), "{bad:?}");
        }
        assert!(validate_key(&"a".repeat(63)).is_ok());
        assert!(validate_key(&"a".repeat(64)).is_err());
    }

    #[test]
    fn a_template_is_filled_with_the_key_or_replaced_by_the_tenants_own() {
        let pg = spec(
            Adapter::Postgres,
            Some("postgres://u@h/site_{tenant}"),
            None,
        );
        assert!(is_template(&pg));
        let acme = Tenant::new("acme").unwrap();
        assert_eq!(
            materialize(&pg, &acme).url.as_deref(),
            Some("postgres://u@h/site_acme")
        );
        let moved = Tenant {
            url: Some("postgres://u@other/acme".into()),
            ..acme.clone()
        };
        assert_eq!(
            materialize(&pg, &moved).url.as_deref(),
            Some("postgres://u@other/acme")
        );

        let solidb = spec(Adapter::Solidb, None, Some("app_{tenant}"));
        assert!(is_template(&solidb));
        assert_eq!(
            materialize(&solidb, &acme).solidb_database.as_deref(),
            Some("app_acme")
        );
        assert!(!is_template(&spec(
            Adapter::Postgres,
            Some("postgres://u@h/main"),
            None
        )));
    }

    #[test]
    fn resolving_needs_a_tenant_and_follows_it() {
        let sqlite = spec(Adapter::Sqlite, Some("sqlite://db/{tenant}.sqlite3"), None);
        clear();
        assert!(resolve(&sqlite).unwrap_err().contains("switch_tenant"));
        let a = scoped(Some(Tenant::new("a").unwrap()), || {
            resolve(&sqlite).unwrap()
        });
        let b = scoped(Some(Tenant::new("b").unwrap()), || {
            resolve(&sqlite).unwrap()
        });
        assert_eq!(a.url.as_deref(), Some("sqlite://db/a.sqlite3"));
        assert_eq!(b.url.as_deref(), Some("sqlite://db/b.sqlite3"));
        assert!(current().is_none(), "scoped puts the previous tenant back");
    }

    #[test]
    fn the_pool_budget_closes_the_oldest_first() {
        let budget = PoolBudget::new();
        let max = max_tenant_pools();
        for i in 0..max {
            assert!(budget.admit(&format!("p{i}")).is_empty());
        }
        // Reopening one moves it to the back; the next new one evicts p1.
        assert!(budget.admit("p0").is_empty());
        assert_eq!(budget.admit("new"), vec!["p1".to_string()]);
    }
}
