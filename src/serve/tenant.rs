//! Per-application state, for a process that may serve more than one.
//!
//! `soli serve` is one application per process today, and a good deal of state
//! is written as a process-global singleton on that assumption: the app root,
//! the views and public directories, the file and image jails, the model and
//! controller registries, the mailer and KV configuration. Two applications in
//! one process would collide on every one of them long before memory became
//! the constraint — a jail in particular, where sharing would let one
//! application read another's files.
//!
//! This module is the seam those singletons move behind, one at a time. A
//! [`Tenant`] owns what belongs to one application; the process holds a
//! registry of them; and each thread knows which one it is currently serving.
//! With a single application the registry holds exactly one tenant and every
//! thread points at it, so nothing observable changes — which is the point:
//! the conversion can proceed global by global, with the existing test suite
//! as the check at every step.
//!
//! # Which state belongs where
//!
//! Two kinds of per-application state, and they cannot live in the same place:
//!
//! - **Shareable across threads** — paths, configuration, anything `Send +
//!   Sync`. It belongs in [`Tenant`], in the process-wide registry, readable
//!   from any thread serving that tenant.
//! - **Confined to one thread** — anything built out of `Rc`: interpreters,
//!   `Rc<Class>` model registries, view helpers, the parsed handler cache.
//!   Those stay in a plain `thread_local!`.
//!
//! Worker threads are bound to one tenant for life (`bind_current` at thread
//! start), so the current tenant is fixed rather than looked up per request,
//! and a thread-local already is per application. The HTTP side is not
//! thread-bound — see `TASK_TENANT`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, RwLock};

/// Identifies one application within the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TenantId(pub u32);

impl TenantId {
    /// The tenant a single-application server uses, and the one a thread that
    /// never bound anything falls back to.
    pub const PRIMARY: TenantId = TenantId(0);

    /// Index into the registry.
    #[inline]
    fn index(self) -> usize {
        self.0 as usize
    }
}

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tenant#{}", self.0)
    }
}

/// One served application's cross-thread state.
///
/// Everything here is `Send + Sync` and reachable from any thread bound to
/// this tenant. Thread-confined state (anything `Rc`-based) does not belong
/// here — see the module docs.
#[derive(Debug)]
pub struct Tenant {
    id: TenantId,
    /// Directory the application was served from. Every path the application
    /// resolves — views, public assets, config — hangs off this.
    root: RwLock<PathBuf>,
    /// Filesystem jail for the `File` / `slurp` builtins. `None` disables
    /// enforcement, which is what the CLI, the REPL and the test runner want.
    file_jail: RwLock<Option<PathBuf>>,
    /// Filesystem jail for the `Image` builtins, installed separately because
    /// it is set from a different place and may legitimately differ.
    image_jail: RwLock<Option<PathBuf>>,
}

impl Tenant {
    fn new(id: TenantId, root: PathBuf) -> Self {
        Self {
            id,
            root: RwLock::new(root),
            file_jail: RwLock::new(None),
            image_jail: RwLock::new(None),
        }
    }

