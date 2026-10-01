//! Which top-level functions can become kernels, and their typed form.
//!
//! This is its own small type checker rather than a reuse of `src/types/`,
//! because it has to be *sound* where that one is lenient: `Any` and
//! `Unknown` let a program through the checker, and here they would let a
//! wrong machine instruction through. Anything this pass cannot type exactly
//! is refused, and a refused function simply keeps running where it always
//! did.
//!
//! The scoping mirrors the tree-walker, which is the stricter engine: a block
//! opens a scope, a binding made inside it ends with it, and a bare `x = …`
//! updates the nearest visible `x` or else creates one. The VM scopes a
//! function's locals more loosely (a variable first set inside an `if` is
//! still readable after it). Kernels refuse every program where the two
//! readings would differ: a name is only read after an assignment in the same
//! or an enclosing scope, so both engines see the same value.

use std::collections::{HashMap, HashSet};

use super::ir::{ArithOp, CmpKind, CmpOp, KExpr, KFunc, KStmt, Ty};
use crate::ast::{
    expr::Argument, BinaryOp, CompoundOp, Expr, ExprKind, FunctionDecl, Program, Stmt, StmtKind,
    TypeKind, UnaryOp,
};

/// The outcome of analysing a program.
pub struct Analysis<'p> {
    /// Kernels, in the order the lowering will define them. A [`KExpr::Call`]
    /// indexes this list.
    pub funcs: Vec<KFunc>,
    /// The declaration each kernel was built from (same order as `funcs`).
    pub decls: Vec<&'p FunctionDecl>,
    /// Functions that looked like candidates and were turned down, with why.
    pub refusals: Vec<Refusal>,
}

#[derive(Debug, Clone)]
pub struct Refusal {
    pub name: String,
    pub line: u32,
    pub reason: String,
}

/// Most parameters a kernel takes: the engines hand the arguments over in a
/// fixed array on the stack.
pub const MAX_ARGS: usize = 16;

struct Candidate<'p> {
    decl: &'p FunctionDecl,
    params: Vec<Ty>,
    ret: Ty,
}

/// Analyse `program`. `is_builtin` says whether a name is bound before the
/// program runs (a builtin function or class): a kernel must not assign it.
pub fn analyze<'p>(program: &'p Program, is_builtin: &dyn Fn(&str) -> bool) -> Analysis<'p> {
    let mut top_level: Vec<&'p Stmt> = Vec::new();
    for stmt in &program.statements {
        flatten_export(stmt, &mut top_level);
    }

    // Every name the program can bind at the top level. A kernel must neither
    // assign one (the engines would write the global) nor be one of them under
    // two definitions.
    let mut def_counts: HashMap<&str, usize> = HashMap::new();
    let mut globals: HashSet<String> = HashSet::new();
    for stmt in &top_level {
        if let StmtKind::Function(decl) = &stmt.kind {
            *def_counts.entry(decl.name.as_str()).or_default() += 1;
            globals.insert(decl.name.clone());
        } else {
            collect_bound_names(stmt, &mut globals);
        }
    }
    let other_bindings: HashSet<String> = {
        let mut names = HashSet::new();
        for stmt in &top_level {
            if !matches!(stmt.kind, StmtKind::Function(_)) {
                collect_bound_names(stmt, &mut names);
            }
        }
        names
    };

    let mut refusals = Vec::new();
    let mut candidates: Vec<Candidate<'p>> = Vec::new();
    for stmt in &top_level {
        let StmtKind::Function(decl) = &stmt.kind else {
            continue;
        };
        match signature(decl) {
            Ok(None) => {}
            Ok(Some((params, ret))) => {
                if def_counts.get(decl.name.as_str()).copied().unwrap_or(0) > 1 {
                    refusals.push(refusal(decl, "defined more than once".into()));
                } else if other_bindings.contains(&decl.name) {
                    refusals.push(refusal(
                        decl,
                        "the name is also bound by something other than this def".into(),
                    ));
                } else {
                    candidates.push(Candidate { decl, params, ret });
                }
            }
            Err(reason) => refusals.push(refusal(decl, reason)),
        }
    }

    // A candidate is out when its body cannot be typed, or when it calls one
    // that is out. Repeat until nothing changes.
    let mut alive = vec![true; candidates.len()];
    loop {
        let index = alive_index(&candidates, &alive);
        let mut changed = false;
        for (i, candidate) in candidates.iter().enumerate() {
            if !alive[i] {
                continue;
            }
            if let Err(reason) = type_function(candidate, &index, &globals, is_builtin) {
                alive[i] = false;
                changed = true;
                refusals.push(refusal(candidate.decl, reason));
            }
        }
        if !changed {
            break;
        }
    }

    let index = alive_index(&candidates, &alive);
    let mut funcs = Vec::new();
    let mut decls = Vec::new();
    for (i, candidate) in candidates.iter().enumerate() {
        if !alive[i] {
            continue;
        }
        // Typed once already in the loop above, with the same alive set.
        if let Ok(func) = type_function(candidate, &index, &globals, is_builtin) {
            funcs.push(func);
            decls.push(candidate.decl);
        }
    }

    Analysis {
        funcs,
        decls,
        refusals,
    }
}

