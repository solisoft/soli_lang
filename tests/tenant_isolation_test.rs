//! Two applications in one process must not see each other's state.
//!
//! Every one of these assertions used to be impossible: the values behind them
//! were process-global singletons, so the second application to boot simply
//! overwrote the first. They are the acceptance criteria for the tenant seam —
//! and two of them are security properties, not tidiness.
//!
//! Deliberately exercised through the public API a host would actually call
//! (`scoped`, `set_file_jail`, `register_collection_type`, …) rather than by
//! reaching into the registries, so the test fails if the seam is bypassed
//! somewhere rather than only if a map is keyed wrong.

use std::path::PathBuf;

use solilang::interpreter::builtins::file::{jail_root, set_file_jail};
use solilang::interpreter::builtins::model::{get_collection_type, register_collection_type};
use solilang::serve::tenant::{app_root, register, scoped, set_app_root, TenantId};

/// Mount-time state each application writes for itself.
fn mount(root: &str, collection: &str, collection_type: &str) {
    set_app_root(PathBuf::from(root));
    set_file_jail(PathBuf::from(root));
    register_collection_type(collection, collection_type);
}

#[test]
fn two_applications_keep_their_own_roots_jails_and_collections() {
    let blog = register(PathBuf::from("/srv/blog"));
    let shop = register(PathBuf::from("/srv/shop"));
    assert_ne!(blog, shop);

    // Mounting happens under `scoped`, which is the whole point: both mounts
    // run on this one thread, and each must land on its own tenant.
    scoped(blog, || mount("/srv/blog", "posts", "edge"));
    scoped(shop, || mount("/srv/shop", "orders", "timeseries"));

    scoped(blog, || {
        assert_eq!(app_root(), PathBuf::from("/srv/blog"));
        // SEC-006: a shared jail would let either application resolve paths
        // under the other's root.
        assert_eq!(jail_root(), Some(PathBuf::from("/srv/blog")));
        assert_eq!(get_collection_type("posts"), Some("edge".to_string()));
        assert_eq!(
            get_collection_type("orders"),
            None,
            "the blog must not see the shop's collections"
        );
    });

    scoped(shop, || {
        assert_eq!(app_root(), PathBuf::from("/srv/shop"));
        assert_eq!(jail_root(), Some(PathBuf::from("/srv/shop")));
        assert_eq!(
            get_collection_type("orders"),
            Some("timeseries".to_string())
        );
        assert_eq!(
            get_collection_type("posts"),
            None,
            "the shop must not see the blog's collections"
        );
    });
}

#[test]
fn a_tenants_state_is_visible_from_every_thread_serving_it() {
    // Worker threads are spawned after the mount and are pinned to the tenant,
    // so what the mounting thread wrote has to be readable from them. This is
    // why the app root and the jails live in the process-wide registry rather
    // than in a `thread_local!`.
    let app = register(PathBuf::from("/srv/threaded"));
    scoped(app, || mount("/srv/threaded", "widgets", "edge"));

    let seen = std::thread::spawn(move || {
        scoped(app, || {
            (app_root(), jail_root(), get_collection_type("widgets"))
        })
    })
    .join()
    .expect("the worker thread must not panic");

    assert_eq!(seen.0, PathBuf::from("/srv/threaded"));
    assert_eq!(seen.1, Some(PathBuf::from("/srv/threaded")));
    assert_eq!(seen.2, Some("edge".to_string()));
}

#[test]
fn the_primary_tenant_is_untouched_by_the_others() {
    // A single-application server is the primary tenant and nothing else, so
    // registering extra tenants must leave it exactly as it was — this is what
    // makes the whole conversion a no-op for `soli serve`.
    let before = scoped(TenantId::PRIMARY, app_root);

    let other = register(PathBuf::from("/srv/noise"));
    scoped(other, || mount("/srv/noise", "noise", "edge"));

    scoped(TenantId::PRIMARY, || {
        assert_eq!(app_root(), before);
        assert_eq!(
            get_collection_type("noise"),
            None,
            "another tenant's collection must not leak into the primary"
        );
    });
}