    /// Read a jail, cloning it out: unlike the `OnceLock` this replaces, a
    /// per-tenant jail cannot hand out a `&'static Path`.
    fn read_jail(lock: &RwLock<Option<PathBuf>>) -> Option<PathBuf> {
        // Poison-tolerant: a jail that read as "none" after a panic elsewhere
        // would lift a security boundary, which is the one failure this value
        // must never have.
        lock.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Install a jail, first write wins.
    ///
    /// The `OnceLock` semantics this replaces are deliberate, not incidental:
    /// changing a jail while requests are in flight would leave some of them
    /// resolving against the old root. Ignoring a second write keeps that
    /// window closed.
    fn set_jail_once(lock: &RwLock<Option<PathBuf>>, path: PathBuf) {
        let mut jail = lock.write().unwrap_or_else(|e| e.into_inner());
        if jail.is_none() {
            *jail = Some(path);
        }
    }

    /// This tenant's identifier.
    #[inline]
    pub fn id(&self) -> TenantId {
        self.id
    }

    /// The directory this application is served from.
    ///
    /// Poison-tolerant rather than falling back to `.`: after a panic
    /// elsewhere, `.` would resolve views and assets against the process's
    /// working directory instead of the application's — a wrong answer that
    /// looks like a right one.
    pub fn root(&self) -> PathBuf {
        self.root.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Point this tenant at a directory. Called once when the server boots,
    /// and by tests that render against a scratch directory.
    pub fn set_root(&self, path: PathBuf) {
        *self.root.write().unwrap_or_else(|e| e.into_inner()) = path;
    }

    /// This application's filesystem jail, or `None` when none is enforced.
    pub fn file_jail(&self) -> Option<PathBuf> {
        Self::read_jail(&self.file_jail)
    }

    /// Install this application's filesystem jail. First write wins.
    pub fn set_file_jail(&self, path: PathBuf) {
        Self::set_jail_once(&self.file_jail, path);
    }

    /// This application's image jail, or `None` when none is enforced.
    pub fn image_jail(&self) -> Option<PathBuf> {
        Self::read_jail(&self.image_jail)
    }

    /// Install this application's image jail. First write wins.
    pub fn set_image_jail(&self, path: PathBuf) {
        Self::set_jail_once(&self.image_jail, path);
    }
}

/// The process's tenants, indexed by [`TenantId`].
///
/// Written only when a tenant is registered — once at boot for a
/// single-application server — and read on every `current()` that has to
/// repopulate a thread's cache, so the lock is uncontended in practice.
static TENANTS: LazyLock<RwLock<Vec<Arc<Tenant>>>> = LazyLock::new(|| {
    RwLock::new(vec![Arc::new(Tenant::new(
        TenantId::PRIMARY,
        PathBuf::from("."),
    ))])
});

thread_local! {
    /// The tenant this thread is serving, cached as the `Arc` itself so the
    /// common case costs a refcount bump and no global lock.
    static CURRENT: std::cell::RefCell<Option<Arc<Tenant>>> =
        const { std::cell::RefCell::new(None) };
}

tokio::task_local! {
    /// The tenant a tokio *task* is serving.
    ///
    /// Worker threads are pinned and use the thread-local above. The HTTP side
    /// is not: a request future runs on whichever tokio thread polls it next,
    /// and moves between them at every `.await`. A thread-local binding is
    /// therefore meaningless there — the thread that starts a request is not
    /// the one that finishes it — while everything the request path consults
    /// before handing off to a worker (CORS rules, CSRF exemptions, the cookie
    /// jar, the session config, the dev-bar store) is keyed by tenant. This
    /// binding travels with the future instead, and `current_id` consults it
    /// first.
    ///
    /// It does not cross `tokio::spawn`: a task spawned from a scoped request
    /// starts unbound. The WebSocket, EUI and LiveView upgrades spawn such
    /// tasks and still have to be handed their tenant explicitly — see the
    /// step-4 notes in `www/docs/internals/serve.md`.
    static TASK_TENANT: TenantId;
}

/// Run `future` as tenant `id`, for every thread it is polled on.
///
/// The async counterpart of [`scoped`]. Use it around a request future once
/// the `Host` header has said which application it belongs to.
pub fn task_scope<F: std::future::Future>(
    id: TenantId,
    future: F,
) -> tokio::task::futures::TaskLocalFuture<TenantId, F> {
    TASK_TENANT.scope(id, future)
}

/// `tokio::spawn`, carrying the current tenant into the new task.
///
/// A task-local does not cross `tokio::spawn`: a task spawned from inside a
/// [`task_scope`]d future starts unbound and falls back to `PRIMARY`. Every
/// spawn on the request path — the WebSocket, EUI and LiveView upgrades, their
/// writer tasks, the LiveView reaper — goes through this instead, so the new
/// task serves the same application as the request that spawned it.
pub fn spawn<F>(future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(task_scope(current_id(), future))
}

/// The tenant bound to the current tokio task, if this code runs inside one
/// that was started through [`task_scope`].
#[inline]
fn task_tenant() -> Option<TenantId> {
    TASK_TENANT.try_with(|id| *id).ok()
}

/// Register another application and return its id.
///
/// A single-application server never calls this: [`TenantId::PRIMARY`] exists
/// from the start, which is why `set_root` on it is all `soli serve` needs.
pub fn register(root: PathBuf) -> TenantId {
    let mut tenants = TENANTS.write().unwrap_or_else(|e| e.into_inner());
    let id = TenantId(tenants.len() as u32);
    tenants.push(Arc::new(Tenant::new(id, root)));
    id
}

/// Look a tenant up by id, or `None` if nothing is registered under it.
pub fn get(id: TenantId) -> Option<Arc<Tenant>> {
    TENANTS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(id.index())
        .cloned()
}

/// The tenant this thread is serving.
///
/// A thread that never called [`bind_current`] serves
/// [`TenantId::PRIMARY`] — which is every thread in a single-application
/// server, and the reason this conversion changes no behavior there.
pub fn current() -> Arc<Tenant> {
    // Task binding first: on a tokio thread the thread-local is either unset or
    // a stale cache from an earlier, unrelated request, and must not shadow the
    // tenant this task was scoped to.
    if let Some(id) = task_tenant() {
        return get(id).unwrap_or_else(|| panic!("{id} is not registered"));
    }
    CURRENT.with(|cell| {
        if let Some(tenant) = cell.borrow().as_ref() {
            return tenant.clone();
        }
        let tenant = get(TenantId::PRIMARY).expect("the primary tenant always exists");
        *cell.borrow_mut() = Some(tenant.clone());
        tenant
    })
}

/// The id of the tenant this thread (or tokio task) is serving, without
/// cloning the `Arc`.
pub fn current_id() -> TenantId {
    if let Some(id) = task_tenant() {
        return id;
    }
    CURRENT.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|t| t.id())
            .unwrap_or(TenantId::PRIMARY)
    })
}