fn refusal(decl: &FunctionDecl, reason: String) -> Refusal {
    Refusal {
        name: decl.name.clone(),
        line: decl.span.line,
        reason,
    }
}

fn flatten_export<'p>(stmt: &'p Stmt, out: &mut Vec<&'p Stmt>) {
    match &stmt.kind {
        StmtKind::Export(inner) => flatten_export(inner, out),
        _ => out.push(stmt),
    }
}

/// Names a top-level statement can bind, looking through blocks and control
/// flow but not into function or class bodies.
fn collect_bound_names(stmt: &Stmt, names: &mut HashSet<String>) {
    match &stmt.kind {
        StmtKind::Let { name, .. } | StmtKind::Const { name, .. } => {
            names.insert(name.clone());
        }
        StmtKind::Function(decl) => {
            names.insert(decl.name.clone());
        }
        StmtKind::Class(decl) => {
            names.insert(decl.name.clone());
        }
        StmtKind::Enum(decl) => {
            names.insert(decl.name.clone());
        }
        StmtKind::Interface(decl) => {
            names.insert(decl.name.clone());
        }
        StmtKind::Import(import) => match &import.specifier {
            crate::ast::ImportSpecifier::All => {}
            crate::ast::ImportSpecifier::Named(items) => {
                for item in items {
                    names.insert(item.alias.clone().unwrap_or_else(|| item.name.clone()));
                }
            }
            crate::ast::ImportSpecifier::Namespace(name) => {
                names.insert(name.clone());
            }
        },
        StmtKind::Export(inner) => collect_bound_names(inner, names),
        StmtKind::Expression(expr) | StmtKind::Throw(expr) | StmtKind::Return(Some(expr)) => {
            collect_assigned_names(expr, names)
        }
        StmtKind::Block(stmts) => {
            for s in stmts {
                collect_bound_names(s, names);
            }
        }
        StmtKind::If {
            condition,
            then_branch,
            else_branch,
        }
        | StmtKind::Unless {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_assigned_names(condition, names);
            collect_bound_names(then_branch, names);
            if let Some(other) = else_branch {
                collect_bound_names(other, names);
            }
        }
        StmtKind::While { condition, body } => {
            collect_assigned_names(condition, names);
            collect_bound_names(body, names);
        }
        StmtKind::For {
            variable,
            index_variable,
            iterable,
            body,
        } => {
            names.insert(variable.clone());
            if let Some(index) = index_variable {
                names.insert(index.clone());
            }
            collect_assigned_names(iterable, names);
            collect_bound_names(body, names);
        }
        StmtKind::Try {
            try_block,
            catch_clauses,
            finally_block,
        } => {
            collect_bound_names(try_block, names);
            for clause in catch_clauses {
                if let Some(var) = &clause.var_name {
                    names.insert(var.clone());
                }
                collect_bound_names(&clause.body, names);
            }
            if let Some(finally) = finally_block {
                collect_bound_names(finally, names);
            }
        }
        StmtKind::Return(None) | StmtKind::Break => {}
    }
}

