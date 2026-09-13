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

use solilang::db::{clear_registry_override, registry, set_registry_for_tests};
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

#[test]
fn two_applications_do_not_share_a_database() {
    // The one that matters most. `db::registry` decides which database a query
    // goes to, and it used to be a process-global `OnceLock`: the first
    // application to boot froze it for every application after it, silently,
    // so each later one read and wrote the first one's data with the first
    // one's credentials.
    let alpha = register(PathBuf::from("/srv/alpha"));
    let beta = register(PathBuf::from("/srv/beta"));

    let alpha_registry = scoped(alpha, registry);
    let mut beta_registry = scoped(beta, registry);
    // Give beta a distinguishable default so the assertion cannot pass by both
    // sides happening to hold the same env-derived registry.
    beta_registry.default = "beta_primary".into();
    let spec = beta_registry
        .connections
        .values()
        .next()
        .cloned()
        .expect("the env fallback always yields one connection");
    beta_registry
        .connections
        .insert("beta_primary".into(), spec);

    scoped(beta, || set_registry_for_tests(beta_registry.clone()));

    scoped(beta, || {
        assert_eq!(
            registry().default,
            "beta_primary",
            "beta must see the registry it installed"
        );
    });
    scoped(alpha, || {
        assert_eq!(
            registry().default,
            alpha_registry.default,
            "alpha must still see its own registry, not beta's"
        );
        assert!(
            !registry().connections.contains_key("beta_primary"),
            "beta's connection must not appear in alpha's registry"
        );
    });

    // Clearing beta's override is beta's business alone.
    scoped(beta, clear_registry_override);
    scoped(alpha, || {
        assert_eq!(registry().default, alpha_registry.default);
    });
}
