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
        }
    }
}
