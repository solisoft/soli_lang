use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct CoverageData {
    pub source_file: PathBuf,
    pub lines: HashMap<usize, LineCoverage>,
    pub branches: HashMap<usize, BranchCoverage>,
    pub total_statements: u32,
    pub covered_statements: u32,
}

#[derive(Debug, Clone)]
pub struct LineCoverage {
    pub line_number: usize,
    pub hits: u32,
    pub source_code: String,
    pub is_executable: bool,
}

#[derive(Debug, Clone)]
pub struct BranchCoverage {
    pub line_number: usize,
    pub branch_type: BranchType,
    pub hits_true: u32,
    pub hits_false: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchType {
    If,
    Match,
    LogicalAnd,
    LogicalOr,
    Ternary,
}

#[derive(Debug, Clone)]
pub struct FileCoverage {
    pub path: PathBuf,
    pub lines: HashMap<usize, LineCoverage>,
    pub branches: HashMap<usize, BranchCoverage>,
    pub total_lines: u32,
    pub covered_lines: u32,
    pub total_branches: u32,
    pub covered_branches: u32,
}

impl FileCoverage {
    pub fn line_coverage_percent(&self) -> f64 {
        if self.total_lines == 0 {
            return 100.0;
        }
        (self.covered_lines as f64 / self.total_lines as f64) * 100.0
    }

    pub fn branch_coverage_percent(&self) -> f64 {
        if self.total_branches == 0 {
            return 100.0;
        }
        (self.covered_branches as f64 / self.total_branches as f64) * 100.0
    }

    pub fn combined_coverage_percent(&self) -> f64 {
        let total = self.total_lines + self.total_branches;
        if total == 0 {
            return 100.0;
        }
        let covered = self.covered_lines + self.covered_branches;
        (covered as f64 / total as f64) * 100.0
    }
}

#[derive(Debug, Clone)]
pub struct TestCoverage {
    pub test_name: String,
    pub file_coverages: HashMap<PathBuf, FileCoverage>,
    pub start_time: std::time::Instant,
    pub end_time: Option<std::time::Instant>,
}

impl TestCoverage {
    pub fn new(test_name: String) -> Self {
        Self {
            test_name,
            file_coverages: HashMap::new(),
            start_time: std::time::Instant::now(),
            end_time: None,
        }
    }

    pub fn duration(&self) -> std::time::Duration {
        let end = self.end_time.unwrap_or_else(std::time::Instant::now);
        end - self.start_time
    }
}

#[derive(Debug, Clone)]
pub struct AggregatedCoverage {
    pub file_coverages: HashMap<PathBuf, FileCoverage>,
    pub test_count: usize,
    pub passed_count: usize,
    pub failed_count: usize,
    pub pending_count: usize,
}

impl Default for AggregatedCoverage {
    fn default() -> Self {
        Self::new()
    }
}

impl AggregatedCoverage {
    pub fn new() -> Self {
        Self {
            file_coverages: HashMap::new(),
            test_count: 0,
            passed_count: 0,
            failed_count: 0,
            pending_count: 0,
        }
    }

    pub fn total_line_coverage_percent(&self) -> f64 {
        let mut total_lines = 0;
        let mut covered_lines = 0;

        for file_cov in self.file_coverages.values() {
            total_lines += file_cov.total_lines;
            covered_lines += file_cov.covered_lines;
        }

        if total_lines == 0 {
            return 100.0;
        }
        (covered_lines as f64 / total_lines as f64) * 100.0
    }

    pub fn total_branch_coverage_percent(&self) -> f64 {
        let mut total_branches = 0;
        let mut covered_branches = 0;

        for file_cov in self.file_coverages.values() {
            total_branches += file_cov.total_branches;
            covered_branches += file_cov.covered_branches;
        }

        if total_branches == 0 {
            return 100.0;
        }
        (covered_branches as f64 / total_branches as f64) * 100.0
    }

    pub fn total_lines(&self) -> u32 {
        self.file_coverages.values().map(|f| f.total_lines).sum()
    }

    pub fn covered_lines(&self) -> u32 {
        self.file_coverages.values().map(|f| f.covered_lines).sum()
    }