/// Pin this thread to a tenant, for as long as it lives.
///
/// Called once when a worker thread starts. Panics on an unregistered id
/// rather than silently serving the wrong application's files.
pub fn bind_current(id: TenantId) {
    let tenant = get(id).unwrap_or_else(|| panic!("{id} is not registered"));
    CURRENT.with(|cell| *cell.borrow_mut() = Some(tenant));
}

// ---------------------------------------------------------------------------
// Per-tenant replacements for a process-global singleton
// ---------------------------------------------------------------------------

/// A value that used to be one per process and is now one per application.
///
/// Most of the singletons still to convert are a `Mutex<Option<T>>`, a
/// `OnceLock<T>` or a `Mutex<HashMap<..>>` declared next to the code that uses
/// them. Hanging each one off [`Tenant`] would mean a field, two accessors and
/// a `use` per global, and would collect two dozen unrelated locks in one
/// struct. This keeps each global where it is and changes only what it is:
///
/// ```ignore
/// static MAILER_CONFIG: Mutex<Option<MailerConfig>> = Mutex::new(None);
/// // becomes
/// static MAILER_CONFIG: TenantCell<MailerConfig> = TenantCell::new();
/// ```
///
/// Call sites keep their shape — `get()`, `set()`, `get_or_init()` — and
/// silently address the tenant the calling thread is serving. With one
/// application that is always [`TenantId::PRIMARY`], so the map holds a single
/// entry and behaviour is unchanged.
///
/// `T` must be `Send + Sync`: this is read from any thread serving the tenant.
/// Anything `Rc`-based stays in a `thread_local!`: worker threads serve one
/// application for life, so a thread-local already is per application.
pub struct TenantCell<T> {
    slots: RwLock<Option<std::collections::HashMap<TenantId, T>>>,
}

impl<T: Clone> Default for TenantCell<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Clone> TenantCell<T> {
    /// `const` so it can replace a `Mutex::new(None)` in a `static` directly,
    /// without dragging in `LazyLock` at every call site.
    pub const fn new() -> Self {
        Self {
            slots: RwLock::new(None),
        }
    }

    /// This tenant's value, or `None` if it never set one.
    pub fn get(&self) -> Option<T> {
        // Poison-tolerant throughout this type, for the same reason as
        // `TenantValue::read`: a value that reads as absent after a panic would
        // make `set_once` re-install and `get_or_init` rebuild what is there.
        let slots = self.slots.read().unwrap_or_else(|e| e.into_inner());
        slots.as_ref()?.get(&current_id()).cloned()
    }

    /// Replace this tenant's value.
    pub fn set(&self, value: T) {
        let mut slots = self.slots.write().unwrap_or_else(|e| e.into_inner());
        slots
            .get_or_insert_with(Default::default)
            .insert(current_id(), value);
    }