/// Assignment targets inside a top-level expression. Conservative: any
/// `name = …` or `name op= …` that is not inside a lambda body counts.
fn collect_assigned_names(expr: &Expr, names: &mut HashSet<String>) {
    match &expr.kind {
        ExprKind::Assign { target, value } => {
            if let ExprKind::Variable(name) = &target.kind {
                names.insert(name.clone());
            }
            collect_assigned_names(value, names);
        }
        ExprKind::CompoundAssign { target, value, .. } => {
            if let ExprKind::Variable(name) = &target.kind {
                names.insert(name.clone());
            }
            collect_assigned_names(value, names);
        }
        ExprKind::Binary { left, right, .. }
        | ExprKind::LogicalAnd { left, right }
        | ExprKind::LogicalOr { left, right }
        | ExprKind::NullishCoalescing { left, right }
        | ExprKind::Pipeline { left, right } => {
            collect_assigned_names(left, names);
            collect_assigned_names(right, names);
        }
        ExprKind::Unary { operand, .. } | ExprKind::Grouping(operand) => {
            collect_assigned_names(operand, names)
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_assigned_names(condition, names);
            collect_assigned_names(then_branch, names);
            if let Some(other) = else_branch {
                collect_assigned_names(other, names);
            }
        }
        ExprKind::Call { callee, arguments } => {
            collect_assigned_names(callee, names);
            for arg in arguments {
                match arg {
                    Argument::Positional(e) | Argument::Block(e) => {
                        collect_assigned_names(e, names)
                    }
                    Argument::Named(named) => collect_assigned_names(&named.value, names),
                }
            }
        }
        ExprKind::Block(stmts) => {
            for s in stmts {
                collect_bound_names(s, names);
            }
        }
        _ => {}
    }
}

/// `Ok(None)`: not even a candidate (an untyped helper, the common case, which
/// is not worth a log line). `Err`: typed like a kernel, but not one.
fn signature(decl: &FunctionDecl) -> Result<Option<(Vec<Ty>, Ty)>, String> {
    let Some(ret_ann) = &decl.return_type else {
        return Ok(None);
    };
    let mut params = Vec::with_capacity(decl.params.len());
    for param in &decl.params {
        let TypeKind::Named(name) = &param.type_annotation.kind else {
            return Ok(None);
        };
        let Some(ty) = Ty::from_annotation(name) else {
            // Unannotated parameters are `Any` to the parser: an ordinary
            // dynamic function, not a refused kernel.
            return Ok(None);
        };
        if param.default_value.is_some() {
            return Err(format!("parameter `{}` has a default value", param.name));
        }
        if param.is_block_param {
            return Err(format!("parameter `{}` is a block parameter", param.name));
        }
        params.push(ty);
    }
    let ret = match &ret_ann.kind {
        TypeKind::Named(name) => match Ty::from_annotation(name) {
            Some(ty) => ty,
            None if params.is_empty() => return Ok(None),
            None => return Err(format!("returns {name}, not Int, Float or Bool")),
        },
        _ if params.is_empty() => return Ok(None),
        _ => return Err(format!("returns {ret_ann}, not Int, Float or Bool")),
    };
    if params.len() > MAX_ARGS {
        return Err(format!("has more than {MAX_ARGS} parameters"));
    }
    if params.is_empty() {
        // A kernel with no inputs is a constant; not worth compiling.
        return Ok(None);
    }
    Ok(Some((params, ret)))
}

