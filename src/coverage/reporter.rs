use crate::coverage::data::*;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const VERSION: &str = env!("CARGO_PKG_VERSION", "0.2.0");

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
                    if !output.is_empty() {
                        println!("{}", output);
                    }
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

        let total_color = ansi_coverage_color(total_percent);
        output.push_str(&format!(
            "\nCoverage: {}{:.1}%\x1b[0m ({}/{}{}) {}\n",
            total_color,
            total_percent,
            covered_lines,
            total_lines,
            if let Some(threshold) = self.config.threshold {
                format!(", threshold: {:.0}%", threshold)
            } else {
                String::new()
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

        // Pre-compute display paths and the column width so the bar and
        // percentage align across rows, even when some filenames are longer
        // than the default padding.
        let rows: Vec<(String, &FileCoverage)> = file_data
            .iter()
            .map(|(path, file_cov)| {
                let display_path = if let Some(ref root) = self.config.root_dir {
                    path.strip_prefix(root)
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|_| path.to_string_lossy().to_string())
                } else {
                    path.to_string_lossy().to_string()
                };
                (display_path, *file_cov)
            })
            .collect();
        let name_width = rows
            .iter()
            .map(|(p, _)| p.chars().count())
            .max()
            .unwrap_or(40)
            .max(40);

        for (display_path, file_cov) in &rows {
            let percent = file_cov.combined_coverage_percent();
            let bar = self.progress_bar(percent);
            let pad = name_width.saturating_sub(display_path.chars().count());
            let color = ansi_coverage_color(percent);

            output.push_str(&format!(
                "  {}{} {} {}{:>6.1}%\x1b[0m\n",
                display_path,
                " ".repeat(pad),
                bar,
                color,
                percent
            ));
        }

        if self.config.show_uncovered {
            let uncovered = coverage.uncovered_lines();
            if !uncovered.is_empty() {
                output.push_str("\nUncovered lines:\n");
                for uncov in uncovered.iter().take(20) {
                    let display_path = if let Some(ref root) = self.config.root_dir {
                        uncov
                            .path
                            .strip_prefix(root)
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_else(|_| uncov.path.to_string_lossy().to_string())
                    } else {
                        uncov.path.to_string_lossy().to_string()
                    };
                    output.push_str(&format!(
                        "  {}:{} ({}))\n",
                        display_path,
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
        let color = ansi_coverage_color(percent);

        let mut bar = String::new();
        bar.push_str(color);
        for _ in 0..filled {
            bar.push('▓');
        }
        bar.push_str("\x1b[90m");
        for _ in 0..empty {
            bar.push('░');
        }
        bar.push_str("\x1b[0m");
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

    fn write_html_assets(&self, assets_dir: &Path) {
        let style = r#"* { margin: 0; padding: 0; box-sizing: border-box; }
body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #f8fafc; color: #1e293b; }
.container { max-width: 1200px; margin: 0 auto; padding: 24px; }
header { margin-bottom: 32px; }
h1 { font-size: 24px; font-weight: 600; margin-bottom: 16px; }
.back-link { display: inline-block; margin-top: 8px; font-size: 14px; color: #6366f1; text-decoration: none; }
.back-link:hover { text-decoration: underline; }
.summary { display: flex; gap: 16px; }
.summary-card { background: white; padding: 16px 24px; border-radius: 8px; box-shadow: 0 1px 3px rgba(0,0,0,0.1); text-align: center; }
.summary-value { font-size: 32px; font-weight: 700; display: block; }
.summary-label { font-size: 14px; color: #64748b; }
table { width: 100%; background: white; border-radius: 8px; box-shadow: 0 1px 3px rgba(0,0,0,0.1); border-collapse: collapse; }
th, td { padding: 12px 16px; text-align: left; border-bottom: 1px solid #e2e8f0; }
th { background: #f1f5f9; font-weight: 600; font-size: 14px; }
.clickable-row { cursor: pointer; transition: background 0.15s; }
.clickable-row:hover { background: #f8fafc; }
.coverage-pill { padding: 4px 12px; border-radius: 999px; font-size: 12px; font-weight: 600; color: white; }

/* Source code view */
.source-view { background: #0f172a; color: #e2e8f0; border-radius: 8px; overflow: hidden; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }
.source-view table { background: transparent; box-shadow: none; border-radius: 0; }
.source-view th, .source-view td { border-bottom: none; padding: 0; }
.code-line td { padding: 0 0 0 0; }
.code-line { font-family: 'SF Mono', 'Monaco', 'Menlo', 'Consolas', 'Liberation Mono', monospace; font-size: 13px; line-height: 20px; }
.code-line:hover { background: #1e293b; }
.line-num { padding: 0 12px 0 16px; color: #64748b; text-align: right; user-select: none; width: 60px; min-width: 60px; border-right: 1px solid #1e293b; background: #0b1220; }
.line-indicator { padding: 0 8px; width: 28px; text-align: center; font-weight: 700; user-select: none; }
.code-line.covered .line-indicator { color: #22c55e; }
.code-line.uncovered .line-indicator { color: #ef4444; }
.code-line.covered { background: rgba(34, 197, 94, 0.08); }
.code-line.uncovered { background: rgba(239, 68, 68, 0.12); }
.code-line.covered:hover { background: rgba(34, 197, 94, 0.14); }
.code-line.uncovered:hover { background: rgba(239, 68, 68, 0.20); }
.code-content { padding: 0 16px; width: 100%; }
.code-content pre { white-space: pre; font-family: inherit; font-size: inherit; line-height: inherit; margin: 0; overflow-x: auto; }

/* Syntax highlighting (Monokai-ish) */
.tok-kw      { color: #f472b6; font-weight: 600; }
.tok-type    { color: #60a5fa; }
.tok-ident   { color: #e2e8f0; }
.tok-num     { color: #fbbf24; }
.tok-str     { color: #86efac; }
.tok-bool    { color: #fbbf24; font-weight: 600; }
.tok-null    { color: #fbbf24; font-style: italic; }
.tok-op      { color: #f87171; }
.tok-punct   { color: #cbd5e1; }
.tok-comment { color: #64748b; font-style: italic; }
"#;

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

    fn write_html_source_files(&self, coverage: &AggregatedCoverage, output_dir: &Path) {
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

            let highlighted_line = html_highlight_soli(line);

            line_rows.push_str(&format!(
                r#"<tr class="{}">
                    <td class="line-num">{}</td>
                    <td class="line-indicator">{}</td>
                    <td class="code-content"><pre>{}</pre></td>
                </tr>"#,
                row_class, line_num, coverage_indicator, highlighted_line
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
    <style>{}</style>
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
            file_name,
            source_view_inline_css(),
            file_name,
            line_rows
        )
    }

    fn write_html_breakdown_json(&self, coverage: &AggregatedCoverage, output_dir: &Path) {
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
            "version": VERSION,
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

/// Map a coverage percentage to a green / orange / red ANSI color escape.
/// Mirrors the HTML pill thresholds: 80% green, 50% orange, below that red.
fn ansi_coverage_color(percent: f64) -> &'static str {
    if percent >= 80.0 {
        "\x1b[32m"
    } else if percent >= 50.0 {
        "\x1b[38;5;208m"
    } else {
        "\x1b[31m"
    }
}

/// Inline CSS embedded directly in each per-source HTML page. Duplicates the
/// visual parts of `assets/style.css` but avoids cache headaches — a browser
/// that once fetched the stale pre-highlight style.css would otherwise keep
/// serving it until the user hard-refreshes. Inlining sidesteps that entirely
/// and makes the file standalone (copy it anywhere, it renders).
fn source_view_inline_css() -> &'static str {
    r#"
* { margin: 0; padding: 0; box-sizing: border-box; }
body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #f8fafc; color: #1e293b; }
.container { max-width: 1200px; margin: 0 auto; padding: 24px; }
header { margin-bottom: 32px; }
h1 { font-size: 24px; font-weight: 600; margin-bottom: 16px; }
.back-link { display: inline-block; margin-top: 8px; font-size: 14px; color: #6366f1; text-decoration: none; }
.back-link:hover { text-decoration: underline; }

.source-view { background: #0f172a; color: #e2e8f0; border-radius: 8px; overflow: hidden; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }
.source-view table { width: 100%; background: transparent; border-collapse: collapse; }
.source-view th, .source-view td { border-bottom: none; padding: 0; text-align: left; }
.code-line { font-family: 'SF Mono', 'Monaco', 'Menlo', 'Consolas', 'Liberation Mono', monospace; font-size: 13px; line-height: 20px; }
.code-line:hover { background: #1e293b; }
.line-num { padding: 0 12px 0 16px; color: #64748b; text-align: right; user-select: none; width: 60px; min-width: 60px; border-right: 1px solid #1e293b; background: #0b1220; }
.line-indicator { padding: 0 8px; width: 28px; text-align: center; font-weight: 700; user-select: none; color: #64748b; }
.code-line.covered .line-indicator { color: #22c55e; }
.code-line.uncovered .line-indicator { color: #ef4444; }
.code-line.covered { background: rgba(34, 197, 94, 0.08); }
.code-line.uncovered { background: rgba(239, 68, 68, 0.12); }
.code-line.covered:hover { background: rgba(34, 197, 94, 0.14); }
.code-line.uncovered:hover { background: rgba(239, 68, 68, 0.20); }
.code-content { padding: 0 16px; width: 100%; }
.code-content pre { white-space: pre; font-family: inherit; font-size: inherit; line-height: inherit; margin: 0; overflow-x: auto; }

/* Syntax token palette */
.tok-kw      { color: #f472b6; font-weight: 600; }
.tok-type    { color: #60a5fa; }
.tok-ident   { color: #e2e8f0; }
.tok-num     { color: #fbbf24; }
.tok-str     { color: #86efac; }
.tok-bool    { color: #fbbf24; font-weight: 600; }
.tok-null    { color: #fbbf24; font-style: italic; }
.tok-op      { color: #f87171; }
.tok-punct   { color: #cbd5e1; }
.tok-comment { color: #64748b; font-style: italic; }
"#
}

/// Tokenise a line of Soli source and wrap each token in a `<span class="tok-*">`
/// so the HTML coverage view can style it. Keeps leading whitespace intact and
/// HTML-escapes everything that's passed through to the output.
pub(crate) fn html_highlight_soli(line: &str) -> String {
    use crate::lexer::token::TokenKind;
    use crate::lexer::Scanner;

    let tokens = match Scanner::new(line).scan_tokens() {
        Ok(t) => t,
        Err(_) => return escape_html(line),
    };

    let mut out = String::with_capacity(line.len() * 2);
    let mut cursor = 0usize;
    for tok in tokens {
        if matches!(tok.kind, TokenKind::Eof) {
            break;
        }
        let start = tok.span.start.min(line.len());
        let end = tok.span.end.min(line.len());
        if start < cursor || start > line.len() {
            continue;
        }
        // Emit any inter-token whitespace/comments unchanged (escaped).
        if start > cursor {
            let gap = &line[cursor..start];
            out.push_str(&wrap_inter_token(gap));
        }
        if end <= start {
            cursor = start;
            continue;
        }
        let text = &line[start..end];
        let class = token_class(&tok.kind);
        if class.is_empty() {
            out.push_str(&escape_html(text));
        } else {
            out.push_str(&format!(
                "<span class=\"{}\">{}</span>",
                class,
                escape_html(text)
            ));
        }
        cursor = end;
    }
    if cursor < line.len() {
        out.push_str(&wrap_inter_token(&line[cursor..]));
    }
    out
}

/// The lexer skips comments, so anything between tokens that contains `#` is
/// almost certainly a line-comment. Style it differently.
fn wrap_inter_token(s: &str) -> String {
    if let Some(idx) = s.find('#') {
        let (before, comment) = s.split_at(idx);
        format!(
            "{}<span class=\"tok-comment\">{}</span>",
            escape_html(before),
            escape_html(comment)
        )
    } else {
        escape_html(s)
    }
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn token_class(kind: &crate::lexer::token::TokenKind) -> &'static str {
    use crate::lexer::token::TokenKind::*;
    match kind {
        IntLiteral(_) | FloatLiteral(_) | DecimalLiteral(_) => "tok-num",
        StringLiteral(_) | InterpolatedString(_) | BacktickString(_) => "tok-str",
        BoolLiteral(_) => "tok-bool",
        Null => "tok-null",
        Let | Const | Fn | Return | If | Else | Elsif | While | For | In | Class | Extends
        | Implements | Interface | New | This | SelfKeyword | Super | Public | Private
        | Protected | Static | Try | Catch | Finally | Throw | Not | Async | Await | Match
        | Case | When | Do | End | Unless | Then | Import | Export | From | As | Int | Float
        | Bool | String | Void | Decimal => "tok-kw",
        Plus | Minus | Star | Slash | Percent | Equal | EqualEqual | BangEqual | Less
        | LessEqual | Greater | GreaterEqual | Bang | And | Or | Pipeline | Pipe
        | NullishCoalescing | SafeNavigation | DoubleColon | Arrow | FatArrow | Spread | Range
        | PlusPlus | MinusMinus | PlusEqual | MinusEqual | StarEqual | SlashEqual
        | PercentEqual => "tok-op",
        LeftParen | RightParen | LeftBrace | RightBrace | LeftBracket | RightBracket | Comma
        | Dot | Colon | Semicolon | Question => "tok-punct",
        Identifier(name) => {
            // PascalCase names are conventionally classes/types.
            if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                "tok-type"
            } else {
                "tok-ident"
            }
        }
        _ => "",
    }
}
