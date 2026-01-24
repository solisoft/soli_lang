use crate::coverage::data::*;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub struct CoverageReporter {
    config: CoverageConfig,
}

impl CoverageReporter {
    pub fn new(config: CoverageConfig) -> Self {
        Self { config }
    }

    pub fn generate_reports(&self, coverage: &AggregatedCoverage) -> Vec<String> {
        let mut reports = Vec::new();

        for format in &self.config.formats {
            match format {
                OutputFormat::Console => {
                    let output = self.generate_console_report(coverage);
                    println!("{}", output);
                    reports.push("console".to_string());
                }
                OutputFormat::Html => {
                    self.generate_html_report(coverage);
                    reports.push("HTML report: coverage/index.html".to_string());
                }
                OutputFormat::Json => {
                    self.generate_json_report(coverage);
                    reports.push("JSON report: coverage/coverage.json".to_string());
                }
                OutputFormat::Xml => {
                    self.generate_xml_report(coverage);
                    reports.push("XML report: coverage/cobertura.xml".to_string());
                }
            }
        }

        reports
    }

    pub fn generate_console_report(&self, coverage: &AggregatedCoverage) -> String {
        let mut output = String::new();

        let total_percent = coverage.total_line_coverage_percent();
        let total_lines = coverage.total_lines();
        let covered_lines = coverage.covered_lines();

        let status = if total_percent >= self.config.threshold.unwrap_or(0.0) {
            "✓"
        } else {
            "❌"
        };

        output.push_str(&format!(
            "\nCoverage: {:.1}% ({}/{}{}) {}\n",
            total_percent,
            covered_lines,
            total_lines,
            if self.config.threshold.is_some() {
                format!(", threshold: {:.0}%", self.config.threshold.unwrap())
            } else {
                "".to_string()
            },
            status
        ));

        let mut file_data: Vec<(&PathBuf, &FileCoverage)> =
            coverage.file_coverages.iter().collect();
        file_data.sort_by(|a, b| {
            b.1.combined_coverage_percent()
                .partial_cmp(&a.1.combined_coverage_percent())
                .unwrap()
        });

        for (path, file_cov) in &file_data {
            let percent = file_cov.combined_coverage_percent();
            let bar = self.progress_bar(percent);

            let display_path = path.to_string_lossy();
            output.push_str(&format!(
                "  {:<40} {} {:>6.1}%\n",
                display_path, bar, percent
            ));
        }

        if self.config.show_uncovered {
            let uncovered = coverage.uncovered_lines();
            if !uncovered.is_empty() {
                output.push_str("\nUncovered lines:\n");
                for uncov in uncovered.iter().take(20) {
                    output.push_str(&format!(
                        "  {}:{} ({}))\n",
                        uncov.path.to_string_lossy(),
                        uncov.line_number,
                        uncov.source_code.trim()
                    ));
                }
                if uncovered.len() > 20 {
                    output.push_str(&format!("  ... and {} more\n", uncovered.len() - 20));
                }
            }
        }

        if let Some(threshold) = self.config.threshold {
            if total_percent < threshold {
                output.push_str(&format!(
                    "\n❌ Coverage {:.1}% is below threshold {:.0}%\n",
                    total_percent, threshold
                ));
            }
        }

        output
    }

    fn progress_bar(&self, percent: f64) -> String {
        let filled = ((percent / 10.0).floor() as usize).min(10);
        let empty = 10 - filled;

        let mut bar = String::new();
        for _ in 0..filled {
            bar.push('▓');
        }
        for _ in 0..empty {
            bar.push('░');
        }
        bar
    }

    pub fn generate_html_report(&self, coverage: &AggregatedCoverage) {
        let output_dir = &self.config.output_dir;
        let _ = fs::create_dir_all(output_dir);

        let html = self.html_dashboard(coverage);
        let _ = fs::write(output_dir.join("index.html"), html);

        let assets_dir = output_dir.join("assets");
        let _ = fs::create_dir_all(&assets_dir);

        self.write_html_assets(&assets_dir);
        self.write_html_source_files(coverage, output_dir);
        self.write_html_breakdown_json(coverage, output_dir);
    }