/// Name → (kernel index among the alive, parameter types, return type).
type Index<'p> = HashMap<&'p str, (u32, &'p [Ty], Ty)>;

fn alive_index<'p>(candidates: &'p [Candidate<'p>], alive: &[bool]) -> Index<'p> {
    let mut index = HashMap::new();
    let mut next = 0u32;
    for (i, candidate) in candidates.iter().enumerate() {
        if alive[i] {
            index.insert(
                candidate.decl.name.as_str(),
                (next, candidate.params.as_slice(), candidate.ret),
            );
            next += 1;
        }
    }
    index
}

fn type_function(
    candidate: &Candidate<'_>,
    index: &Index<'_>,
    globals: &HashSet<String>,
    is_builtin: &dyn Fn(&str) -> bool,
) -> Result<KFunc, String> {
    let decl = candidate.decl;
    if decl.body.is_empty() {
        return Err("the body is empty".into());
    }
    let mut typer = Typer {
        index,
        globals,
        is_builtin,
        ret: candidate.ret,
        scopes: vec![HashMap::new()],
        slots: Vec::new(),
        loop_depth: 0,
        callees: Vec::new(),
    };
    for (param, ty) in decl.params.iter().zip(&candidate.params) {
        if typer.scopes[0].contains_key(&param.name) {
            return Err(format!("parameter `{}` appears twice", param.name));
        }
        typer.bind(&param.name, *ty);
    }
    let body = typer.block(&decl.body)?;
    typer.check_tail(&body)?;
    let mut callees = typer.callees;
    callees.sort_unstable();
    callees.dedup();
    Ok(KFunc {
        name: decl.name.clone(),
        params: candidate.params.clone(),
        ret: candidate.ret,
        slots: typer.slots,
        body,
        callees,
        line: decl.span.line,
    })
}

struct Typer<'a> {
    index: &'a Index<'a>,
    globals: &'a HashSet<String>,
    is_builtin: &'a dyn Fn(&str) -> bool,
    ret: Ty,
    scopes: Vec<HashMap<String, u32>>,
    slots: Vec<Ty>,
    loop_depth: u32,
    callees: Vec<u32>,
}

impl Typer<'_> {
    fn bind(&mut self, name: &str, ty: Ty) -> u32 {
        let slot = self.slots.len() as u32;
        self.slots.push(ty);
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), slot);
        }
        slot
    }

    fn lookup(&self, name: &str) -> Option<u32> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    /// Statements of a body or a block, in a fresh scope.
    fn block(&mut self, stmts: &[Stmt]) -> Result<Vec<KStmt>, String> {
        self.scopes.push(HashMap::new());
        let result = self.stmts(stmts);
        self.scopes.pop();
        result
    }

    fn stmts(&mut self, stmts: &[Stmt]) -> Result<Vec<KStmt>, String> {
        let mut out = Vec::with_capacity(stmts.len());
        for stmt in stmts {
            self.stmt(stmt, &mut out)?;
        }
        Ok(out)
    }

    /// A branch or a loop body. A `Block` is its own scope already; a single
    /// statement (a postfix `if`'s body) runs in the enclosing environment in
    /// the tree-walker, but only sometimes, so a binding it makes is not
    /// readable afterwards here either.
    fn branch(&mut self, stmt: &Stmt) -> Result<Vec<KStmt>, String> {
        match &stmt.kind {
            StmtKind::Block(stmts) => self.block(stmts),
            _ => self.block(std::slice::from_ref(stmt)),
        }
    }

    fn stmt(&mut self, stmt: &Stmt, out: &mut Vec<KStmt>) -> Result<(), String> {
        let line = stmt.span.line;
        match &stmt.kind {
            StmtKind::Expression(expr) => {
                let e = self.value(expr)?;
                out.push(KStmt::Expr(e));
            }
            StmtKind::Let {
                name,
                type_annotation,
                initializer,
            } => {
                let Some(init) = initializer else {
                    return Err(format!("`let {name}` without a value (line {line})"));
                };
                if self.lookup(name).is_some() {
                    return Err(format!("`let {name}` shadows a variable (line {line})"));
                }
                let value = self.value(init)?;
                let ty = value.ty().expect("value() returns typed expressions");
                if let Some(ann) = type_annotation {
                    let declared = match &ann.kind {
                        TypeKind::Named(n) => Ty::from_annotation(n),
                        _ => None,
                    };
                    if declared != Some(ty) {
                        return Err(format!(
                            "`let {name}: {ann}` is given {} (line {line})",
                            ty.a()
                        ));
                    }
                }
                // No global check here, unlike a bare assignment: `let` makes
                // a local in both engines even when a global has the name.
                let slot = self.bind(name, ty);
                out.push(KStmt::Nothing(Box::new(KStmt::Expr(KExpr::Assign(
                    slot,
                    Box::new(value),
                    ty,
                )))));
            }
            StmtKind::Block(stmts) => {
                // Flattened: the slots are already resolved, and the block's
                // last statement stays the body's last statement.
                let inner = self.block(stmts)?;
                out.extend(inner);
            }
            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond = self.expr(condition)?;
                out.push(self.if_stmt(cond, then_branch, else_branch.as_deref())?);
            }
            StmtKind::Unless {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond = KExpr::Not(Box::new(self.expr(condition)?));
                out.push(self.if_stmt(cond, then_branch, else_branch.as_deref())?);
            }
            StmtKind::While { condition, body } => {
                let cond = self.expr(condition)?;
                self.loop_depth += 1;
                let body = self.branch(body);
                self.loop_depth -= 1;
                out.push(KStmt::While { cond, body: body? });
            }
            StmtKind::Break => {
                if self.loop_depth == 0 {
                    return Err(format!("`break` outside a loop (line {line})"));
                }
                out.push(KStmt::Break);
            }
            StmtKind::Return(Some(expr)) => {
                let value = self.value(expr)?;
                let ty = value.ty().expect("typed");
                if ty != self.ret {
                    return Err(format!(
                        "returns {} where {} is declared (line {line})",
                        ty.a(),
                        self.ret.name()
                    ));
                }
                out.push(KStmt::Return(value));
            }
            StmtKind::Return(None) => {
                return Err(format!("`return` without a value (line {line})"));
            }
            other => return Err(format!("{} (line {line})", describe_stmt(other))),
        }
        Ok(())
    }

    fn if_stmt(
        &mut self,
        cond: KExpr,
        then_branch: &Stmt,
        else_branch: Option<&Stmt>,
    ) -> Result<KStmt, String> {
        let then = self.branch(then_branch)?;
        let otherwise = match else_branch {
            Some(other) => self.branch(other)?,
            None => Vec::new(),
        };
        Ok(KStmt::If {
            cond,
            then,
            otherwise,
            has_else: else_branch.is_some(),
        })
    }

    /// The body's value is its last statement's. Every way out must carry the
    /// declared type, or the engines disagree on what happens (the
    /// tree-walker raises, the VM returns the value as is), and a kernel must
    /// match both. A path that falls off the end is fine: the lowering
    /// declines it, and the engine does whatever it does today.
    fn check_tail(&self, body: &[KStmt]) -> Result<(), String> {
        let Some(last) = body.last() else {
            return Ok(());
        };
        match last {
            KStmt::Expr(e) => {
                let ty = e.ty().expect("typed");
                if ty != self.ret {
                    return Err(format!(
                        "its last expression is {} where {} is declared",
                        ty.a(),
                        self.ret.name()
                    ));
                }
                Ok(())
            }
            KStmt::If {
                then, otherwise, ..
            } => {
                self.check_tail(then)?;
                self.check_tail(otherwise)
            }
            KStmt::While { .. } | KStmt::Break | KStmt::Return(_) | KStmt::Nothing(_) => Ok(()),
        }
    }

    fn check_assignable_name(&self, name: &str, line: u32) -> Result<(), String> {
        if self.globals.contains(name) || (self.is_builtin)(name) {
            return Err(format!(
                "assigns `{name}`, which names a global (line {line})"
            ));
        }
        Ok(())
    }

    /// An expression whose value is used.
    fn value(&mut self, expr: &Expr) -> Result<KExpr, String> {
        let e = self.expr(expr)?;
        if e.ty().is_none() {
            return Err(format!(
                "`&&`/`||` mixes types where its value is used (line {})",
                expr.span.line
            ));
        }
        Ok(e)
    }

    /// Any expression. Its type may be missing (`&&`/`||` over two types),
    /// which is fine where only its truthiness is read: callers that use the
    /// value go through [`Self::value`].
    fn expr(&mut self, expr: &Expr) -> Result<KExpr, String> {
        let line = expr.span.line;
        Ok(match &expr.kind {
            ExprKind::IntLiteral(n) => KExpr::Int(*n),
            ExprKind::FloatLiteral(f) => KExpr::Float(*f),
            ExprKind::BoolLiteral(b) => KExpr::Bool(*b),
            ExprKind::Grouping(inner) => self.expr(inner)?,
            ExprKind::Variable(name) => match self.lookup(name) {
                Some(slot) => KExpr::Local(slot, self.slots[slot as usize]),
                None => {
                    return Err(format!(
                        "reads `{name}`, which is not a local (line {line})"
                    ))
                }
            },
            ExprKind::Binary {
                left,
                operator,
                right,
            } => {
                let l = self.value(left)?;
                let r = self.value(right)?;
                self.binary(*operator, l, r, line)?
            }
            ExprKind::Unary { operator, operand } => match operator {
                UnaryOp::Negate => {
                    let e = self.value(operand)?;
                    let ty = e.ty().expect("typed");
                    if !ty.is_numeric() {
                        return Err(format!("negates {} (line {line})", ty.a()));
                    }
                    KExpr::Neg(Box::new(e), ty)
                }
                UnaryOp::Not => KExpr::Not(Box::new(self.expr(operand)?)),
            },
            ExprKind::LogicalAnd { left, right } | ExprKind::LogicalOr { left, right } => {
                let l = self.expr(left)?;
                let r = self.expr(right)?;
                let ty = match (l.ty(), r.ty()) {
                    (Some(a), Some(b)) if a == b => Some(a),
                    _ => None,
                };
                if matches!(expr.kind, ExprKind::LogicalAnd { .. }) {
                    KExpr::And(Box::new(l), Box::new(r), ty)
                } else {
                    KExpr::Or(Box::new(l), Box::new(r), ty)
                }
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let Some(else_branch) = else_branch else {
                    return Err(format!(
                        "a conditional expression without `else` (line {line})"
                    ));
                };
                let cond = self.expr(condition)?;
                let then = self.value(then_branch)?;
                let otherwise = self.value(else_branch)?;
                let (a, b) = (then.ty().expect("typed"), otherwise.ty().expect("typed"));
                if a != b {
                    return Err(format!(
                        "a conditional expression yields {} or {} (line {line})",
                        a.name(),
                        b.name()
                    ));
                }
                KExpr::Select {
                    cond: Box::new(cond),
                    then: Box::new(then),
                    otherwise: Box::new(otherwise),
                    ty: a,
                }
            }
            ExprKind::Call { callee, arguments } => self.call(callee, arguments, line)?,
            ExprKind::Assign { target, value } => {
                let ExprKind::Variable(name) = &target.kind else {
                    return Err(format!(
                        "assigns something other than a variable (line {line})"
                    ));
                };
                let value = self.value(value)?;
                let ty = value.ty().expect("typed");
                let slot = match self.lookup(name) {
                    Some(slot) => {
                        let existing = self.slots[slot as usize];
                        if existing != ty {
                            return Err(format!(
                                "`{name}` holds {} and is assigned {} (line {line})",
                                existing.a(),
                                ty.a()
                            ));
                        }
                        slot
                    }
                    None => {
                        self.check_assignable_name(name, line)?;
                        self.bind(name, ty)
                    }
                };
                KExpr::Assign(slot, Box::new(value), ty)
            }
            ExprKind::CompoundAssign {
                target,
                operator,
                value,
            } => {
                let ExprKind::Variable(name) = &target.kind else {
                    return Err(format!(
                        "assigns something other than a variable (line {line})"
                    ));
                };
                let Some(slot) = self.lookup(name) else {
                    return Err(format!(
                        "updates `{name}`, which is not a local (line {line})"
                    ));
                };
                let op = match operator {
                    CompoundOp::Add => BinaryOp::Add,
                    CompoundOp::Subtract => BinaryOp::Subtract,
                    CompoundOp::Multiply => BinaryOp::Multiply,
                    CompoundOp::Divide => BinaryOp::Divide,
                    CompoundOp::Modulo => BinaryOp::Modulo,
                    _ => {
                        return Err(format!(
                            "uses a short-circuit assignment operator (line {line})"
                        ))
                    }
                };
                let current = KExpr::Local(slot, self.slots[slot as usize]);
                let rhs = self.value(value)?;
                let result = self.binary(op, current, rhs, line)?;
                let ty = result.ty().expect("typed");
                if ty != self.slots[slot as usize] {
                    return Err(format!(
                        "`{name}` holds {} and is updated to {} (line {line})",
                        self.slots[slot as usize].a(),
                        ty.a()
                    ));
                }
                KExpr::Assign(slot, Box::new(result), ty)
            }
            other => return Err(format!("{} (line {line})", describe_expr(other))),
        })
    }

    fn binary(&self, op: BinaryOp, l: KExpr, r: KExpr, line: u32) -> Result<KExpr, String> {
        let (lt, rt) = (l.ty().expect("typed"), r.ty().expect("typed"));
        let arith = match op {
            BinaryOp::Add => Some(ArithOp::Add),
            BinaryOp::Subtract => Some(ArithOp::Sub),
            BinaryOp::Multiply => Some(ArithOp::Mul),
            BinaryOp::Divide => Some(ArithOp::Div),
            BinaryOp::Modulo => Some(ArithOp::Rem),
            _ => None,
        };
        if let Some(op) = arith {
            if !lt.is_numeric() || !rt.is_numeric() {
                return Err(format!(
                    "applies `{}` to {} and {} (line {line})",
                    binary_symbol(op),
                    lt.name(),
                    rt.name()
                ));
            }
            let ty = if lt == Ty::Int && rt == Ty::Int {
                Ty::Int
            } else {
                Ty::Float
            };
            return Ok(KExpr::Arith {
                op,
                left: Box::new(l),
                right: Box::new(r),
                ty,
            });
        }
        let Some(cmp) = CmpOp::from_binary(op) else {
            return Err(format!("uses the `{op}` operator (line {line})"));
        };
        let kind = match (lt, rt) {
            (Ty::Int, Ty::Int) => CmpKind::Int,
            (a, b) if a.is_numeric() && b.is_numeric() => CmpKind::Float,
            (Ty::Bool, Ty::Bool) if !cmp.is_ordering() => CmpKind::Bool,
            _ => {
                return Err(format!(
                    "compares {} with {} using `{op}` (line {line})",
                    lt.name(),
                    rt.name()
                ))
            }
        };
        Ok(KExpr::Cmp {
            op: cmp,
            kind,
            left: Box::new(l),
            right: Box::new(r),
        })
    }

    fn call(&mut self, callee: &Expr, arguments: &[Argument], line: u32) -> Result<KExpr, String> {
        let ExprKind::Variable(name) = &callee.kind else {
            return Err(format!("{} (line {line})", describe_expr(&callee.kind)));
        };
        if self.lookup(name).is_some() {
            return Err(format!(
                "calls `{name}`, which is a local here (line {line})"
            ));
        }
        let Some(&(kernel, params, ty)) = self.index.get(name.as_str()) else {
            return Err(format!("calls `{name}` (line {line})"));
        };
        if arguments.len() != params.len() {
            return Err(format!(
                "calls `{name}` with {} arguments for {} parameters (line {line})",
                arguments.len(),
                params.len()
            ));
        }
        let mut args = Vec::with_capacity(arguments.len());
        for (arg, expected) in arguments.iter().zip(params) {
            let Argument::Positional(expr) = arg else {
                return Err(format!(
                    "calls `{name}` with a named or block argument (line {line})"
                ));
            };
            let value = self.value(expr)?;
            let ty = value.ty().expect("typed");
            if ty != *expected {
                return Err(format!(
                    "passes {} to `{name}` where {} is declared (line {line})",
                    ty.a(),
                    expected.name()
                ));
            }
            args.push(value);
        }
        self.callees.push(kernel);
        Ok(KExpr::Call { kernel, args, ty })
    }
}

