//! File tracking utilities for hot reload

use std::path::Path;

use crate::serve::hot_reload::FileTracker;
use crate::serve::server_constants;

/// Recursively track view files (.erb) in a directory
pub fn track_views_recursive(dir: &Path, tracker: &mut FileTracker) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                track_views_recursive(&path, tracker);
            } else if path.extension().is_some_and(|ext| ext == "erb") {
                tracker.track(&path);
            }
        }
    }
}

/// Recursively track static files (CSS, JS, etc.) in a directory
pub fn track_static_recursive(dir: &Path, tracker: &mut FileTracker) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                track_static_recursive(&path, tracker);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if server_constants::is_tracked_static_extension(ext) {
                    tracker.track(&path);
                }
            }
        }
    }
}
