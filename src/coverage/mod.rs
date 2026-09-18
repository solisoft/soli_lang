pub mod data;
pub mod reporter;
pub mod tracker;

pub use data::*;
pub use reporter::CoverageReporter;
pub use tracker::{
    clear_global_coverage_tracker, get_global_coverage_tracker, set_global_coverage_tracker,
    CoverageTracker,
};

/// The key a file is recorded under in coverage, for both the registration
/// walk (which supplies the denominator) and the line hits (the numerator).
///
/// Absolute, so a walk that hands out relative paths and a loader that hands
/// out absolute ones agree — and deliberately NOT `canonicalize`, which
/// resolves symlinks. An app that shares code by symlinking it in (cosywee's
/// `bin/link_configurateur.sh` links one configurateur tree into b2b and b2c)
/// otherwise gets two entries per shared file: the walk finds
/// `<app>/app/models/product.sl` and registers every executable line there,
/// while a canonicalized loader path sends the hits to
/// `<shared>/app/models/product.sl`. The report then shows the same file
/// twice — once at its real coverage, once at 100% — and counts its lines
/// twice in the total.
///
/// `..` segments are left alone, since resolving them means resolving
/// symlinks, which is the thing this avoids.
pub fn coverage_path_key(path: &std::path::Path) -> std::path::PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod key_tests {
    use super::coverage_path_key;

    /// The regression, in one line: `canonicalize` here resolved the link and
    /// gave the shared tree's path, while the source walk kept the app's own.
    /// One file, two keys, two rows in the report.
    #[test]
    #[cfg(unix)]
    fn a_symlinked_source_is_keyed_by_the_app_s_own_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared = dir.path().join("shared.sl");
        std::fs::write(&shared, "def f()\n  1\nend\n").unwrap();
        let linked = dir.path().join("linked.sl");
        std::os::unix::fs::symlink(&shared, &linked).unwrap();

        assert_eq!(coverage_path_key(&linked), linked);
        assert_ne!(coverage_path_key(&linked), coverage_path_key(&shared));
    }

    /// Relative and absolute spellings of one file still have to meet: the
    /// walk and the loader do not always agree on which they hand over.
    #[test]
    fn a_relative_path_is_keyed_absolutely() {
        let key = coverage_path_key(std::path::Path::new("app/models/product.sl"));
        assert!(key.is_absolute());
        assert_eq!(
            key,
            std::env::current_dir()
                .unwrap()
                .join("app/models/product.sl")
        );
    }
}