    fn html_dashboard(&self, coverage: &AggregatedCoverage) -> String {
        let total_percent = coverage.total_line_coverage_percent();
        let _pie_data = format!(
            "{{\"labels\": [\"Covered\", \"Uncovered\"], \"data\": [{}, {}]}}",
            coverage.covered_lines(),
            coverage.total_lines() - coverage.covered_lines()
        );

        let mut file_rows = String::new();
        let mut file_data: Vec<(&PathBuf, &FileCoverage)> =
            coverage.file_coverages.iter().collect();
        file_data.sort_by(|a, b| {
            b.1.combined_coverage_percent()
                .partial_cmp(&a.1.combined_coverage_percent())
                .unwrap()
        });

        for (path, file_cov) in &file_data {
            let percent = file_cov.combined_coverage_percent();
            let color = self.html_coverage_color(percent);
            let display_path = path.to_string_lossy();
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();

            file_rows.push_str(&format!(
                r#"<tr class="clickable-row" data-href="src/{}.html">
                    <td><span class="coverage-pill" style="background: {}">{:.1}%</span></td>
                    <td>{}</td>
                    <td>{}/{}</td>
                    <td>{:.1}%</td>
                </tr>"#,
                file_name,
                color,
                percent,
                display_path,
                file_cov.covered_lines,
                file_cov.total_lines,
                file_cov.branch_coverage_percent()
            ));
        }

        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Coverage Report</title>
    <link rel="stylesheet" href="assets/style.css">
</head>
<body>
    <div class="container">
        <header>
            <h1>Coverage Report</h1>
            <div class="summary">
                <div class="summary-card">
                    <span class="summary-value">{:.1}%</span>
                    <span class="summary-label">Line Coverage</span>
                </div>
                <div class="summary-card">
                    <span class="summary-value">{}</span>
                    <span class="summary-label">Files</span>
                </div>
                <div class="summary-card">
                    <span class="summary-value">{}</span>
                    <span class="summary-label">Lines</span>
                </div>
            </div>
        </header>

        <section class="file-list">
            <table>
                <thead>
                    <tr>
                        <th>Coverage</th>
                        <th>File</th>
                        <th>Lines</th>
                        <th>Branches</th>
                    </tr>
                </thead>
                <tbody>
                    {}
                </tbody>
            </table>
        </section>
    </div>
    <script src="assets/app.js"></script>
</body>
</html>"#,
            total_percent,
            coverage.file_coverages.len(),
            coverage.total_lines(),
            file_rows
        )
    }

    fn html_coverage_color(&self, percent: f64) -> String {
        if percent >= 90.0 {
            "#22c55e".to_string()
        } else if percent >= 75.0 {
            "#eab308".to_string()
        } else if percent >= 50.0 {
            "#f97316".to_string()
        } else {
            "#ef4444".to_string()
        }
    }

    fn write_html_assets(&self, assets_dir: &PathBuf) {
        let style = r#"* { margin: 0; padding: 0; box-sizing: border-box; }
body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #f8fafc; color: #1e293b; }
.container { max-width: 1200px; margin: 0 auto; padding: 24px; }
header { margin-bottom: 32px; }
h1 { font-size: 24px; font-weight: 600; margin-bottom: 16px; }
.summary { display: flex; gap: 16px; }
.summary-card { background: white; padding: 16px 24px; border-radius: 8px; box-shadow: 0 1px 3px rgba(0,0,0,0.1); text-align: center; }
.summary-value { font-size: 32px; font-weight: 700; display: block; }
.summary-label { font-size: 14px; color: #64748b; }
table { width: 100%; background: white; border-radius: 8px; box-shadow: 0 1px 3px rgba(0,0,0,0.1); border-collapse: collapse; }
th, td { padding: 12px 16px; text-align: left; border-bottom: 1px solid #e2e8f0; }
th { background: #f1f5f9; font-weight: 600; font-size: 14px; }
.clickable-row { cursor: pointer; transition: background 0.15s; }
.clickable-row:hover { background: #f8fafc; }
.coverage-pill { padding: 4px 12px; border-radius: 999px; font-size: 12px; font-weight: 600; color: white; }"#;

        let _ = fs::write(assets_dir.join("style.css"), style);

        let app_js = r#"document.addEventListener('DOMContentLoaded', function() {
    document.querySelectorAll('.clickable-row').forEach(function(row) {
        row.addEventListener('click', function() {
            var href = this.getAttribute('data-href');
            if (href) window.location.href = href;
        });
    });
});"#;

        let _ = fs::write(assets_dir.join("app.js"), app_js);
    }

    fn write_html_source_files(&self, coverage: &AggregatedCoverage, output_dir: &PathBuf) {
        let src_dir = output_dir.join("src");
        let _ = fs::create_dir_all(&src_dir);

        for (path, file_cov) in &coverage.file_coverages {
            let source_html = self.html_source_file(path, file_cov);
            if let Some(file_name) = path.file_name() {
                let _ = fs::write(
                    src_dir.join(format!("{}.html", file_name.to_string_lossy())),
                    source_html,
                );
            }
        }
    }

    fn html_source_file(&self, path: &PathBuf, file_cov: &FileCoverage) -> String {
        let file_content = std::fs::read_to_string(path).unwrap_or_default();
        let lines: Vec<&str> = file_content.lines().collect();

        let mut line_rows = String::new();

        for (i, line) in lines.iter().enumerate() {
            let line_num = i + 1;
            let line_cov = file_cov.lines.get(&line_num);
            let is_covered = line_cov.map(|l| l.hits > 0).unwrap_or(false);
            let is_executable = line_cov.map(|l| l.is_executable).unwrap_or(false);

            let mut row_class = "code-line";
            let mut coverage_indicator = "";

            if is_executable {
                if is_covered {
                    row_class = "code-line covered";
                    coverage_indicator = "✓";
                } else {
                    row_class = "code-line uncovered";
                    coverage_indicator = "✗";
                }
            }

            let escaped_line = line
                .replace("&", "&amp;")
                .replace("<", "&lt;")
                .replace(">", "&gt;");

            line_rows.push_str(&format!(
                r#"<tr class="{}">
                    <td class="line-num">{}</td>
                    <td class="line-indicator">{}</td>
                    <td class="code-content"><pre>{}</pre></td>
                </tr>"#,
                row_class, line_num, coverage_indicator, escaped_line
            ));
        }

        let file_name = path.file_name().unwrap_or_default().to_string_lossy();

        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{} - Coverage</title>
    <link rel="stylesheet" href="../assets/style.css">
</head>
<body>
    <div class="container">
        <header>
            <h1>{}</h1>
            <a href="../index.html" class="back-link">← Back to Dashboard</a>
        </header>
        <section class="source-view">
            <table>
                <tbody>
                    {}
                </tbody>
            </table>
        </section>
    </div>
</body>
</html>"#,
            file_name, file_name, line_rows
        )
    }

    fn write_html_breakdown_json(&self, coverage: &AggregatedCoverage, output_dir: &PathBuf) {
        let breakdown = self.generate_json_output(coverage);
        let _ = fs::write(output_dir.join("breakdown.json"), breakdown);
    }

    pub fn generate_json_report(&self, coverage: &AggregatedCoverage) {
        let json = self.generate_json_output(coverage);
        let _ = fs::create_dir_all(&self.config.output_dir);
        let _ = fs::write(self.config.output_dir.join("coverage.json"), json);
    }

    fn generate_json_output(&self, coverage: &AggregatedCoverage) -> String {
        let mut files = Vec::new();

        for (path, file_cov) in &coverage.file_coverages {
            let mut line_coverage = HashMap::new();
            for (line_num, line_cov) in &file_cov.lines {
                line_coverage.insert(line_num.to_string(), line_cov.hits);
            }

            files.push(serde_json::json!({
                "path": path.to_string_lossy(),
                "coverage_percent": file_cov.combined_coverage_percent(),
                "line_coverage_percent": file_cov.line_coverage_percent(),
                "branch_coverage_percent": file_cov.branch_coverage_percent(),
                "total_lines": file_cov.total_lines,
                "covered_lines": file_cov.covered_lines,
                "total_branches": file_cov.total_branches,
                "covered_branches": file_cov.covered_branches,
                "line_coverage": line_coverage
            }));
        }

        let json = serde_json::json!({
            "version": "0.1.0",
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "generator": "soli_lang coverage",
            "summary": {
                "total_lines": coverage.total_lines(),
                "covered_lines": coverage.covered_lines(),
                "coverage_percent": coverage.total_line_coverage_percent(),
                "total_files": coverage.file_coverages.len(),
                "test_count": coverage.test_count,
                "passed_count": coverage.passed_count,
                "failed_count": coverage.failed_count
            },
            "files": files
        });

        serde_json::to_string_pretty(&json).unwrap_or_default()
    }

    pub fn generate_xml_report(&self, coverage: &AggregatedCoverage) {
        let xml = self.generate_xml_output(coverage);
        let _ = fs::create_dir_all(&self.config.output_dir);
        let _ = fs::write(self.config.output_dir.join("cobertura.xml"), xml);
    }

    fn generate_xml_output(&self, coverage: &AggregatedCoverage) -> String {
        let mut packages = String::new();

        let mut file_groups: HashMap<String, Vec<&FileCoverage>> = HashMap::new();
        for (path, file_cov) in &coverage.file_coverages {
            if let Some(parent) = path.parent() {
                let key = parent.to_string_lossy();
                file_groups
                    .entry(key.to_string())
                    .or_default()
                    .push(file_cov);
            }
        }

        for (dir_path, files) in file_groups {
            let mut classes = String::new();
            for file_cov in files {
                let file_name = file_cov
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                classes.push_str(&format!(
                    r#"    <class name="{}" filename="{}" line-rate="{:.3}" branch-rate="{:.3}">
        <lines>
"#,
                    file_name,
                    file_cov.path.to_string_lossy(),
                    file_cov.line_coverage_percent() / 100.0,
                    file_cov.branch_coverage_percent() / 100.0
                ));

                for (line_num, line_cov) in &file_cov.lines {
                    classes.push_str(&format!(
                        r#"            <line number="{}" hits="{}"/>
"#,
                        line_num, line_cov.hits
                    ));
                }

                classes.push_str("        </lines>\n    </class>\n");
            }

            packages.push_str(&format!(
                r#"  <package name="{}" line-rate="{:.3}" branch-rate="{:.3}">
{}
  </package>
"#,
                dir_path,
                coverage.total_line_coverage_percent() / 100.0,
                coverage.total_branch_coverage_percent() / 100.0,
                classes
            ));
        }

        format!(
            r#"<?xml version="1.0" ?>
<coverage version="5.5" timestamp="{}" lines-valid="{}" lines-covered="{}" line-rate="{:.3}" branches-covered="0" branches-valid="0" branch-rate="0" complexity="0">
  <packages>
{}
  </packages>
</coverage>"#,
            chrono::Utc::now().timestamp(),
            coverage.total_lines(),
            coverage.covered_lines(),
            coverage.total_line_coverage_percent() / 100.0,
            packages
        )
    }

    pub fn check_threshold(&self, coverage: &AggregatedCoverage) -> bool {
        if let Some(threshold) = self.config.threshold {
            return coverage.total_line_coverage_percent() >= threshold;
        }
        true
    }
}