fn binary_symbol(op: ArithOp) -> &'static str {
    match op {
        ArithOp::Add => "+",
        ArithOp::Sub => "-",
        ArithOp::Mul => "*",
        ArithOp::Div => "/",
        ArithOp::Rem => "%",
    }
}

fn describe_stmt(kind: &StmtKind) -> &'static str {
    match kind {
        StmtKind::Const { .. } => "declares a constant",
        StmtKind::For { .. } => "uses a `for` loop",
        StmtKind::Throw(_) => "throws",
        StmtKind::Try { .. } => "uses try/catch",
        StmtKind::Function(_) => "defines a nested function",
        StmtKind::Class(_) | StmtKind::Enum(_) | StmtKind::Interface(_) => "declares a type",
        StmtKind::Import(_) | StmtKind::Export(_) => "imports or exports",
        _ => "uses an unsupported statement",
    }
}

fn describe_expr(kind: &ExprKind) -> &'static str {
    match kind {
        ExprKind::StringLiteral(_) | ExprKind::InterpolatedString(_) => "uses a string",
        ExprKind::DecimalLiteral(_) => "uses a Decimal",
        ExprKind::Null => "uses nil",
        ExprKind::Array(_) | ExprKind::Spread(_) => "uses an array",
        ExprKind::Hash(_) => "uses a hash",
        ExprKind::Member { .. } | ExprKind::SafeMember { .. } => "calls a method or reads a field",
        ExprKind::Index { .. } => "indexes a collection",
        ExprKind::Lambda { .. } => "creates a closure",
        ExprKind::Match { .. } => "uses `match`",
        ExprKind::This | ExprKind::Super => "uses `this` or `super`",
        ExprKind::Pipeline { .. } => "uses a pipeline",
        ExprKind::Throw(_) | ExprKind::Rescue { .. } => "throws or rescues",
        ExprKind::PostfixIncrement(_) | ExprKind::PostfixDecrement(_) => "uses `++` or `--`",
        ExprKind::NullishCoalescing { .. } => "uses `??`",
        ExprKind::Call { .. } => "calls something other than a plain function name",
        _ => "uses an unsupported expression",
    }
}
