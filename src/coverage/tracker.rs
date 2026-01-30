use crate::ast::expr::Argument;
use crate::coverage::data::*;
use crate::lexer::Scanner;
use crate::parser::Parser;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

thread_local! {
    static CURRENT_COVERAGE: RefCell<Option<TestCoverage>> = const { RefCell::new(None) };
}

pub struct CoverageTracker {
    global_coverage: Rc<RefCell<AggregatedCoverage>>,
    #[allow(dead_code)]
    config: CoverageConfig,
    executable_lines: HashMap<PathBuf, HashMap<usize, String>>,
}

impl CoverageTracker {
    pub fn new(config: CoverageConfig) -> Self {
        Self {
            global_coverage: Rc::new(RefCell::new(AggregatedCoverage::new())),
            config,
            executable_lines: HashMap::new(),
        }
    }

    pub fn register_executable_line(&mut self, path: &PathBuf, line: usize, source: String) {
        self.executable_lines
            .entry(path.clone())
            .or_default()
            .insert(line, source);
    }

    pub fn start_test(&self, test_name: &str) {
        CURRENT_COVERAGE.with(|cov| {
            *cov.borrow_mut() = Some(TestCoverage::new(test_name.to_string()));
        });
    }

    pub fn end_test(&mut self) -> Option<TestCoverage> {
        let mut result: Option<TestCoverage> = None;

        CURRENT_COVERAGE.with(|cov| {
            if let Some(test_cov) = cov.borrow_mut().take() {
                let mut aggregated = self.global_coverage.borrow_mut();

                for (path, file_cov) in &test_cov.file_coverages {
                    let aggregated_file = aggregated
                        .file_coverages
                        .entry(path.clone())
                        .or_insert_with(|| FileCoverage {
                            path: path.clone(),
                            lines: HashMap::new(),
                            branches: HashMap::new(),
                            total_lines: 0,
                            covered_lines: 0,
                            total_branches: 0,
                            covered_branches: 0,
                        });

                    for (line_num, line_cov) in &file_cov.lines {
                        let is_executable = self
                            .executable_lines
                            .get(&file_cov.path)
                            .and_then(|lines| lines.get(line_num))
                            .is_some();

                        let aggregated_line = aggregated_file
                            .lines
                            .entry(*line_num)
                            .or_insert_with(|| LineCoverage {
                                line_number: *line_num,
                                hits: 0,
                                source_code: line_cov.source_code.clone(),
                                is_executable,
                            });

                        aggregated_line.hits += line_cov.hits;
                    }

                    for (line_num, branch_cov) in &file_cov.branches {
                        let aggregated_branch = aggregated_file
                            .branches
                            .entry(*line_num)
                            .or_insert_with(|| BranchCoverage {
                                line_number: *line_num,
                                branch_type: branch_cov.branch_type.clone(),
                                hits_true: 0,
                                hits_false: 0,
                            });

                        aggregated_branch.hits_true += branch_cov.hits_true;
                        aggregated_branch.hits_false += branch_cov.hits_false;

                        if (aggregated_branch.hits_true > 0 || aggregated_branch.hits_false > 0)
                            && aggregated_branch.hits_true == branch_cov.hits_true
                            && aggregated_branch.hits_false == branch_cov.hits_false
                        {
                            aggregated_file.covered_branches += 1;
                        }

                        aggregated_file.total_branches += 1;
                    }
                }

                for (path, executable) in &self.executable_lines {
                    let aggregated_file = aggregated
                        .file_coverages
                        .entry(path.clone())
                        .or_insert_with(|| FileCoverage {
                            path: path.clone(),
                            lines: HashMap::new(),
                            branches: HashMap::new(),
                            total_lines: 0,
                            covered_lines: 0,
                            total_branches: 0,
                            covered_branches: 0,
                        });

                    aggregated_file.total_lines = executable.len() as u32;
                    for (line_num, source) in executable.iter() {
                        let hits = aggregated_file
                            .lines
                            .get(line_num)
                            .map(|l| l.hits)
                            .unwrap_or(0);
                        let is_executable = true;
                        aggregated_file
                            .lines
                            .entry(*line_num)
                            .or_insert_with(|| LineCoverage {
                                line_number: *line_num,
                                hits,
                                source_code: source.clone(),
                                is_executable,
                            });
                        if hits > 0 {
                            aggregated_file.covered_lines += 1;
                        }
                    }
                }

                result = Some(test_cov);
            }
        });

        result
    }

