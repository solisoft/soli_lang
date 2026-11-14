//! Read replicas: the `writing` and `reading` roles.
//!
//! A Postgres or MySQL connection may name read replicas:
//!
//! ```toml
//! [connections.primary]
//! adapter = "postgres"
//! url = "${DATABASE_URL}"
//! replicas = ["${REPLICA_URL}"]
//! replica_delay = 2        # seconds a client that wrote keeps reading the primary
//! ```
//!
//! Every query runs under a role. **`writing`** (the default) sends everything
//! to the primary. Under **`reading`**, the *reads* of a connection with
//! replicas go to one of them; writes still go to the primary — they are not
//! refused, unlike Rails' `ReadOnlyError` — and the first one pins the rest of
//! the request there, so a request reads what it just wrote.
//!
//! The role comes from the request: a `GET` or `HEAD` runs under `reading`,
//! everything else under `writing`. A request that wrote leaves a short-lived
//! `soli_db_primary` cookie, and the same client's `GET`s run under `writing`
//! until it expires (`replica_delay`), so the page a `POST` redirects to does
//! not read a replica that has not caught up yet. Jobs, socket events, the
//! console and migrations run under `writing`. `with_role("reading") do … end`
//! and `with_role("writing") do … end` choose it for a block; an explicit
//! `reading` block reads a replica even after a write.
//!
//! Reads go to a replica only from inside [`read`], which the read functions
//! of the SQL facade (`sql`, `columns`) wrap around their dispatch. Anything
//! else — writes, DDL, migrations, `ensure_table`, `db.execute` — never sees a
//! replica, whatever the role. An open transaction keeps every read on the
//! primary too.
//!
//! Resolution happens in [`super::registry::with_active_spec`], like a tenant's:
//! the spec a read resolves to is the connection with the replica's URL, so the
//! pools and per-URL caches keep primary and replica apart on their own.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use super::registry::{ConnectionRegistry, ConnectionSpec};
use super::tenancy;

/// Seconds a client that wrote keeps reading the primary, when the connection
/// does not say (`replica_delay`). Rails' default too.
pub const DEFAULT_DELAY_SECS: u64 = 2;

/// The cookie that remembers a client wrote a moment ago.
pub const STICKY_COOKIE: &str = "soli_db_primary";

/// Which databases a query may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The primary, for everything.
    Writing,
    /// A replica for reads, where the connection has one.
    Reading,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Writing => "writing",
            Role::Reading => "reading",
        }
    }

    pub fn parse(name: &str) -> Result<Role, String> {
        match name {
            "writing" => Ok(Role::Writing),
            "reading" => Ok(Role::Reading),
            other => Err(format!(
                "unknown database role {other:?}: use \"writing\" or \"reading\""
            )),
        }
    }
}

crate::request_local! {
    /// The role this request, or the `with_role` block around this code, runs
    /// under.
    static ROLE: Cell<Role> = const { Cell::new(Role::Writing) };
    /// Whether `ROLE` was chosen with `with_role` (it then ignores `WROTE`).
    static EXPLICIT: Cell<bool> = const { Cell::new(false) };
    /// Whether this request has written: its reads stay on the primary.
    static WROTE: Cell<bool> = const { Cell::new(false) };
    /// Whether this is a request that sets the sticky cookie when it writes.
    static IN_REQUEST: Cell<bool> = const { Cell::new(false) };
    /// Set while a read of the SQL facade runs (see [`read`]).
    static READ_OP: Cell<bool> = const { Cell::new(false) };
    /// Whether the read running now resolved to a replica: the query log
    /// marks its entry (see [`on_replica`]).
    static ON_REPLICA: Cell<bool> = const { Cell::new(false) };
    /// Whether this request (or job) has read a replica at all: its record
    /// writes then send only what changed (see [`read_a_replica`]).
    static READ_REPLICA: Cell<bool> = const { Cell::new(false) };
    /// Which replica this request reads, before `% replicas.len()`.
    static PICK: Cell<usize> = const { Cell::new(0) };
}