    /// Install a value only if this tenant has none — the `OnceLock::set`
    /// shape. Returns whether it was installed.
    pub fn set_once(&self, value: T) -> bool {
        let mut slots = self.slots.write().unwrap_or_else(|e| e.into_inner());
        let map = slots.get_or_insert_with(Default::default);
        match map.entry(current_id()) {
            std::collections::hash_map::Entry::Occupied(_) => false,
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(value);
                true
            }
        }
    }

    /// This tenant's value, creating it on first use.
    pub fn get_or_init(&self, init: impl FnOnce() -> T) -> T {
        if let Some(existing) = self.get() {
            return existing;
        }
        // `init` runs outside the write lock: it may itself touch the tenant
        // registry, and re-entering this lock would deadlock.
        let value = init();
        let mut slots = self.slots.write().unwrap_or_else(|e| e.into_inner());
        slots
            .get_or_insert_with(Default::default)
            .entry(current_id())
            .or_insert(value)
            .clone()
    }

    /// Drop this tenant's value, so the next `get_or_init` rebuilds it. Used by
    /// hot reload, and by a tenant going to sleep.
    pub fn clear(&self) {
        let mut slots = self.slots.write().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = slots.as_mut() {
            map.remove(&current_id());
        }
    }
}

/// A value that used to be one per process, where every application needs its
/// own and there is an obvious way to build a fresh one.
///
/// [`TenantCell`] models `Mutex<Option<T>>` — a slot that may be empty.
/// This models the other common shape, `lazy_static! { RwLock<T> }`: a value
/// that always exists because it can be constructed on demand, and that callers
/// mutate in place. The constructor is part of the declaration, so it replaces
/// `lazy_static!` outright and stays `const`:
///
/// ```ignore
/// lazy_static! { static ref MAILER_CONFIG: RwLock<MailerConfig> =
///     RwLock::new(MailerConfig::from_env()); }
/// // becomes
/// static MAILER_CONFIG: TenantValue<MailerConfig> =
///     TenantValue::new(MailerConfig::from_env);
/// ```
///
/// **One lock per tenant**, not one lock over all of them. The outer lock only
/// guards the map of slots and is held for a lookup; each slot carries its own
/// `RwLock`, so a closure that blocks under `write` — the database JWT login
/// does, for up to its network timeout — stalls that tenant alone, exactly as
/// the process-wide mutex it replaced stalled the one application it served.
/// A shared lock coupled tenants the rest of this module keeps apart.
///
/// `read`/`write` take a closure rather than returning a guard: the value lives
/// inside a map inside a lock, and stable Rust has no way to hand out a guard
/// borrowed into it. In exchange the lock is always released.
///
/// **Slots are never removed or replaced** — a value changes in place, under
/// its own lock — so each slot is allocated once and leaked to `'static`,
/// exactly as long-lived as the `static` that owns the map. That lets every
/// thread remember the slots it has used in [`TENANT_VALUE_SLOTS`] and reach
/// its value without the shared map's lock, its hash lookup, or a reference
/// count bumped on a cache line every worker shares. It is also why `read` and
/// `write` take `&'static self`: a `TenantValue` is always declared `static`.
pub struct TenantValue<T: 'static> {
    init: fn() -> T,
    slots: RwLock<Option<std::collections::HashMap<TenantId, &'static RwLock<T>>>>,
}

thread_local! {
    /// Per-thread memo of `TenantValue` slots: `(address of the static, tenant)`
    /// → that tenant's slot. Sound because both halves of the key are
    /// permanent — the `TenantValue` is `'static` and its slots are never
    /// removed — and the entry is type-checked on the way out by `downcast_ref`.
    static TENANT_VALUE_SLOTS: std::cell::RefCell<
        ahash::AHashMap<(usize, u32), &'static dyn std::any::Any>,
    > = std::cell::RefCell::new(ahash::AHashMap::new());
}