    pub fn uncovered_lines(&self) -> Vec<UncoveredLine> {
        let mut uncovered: Vec<UncoveredLine> = Vec::new();

        for (path, file_cov) in &self.file_coverages {
            for (line_num, line_cov) in &file_cov.lines {
                if line_cov.is_executable && line_cov.hits == 0 {
                    uncovered.push(UncoveredLine {
                        path: path.clone(),
                        line_number: *line_num,
                        source_code: line_cov.source_code.clone(),
                    });
                }
            }
        }

        uncovered.sort_by_key(|u| (u.path.clone(), u.line_number));
        uncovered
    }
}

#[derive(Debug, Clone)]
pub struct UncoveredLine {
    pub path: PathBuf,
    pub line_number: usize,
    pub source_code: String,
}

#[derive(Debug, Clone, Default)]
pub struct CoverageConfig {
    pub enabled: bool,
    pub output_dir: PathBuf,
    pub formats: Vec<OutputFormat>,
    pub threshold: Option<f64>,
    pub exclude_patterns: Vec<String>,
    pub exclude_lines: Vec<(PathBuf, usize)>,
    pub show_uncovered: bool,
    pub per_test: bool,
    pub root_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    Console,
    Html,
    Json,
    Xml,
}

impl CoverageConfig {
    pub fn new() -> Self {
        Self {
            enabled: true,
            output_dir: PathBuf::from("coverage"),
            formats: vec![OutputFormat::Console, OutputFormat::Html],
            threshold: Some(80.0),
            exclude_patterns: Vec::new(),
            exclude_lines: Vec::new(),
            show_uncovered: true,
            per_test: false,
            root_dir: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(num: usize, hits: u32, executable: bool, src: &str) -> LineCoverage {
        LineCoverage {
            line_number: num,
            hits,
            source_code: src.to_string(),
            is_executable: executable,
        }
    }

    fn file_cov(
        path: &str,
        total_lines: u32,
        covered_lines: u32,
        total_branches: u32,
        covered_branches: u32,
    ) -> FileCoverage {
        FileCoverage {
            path: PathBuf::from(path),
            lines: HashMap::new(),
            branches: HashMap::new(),
            total_lines,
            covered_lines,
            total_branches,
            covered_branches,
        }
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    // ---------- FileCoverage percentages ----------

    #[test]
    fn line_percent_zero_total_returns_100() {
        // No executable lines = "fully covered" by convention.
        let f = file_cov("a.sl", 0, 0, 0, 0);
        assert!(approx(f.line_coverage_percent(), 100.0));
    }

    #[test]
    fn line_percent_full_coverage() {
        let f = file_cov("a.sl", 10, 10, 0, 0);
        assert!(approx(f.line_coverage_percent(), 100.0));
    }

    #[test]
    fn line_percent_half_coverage() {
        let f = file_cov("a.sl", 10, 5, 0, 0);
        assert!(approx(f.line_coverage_percent(), 50.0));
    }

    #[test]
    fn branch_percent_zero_total_returns_100() {
        let f = file_cov("a.sl", 0, 0, 0, 0);
        assert!(approx(f.branch_coverage_percent(), 100.0));
    }

    #[test]
    fn branch_percent_proportional() {
        let f = file_cov("a.sl", 0, 0, 4, 1);
        assert!(approx(f.branch_coverage_percent(), 25.0));
    }

    #[test]
    fn combined_percent_zero_total_returns_100() {
        let f = file_cov("a.sl", 0, 0, 0, 0);
        assert!(approx(f.combined_coverage_percent(), 100.0));
    }

    #[test]
    fn combined_percent_mixes_lines_and_branches() {
        // 8/10 lines covered, 2/4 branches covered => 10/14 ≈ 71.428...%
        let f = file_cov("a.sl", 10, 8, 4, 2);
        assert!(approx(f.combined_coverage_percent(), 10.0 / 14.0 * 100.0));
    }

    // ---------- AggregatedCoverage totals & percentages ----------

    #[test]
    fn aggregated_default_is_empty_and_100_percent() {
        let agg = AggregatedCoverage::default();
        assert_eq!(agg.test_count, 0);
        assert_eq!(agg.total_lines(), 0);
        assert_eq!(agg.covered_lines(), 0);
        assert!(approx(agg.total_line_coverage_percent(), 100.0));
        assert!(approx(agg.total_branch_coverage_percent(), 100.0));
    }

    #[test]
    fn aggregated_total_lines_sums_across_files() {
        let mut agg = AggregatedCoverage::new();
        agg.file_coverages
            .insert(PathBuf::from("a.sl"), file_cov("a.sl", 10, 7, 0, 0));
        agg.file_coverages
            .insert(PathBuf::from("b.sl"), file_cov("b.sl", 20, 15, 0, 0));
        assert_eq!(agg.total_lines(), 30);
        assert_eq!(agg.covered_lines(), 22);
        // 22/30 ≈ 73.333...%
        assert!(approx(
            agg.total_line_coverage_percent(),
            22.0 / 30.0 * 100.0
        ));
    }

    #[test]
    fn aggregated_branch_percent_uses_branch_totals_only() {
        let mut agg = AggregatedCoverage::new();
        agg.file_coverages
            .insert(PathBuf::from("a.sl"), file_cov("a.sl", 100, 0, 4, 3));
        // Branch percent is independent of line counts.
        assert!(approx(agg.total_branch_coverage_percent(), 75.0));
    }

    #[test]
    fn aggregated_uncovered_lines_filters_executable_zero_hits() {
        let mut f = file_cov("a.sl", 3, 1, 0, 0);
        // Mix: one covered, one uncovered-executable, one non-executable blank line.
        f.lines.insert(1, line(1, 5, true, "let x = 1;"));
        f.lines.insert(2, line(2, 0, true, "  do_thing();"));
        f.lines.insert(3, line(3, 0, false, "// comment"));
        let mut agg = AggregatedCoverage::new();
        agg.file_coverages.insert(PathBuf::from("a.sl"), f);

        let uncovered = agg.uncovered_lines();
        assert_eq!(uncovered.len(), 1);
        assert_eq!(uncovered[0].line_number, 2);
        assert_eq!(uncovered[0].source_code, "  do_thing();");
    }

    #[test]
    fn aggregated_uncovered_lines_sorted_by_path_then_line() {
        // Build files in "wrong" order to verify sort.
        let mut fb = file_cov("b.sl", 2, 0, 0, 0);
        fb.lines.insert(2, line(2, 0, true, "B2"));
        fb.lines.insert(1, line(1, 0, true, "B1"));
        let mut fa = file_cov("a.sl", 1, 0, 0, 0);
        fa.lines.insert(5, line(5, 0, true, "A5"));

        let mut agg = AggregatedCoverage::new();
        agg.file_coverages.insert(PathBuf::from("b.sl"), fb);
        agg.file_coverages.insert(PathBuf::from("a.sl"), fa);

        let uncovered = agg.uncovered_lines();
        let order: Vec<(String, usize)> = uncovered
            .iter()
            .map(|u| (u.path.display().to_string(), u.line_number))
            .collect();
        assert_eq!(
            order,
            vec![
                ("a.sl".to_string(), 5),
                ("b.sl".to_string(), 1),
                ("b.sl".to_string(), 2),
            ]
        );
    }

    // ---------- TestCoverage ----------

    #[test]
    fn test_coverage_duration_uses_end_time_when_set() {
        let mut tc = TestCoverage::new("t".to_string());
        // Simulate a finished test by clamping end_time to start_time —
        // duration should then be effectively zero (not negative, not nanosec
        // drift from std::time::Instant::now()).
        tc.end_time = Some(tc.start_time);
        assert_eq!(tc.duration(), std::time::Duration::ZERO);
    }

    #[test]
    fn test_coverage_duration_falls_back_to_now_when_unset() {
        let tc = TestCoverage::new("t".to_string());
        // Unset end_time uses Instant::now(); duration is non-negative and small.
        let d = tc.duration();
        assert!(d.as_secs() < 5, "unexpectedly large duration: {:?}", d);
    }

    // ---------- CoverageConfig ----------

    #[test]
    fn config_new_has_console_and_html_formats_by_default() {
        let cfg = CoverageConfig::new();
        assert!(cfg.enabled);
        assert!(cfg.formats.contains(&OutputFormat::Console));
        assert!(cfg.formats.contains(&OutputFormat::Html));
        assert_eq!(cfg.threshold, Some(80.0));
        assert!(cfg.show_uncovered);
        assert!(!cfg.per_test);
    }
}