thread_local! {
    /// The replica spec this thread resolved last, under the connection name
    /// and URL it stands for: a request makes many reads against one replica.
    static RESOLVED: RefCell<Option<(String, String, Arc<ConnectionSpec>)>> =
        const { RefCell::new(None) };
}

/// Set once a registry with a replica is installed: until then a request
/// starts without looking at its cookies.
static ANY_REPLICAS: AtomicBool = AtomicBool::new(false);
/// The longest `replica_delay` of any installed registry.
static DELAY_SECS: AtomicU64 = AtomicU64::new(DEFAULT_DELAY_SECS);
/// Spreads requests across the replicas.
static NEXT_PICK: AtomicUsize = AtomicUsize::new(0);

/// Note an installed registry's replicas (see [`ANY_REPLICAS`]).
pub fn note_registry(registry: &ConnectionRegistry) {
    let with_replicas: Vec<&ConnectionSpec> = registry
        .connections
        .values()
        .filter(|spec| !spec.replicas.is_empty())
        .collect();
    if with_replicas.is_empty() {
        return;
    }
    let delay = with_replicas
        .iter()
        .map(|spec| spec.replica_delay.unwrap_or(DEFAULT_DELAY_SECS))
        .max()
        .unwrap_or(DEFAULT_DELAY_SECS);
    DELAY_SECS.store(delay, Ordering::Relaxed);
    ANY_REPLICAS.store(true, Ordering::Relaxed);
}

/// Whether any installed registry has a connection with replicas.
pub fn any_replicas() -> bool {
    ANY_REPLICAS.load(Ordering::Relaxed)
}

/// The role code here runs under.
pub fn current() -> Role {
    ROLE.get()
}

/// Back to `writing`, no write noted, outside any request. Socket events and
/// jobs start here.
pub fn clear() {
    ROLE.set(Role::Writing);
    EXPLICIT.set(false);
    WROTE.set(false);
    IN_REQUEST.set(false);
    READ_OP.set(false);
    READ_REPLICA.set(false);
}

fn mark_replica_read() {
    ON_REPLICA.set(true);
    READ_REPLICA.set(true);
}

/// Whether this request has read a replica. A record it loaded may then hold
/// values older than the primary's, so `save` / `update` write only the
/// fields that changed (and the ones `update` was given) instead of the
/// whole record, which would put the replica's lagging values back on the
/// primary.
pub fn read_a_replica() -> bool {
    READ_REPLICA.get()
}

/// Start a request: `reading` for a `GET` or `HEAD` from a client that has not
/// written within `replica_delay`, `writing` otherwise.
pub fn begin_request(method: &str, cookie_header: Option<&str>) {
    clear();
    if !any_replicas() {
        return;
    }
    IN_REQUEST.set(true);
    PICK.set(NEXT_PICK.fetch_add(1, Ordering::Relaxed));
    let safe = method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD");
    if safe && !wrote_recently(cookie_header, now_secs()) {
        ROLE.set(Role::Reading);
    }
}

/// Whether the request's cookies say this client wrote until a moment ago.
/// The cookie is not signed: forging it only sends a client's own reads to
/// the primary.
fn wrote_recently(cookie_header: Option<&str>, now: u64) -> bool {
    let Some(header) = cookie_header else {
        return false;
    };
    header.split(';').any(|pair| {
        let Some((name, value)) = pair.trim().split_once('=') else {
            return false;
        };
        name == STICKY_COOKIE && value.trim().parse::<u64>().is_ok_and(|until| until > now)
    })
}

fn now_secs() -> u64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Note a write: this request's reads stay on the primary from now on, and the
/// client's next ones too, for `replica_delay` seconds.
pub fn note_write() {
    if WROTE.get() {
        return;
    }
    WROTE.set(true);
    if !IN_REQUEST.get() {
        return;
    }
    let delay = DELAY_SECS.load(Ordering::Relaxed);
    if delay == 0 {
        return;
    }
    let until = now_secs() + delay;
    crate::interpreter::builtins::session::set_response_cookie_with_attrs(
        STICKY_COOKIE,
        &until.to_string(),
        &format!("; Max-Age={delay}; Path=/; HttpOnly; SameSite=Lax"),
    );
}

