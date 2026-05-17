//! Symbol table for LSP, built on top of the existing type system.

use crate::span::Span;

#[derive(Debug, Clone)]
pub enum SymbolKind {
    Variable,
    Function,
    Class,
    Parameter,
    Property,
    Method,
    Constant,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub span: Span,
    pub type_name: Option<String>,
    pub scope_level: usize,
}

#[derive(Debug, Clone)]
pub struct ScopedSymbol {
    pub symbol: Symbol,
    pub scope_start: usize,
    pub scope_end: usize,
}

#[derive(Debug, Clone, Default)]
pub struct SymbolTable {
    pub symbols: Vec<ScopedSymbol>,
    pub scope_starts: Vec<usize>,
    pub scope_ends: Vec<usize>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn find_at_position(&self, pos: usize) -> Option<&ScopedSymbol> {
        self.symbols
            .iter()
            .find(|s| s.symbol.span.start <= pos && pos <= s.symbol.span.end)
    }

    pub fn find_by_name(&self, name: &str) -> Vec<&ScopedSymbol> {
        self.symbols
            .iter()
            .filter(|s| s.symbol.name == name)
            .collect()
    }

    pub fn get_in_scope(&self, scope_level: usize, name: &str) -> Option<&ScopedSymbol> {
        self.symbols
            .iter()
            .find(|s| s.symbol.scope_level == scope_level && s.symbol.name == name)
    }
}

pub fn build_symbol_table(source: &str) -> Option<SymbolTable> {
    let tokens = crate::lexer::Scanner::new(source).scan_tokens().ok()?;
    let program = crate::parser::Parser::new(tokens).parse().ok()?;

    let mut table = SymbolTable::default();
    let mut scope_level = 0;

    build_symbols_recursive(&program.statements, &mut table, &mut scope_level);

    Some(table)
}

fn build_symbols_recursive(
    statements: &[crate::ast::Stmt],
    table: &mut SymbolTable,
    scope_level: &mut usize,
) {
    for stmt in statements {
        match &stmt.kind {
            crate::ast::StmtKind::Let {
                name, initializer, ..
            } => {
                table.symbols.push(ScopedSymbol {
                    symbol: Symbol {
                        name: name.clone(),
                        kind: SymbolKind::Variable,
                        span: stmt.span,
                        type_name: None,
                        scope_level: *scope_level,
                    },
                    scope_start: stmt.span.start,
                    scope_end: stmt.span.end,
                });
                if let Some(expr) = initializer {
                    extract_symbols_from_expr(expr, table, *scope_level);
                }
            }
            crate::ast::StmtKind::Const {
                name, initializer, ..
            } => {
                table.symbols.push(ScopedSymbol {
                    symbol: Symbol {
                        name: name.clone(),
                        kind: SymbolKind::Constant,
                        span: stmt.span,
                        type_name: None,
                        scope_level: *scope_level,
                    },
                    scope_start: stmt.span.start,
                    scope_end: stmt.span.end,
                });
                extract_symbols_from_expr(initializer, table, *scope_level);
            }
            crate::ast::StmtKind::Function(func_decl) => {
                table.symbols.push(ScopedSymbol {
                    symbol: Symbol {
                        name: func_decl.name.clone(),
                        kind: SymbolKind::Function,
                        span: stmt.span,
                        type_name: None,
                        scope_level: *scope_level,
                    },
                    scope_start: stmt.span.start,
                    scope_end: stmt.span.end,
                });

                *scope_level += 1;
                for param in &func_decl.params {
                    table.symbols.push(ScopedSymbol {
                        symbol: Symbol {
                            name: param.name.clone(),
                            kind: SymbolKind::Parameter,
                            span: param.span,
                            type_name: Some(format!("{:?}", param.type_annotation)),
                            scope_level: *scope_level,
                        },
                        scope_start: param.span.start,
                        scope_end: param.span.end,
                    });
                }
                build_symbols_recursive(&func_decl.body, table, scope_level);
                *scope_level -= 1;
            }
            crate::ast::StmtKind::Class(class_decl) => {
                table.symbols.push(ScopedSymbol {
                    symbol: Symbol {
                        name: class_decl.name.clone(),
                        kind: SymbolKind::Class,
                        span: stmt.span,
                        type_name: Some(class_decl.name.clone()),
                        scope_level: *scope_level,
                    },
                    scope_start: stmt.span.start,
                    scope_end: stmt.span.end,
                });

                *scope_level += 1;
                for field in &class_decl.fields {
                    table.symbols.push(ScopedSymbol {
                        symbol: Symbol {
                            name: field.name.clone(),
                            kind: SymbolKind::Property,
                            span: field.span,
                            type_name: Some(format!("{:?}", field.type_annotation)),
                            scope_level: *scope_level,
                        },
                        scope_start: field.span.start,
                        scope_end: field.span.end,
                    });
                }
                for method in &class_decl.methods {
                    table.symbols.push(ScopedSymbol {
                        symbol: Symbol {
                            name: method.name.clone(),
                            kind: SymbolKind::Method,
                            span: method.span,
                            type_name: None,
                            scope_level: *scope_level,
                        },
                        scope_start: method.span.start,
                        scope_end: method.span.end,
                    });

                    *scope_level += 1;
                    for param in &method.params {
                        table.symbols.push(ScopedSymbol {
                            symbol: Symbol {
                                name: param.name.clone(),
                                kind: SymbolKind::Parameter,
                                span: param.span,
                                type_name: Some(format!("{:?}", param.type_annotation)),
                                scope_level: *scope_level,
                            },
                            scope_start: param.span.start,
                            scope_end: param.span.end,
                        });
                    }
                    build_symbols_recursive(&method.body, table, scope_level);
                    *scope_level -= 1;
                }
                *scope_level -= 1;
            }
            crate::ast::StmtKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                build_symbols_recursive(std::slice::from_ref(&*then_branch), table, scope_level);
                if let Some(else_stmt) = else_branch {
                    build_symbols_recursive(std::slice::from_ref(&*else_stmt), table, scope_level);
                }
            }
            crate::ast::StmtKind::While { body, .. } => {
                *scope_level += 1;
                build_symbols_recursive(std::slice::from_ref(&*body), table, scope_level);
                *scope_level -= 1;
            }
            crate::ast::StmtKind::For { body, .. } => {
                *scope_level += 1;
                build_symbols_recursive(std::slice::from_ref(&*body), table, scope_level);
                *scope_level -= 1;
            }
            crate::ast::StmtKind::Block(statements) => {
                build_symbols_recursive(statements, table, scope_level);
            }
            crate::ast::StmtKind::Expression(expr) => {
                extract_symbols_from_expr(expr, table, *scope_level);
            }
            _ => {}
        }
    }
}

