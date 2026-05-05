//! File tracking utilities for hot reload

use std::path::Path;

use crate::serve::hot_reload::FileTracker;
use crate::serve::server_constants;

/// Recursively track view files (.erb) in a directory
///
/// SEC-049: skip symlinked entries — both files and directories. An
/// attacker who can drop a symlink into a watched tree would otherwise
/// have us recurse into or track files outside the project.
#[allow(dead_code)]
pub fn track_views_recursive(dir: &Path, tracker: &mut FileTracker) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                track_views_recursive(&path, tracker);
            } else if path
                .extension()
                .is_some_and(|ext| ext == "erb" || ext == "md")
            {
                tracker.track(&path);
            }
        }
    }
}

/// Recursively track static files (CSS, JS, etc.) in a directory
///
/// SEC-049: skip symlinked entries; same threat as `track_views_recursive`.
#[allow(dead_code)]
pub fn track_static_recursive(dir: &Path, tracker: &mut FileTracker) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                track_static_recursive(&path, tracker);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if server_constants::is_tracked_static_extension(ext) {
                    tracker.track(&path);
                }
            }
        }
    }
}