/// Run `f` under `role`, then put back the role before — also when `f`
/// panics.
pub fn scoped<T>(role: Role, f: impl FnOnce() -> T) -> T {
    struct Restore(Role, bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            ROLE.set(self.0);
            EXPLICIT.set(self.1);
        }
    }
    let _restore = Restore(ROLE.get(), EXPLICIT.get());
    ROLE.set(role);
    EXPLICIT.set(true);
    f()
}

/// Run a read of the SQL facade: inside, the active connection may resolve to
/// a replica.
pub fn read<T>(f: impl FnOnce() -> T) -> T {
    if !any_replicas() {
        return f();
    }
    struct Restore(bool, bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            READ_OP.set(self.0);
            ON_REPLICA.set(self.1);
        }
    }
    let _restore = Restore(READ_OP.get(), ON_REPLICA.get());
    READ_OP.set(true);
    ON_REPLICA.set(false);
    f()
}

/// Whether the statement running now goes to a replica — asked by the query
/// log when it records one, so the dev bar can say where each query went.
pub fn on_replica() -> bool {
    READ_OP.get() && ON_REPLICA.get()
}

/// What the dev bar shows about this request's role, or `None` when no
/// connection has replicas (nothing to say).
pub fn summary() -> Option<String> {
    if !any_replicas() {
        return None;
    }
    Some(match (ROLE.get(), WROTE.get()) {
        (Role::Reading, true) => "reading · primary after a write".to_string(),
        (role, _) => role.as_str().to_string(),
    })
}

/// Whether a raw statement can go to a replica: a plain `SELECT`. A `WITH`
/// may hold a writing CTE, so it stays on the primary.
pub fn is_plain_select(sql: &str) -> bool {
    let head = sql.trim_start();
    head.len() >= 6 && head[..6].eq_ignore_ascii_case("select")
}

