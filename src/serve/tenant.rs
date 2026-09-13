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