impl<T: 'static> TenantValue<T> {
    pub const fn new(init: fn() -> T) -> Self {
        Self {
            init,
            slots: RwLock::new(None),
        }
    }

    /// This tenant's slot, created on first use. Served from this thread's
    /// [`TENANT_VALUE_SLOTS`] memo after the first access.
    fn slot(&'static self) -> &'static RwLock<T> {
        let id = current_id();
        let key = (self as *const Self as usize, id.0);
        // `try_with`: during thread teardown the memo may already be gone, in
        // which case the shared map still answers.
        let memo = TENANT_VALUE_SLOTS
            .try_with(|memo| memo.borrow().get(&key).copied())
            .ok()
            .flatten();
        if let Some(slot) = memo.and_then(|any| any.downcast_ref::<RwLock<T>>()) {
            return slot;
        }
        let slot = self.shared_slot(id);
        let _ = TENANT_VALUE_SLOTS.try_with(|memo| {
            memo.borrow_mut()
                .insert(key, slot as &'static dyn std::any::Any);
        });
        slot
    }

    /// This tenant's slot from the shared map, created on first use.
    ///
    /// `init` runs outside every lock: it may read another `TenantValue`, the
    /// filesystem (the EUI publisher key) or the network, and none of that
    /// belongs under a lock other tenants wait on. Two threads racing to build
    /// the same slot both run `init`; the first insert wins and the other value
    /// is dropped, which is the price of not holding a lock across it.
    fn shared_slot(&self, id: TenantId) -> &'static RwLock<T> {
        {
            // Poison-tolerant, like every lock in this file: see `read`.
            let slots = self.slots.read().unwrap_or_else(|e| e.into_inner());
            if let Some(slot) = slots.as_ref().and_then(|m| m.get(&id).copied()) {
                return slot;
            }
        }
        let fresh = (self.init)();
        let mut slots = self.slots.write().unwrap_or_else(|e| e.into_inner());
        let slot: &mut &'static RwLock<T> = slots
            .get_or_insert_with(Default::default)
            .entry(id)
            .or_insert_with(|| -> &'static RwLock<T> { Box::leak(Box::new(RwLock::new(fresh))) });
        let slot: &'static RwLock<T> = slot;
        slot
    }

    /// Read this tenant's value, building it on first use.
    ///
    /// `f` runs under the slot's *read* lock, so a value that reads itself back
    /// from inside `f` — resolving a model's collection name reads the model
    /// registry again — takes a second read guard, as it did under the
    /// `lazy_static` this replaced. Poison is tolerated: a poisoned lock read
    /// as "absent" would spin `slot`, a jail read as "none" would lift a
    /// security boundary, and the value is a plain map that a panicking writer
    /// leaves with an entry, not torn.
    pub fn read<R>(&'static self, f: impl FnOnce(&T) -> R) -> R {
        let slot = self.slot();
        let guard = slot.read().unwrap_or_else(|e| e.into_inner());
        f(&guard)
    }

    /// Mutate this tenant's value, building it on first use.
    ///
    /// `f` runs under the slot's write lock — the same as the `RwLock::write()`
    /// guard this replaces, so a closure that re-enters the same value
    /// deadlocks exactly as it would have before, and a closure that blocks
    /// blocks this tenant only.
    pub fn write<R>(&'static self, f: impl FnOnce(&mut T) -> R) -> R {
        let slot = self.slot();
        let mut guard = slot.write().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }
}

/// Serve `id` for the duration of `f`, then put back whatever this thread was
/// serving before.
///
/// Mounting an application writes to its tenant — the app root, the jails, the
/// views directory, the model and controller registries — and every one of
/// those writes lands on whichever tenant the *calling thread* is bound to. A
/// host that mounts a second application from the thread that booted the first
/// would otherwise overwrite it. This is the seam that makes mounting from any
/// thread correct.
///
/// The binding is restored even if `f` panics, so a failed mount cannot leave
/// the thread pointing at a half-built tenant.
pub fn scoped<R>(id: TenantId, f: impl FnOnce() -> R) -> R {
    let previous = CURRENT.with(|cell| cell.borrow().clone());
    bind_current(id);
    struct Restore(Option<Arc<Tenant>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let previous = self.0.take();
            CURRENT.with(|cell| *cell.borrow_mut() = previous);
        }
    }
    let _restore = Restore(previous);
    // The task-local is consulted *before* the thread-local, so binding only
    // the latter would be silently ignored inside a `task_scope`d future — a
    // host mounting an application from a request handler would write every
    // mount-time value onto the request's tenant. `sync_scope` restores the
    // previous task binding on its own way out, unwinding included.
    TASK_TENANT.sync_scope(id, f)
}

// ---------------------------------------------------------------------------
// Convenience wrappers for the app root
// ---------------------------------------------------------------------------

/// The directory the application on this thread is served from.
///
/// Replaces the `APP_ROOT` global that used to live in `live::component`.
pub fn app_root() -> PathBuf {
    current().root()
}