    pub fn record_line_hit(&self, path: &PathBuf, line: usize) {
        CURRENT_COVERAGE.with(|cov| {
            if let Some(ref mut test_cov) = *cov.borrow_mut() {
                let file_cov = test_cov
                    .file_coverages
                    .entry(path.clone())
                    .or_insert_with(|| FileCoverage {
                        path: path.clone(),
                        lines: HashMap::new(),
                        branches: HashMap::new(),
                        total_lines: 0,
                        covered_lines: 0,
                        total_branches: 0,
                        covered_branches: 0,
                    });

                let line_cov = file_cov.lines.entry(line).or_insert_with(|| {
                    let source = self
                        .executable_lines
                        .get(path)
                        .and_then(|lines| lines.get(&line))
                        .cloned()
                        .unwrap_or_default();

                    LineCoverage {
                        line_number: line,
                        hits: 0,
                        source_code: source,
                        is_executable: true,
                    }
                });

                line_cov.hits += 1;
            }
        });
    }

    pub fn record_branch(
        &self,
        path: &PathBuf,
        line: usize,
        branch_type: BranchType,
        taken_true: bool,
    ) {
        CURRENT_COVERAGE.with(|cov| {
            if let Some(ref mut test_cov) = *cov.borrow_mut() {
                let file_cov = test_cov
                    .file_coverages
                    .entry(path.clone())
                    .or_insert_with(|| FileCoverage {
                        path: path.clone(),
                        lines: HashMap::new(),
                        branches: HashMap::new(),
                        total_lines: 0,
                        covered_lines: 0,
                        total_branches: 0,
                        covered_branches: 0,
                    });

                let branch_cov = file_cov
                    .branches
                    .entry(line)
                    .or_insert_with(|| BranchCoverage {
                        line_number: line,
                        branch_type,
                        hits_true: 0,
                        hits_false: 0,
                    });

                if taken_true {
                    branch_cov.hits_true += 1;
                } else {
                    branch_cov.hits_false += 1;
                }
            }
        });
    }

    pub fn get_aggregated_coverage(&self) -> AggregatedCoverage {
        self.global_coverage.borrow().clone()
    }

    pub fn merge_test_coverage(&mut self, test_cov: TestCoverage) {
        let mut aggregated = self.global_coverage.borrow_mut();

        for (path, file_cov) in test_cov.file_coverages {
            let aggregated_file =
                aggregated
                    .file_coverages
                    .entry(path)
                    .or_insert_with(|| FileCoverage {
                        path: file_cov.path.clone(),
                        lines: HashMap::new(),
                        branches: HashMap::new(),
                        total_lines: 0,
                        covered_lines: 0,
                        total_branches: 0,
                        covered_branches: 0,
                    });

            for (line_num, line_cov) in file_cov.lines {
                let aggregated_line =
                    aggregated_file
                        .lines
                        .entry(line_num)
                        .or_insert_with(|| LineCoverage {
                            line_number: line_num,
                            hits: 0,
                            source_code: line_cov.source_code.clone(),
                            is_executable: line_cov.is_executable,
                        });

                aggregated_line.hits += line_cov.hits;

                if aggregated_line.hits > 0 && line_cov.hits > 0 && aggregated_line.is_executable {
                    aggregated_file.covered_lines += 1;
                }

                if aggregated_line.is_executable {
                    aggregated_file.total_lines += 1;
                }
            }

            for (line_num, branch_cov) in file_cov.branches {
                let aggregated_branch =
                    aggregated_file
                        .branches
                        .entry(line_num)
                        .or_insert_with(|| BranchCoverage {
                            line_number: line_num,
                            branch_type: branch_cov.branch_type.clone(),
                            hits_true: 0,
                            hits_false: 0,
                        });

                aggregated_branch.hits_true += branch_cov.hits_true;
                aggregated_branch.hits_false += branch_cov.hits_false;

                if aggregated_branch.hits_true > 0 || aggregated_branch.hits_false > 0 {
                    aggregated_file.covered_branches += 1;
                }

                aggregated_file.total_branches += 1;
            }
        }
    }

    pub fn reset(&mut self) {
        self.global_coverage = Rc::new(RefCell::new(AggregatedCoverage::new()));
    }
}

