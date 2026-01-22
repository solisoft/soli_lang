//! Hot reload support for MVC framework.
//!
//! Tracks file modification times and detects changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Tracks file modification times for hot reload detection.
pub struct FileTracker {
    /// Map from file path to last known modification time.
    files: HashMap<PathBuf, SystemTime>,
    /// Last time we checked for changes (throttling)
    last_check: Instant,
    /// Minimum interval between checks
    check_interval: Duration,
}

impl FileTracker {
    /// Create a new file tracker.
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            last_check: Instant::now(),
            check_interval: Duration::from_secs(1), // Check at most once per second
        }
    }

    /// Create a new file tracker with a custom check interval.
    #[cfg(test)]
    pub fn with_check_interval(interval: Duration) -> Self {
        Self {
            files: HashMap::new(),
            last_check: Instant::now() - interval, // Start in the past so first check works
            check_interval: interval,
        }
    }

    /// Start tracking a file, recording its current modification time.
    pub fn track(&mut self, path: &Path) {
        if let Ok(metadata) = std::fs::metadata(path) {
            if let Ok(mtime) = metadata.modified() {
                self.files.insert(path.to_path_buf(), mtime);
            }
        }
    }

    /// Check all tracked files for changes (throttled).
    /// Returns a list of paths that have been modified since last check.
    /// Only performs actual file system checks if enough time has passed.
    pub fn get_changed_files(&mut self) -> Vec<PathBuf> {
        // Throttle: only check if enough time has passed
        let now = Instant::now();
        if now.duration_since(self.last_check) < self.check_interval {
            return Vec::new();
        }
        self.last_check = now;

        let mut changed = Vec::new();

        for (path, last_mtime) in &self.files {
            if let Ok(metadata) = std::fs::metadata(path) {
                if let Ok(current_mtime) = metadata.modified() {
                    if current_mtime > *last_mtime {
                        changed.push(path.clone());
                    }
                }
            }
        }

        // Update mtimes for changed files
        for path in &changed {
            if let Ok(metadata) = std::fs::metadata(path) {
                if let Ok(mtime) = metadata.modified() {
                    self.files.insert(path.clone(), mtime);
                }
            }
        }

        changed
    }

    /// Check if any tracked files have been modified.
    pub fn has_changes(&self) -> bool {
        for (path, last_mtime) in &self.files {
            if let Ok(metadata) = std::fs::metadata(path) {
                if let Ok(current_mtime) = metadata.modified() {
                    if current_mtime > *last_mtime {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Stop tracking a file.
    pub fn untrack(&mut self, path: &Path) {
        self.files.remove(path);
    }

    /// Get the number of tracked files.
    pub fn tracked_count(&self) -> usize {
        self.files.len()
    }
}

impl Default for FileTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_file_tracker_basic() {
        // Use a short check interval for testing (no throttle delay)
        let mut tracker = FileTracker::with_check_interval(std::time::Duration::from_millis(0));
        assert_eq!(tracker.tracked_count(), 0);

        // Create a temp file
        let temp_dir = std::env::temp_dir();
        let temp_file = temp_dir.join("test_hot_reload.txt");

        // Create the file
        std::fs::write(&temp_file, "initial content").unwrap();

        // Track it
        tracker.track(&temp_file);
        assert_eq!(tracker.tracked_count(), 1);

        // No changes yet
        let changes = tracker.get_changed_files();
        assert!(changes.is_empty());

        // Wait enough for filesystem mtime granularity (some filesystems have 1s resolution)
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&temp_file, "modified content").unwrap();

        // Should detect change
        let changes = tracker.get_changed_files();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0], temp_file);

        // Clean up
        std::fs::remove_file(&temp_file).ok();
    }
}
