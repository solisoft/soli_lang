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
//!   Those stay in a `thread_local!`, but keyed by [`TenantId`] rather than
//!   holding a single unkeyed value.
//!
//! Worker threads are pinned to one tenant, so the current tenant is fixed
//! when the thread starts rather than looked up per request, and a keyed
//! `thread_local!` holds exactly one entry in practice. Keying it anyway is
//! what makes the design correct if a thread ever drains work for two.

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
        lock.read().ok().and_then(|j| j.clone())
    }

    /// Install a jail, first write wins.
    ///
    /// The `OnceLock` semantics this replaces are deliberate, not incidental:
    /// changing a jail while requests are in flight would leave some of them
    /// resolving against the old root. Ignoring a second write keeps that
    /// window closed.
    fn set_jail_once(lock: &RwLock<Option<PathBuf>>, path: PathBuf) {
        if let Ok(mut jail) = lock.write() {
            if jail.is_none() {
                *jail = Some(path);
            }
        }
    }

    /// This tenant's identifier.
    #[inline]
    pub fn id(&self) -> TenantId {
        self.id
    }

    /// The directory this application is served from.
    ///
    /// Falls back to `.` if the lock was poisoned by a panicking writer, which
    /// is what the global this replaced did: a poisoned lock must not take
    /// down request handling on top of whatever already panicked.
    pub fn root(&self) -> PathBuf {
        self.root
            .read()
            .map(|r| r.clone())
            .unwrap_or_else(|_| PathBuf::from("."))
    }

    /// Point this tenant at a directory. Called once when the server boots,
    /// and by tests that render against a scratch directory.
    pub fn set_root(&self, path: PathBuf) {
        if let Ok(mut root) = self.root.write() {
            *root = path;
        }
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
    CURRENT.with(|cell| {
        if let Some(tenant) = cell.borrow().as_ref() {
            return tenant.clone();
        }
        let tenant = get(TenantId::PRIMARY).expect("the primary tenant always exists");
        *cell.borrow_mut() = Some(tenant.clone());
        tenant
    })
}

/// The id of the tenant this thread is serving, without cloning the `Arc`.
pub fn current_id() -> TenantId {
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
/// Anything `Rc`-based belongs in a [`TenantLocal`] instead.
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
        self.slots
            .read()
            .ok()
            .and_then(|slots| slots.as_ref()?.get(&current_id()).cloned())
    }

    /// Replace this tenant's value.
    pub fn set(&self, value: T) {
        if let Ok(mut slots) = self.slots.write() {
            slots
                .get_or_insert_with(Default::default)
                .insert(current_id(), value);
        }
    }

    /// Install a value only if this tenant has none — the `OnceLock::set`
    /// shape. Returns whether it was installed.
    pub fn set_once(&self, value: T) -> bool {
        match self.slots.write() {
            Ok(mut slots) => {
                let map = slots.get_or_insert_with(Default::default);
                match map.entry(current_id()) {
                    std::collections::hash_map::Entry::Occupied(_) => false,
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        slot.insert(value);
                        true
                    }
                }
            }
            Err(_) => false,
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
        match self.slots.write() {
            Ok(mut slots) => slots
                .get_or_insert_with(Default::default)
                .entry(current_id())
                .or_insert(value)
                .clone(),
            Err(_) => value,
        }
    }

    /// Drop this tenant's value, so the next `get_or_init` rebuilds it. Used by
    /// hot reload, and by a tenant going to sleep.
    pub fn clear(&self) {
        if let Ok(mut slots) = self.slots.write() {
            if let Some(map) = slots.as_mut() {
                map.remove(&current_id());
            }
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
/// `read`/`write` take a closure rather than returning a guard: the value lives
/// inside a map inside the lock, and stable Rust has no way to hand out a guard
/// borrowed into it. In exchange the lock is always released.
pub struct TenantValue<T: 'static> {
    init: fn() -> T,
    slots: RwLock<Option<std::collections::HashMap<TenantId, T>>>,
}

impl<T: 'static> TenantValue<T> {
    pub const fn new(init: fn() -> T) -> Self {
        Self {
            init,
            slots: RwLock::new(None),
        }
    }

    /// Read this tenant's value, building it on first use.
    pub fn read<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        // Fast path: a shared lock, which is what almost every call wants.
        if let Ok(slots) = self.slots.read() {
            if let Some(value) = slots.as_ref().and_then(|m| m.get(&current_id())) {
                return f(value);
            }
        }
        self.write(|value| f(value))
    }

    /// Mutate this tenant's value, building it on first use.
    pub fn write<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let id = current_id();
        let mut slots = self.slots.write().unwrap_or_else(|e| e.into_inner());
        let map = slots.get_or_insert_with(Default::default);
        f(map.entry(id).or_insert_with(self.init))
    }

    /// Drop this tenant's value, so the next access rebuilds it.
    pub fn reset(&self) {
        if let Ok(mut slots) = self.slots.write() {
            if let Some(map) = slots.as_mut() {
                map.remove(&current_id());
            }
        }
    }
}

/// The thread-confined counterpart of [`TenantCell`], for anything `Rc`-based:
/// interpreters, `Rc<Class>` registries, view helpers, parsed handler caches.
///
/// Those cannot live in the process-wide registry at all — they are not `Send`
/// — so they stay in a `thread_local!`. What changes is that the slot is keyed
/// by tenant instead of holding one unkeyed value, so a thread that ever serves
/// two applications keeps their classes apart. Worker threads are pinned, so in
/// practice the map holds one entry.
///
/// Declare it inside a `thread_local!` exactly as the `RefCell` it replaces.
pub struct TenantLocal<T> {
    slots: std::cell::RefCell<std::collections::HashMap<TenantId, T>>,
}

impl<T> Default for TenantLocal<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> TenantLocal<T> {
    pub fn new() -> Self {
        Self {
            slots: std::cell::RefCell::new(std::collections::HashMap::new()),
        }
    }

    /// Run `f` against this tenant's value on this thread, creating it on first
    /// use.
    pub fn with<R>(&self, init: impl FnOnce() -> T, f: impl FnOnce(&mut T) -> R) -> R {
        let id = current_id();
        let mut slots = self.slots.borrow_mut();
        f(slots.entry(id).or_insert_with(init))
    }

    /// Drop this tenant's value on this thread.
    pub fn clear(&self) {
        self.slots.borrow_mut().remove(&current_id());
    }
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
    #[should_panic(expected = "is not registered")]
    fn binding_an_unknown_tenant_panics_rather_than_serving_the_wrong_files() {
        std::thread::spawn(|| bind_current(TenantId(u32::MAX)))
            .join()
            .map_err(|e| std::panic::resume_unwind(e))
            .ok();
    }
}
