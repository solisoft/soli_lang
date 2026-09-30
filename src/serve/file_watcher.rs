//! The dev-mode file watcher: the thread that notices a source change and
//! tells the workers (and the browser) to reload.
//!
//! Lifted out of `run_hyper_server_worker_pool`, which was 1,173 lines of boot
//! sequence — channels, the tokio thread, this watcher, the route snapshot, the
//! banner, readiness, the worker partition and the spawn loop, in one function.
//! This was the largest self-contained piece of it, and the easiest to move
//! honestly: every value it captures was already cloned into a named local
//! immediately above the `thread::spawn`, so the move is a signature rather
//! than a rewrite.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;

use super::{files, server_constants, tailwind, HotReloadVersions};

/// Everything under the served folder that a change should be noticed in.
pub(super) struct WatchPaths {
    pub controllers: PathBuf,
    pub views: PathBuf,
    pub middleware: PathBuf,
    pub helpers: PathBuf,
    /// `app/components`: component classes load with the view helpers, so an
    /// edit reloads them the same way.
    pub components: PathBuf,
    pub models: PathBuf,
    pub services: PathBuf,
    pub policies: PathBuf,
    pub mailers: PathBuf,
    pub jobs: PathBuf,
    pub public: PathBuf,
    pub routes_file: PathBuf,
    pub assets_css: PathBuf,
    /// `config/locales`: translation files, reloaded without a restart.
    pub locales: PathBuf,
    pub folder: PathBuf,
}

impl WatchPaths {
    /// The conventional layout under `folder`, given the directories the server
    /// already resolved for itself.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        folder: &Path,
        controllers: &Path,
        views: &Path,
        middleware: &Path,
        helpers: &Path,
        models: &Path,
        jobs: &Path,
        public: &Path,
        routes_file: &Path,
    ) -> Self {
        Self {
            controllers: controllers.to_path_buf(),
            views: views.to_path_buf(),
            middleware: middleware.to_path_buf(),
            helpers: helpers.to_path_buf(),
            components: folder.join("app/components"),
            models: models.to_path_buf(),
            services: folder.join("app/services"),
            policies: folder.join("app/policies"),
            mailers: folder.join("app/mailers"),
            jobs: jobs.to_path_buf(),
            public: public.to_path_buf(),
            routes_file: routes_file.to_path_buf(),
            assets_css: folder.join("app/assets/css"),
            locales: folder.join("config/locales"),
            folder: folder.to_path_buf(),
        }
    }
}

/// What a directory created after boot reloads (see `late_dirs` in [`spawn`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LateDir {
    Controllers,
    Views,
    Middleware,
    Helpers,
    Models,
    Jobs,
    Locales,
}