fn extract_symbols_from_expr(expr: &crate::ast::Expr, table: &mut SymbolTable, scope_level: usize) {
    match &expr.kind {
        crate::ast::ExprKind::Lambda { params, body, .. } => {
            let mut inner_level = scope_level + 1;
            for param in params {
                table.symbols.push(ScopedSymbol {
                    symbol: Symbol {
                        name: param.name.clone(),
                        kind: SymbolKind::Parameter,
                        span: param.span,
                        type_name: Some(format!("{:?}", param.type_annotation)),
                        scope_level: inner_level,
                    },
                    scope_start: param.span.start,
                    scope_end: param.span.end,
                });
            }
            build_symbols_recursive(body, table, &mut inner_level);
        }
        crate::ast::ExprKind::Call { callee, arguments } => {
            extract_symbols_from_expr(callee, table, scope_level);
            for arg in arguments {
                match arg {
                    crate::ast::expr::Argument::Positional(expr) => {
                        extract_symbols_from_expr(expr, table, scope_level);
                    }
                    crate::ast::expr::Argument::Named(named) => {
                        extract_symbols_from_expr(&named.value, table, scope_level);
                    }
                    crate::ast::expr::Argument::Block(expr) => {
                        extract_symbols_from_expr(expr, table, scope_level);
                    }
                }
            }
        }
        crate::ast::ExprKind::Member { object, .. } => {
            extract_symbols_from_expr(object, table, scope_level);
        }
        crate::ast::ExprKind::Binary { left, right, .. } => {
            extract_symbols_from_expr(left, table, scope_level);
            extract_symbols_from_expr(right, table, scope_level);
        }
        crate::ast::ExprKind::Unary { operand, .. } => {
            extract_symbols_from_expr(operand, table, scope_level);
        }
        crate::ast::ExprKind::Assign { target, value, .. } => {
            extract_symbols_from_expr(target, table, scope_level);
            extract_symbols_from_expr(value, table, scope_level);
        }
        crate::ast::ExprKind::CompoundAssign { target, value, .. } => {
            extract_symbols_from_expr(target, table, scope_level);
            extract_symbols_from_expr(value, table, scope_level);
        }
        crate::ast::ExprKind::Index { object, index, .. } => {
            extract_symbols_from_expr(object, table, scope_level);
            extract_symbols_from_expr(index, table, scope_level);
        }
        crate::ast::ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            extract_symbols_from_expr(condition, table, scope_level);
            extract_symbols_from_expr(then_branch, table, scope_level);
            if let Some(else_e) = else_branch {
                extract_symbols_from_expr(else_e, table, scope_level);
            }
        }
        crate::ast::ExprKind::Match {
            expression, arms, ..
        } => {
            extract_symbols_from_expr(expression, table, scope_level);
            for arm in arms {
                extract_symbols_from_expr(&arm.body, table, scope_level);
            }
        }
        crate::ast::ExprKind::Block(statements) => {
            build_symbols_recursive(statements, table, &mut scope_level.clone());
        }
        crate::ast::ExprKind::Array(elements) => {
            for elem in elements {
                extract_symbols_from_expr(elem, table, scope_level);
            }
        }
        crate::ast::ExprKind::Hash(pairs) => {
            for (_, value) in pairs {
                extract_symbols_from_expr(value, table, scope_level);
            }
        }
        _ => {}
    }
}