pub fn with_test_coverage<F, R>(test_name: &str, f: F) -> R
where
    F: FnOnce() -> R,
{
    CURRENT_COVERAGE.with(|cov| {
        *cov.borrow_mut() = Some(TestCoverage::new(test_name.to_string()));
    });

    f()
}

pub fn current_coverage<F, R>(f: F) -> R
where
    F: FnOnce(Option<&TestCoverage>) -> R,
{
    CURRENT_COVERAGE.with(|cov| f(cov.borrow().as_ref()))
}

impl CoverageTracker {
    pub fn register_executable_lines_from_source(&mut self, path: &PathBuf, source: &str) {
        let tokens = Scanner::new(source).scan_tokens();
        if tokens.is_err() {
            return;
        }

        let parse_result = Parser::new(tokens.unwrap()).parse();
        if parse_result.is_err() {
            return;
        }

        let program = parse_result.unwrap();
        self.collect_lines_from_program(path, source, &program);
    }

    fn collect_lines_from_program(
        &mut self,
        path: &PathBuf,
        source: &str,
        program: &crate::ast::Program,
    ) {
        let lines: Vec<&str> = source.lines().collect();
        for stmt in &program.statements {
            self.collect_lines_from_stmt(path, &lines, stmt);
        }
    }

    fn collect_lines_from_stmt(&mut self, path: &PathBuf, lines: &[&str], stmt: &crate::ast::Stmt) {
        let line_num = stmt.span.line;
        if line_num > 0 && line_num <= lines.len() {
            let source_line = lines[line_num - 1].to_string();
            self.register_executable_line(path, line_num, source_line);
        }

        use crate::ast::StmtKind::*;
        match &stmt.kind {
            Expression(expr) => self.collect_lines_from_expr(path, lines, expr),
            Let { initializer, .. } => {
                if let Some(expr) = initializer {
                    self.collect_lines_from_expr(path, lines, expr);
                }
            }
            Const { initializer, .. } => {
                self.collect_lines_from_expr(path, lines, initializer);
            }
            Block(stmts) => {
                for s in stmts {
                    self.collect_lines_from_stmt(path, lines, s);
                }
            }
            If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.collect_lines_from_expr(path, lines, condition);
                self.collect_lines_from_stmt(path, lines, then_branch);
                if let Some(else_stmt) = else_branch {
                    self.collect_lines_from_stmt(path, lines, else_stmt);
                }
            }
            While { condition, body } => {
                self.collect_lines_from_expr(path, lines, condition);
                self.collect_lines_from_stmt(path, lines, body);
            }
            For {
                variable: _,
                iterable,
                body,
            } => {
                self.collect_lines_from_expr(path, lines, iterable);
                self.collect_lines_from_stmt(path, lines, body);
            }
            Return(expr) => {
                if let Some(e) = expr {
                    self.collect_lines_from_expr(path, lines, e);
                }
            }
            Throw(expr) => {
                self.collect_lines_from_expr(path, lines, expr);
            }
            Try {
                try_block,
                catch_block,
                finally_block,
                ..
            } => {
                self.collect_lines_from_stmt(path, lines, try_block);
                if let Some(catch) = catch_block {
                    self.collect_lines_from_stmt(path, lines, catch);
                }
                if let Some(finally) = finally_block {
                    self.collect_lines_from_stmt(path, lines, finally);
                }
            }
            Function(decl) => {
                for stmt in &decl.body {
                    self.collect_lines_from_stmt(path, lines, stmt);
                }
            }
            Class(decl) => {
                for stmt in &decl.class_statements {
                    self.collect_lines_from_stmt(path, lines, stmt);
                }
                if let Some(ctor) = &decl.constructor {
                    for stmt in &ctor.body {
                        self.collect_lines_from_stmt(path, lines, stmt);
                    }
                }
                for method in &decl.methods {
                    for stmt in &method.body {
                        self.collect_lines_from_stmt(path, lines, stmt);
                    }
                }
                for field in &decl.fields {
                    if let Some(init) = &field.initializer {
                        self.collect_lines_from_expr(path, lines, init);
                    }
                }
            }
            Interface(_) | Import(_) | Export(_) => {}
        }
    }

    fn collect_lines_from_expr(&mut self, path: &PathBuf, lines: &[&str], expr: &crate::ast::Expr) {
        let line_num = expr.span.line;
        if line_num > 0 && line_num <= lines.len() {
            let source_line = lines[line_num - 1].to_string();
            self.register_executable_line(path, line_num, source_line);
        }

        use crate::ast::ExprKind::*;
        match &expr.kind {
            Binary { left, right, .. } => {
                self.collect_lines_from_expr(path, lines, left);
                self.collect_lines_from_expr(path, lines, right);
            }
            Unary { operand, .. } => {
                self.collect_lines_from_expr(path, lines, operand);
            }
            IntLiteral(_) | FloatLiteral(_) | StringLiteral(_) | BoolLiteral(_) | Null => {}
            Variable(_) => {}
            Assign { value, .. } => {
                self.collect_lines_from_expr(path, lines, value);
            }
            Call { arguments, .. } => {
                for arg in arguments {
                    match arg {
                        Argument::Positional(expr) => {
                            self.collect_lines_from_expr(path, lines, expr);
                        }
                        Argument::Named(named) => {
                            self.collect_lines_from_expr(path, lines, &named.value);
                        }
                    }
                }
            }
            Member { object, .. } => {
                self.collect_lines_from_expr(path, lines, object);
            }
            Index { object, index, .. } => {
                self.collect_lines_from_expr(path, lines, object);
                self.collect_lines_from_expr(path, lines, index);
            }
            This | Super => {}
            New { arguments, .. } => {
                for arg in arguments {
                    match arg {
                        Argument::Positional(expr) => {
                            self.collect_lines_from_expr(path, lines, expr);
                        }
                        Argument::Named(named) => {
                            self.collect_lines_from_expr(path, lines, &named.value);
                        }
                    }
                }
            }
            Array(elements) => {
                for elem in elements {
                    self.collect_lines_from_expr(path, lines, elem);
                }
            }
            Hash(pairs) => {
                for (_, value) in pairs {
                    self.collect_lines_from_expr(path, lines, value);
                }
            }
            LogicalAnd { left, right } | LogicalOr { left, right } => {
                self.collect_lines_from_expr(path, lines, left);
                self.collect_lines_from_expr(path, lines, right);
            }
            NullishCoalescing { left, right } => {
                self.collect_lines_from_expr(path, lines, left);
                self.collect_lines_from_expr(path, lines, right);
            }
            Lambda { body, .. } => {
                for stmt in body {
                    self.collect_lines_from_stmt(path, lines, stmt);
                }
            }
            Grouping(expr) => {
                self.collect_lines_from_expr(path, lines, expr);
            }
            If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.collect_lines_from_expr(path, lines, condition);
                self.collect_lines_from_expr(path, lines, then_branch);
                if let Some(else_expr) = else_branch {
                    self.collect_lines_from_expr(path, lines, else_expr);
                }
            }
            Match {
                expression, arms, ..
            } => {
                self.collect_lines_from_expr(path, lines, expression);
                for arm in arms {
                    self.collect_lines_from_expr(path, lines, &arm.body);
                }
            }
            Pipeline { left, right, .. } => {
                self.collect_lines_from_expr(path, lines, left);
                self.collect_lines_from_expr(path, lines, right);
            }
            ListComprehension {
                element,
                iterable,
                condition,
                ..
            } => {
                self.collect_lines_from_expr(path, lines, element);
                self.collect_lines_from_expr(path, lines, iterable);
                if let Some(cond) = condition {
                    self.collect_lines_from_expr(path, lines, cond);
                }
            }
            HashComprehension {
                key,
                value,
                iterable,
                condition,
                ..
            } => {
                self.collect_lines_from_expr(path, lines, key);
                self.collect_lines_from_expr(path, lines, value);
                self.collect_lines_from_expr(path, lines, iterable);
                if let Some(cond) = condition {
                    self.collect_lines_from_expr(path, lines, cond);
                }
            }
            Await(expr) => {
                self.collect_lines_from_expr(path, lines, expr);
            }
            Spread(expr) => {
                self.collect_lines_from_expr(path, lines, expr);
            }
            Throw(expr) => {
                self.collect_lines_from_expr(path, lines, expr);
            }
            InterpolatedString(parts) => {
                for part in parts {
                    if let crate::ast::expr::InterpolatedPart::Expression(expr) = part {
                        self.collect_lines_from_expr(path, lines, expr);
                    }
                }
            }
        }
    }
}