/// Spawn the watcher. Dev only — the caller decides that.
///
/// `hot_reload_versions` is what the workers read to notice they must reload;
/// `graph_reindex_tx` is `None` unless the code-graph watch is on.
pub(super) fn spawn(
    paths: WatchPaths,
    browser_reload_tx: Option<broadcast::Sender<()>>,
    hot_reload_versions_for_watcher: Arc<HotReloadVersions>,
    graph_reindex_tx: Option<std::sync::mpsc::Sender<()>>,
) {
    let WatchPaths {
        controllers: watch_controllers_dir,
        views: watch_views_dir,
        middleware: watch_middleware_dir,
        helpers: watch_helpers_dir,
        components: watch_components_dir,
        models: watch_models_dir,
        services: watch_services_dir,
        policies: watch_policies_dir,
        mailers: watch_mailers_dir,
        jobs: watch_jobs_dir,
        public: watch_public_dir,
        routes_file: watch_routes_file,
        assets_css: watch_assets_css_dir,
        locales: watch_locales_dir,
        folder: watch_folder,
    } = paths;

    thread::spawn(move || {
        use notify::{RecursiveMode, Watcher};

        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("Hot reload: Failed to create file watcher: {}", e);
                return;
            }
        };

        // Watch directories — handles new files automatically
        let mut watch_count = 0u32;
        if watch_controllers_dir.exists()
            && watcher
                .watch(&watch_controllers_dir, RecursiveMode::Recursive)
                .is_ok()
        {
            watch_count += 1;
        }
        if watch_middleware_dir.exists()
            && watcher
                .watch(&watch_middleware_dir, RecursiveMode::NonRecursive)
                .is_ok()
        {
            watch_count += 1;
        }
        if watch_helpers_dir.exists()
            && watcher
                .watch(&watch_helpers_dir, RecursiveMode::NonRecursive)
                .is_ok()
        {
            watch_count += 1;
        }
        if watch_components_dir.exists()
            && watcher
                .watch(&watch_components_dir, RecursiveMode::NonRecursive)
                .is_ok()
        {
            watch_count += 1;
        }
        // Recursive: models/services load recursively, so nested files
        // (e.g. app/models/billing/invoice.sl) must hot-reload too. The
        // event handler matches by `path.starts_with(dir)`, which already
        // covers nested paths.
        if watch_models_dir.exists()
            && watcher
                .watch(&watch_models_dir, RecursiveMode::Recursive)
                .is_ok()
        {
            watch_count += 1;
        }
        if watch_services_dir.exists()
            && watcher
                .watch(&watch_services_dir, RecursiveMode::Recursive)
                .is_ok()
        {
            watch_count += 1;
        }
        if watch_policies_dir.exists()
            && watcher
                .watch(&watch_policies_dir, RecursiveMode::Recursive)
                .is_ok()
        {
            watch_count += 1;
        }
        if watch_mailers_dir.exists()
            && watcher
                .watch(&watch_mailers_dir, RecursiveMode::Recursive)
                .is_ok()
        {
            watch_count += 1;
        }
        if watch_jobs_dir.exists()
            && watcher
                .watch(&watch_jobs_dir, RecursiveMode::Recursive)
                .is_ok()
        {
            watch_count += 1;
        }
        if watch_views_dir.exists()
            && watcher
                .watch(&watch_views_dir, RecursiveMode::Recursive)
                .is_ok()
        {
            watch_count += 1;
        }
        if watch_public_dir.exists()
            && watcher
                .watch(&watch_public_dir, RecursiveMode::Recursive)
                .is_ok()
        {
            watch_count += 1;
        }
        if let Some(routes_parent) = watch_routes_file.parent() {
            if routes_parent.exists()
                && watcher
                    .watch(routes_parent, RecursiveMode::NonRecursive)
                    .is_ok()
            {
                watch_count += 1;
            }
        }
        if watch_assets_css_dir.exists()
            && watcher
                .watch(&watch_assets_css_dir, RecursiveMode::NonRecursive)
                .is_ok()
        {
            watch_count += 1;
        }
        // Translations: adding or editing a key used to need a restart.
        if watch_locales_dir.exists()
            && watcher
                .watch(&watch_locales_dir, RecursiveMode::Recursive)
                .is_ok()
        {
            watch_count += 1;
        }
        // File mode's extra `--assets` roots. The served folder is already
        // covered — in file mode it *is* the views dir — but an assets root
        // lives outside it, so a picture edited there would otherwise not
        // reload the page that embeds it. `'static` roots, no clone needed.
        for assets_root in files::assets_roots() {
            if watcher.watch(assets_root, RecursiveMode::Recursive).is_ok() {
                watch_count += 1;
            }
        }

        // Directories an app may create after boot — `app/services/` added to
        // a running `--dev` server left `SiteContent` undefined until a
        // restart, because only directories that existed at boot were
        // watched. `app/` and `config/` are watched non-recursively, so a new
        // one shows up as an event: it is watched from then on, and its kind
        // reloads, which loads whatever it already holds.
        let mut late_dirs: Vec<(PathBuf, RecursiveMode, LateDir)> = [
            (
                &watch_controllers_dir,
                RecursiveMode::Recursive,
                LateDir::Controllers,
            ),
            (&watch_views_dir, RecursiveMode::Recursive, LateDir::Views),
            (
                &watch_middleware_dir,
                RecursiveMode::NonRecursive,
                LateDir::Middleware,
            ),
            (
                &watch_helpers_dir,
                RecursiveMode::NonRecursive,
                LateDir::Helpers,
            ),
            (
                &watch_components_dir,
                RecursiveMode::NonRecursive,
                LateDir::Helpers,
            ),
            (&watch_models_dir, RecursiveMode::Recursive, LateDir::Models),
            (
                &watch_services_dir,
                RecursiveMode::Recursive,
                LateDir::Models,
            ),
            (
                &watch_policies_dir,
                RecursiveMode::Recursive,
                LateDir::Models,
            ),
            (
                &watch_mailers_dir,
                RecursiveMode::Recursive,
                LateDir::Models,
            ),
            (&watch_jobs_dir, RecursiveMode::Recursive, LateDir::Jobs),
            (
                &watch_locales_dir,
                RecursiveMode::Recursive,
                LateDir::Locales,
            ),
        ]
        .into_iter()
        .filter(|(dir, _, _)| !dir.exists())
        .map(|(dir, mode, kind)| (dir.clone(), mode, kind))
        .collect();
        let app_dir = watch_folder.join("app");
        if late_dirs
            .iter()
            .any(|(dir, _, _)| dir.parent() == Some(app_dir.as_path()))
            && app_dir.is_dir()
            && watcher.watch(&app_dir, RecursiveMode::NonRecursive).is_ok()
        {
            watch_count += 1;
        }

        println!(
            "Hot reload: Watching {} directories (event-driven)",
            watch_count
        );

        // Debounce: collect events over a short window before processing
        const DEBOUNCE_MS: u64 = 300;
        // Cooldown to prevent reload loops (e.g., when Tailwind rebuilds CSS after view changes)
        const RELOAD_COOLDOWN_MS: u64 = 2000;
        let mut last_reload_time: Option<Instant> = None;

        while let Ok(first) = rx.recv() {
            // Collect additional events that arrive within the debounce window
            let mut raw_events = vec![first];
            let deadline = Instant::now() + Duration::from_millis(DEBOUNCE_MS);
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match rx.recv_timeout(remaining) {
                    Ok(ev) => raw_events.push(ev),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }

            // Extract unique paths from content-change events only.
            // Exclude metadata-only events (e.g. atime updates from reads)
            // which fire IN_ATTRIB on Linux when the server reads templates.
            let mut changed_paths = std::collections::HashSet::new();
            for event in raw_events.into_iter().flatten() {
                use notify::EventKind;
                match event.kind {
                    EventKind::Create(_)
                    | EventKind::Remove(_)
                    | EventKind::Modify(notify::event::ModifyKind::Data(_))
                    | EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                        for path in event.paths {
                            changed_paths.insert(path);
                        }
                    }
                    _ => {} // Ignore Access, Metadata, and Other events
                }
            }

            // A watched-for directory created since boot (see `late_dirs`).
            let mut created: Vec<LateDir> = Vec::new();
            late_dirs.retain(|(dir, mode, kind)| {
                if changed_paths.contains(dir) && dir.is_dir() && watcher.watch(dir, *mode).is_ok()
                {
                    println!("Hot reload: now watching {}", dir.display());
                    created.push(*kind);
                    false
                } else {
                    true
                }
            });

            // Filter to relevant extensions only
            let changed: Vec<PathBuf> = changed_paths
                .into_iter()
                .filter(|path| {
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        matches!(ext, "sl" | "erb" | "slv" | "md")
                            || server_constants::is_tracked_static_extension(ext)
                            || (matches!(ext, "yml" | "yaml")
                                && path.starts_with(&watch_locales_dir))
                    } else {
                        false
                    }
                })
                .collect();

            if changed.is_empty() && created.is_empty() {
                continue;
            }

            // Signal the code-graph reindexer when a `.sl`/`.slv` changed.
            if let Some(tx) = &graph_reindex_tx {
                if changed.iter().any(|p| {
                    matches!(
                        p.extension().and_then(|e| e.to_str()),
                        Some("sl") | Some("slv")
                    )
                }) {
                    let _ = tx.send(());
                }
            }

            println!("\n🔄 Hot reload triggered for:");
            let mut views_changed = created.contains(&LateDir::Views);
            let mut controllers_changed = created.contains(&LateDir::Controllers);
            let mut middleware_changed = created.contains(&LateDir::Middleware);
            let mut helpers_changed = created.contains(&LateDir::Helpers);
            let mut models_changed = created.contains(&LateDir::Models);
            let mut jobs_changed = created.contains(&LateDir::Jobs);
            let mut static_files_changed = false;
            let mut routes_changed = false;
            let mut asset_css_changed = false;
            let mut locales_changed = created.contains(&LateDir::Locales);

            // Track the public/css output directory to distinguish
            // Tailwind output changes from source changes
            let public_css_dir = watch_public_dir.join("css");

            for path in &changed {
                println!("   {}", path.display());

                if path.starts_with(&watch_locales_dir) {
                    locales_changed = true;
                    continue;
                }

                // Check if it's a source CSS file in app/assets/css/
                if path.starts_with(&watch_assets_css_dir) {
                    if path.extension().and_then(|e| e.to_str()) == Some("css") {
                        asset_css_changed = true;
                    }
                    continue; // Don't also count as static file
                }

                // Check if it's a static file (CSS, JS, images)
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if server_constants::is_tracked_static_extension(ext) {
                        // Ignore public/css/ changes caused by Tailwind output
                        // to avoid recompilation loops
                        if !(ext == "css" && path.starts_with(&public_css_dir)) {
                            static_files_changed = true;
                        }
                    }
                }

                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name == "routes.sl" {
                        routes_changed = true;
                    } else if name.ends_with("_controller.sl") {
                        controllers_changed = true;
                    } else if name.ends_with(".sl") && path.starts_with(&watch_middleware_dir) {
                        middleware_changed = true;
                    } else if name.ends_with(".sl")
                        && (path.starts_with(&watch_helpers_dir)
                            || path.starts_with(&watch_components_dir))
                    {
                        helpers_changed = true;
                    } else if name.ends_with(".sl") && path.starts_with(&watch_models_dir) {
                        models_changed = true;
                    } else if name.ends_with(".sl")
                        && (path.starts_with(&watch_services_dir)
                            || path.starts_with(&watch_policies_dir)
                            || path.starts_with(&watch_mailers_dir))
                    {
                        // The models signal covers all three siblings, because
                        // `app_loader::load_models_and_siblings` reloads them
                        // in the same step. `app/policies/` had been left out
                        // of the watch list, so a policy edit on its own
                        // signalled nothing at all.
                        models_changed = true;
                    } else if name.ends_with("_job.sl") && path.starts_with(&watch_jobs_dir) {
                        jobs_changed = true;
                    } else if name.ends_with(".erb")
                        || name.ends_with(".slv")
                        || name.ends_with(".md")
                    {
                        views_changed = true;
                    }
                }
            }

            // Increment version counters - workers will pick this up
            if controllers_changed {
                hot_reload_versions_for_watcher
                    .controllers
                    .fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled controller reload to all workers");
            }
            if middleware_changed {
                hot_reload_versions_for_watcher
                    .middleware
                    .fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled middleware reload to all workers");
            }
            if helpers_changed {
                hot_reload_versions_for_watcher
                    .helpers
                    .fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled view helpers reload to all workers");
            }
            if models_changed {
                hot_reload_versions_for_watcher
                    .models
                    .fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled models reload to all workers");
            }
            if jobs_changed {
                hot_reload_versions_for_watcher
                    .jobs
                    .fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled jobs reload to all workers");
            }
            if views_changed {
                hot_reload_versions_for_watcher
                    .views
                    .fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled template cache clear to all workers");
            }

            if static_files_changed && !views_changed && !asset_css_changed {
                // Only signal static file reload when no views/asset CSS changed.
                // When views or asset CSS change, Tailwind rebuilds CSS into public/
                // which would trigger a redundant reload — the view reload already
                // causes the browser to fetch updated CSS.
                hot_reload_versions_for_watcher
                    .static_files
                    .fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled static file reload to all workers");
            }
            if routes_changed {
                hot_reload_versions_for_watcher
                    .routes
                    .fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled routes reload to all workers");
            }
            if locales_changed {
                hot_reload_versions_for_watcher
                    .locales
                    .fetch_add(1, Ordering::Release);
                // Rendered pages hold the old strings: the views signal is
                // what clears the template and rendered-body caches.
                hot_reload_versions_for_watcher
                    .views
                    .fetch_add(1, Ordering::Release);
                println!("   ✓ Signaled translations reload to all workers");
            }

            // Bump the generation after the per-kind counters (Release) so
            // a worker that observes it (Acquire) also observes every
            // per-kind bump above. Workers only scan the individual
            // counters when this one moved. A bump where no individual
            // counter moved (e.g. the static_files special case) is
            // harmless — the scan just finds nothing to do.
            if controllers_changed
                || middleware_changed
                || helpers_changed
                || models_changed
                || jobs_changed
                || views_changed
                || static_files_changed
                || routes_changed
                || locales_changed
            {
                hot_reload_versions_for_watcher
                    .generation
                    .fetch_add(1, Ordering::Release);
            }

            // Recompile Tailwind CSS when source files change (views may
            // introduce new classes, asset CSS may have new directives).
            // This blocks the watcher thread — possibly for seconds on a
            // cold run (binary download, first compile) — so it must run
            // AFTER the version bumps above or workers keep serving stale
            // cached bodies until Tailwind finishes. It still runs before
            // the browser-reload send (the browser must refetch the new
            // CSS) and before the debounce drain (which swallows the
            // watcher events Tailwind's own writes generate).
            if views_changed || asset_css_changed || controllers_changed || helpers_changed {
                tailwind::compile_tailwind_css_once(&watch_folder);
            }

            // Notify browser for live reload (with cooldown to prevent loops)
            let should_reload = match last_reload_time {
                Some(last_time) => {
                    let elapsed = Instant::now().duration_since(last_time);
                    elapsed.as_millis() as u64 >= RELOAD_COOLDOWN_MS
                }
                None => true,
            };

            if should_reload {
                if let Some(ref tx) = browser_reload_tx {
                    let _ = tx.send(());
                }
                last_reload_time = Some(Instant::now());
                println!(
                    "   -> Browser reload sent (cooldown: {}ms)",
                    RELOAD_COOLDOWN_MS
                );
            } else {
                let elapsed = Instant::now().duration_since(last_reload_time.unwrap());
                println!(
                    "   -> Skipped reload (cooldown active: {}ms remaining)",
                    RELOAD_COOLDOWN_MS.saturating_sub(elapsed.as_millis() as u64)
                );
            }

            println!();

            // Drain any events that arrived during processing to prevent
            // cascading reload loops (e.g. workers reading files can generate
            // inotify events on some Linux configurations).
            std::thread::sleep(Duration::from_millis(DEBOUNCE_MS));
            while rx.try_recv().is_ok() {}
        }
    });
}