/// Point the current thread's application at a directory.
pub fn set_app_root(path: impl AsRef<Path>) {
    current().set_root(path.as_ref().to_path_buf());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unbound_thread_serves_the_primary_tenant() {
        assert_eq!(current().id(), TenantId::PRIMARY);
        assert_eq!(current_id(), TenantId::PRIMARY);
    }

    #[test]
    fn a_registered_tenant_keeps_its_own_root() {
        let id = register(PathBuf::from("/srv/other-app"));
        assert_ne!(id, TenantId::PRIMARY);
        assert_eq!(
            get(id).expect("just registered").root(),
            PathBuf::from("/srv/other-app")
        );
        // Registering one must not disturb the primary tenant, which every
        // other thread in the process is still serving.
        assert_ne!(
            get(TenantId::PRIMARY).expect("always exists").root(),
            PathBuf::from("/srv/other-app")
        );
    }

    #[test]
    fn binding_a_thread_switches_which_root_it_sees() {
        let id = register(PathBuf::from("/srv/bound-app"));
        // A fresh thread, so this test cannot disturb the others: the binding
        // is thread-local and dies with the thread.
        std::thread::spawn(move || {
            assert_eq!(current_id(), TenantId::PRIMARY);
            bind_current(id);
            assert_eq!(current_id(), id);
            assert_eq!(app_root(), PathBuf::from("/srv/bound-app"));
        })
        .join()
        .expect("the bound thread must not panic");
    }

    #[test]
    fn a_jail_belongs_to_one_tenant_and_is_not_visible_from_another() {
        let a = register(PathBuf::from("/srv/a"));
        let b = register(PathBuf::from("/srv/b"));
        get(a).expect("a").set_file_jail(PathBuf::from("/srv/a"));
        get(b).expect("b").set_file_jail(PathBuf::from("/srv/b"));

        // The whole point: neither application can resolve a path under the
        // other's root, which one shared process-wide jail would have allowed.
        assert_eq!(
            get(a).expect("a").file_jail(),
            Some(PathBuf::from("/srv/a"))
        );
        assert_eq!(
            get(b).expect("b").file_jail(),
            Some(PathBuf::from("/srv/b"))
        );
    }

    #[test]
    fn installing_a_jail_twice_keeps_the_first() {
        // Not a quirk to preserve for its own sake: swapping a jail while
        // requests are in flight would leave some of them resolving against
        // the old root.
        let id = register(PathBuf::from("/srv/once"));
        let tenant = get(id).expect("just registered");
        assert_eq!(tenant.file_jail(), None, "no jail until one is installed");
        tenant.set_file_jail(PathBuf::from("/srv/once"));
        tenant.set_file_jail(PathBuf::from("/etc"));
        assert_eq!(tenant.file_jail(), Some(PathBuf::from("/srv/once")));
    }

    #[test]
    fn the_image_jail_is_installed_separately_from_the_file_jail() {
        let id = register(PathBuf::from("/srv/images"));
        let tenant = get(id).expect("just registered");
        tenant.set_file_jail(PathBuf::from("/srv/images"));
        assert_eq!(
            tenant.image_jail(),
            None,
            "setting one must not set the other"
        );
        tenant.set_image_jail(PathBuf::from("/srv/images/public"));
        assert_eq!(
            tenant.image_jail(),
            Some(PathBuf::from("/srv/images/public"))
        );
    }

    #[test]
    fn a_blocked_writer_on_one_tenant_does_not_stall_another() {
        // The database JWT login runs under `write` and can block on the
        // network for its full timeout. One lock over every tenant's slot made
        // that stall every other tenant's reads; a lock per slot does not.
        static COUPLED: TenantValue<u32> = TenantValue::new(|| 0);
        let a = register(PathBuf::from("/srv/slow"));
        let b = register(PathBuf::from("/srv/fast"));
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
        let slow = std::thread::spawn(move || {
            bind_current(a);
            COUPLED.write(|v| {
                let _ = held_tx.send(());
                let _ = release_rx.recv_timeout(std::time::Duration::from_secs(10));
                *v = 1;
            });
        });
        held_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the slow writer must take its lock");
        let (done_tx, done_rx) = std::sync::mpsc::channel::<u32>();
        std::thread::spawn(move || {
            bind_current(b);
            let _ = done_tx.send(COUPLED.read(|v| *v));
        });
        match done_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(value) => assert_eq!(value, 0, "tenant b sees its own, untouched value"),
            Err(_) => panic!("a read on tenant b waited on tenant a's writer"),
        }
        let _ = release_tx.send(());
        slow.join().expect("the slow writer must finish");
    }

    #[test]
    fn scoped_binds_the_task_local_too() {
        // Inside a task scope the task-local wins, so `scoped` must set it or a
        // mount performed from a request handler would land on the wrong
        // tenant with nothing reporting it.
        let request = register(PathBuf::from("/srv/request"));
        let mounted = register(PathBuf::from("/srv/mounted"));
        std::thread::spawn(move || {
            TASK_TENANT.sync_scope(request, || {
                assert_eq!(current_id(), request);
                scoped(mounted, || {
                    assert_eq!(
                        current_id(),
                        mounted,
                        "scoped must override the task binding"
                    );
                    set_app_root("/srv/mounted-root");
                });
                assert_eq!(current_id(), request, "and restore it afterwards");
                assert_eq!(app_root(), PathBuf::from("/srv/request"));
            });
        })
        .join()
        .expect("must not panic");
        assert_eq!(
            get(mounted).expect("registered").root(),
            PathBuf::from("/srv/mounted-root"),
            "the mount landed on the tenant it was scoped to"
        );
    }

    #[test]
    fn a_first_read_does_not_hold_the_write_lock() {
        // The model registry resolves a collection name from inside a closure
        // that is already reading the registry, so `read` must hand `f` a read
        // guard even on the access that creates the value. Under a write guard
        // this deadlocks — so the assertion is made on a thread with a
        // deadline, to fail the test rather than hang the suite.
        static REENTRANT: TenantValue<u32> = TenantValue::new(|| 7);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let id = register(PathBuf::from("/srv/reentrant"));
            bind_current(id);
            // Nothing has touched REENTRANT for this tenant yet, so the outer
            // call is the one that creates it.
            let doubled = REENTRANT.read(|outer| *outer + REENTRANT.read(|inner| *inner));
            let _ = tx.send(doubled);
        });

        match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(value) => assert_eq!(value, 14),
            Err(_) => panic!("a re-entrant read deadlocked: `f` ran under the write lock"),
        }
    }

    #[test]
    fn scoped_restores_the_previous_binding_even_on_panic() {
        let a = register(PathBuf::from("/srv/scoped-a"));
        let b = register(PathBuf::from("/srv/scoped-b"));
        std::thread::spawn(move || {
            bind_current(a);
            assert_eq!(current_id(), a);
            let panicked = std::panic::catch_unwind(|| {
                scoped(b, || {
                    assert_eq!(current_id(), b);
                    panic!("a mount that fails halfway");
                })
            });
            assert!(panicked.is_err(), "the panic must propagate");
            assert_eq!(
                current_id(),
                a,
                "a failed mount must not leave the thread on the half-built tenant"
            );
        })
        .join()
        .expect("the outer thread must not panic");
    }

    #[test]
    fn a_poisoned_value_is_still_readable_rather_than_spinning() {
        // A closure that panics under `write` poisons the lock. `read` retries
        // until it finds the value, so treating poison as "absent" would loop
        // forever — hence the deadline thread rather than a plain assertion.
        static POISONED: TenantValue<u32> = TenantValue::new(|| 1);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let id = register(PathBuf::from("/srv/poison"));
            bind_current(id);
            let _ = std::panic::catch_unwind(|| {
                POISONED.write(|v| {
                    *v = 2;
                    panic!("writer dies holding the lock");
                })
            });
            let _ = tx.send(POISONED.read(|v| *v));
        });
        match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(value) => assert_eq!(value, 2, "the write that completed before the panic stands"),
            Err(_) => panic!("read spun forever on a poisoned lock"),
        }
    }

    #[tokio::test]
    async fn a_task_scope_binds_the_tenant_across_await_points() {
        let id = register(PathBuf::from("/srv/task"));
        task_scope(id, async move {
            assert_eq!(current_id(), id);
            // Yield so the future is re-polled — on a multi-thread runtime,
            // possibly on another thread. The binding must still be there.
            tokio::task::yield_now().await;
            assert_eq!(current_id(), id);
            assert_eq!(app_root(), PathBuf::from("/srv/task"));
        })
        .await;
        // Outside the scope, the task falls back like any unbound thread.
        assert_eq!(current_id(), TenantId::PRIMARY);
    }

    #[test]
    #[should_panic(expected = "is not registered")]
    fn binding_an_unknown_tenant_panics_rather_than_serving_the_wrong_files() {
        std::thread::spawn(|| bind_current(TenantId(u32::MAX)))
            .join()
            .map_err(|e| std::panic::resume_unwind(e))
            .ok();
    }
}