/// The replica `spec` resolves to for this read, or `None` when it goes to the
/// primary: not a read, not the `reading` role, a write already made (outside
/// an explicit `reading` block), a transaction open, or a tenant that brought
/// its own database URL.
pub fn replica_for(spec: &ConnectionSpec) -> Result<Option<Arc<ConnectionSpec>>, String> {
    if !READ_OP.get() || ROLE.get() != Role::Reading || spec.replicas.is_empty() {
        return Ok(None);
    }
    if WROTE.get() && !EXPLICIT.get() {
        return Ok(None);
    }
    if super::sql::has_active_tx() {
        return Ok(None);
    }
    let template = &spec.replicas[PICK.get() % spec.replicas.len()];
    let filled;
    let url: &str = if tenancy::is_template(spec) {
        let tenant = tenancy::current().ok_or_else(|| tenancy::no_tenant_error(&spec.name))?;
        if tenant.url.is_some() {
            return Ok(None);
        }
        filled = template.replace(tenancy::PLACEHOLDER, &tenant.key);
        &filled
    } else {
        template
    };
    let cached = RESOLVED.with(|r| {
        r.borrow()
            .as_ref()
            .filter(|(name, cached_url, _)| *name == spec.name && cached_url == url)
            .map(|(_, _, resolved)| resolved.clone())
    });
    if let Some(resolved) = cached {
        mark_replica_read();
        return Ok(Some(resolved));
    }
    let mut replica = spec.clone();
    replica.url = Some(url.to_string());
    replica.replicas = Vec::new();
    let replica = Arc::new(replica);
    RESOLVED.with(|r| {
        *r.borrow_mut() = Some((spec.name.clone(), url.to_string(), replica.clone()));
    });
    mark_replica_read();
    Ok(Some(replica))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Adapter;

    fn spec(url: &str, replicas: &[&str]) -> ConnectionSpec {
        ConnectionSpec {
            name: "primary".into(),
            adapter: Adapter::Postgres,
            url: Some(url.to_string()),
            solidb_host: None,
            solidb_database: None,
            solidb_username: None,
            solidb_password: None,
            solidb_api_key: None,
            pool_size: None,
            replicas: replicas.iter().map(|r| r.to_string()).collect(),
            replica_delay: None,
        }
    }

    fn reading_request() {
        ANY_REPLICAS.store(true, Ordering::Relaxed);
        begin_request("GET", None);
        PICK.set(0);
    }

    #[test]
    fn only_a_read_under_reading_goes_to_a_replica() {
        let primary = spec("postgres://h/app", &["postgres://r/app"]);
        reading_request();
        assert!(
            replica_for(&primary).unwrap().is_none(),
            "not inside read()"
        );
        let url = read(|| replica_for(&primary).unwrap().and_then(|s| s.url.clone()));
        assert_eq!(url.as_deref(), Some("postgres://r/app"));
        assert!(!READ_OP.get(), "read() puts the flag back");
        let marked = read(|| {
            let _ = replica_for(&primary);
            on_replica()
        });
        assert!(marked, "the query log can tell a replica read");
        assert!(read_a_replica(), "the request remembers it read a replica");
        assert!(!on_replica());
        assert_eq!(summary().as_deref(), Some("reading"));

        begin_request("POST", None);
        assert_eq!(current(), Role::Writing);
        assert!(read(|| replica_for(&primary).unwrap()).is_none());
        clear();
    }

    #[test]
    fn a_write_pins_the_request_to_the_primary_unless_reading_is_explicit() {
        let primary = spec("postgres://h/app", &["postgres://r/app"]);
        reading_request();
        note_write();
        assert!(read(|| replica_for(&primary).unwrap()).is_none());
        assert_eq!(
            summary().as_deref(),
            Some("reading · primary after a write")
        );
        let explicit = scoped(Role::Reading, || read(|| replica_for(&primary).unwrap()));
        assert!(
            explicit.is_some(),
            "with_role(\"reading\") reads the replica"
        );
        assert!(!EXPLICIT.get(), "scoped puts the role back");
        clear();
    }

    #[test]
    fn a_recent_write_cookie_keeps_gets_on_the_primary() {
        assert!(wrote_recently(Some("a=1; soli_db_primary=200"), 100));
        assert!(!wrote_recently(Some("soli_db_primary=50"), 100));
        assert!(!wrote_recently(Some("soli_db_primary=junk"), 100));
        assert!(!wrote_recently(Some("other=200"), 100));
        assert!(!wrote_recently(None, 100));
    }

    #[test]
    fn requests_are_spread_across_replicas_and_tenants_fill_them() {
        let primary = spec(
            "postgres://h/app",
            &["postgres://r1/app", "postgres://r2/app"],
        );
        reading_request();
        PICK.set(1);
        let url = read(|| replica_for(&primary).unwrap().and_then(|s| s.url.clone()));
        assert_eq!(url.as_deref(), Some("postgres://r2/app"));

        let site = spec(
            "postgres://h/site_{tenant}",
            &["postgres://r/site_{tenant}"],
        );
        PICK.set(0);
        assert!(read(|| replica_for(&site)).is_err(), "no tenant chosen");
        let url = tenancy::scoped(Some(tenancy::Tenant::new("acme").unwrap()), || {
            read(|| replica_for(&site).unwrap().and_then(|s| s.url.clone()))
        });
        assert_eq!(url.as_deref(), Some("postgres://r/site_acme"));
        clear();
    }

    #[test]
    fn only_a_plain_select_is_a_read() {
        assert!(is_plain_select("  SELECT * FROM posts"));
        assert!(is_plain_select("select 1"));
        assert!(!is_plain_select(
            "WITH x AS (DELETE FROM t RETURNING *) SELECT * FROM x"
        ));
        assert!(!is_plain_select("UPDATE posts SET x = 1"));
        assert!(!is_plain_select("sel"));
    }
}
