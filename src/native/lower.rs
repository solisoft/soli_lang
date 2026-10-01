//! Typed kernels → Cranelift IR.
//!
//! The semantics are the tree-walker's and the VM's, operator by operator
//! (`src/interpreter/executor/operators.rs`, `src/vm/vm.rs`):
//!
//! - Int `+ - *` are checked; Int `/ %` truncate, and a zero divisor or
//!   `i64::MIN / -1` is an error.
//! - A Float division by zero is an error; a Float `%` by zero is NaN.
//! - An Int meeting a Float is converted to f64 first.
//! - `0` is falsy and every Float is truthy, `0.0` and NaN included.
//! - `&&`/`||` yield an operand, not a Bool.
//!
//! Every case where an engine would raise becomes a *decline*: the kernel
//! sets `ctx.status`, unwinds to its entry, and the engine runs the same call
//! itself and raises exactly what it always raised. So the IR never has to
//! reproduce an error, only to notice one.

use std::collections::HashMap;

use cranelift_codegen::ir::{
    condcodes::{FloatCC, IntCC},
    types, AbiParam, Block, InstBuilder, MemFlagsData, Signature, Value,
};
use cranelift_frontend::{FunctionBuilder, Variable};
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Module};

use super::ir::{ArithOp, CmpKind, CmpOp, KExpr, KFunc, KStmt, Ty};
use super::Decline;

/// Byte offsets inside `KernelCtx` (see `super::jit::KernelCtx`).
pub const CTX_STATUS: i32 = 0;
pub const CTX_REASON: i32 = 4;

pub fn clif_type(ty: Ty) -> types::Type {
    match ty {
        Ty::Int => types::I64,
        Ty::Float => types::F64,
        Ty::Bool => types::I8,
    }
}

/// `k(ctx, depth, params…) -> ret`.
pub fn kernel_signature(module: &JITModule, func: &KFunc) -> Signature {
    let ptr = module.target_config().pointer_type();
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(ptr));
    sig.params.push(AbiParam::new(types::I64));
    for ty in &func.params {
        sig.params.push(AbiParam::new(clif_type(*ty)));
    }
    sig.returns.push(AbiParam::new(clif_type(func.ret)));
    sig
}

/// How large (in IR nodes) a kernel may be and still be copied into its
/// callers. fib is 17; a loop with a few statements is about 40.
const INLINE_BUDGET: usize = 48;

pub struct Lowerer<'a, 'b> {
    pub builder: FunctionBuilder<'b>,
    pub module: &'a mut JITModule,
    pub funcs: &'a [KFunc],
    pub func_ids: &'a [FuncId],
    pub fmod: FuncId,
    pub max_depth: i64,
    /// The function whose body is being emitted: the kernel itself, or the
    /// callee being inlined into it.
    func: &'a KFunc,
    /// The machine function's return type, which every early exit (a bail,
    /// a declined callee) must produce whatever body it is in.
    out_ret: Ty,
    vars: Vec<Variable>,
    ctx: Value,
    depth: Value,
    loop_exits: Vec<Block>,
    bail_blocks: HashMap<Decline, Block>,
    propagate: Option<Block>,
    terminated: bool,
    /// Inside an inlined body: where its `return` goes, and the variable that
    /// carries the value there.
    inline_ret: Option<(Block, Variable)>,
}

impl<'a, 'b> Lowerer<'a, 'b> {
    pub fn new(
        builder: FunctionBuilder<'b>,
        module: &'a mut JITModule,
        funcs: &'a [KFunc],
        func_ids: &'a [FuncId],
        fmod: FuncId,
        max_depth: i64,
        func: &'a KFunc,
    ) -> Self {
        Self {
            builder,
            module,
            funcs,
            func_ids,
            fmod,
            max_depth,
            func,
            out_ret: func.ret,
            vars: Vec::new(),
            ctx: Value::from_u32(0),
            depth: Value::from_u32(0),
            loop_exits: Vec::new(),
            bail_blocks: HashMap::new(),
            propagate: None,
            terminated: false,
            inline_ret: None,
        }
    }

    /// Emit the whole kernel, then hand the builder back for finalising.
    pub fn lower(mut self) -> FunctionBuilder<'b> {
        let entry = self.builder.create_block();
        self.builder.append_block_params_for_function_params(entry);
        self.builder.switch_to_block(entry);
        let params = self.builder.block_params(entry).to_vec();
        self.ctx = params[0];
        self.depth = params[1];

        for ty in &self.func.slots {
            let var = self.builder.declare_var(clif_type(*ty));
            self.vars.push(var);
        }
        for (i, value) in params[2..].iter().enumerate() {
            self.builder.def_var(self.vars[i], *value);
        }

        // The same limit the engines enforce, counted the same way: the
        // caller passes its own depth, each kernel call adds one.
        let too_deep = self.builder.ins().icmp_imm_s(
            IntCC::SignedGreaterThanOrEqual,
            self.depth,
            self.max_depth,
        );
        self.bail_if(too_deep, Decline::Depth);

        let body = &self.func.body;
        self.tail_list(body);
        if !self.terminated {
            self.bail(Decline::FellThrough);
        }
        self.fill_exit_blocks();
        self.builder.seal_all_blocks();
        self.builder
    }

    // ---- control-flow helpers --------------------------------------------

    fn start_block(&mut self, block: Block) {
        self.builder.switch_to_block(block);
        self.terminated = false;
    }

    /// After a terminator: later instructions go to a block nothing reaches.
    fn dead_end(&mut self) {
        let dead = self.builder.create_block();
        self.builder.switch_to_block(dead);
        self.terminated = true;
    }

    fn dummy(&mut self, ty: Ty) -> Value {
        match ty {
            Ty::Int => self.builder.ins().iconst(types::I64, 0),
            Ty::Float => self.builder.ins().f64const(0.0),
            Ty::Bool => self.builder.ins().iconst(types::I8, 0),
        }
    }

    /// The block that declines for `reason`: created on first use, filled
    /// by [`Self::fill_exit_blocks`] once the body is done. Filling it here
    /// would mean leaving the current block half-built, which Cranelift's
    /// frontend refuses in a debug build ("you have to fill your block
    /// before switching").
    fn bail_block(&mut self, reason: Decline) -> Block {
        if let Some(block) = self.bail_blocks.get(&reason) {
            return *block;
        }
        let block = self.builder.create_block();
        self.builder.set_cold_block(block);
        self.bail_blocks.insert(reason, block);
        block
    }

    /// Return early when a kernel this one called has declined: the status is
    /// already in `ctx`, and the outermost entry reads it. Filled, like the
    /// bail blocks, at the end.
    fn propagate_block(&mut self) -> Block {
        if let Some(block) = self.propagate {
            return block;
        }
        let block = self.builder.create_block();
        self.builder.set_cold_block(block);
        self.propagate = Some(block);
        block
    }

    /// Give each exit block its instructions, once nothing else is being
    /// built: a bail stores the status and the reason, both return a dummy of
    /// the machine function's type.
    fn fill_exit_blocks(&mut self) {
        let mut bails: Vec<(Decline, Block)> =
            self.bail_blocks.iter().map(|(r, b)| (*r, *b)).collect();
        bails.sort_by_key(|(reason, _)| *reason as u32);
        for (reason, block) in bails {
            self.builder.switch_to_block(block);
            let one = self.builder.ins().iconst(types::I32, 1);
            self.builder
                .ins()
                .store(MemFlagsData::trusted(), one, self.ctx, CTX_STATUS);
            let code = self.builder.ins().iconst(types::I32, reason as i64);
            self.builder
                .ins()
                .store(MemFlagsData::trusted(), code, self.ctx, CTX_REASON);
            let dummy = self.dummy(self.out_ret);
            self.builder.ins().return_(&[dummy]);
        }
        if let Some(block) = self.propagate {
            self.builder.switch_to_block(block);
            let dummy = self.dummy(self.out_ret);
            self.builder.ins().return_(&[dummy]);
        }
    }

    fn bail_if(&mut self, condition: Value, reason: Decline) {
        let bail = self.bail_block(reason);
        let next = self.builder.create_block();
        self.builder.ins().brif(condition, bail, &[], next, &[]);
        self.start_block(next);
    }

    /// Leave the body being emitted with `v`: a machine `return`, or, in an
    /// inlined callee, a jump to the code after the call.
    fn emit_return(&mut self, v: Value) {
        match self.inline_ret {
            Some((cont, result)) => {
                self.builder.def_var(result, v);
                self.builder.ins().jump(cont, &[]);
            }
            None => {
                self.builder.ins().return_(&[v]);
            }
        }
        self.dead_end();
    }

    fn bail(&mut self, reason: Decline) {
        let bail = self.bail_block(reason);
        self.builder.ins().jump(bail, &[]);
        self.dead_end();
    }

    // ---- statements --------------------------------------------------------

    /// A list whose last statement gives the function's value.
    fn tail_list(&mut self, stmts: &[KStmt]) {
        let Some((last, rest)) = stmts.split_last() else {
            self.bail(Decline::FellThrough);
            return;
        };
        for stmt in rest {
            self.stmt(stmt);
            if self.terminated {
                return;
            }
        }
        self.tail(last);
    }

    fn tail(&mut self, stmt: &KStmt) {
        match stmt {
            KStmt::Expr(e) => {
                let v = self.value(e);
                self.emit_return(v);
            }
            KStmt::If {
                cond,
                then,
                otherwise,
                has_else,
            } => {
                let c = self.cond(cond);
                let then_block = self.builder.create_block();
                let else_block = self.builder.create_block();
                self.builder.ins().brif(c, then_block, &[], else_block, &[]);
                self.start_block(then_block);
                self.tail_list(then);
                if !self.terminated {
                    self.bail(Decline::FellThrough);
                }
                self.start_block(else_block);
                if *has_else {
                    self.tail_list(otherwise);
                }
                if !self.terminated {
                    self.bail(Decline::FellThrough);
                }
            }
            other => {
                self.stmt(other);
                if !self.terminated {
                    self.bail(Decline::FellThrough);
                }
            }
        }
    }

    fn stmts(&mut self, stmts: &[KStmt]) {
        for stmt in stmts {
            self.stmt(stmt);
            if self.terminated {
                return;
            }
        }
    }

    fn stmt(&mut self, stmt: &KStmt) {
        match stmt {
            KStmt::Expr(e) => {
                self.value(e);
            }
            KStmt::Nothing(inner) => self.stmt(inner),
            KStmt::If {
                cond,
                then,
                otherwise,
                ..
            } => {
                let c = self.cond(cond);
                let then_block = self.builder.create_block();
                let else_block = self.builder.create_block();
                let merge = self.builder.create_block();
                self.builder.ins().brif(c, then_block, &[], else_block, &[]);
                self.start_block(then_block);
                self.stmts(then);
                if !self.terminated {
                    self.builder.ins().jump(merge, &[]);
                }
                self.start_block(else_block);
                self.stmts(otherwise);
                if !self.terminated {
                    self.builder.ins().jump(merge, &[]);
                }
                self.start_block(merge);
            }
            KStmt::While { cond, body } => {
                let header = self.builder.create_block();
                let body_block = self.builder.create_block();
                let exit = self.builder.create_block();
                self.builder.ins().jump(header, &[]);
                self.start_block(header);
                let c = self.cond(cond);
                self.builder.ins().brif(c, body_block, &[], exit, &[]);
                self.start_block(body_block);
                self.loop_exits.push(exit);
                self.stmts(body);
                self.loop_exits.pop();
                if !self.terminated {
                    self.builder.ins().jump(header, &[]);
                }
                self.start_block(exit);
            }
            KStmt::Break => {
                let exit = *self
                    .loop_exits
                    .last()
                    .expect("eligibility only accepts `break` inside a loop");
                self.builder.ins().jump(exit, &[]);
                self.dead_end();
            }
            KStmt::Return(e) => {
                let v = self.value(e);
                self.emit_return(v);
            }
        }
    }

    // ---- expressions -------------------------------------------------------

    /// The truthiness of `e` as an I8 (0 or 1), short-circuiting `&&`, `||`
    /// and `!` without materialising their values.
    fn cond(&mut self, e: &KExpr) -> Value {
        match e {
            KExpr::And(l, r, _) | KExpr::Or(l, r, _) => {
                let is_and = matches!(e, KExpr::And(..));
                let result = self.builder.declare_var(types::I8);
                let lt = self.cond(l);
                self.builder.def_var(result, lt);
                let rhs = self.builder.create_block();
                let merge = self.builder.create_block();
                if is_and {
                    self.builder.ins().brif(lt, rhs, &[], merge, &[]);
                } else {
                    self.builder.ins().brif(lt, merge, &[], rhs, &[]);
                }
                self.start_block(rhs);
                let rt = self.cond(r);
                self.builder.def_var(result, rt);
                self.builder.ins().jump(merge, &[]);
                self.start_block(merge);
                self.builder.use_var(result)
            }
            KExpr::Not(inner) => {
                let c = self.cond(inner);
                self.builder.ins().icmp_imm_s(IntCC::Equal, c, 0)
            }
            other => {
                let ty = other.ty().expect("a typed value");
                let v = self.value(other);
                self.truthy(v, ty)
            }
        }
    }

    fn truthy(&mut self, v: Value, ty: Ty) -> Value {
        match ty {
            Ty::Int => self.builder.ins().icmp_imm_s(IntCC::NotEqual, v, 0),
            Ty::Bool => v,
            // Every Float is truthy, 0.0 and NaN included.
            Ty::Float => self.builder.ins().iconst(types::I8, 1),
        }
    }

    fn as_float(&mut self, v: Value, ty: Ty) -> Value {
        match ty {
            Ty::Float => v,
            Ty::Int => self.builder.ins().fcvt_from_sint(types::F64, v),
            Ty::Bool => unreachable!("eligibility refuses arithmetic on Bool"),
        }
    }

    fn value(&mut self, e: &KExpr) -> Value {
        match e {
            KExpr::Int(n) => self.builder.ins().iconst(types::I64, *n),
            KExpr::Float(f) => self.builder.ins().f64const(*f),
            KExpr::Bool(b) => self.builder.ins().iconst(types::I8, *b as i64),
            KExpr::Local(slot, _) => self.builder.use_var(self.vars[*slot as usize]),
            KExpr::Assign(slot, value, _) => {
                let v = self.value(value);
                self.builder.def_var(self.vars[*slot as usize], v);
                v
            }
            KExpr::Arith {
                op,
                left,
                right,
                ty,
            } => {
                let lt = left.ty().expect("typed");
                let rt = right.ty().expect("typed");
                let l = self.value(left);
                let r = self.value(right);
                match ty {
                    Ty::Int => self.int_arith(*op, l, r),
                    _ => {
                        let l = self.as_float(l, lt);
                        let r = self.as_float(r, rt);
                        self.float_arith(*op, l, r)
                    }
                }
            }
            KExpr::Cmp {
                op,
                kind,
                left,
                right,
            } => {
                let lt = left.ty().expect("typed");
                let rt = right.ty().expect("typed");
                let l = self.value(left);
                let r = self.value(right);
                match kind {
                    CmpKind::Int | CmpKind::Bool => self.builder.ins().icmp(int_cc(*op), l, r),
                    CmpKind::Float => {
                        let l = self.as_float(l, lt);
                        let r = self.as_float(r, rt);
                        self.builder.ins().fcmp(float_cc(*op), l, r)
                    }
                }
            }
            KExpr::Neg(inner, ty) => {
                let v = self.value(inner);
                match ty {
                    Ty::Int => {
                        // `-i64::MIN` has no Int; let the engine decide what
                        // that means rather than guess here.
                        let is_min = self.builder.ins().icmp_imm_s(IntCC::Equal, v, i64::MIN);
                        self.bail_if(is_min, Decline::Overflow);
                        self.builder.ins().ineg(v)
                    }
                    _ => self.builder.ins().fneg(v),
                }
            }
            KExpr::Not(_) => self.cond(e),
            KExpr::And(l, r, ty) | KExpr::Or(l, r, ty) => {
                let ty = ty.expect("eligibility types `&&`/`||` used as a value");
                let is_and = matches!(e, KExpr::And(..));
                let result = self.builder.declare_var(clif_type(ty));
                let lv = self.value(l);
                self.builder.def_var(result, lv);
                let lt = self.truthy(lv, ty);
                let rhs = self.builder.create_block();
                let merge = self.builder.create_block();
                if is_and {
                    self.builder.ins().brif(lt, rhs, &[], merge, &[]);
                } else {
                    self.builder.ins().brif(lt, merge, &[], rhs, &[]);
                }
                self.start_block(rhs);
                let rv = self.value(r);
                self.builder.def_var(result, rv);
                self.builder.ins().jump(merge, &[]);
                self.start_block(merge);
                self.builder.use_var(result)
            }
            KExpr::Select {
                cond,
                then,
                otherwise,
                ty,
            } => {
                let result = self.builder.declare_var(clif_type(*ty));
                let c = self.cond(cond);
                let then_block = self.builder.create_block();
                let else_block = self.builder.create_block();
                let merge = self.builder.create_block();
                self.builder.ins().brif(c, then_block, &[], else_block, &[]);
                self.start_block(then_block);
                let tv = self.value(then);
                self.builder.def_var(result, tv);
                self.builder.ins().jump(merge, &[]);
                self.start_block(else_block);
                let ev = self.value(otherwise);
                self.builder.def_var(result, ev);
                self.builder.ins().jump(merge, &[]);
                self.start_block(merge);
                self.builder.use_var(result)
            }
            KExpr::Call { kernel, args, .. }
                if self.inline_ret.is_none()
                    && body_size(&self.funcs[*kernel as usize].body) <= INLINE_BUDGET =>
            {
                self.inline_call(*kernel, args)
            }
            KExpr::Call { kernel, args, .. } => {
                let mut call_args = Vec::with_capacity(args.len() + 2);
                call_args.push(self.ctx);
                let next_depth = self.builder.ins().iadd_imm_s(self.depth, 1);
                call_args.push(next_depth);
                for arg in args {
                    let v = self.value(arg);
                    call_args.push(v);
                }
                let callee = self.func_ids[*kernel as usize];
                let fref = self.module.declare_func_in_func(callee, self.builder.func);
                let call = self.builder.ins().call(fref, &call_args);
                let result = self.builder.inst_results(call)[0];
                let status = self.builder.ins().load(
                    types::I32,
                    MemFlagsData::trusted(),
                    self.ctx,
                    CTX_STATUS,
                );
                let propagate = self.propagate_block();
                let next = self.builder.create_block();
                self.builder.ins().brif(status, propagate, &[], next, &[]);
                self.start_block(next);
                result
            }
        }
    }

    /// Emit a call to a small kernel as a copy of its body: no call, no
    /// prologue, and the common early exit (`return n if n < 2`) becomes a
    /// branch. One level only — calls inside the copy are real calls — so
    /// recursion cannot unroll without end.
    ///
    /// Nothing observable changes. The copy runs at the depth the callee
    /// would have had and checks it the same way; a bail inside it leaves
    /// the whole machine function, as a bail in a real callee would after
    /// its status was propagated.
    fn inline_call(&mut self, kernel: u32, args: &[KExpr]) -> Value {
        let funcs = self.funcs;
        let callee: &'a KFunc = &funcs[kernel as usize];
        let arg_values: Vec<Value> = args.iter().map(|a| self.value(a)).collect();

        let callee_depth = self.builder.ins().iadd_imm_s(self.depth, 1);
        let too_deep = self.builder.ins().icmp_imm_s(
            IntCC::SignedGreaterThanOrEqual,
            callee_depth,
            self.max_depth,
        );
        self.bail_if(too_deep, Decline::Depth);

        let callee_vars: Vec<Variable> = callee
            .slots
            .iter()
            .map(|ty| self.builder.declare_var(clif_type(*ty)))
            .collect();
        for (var, value) in callee_vars.iter().zip(&arg_values) {
            self.builder.def_var(*var, *value);
        }
        let result = self.builder.declare_var(clif_type(callee.ret));
        let cont = self.builder.create_block();

        let caller_vars = std::mem::replace(&mut self.vars, callee_vars);
        let caller_depth = std::mem::replace(&mut self.depth, callee_depth);
        let caller_loops = std::mem::take(&mut self.loop_exits);
        let caller_func = std::mem::replace(&mut self.func, callee);
        self.inline_ret = Some((cont, result));

        self.tail_list(&callee.body);
        if !self.terminated {
            self.bail(Decline::FellThrough);
        }

        self.inline_ret = None;
        self.func = caller_func;
        self.loop_exits = caller_loops;
        self.depth = caller_depth;
        self.vars = caller_vars;
        self.start_block(cont);
        self.builder.use_var(result)
    }

    fn int_arith(&mut self, op: ArithOp, l: Value, r: Value) -> Value {
        match op {
            ArithOp::Add | ArithOp::Sub | ArithOp::Mul => {
                let (value, overflow) = match op {
                    ArithOp::Add => self.builder.ins().sadd_overflow(l, r),
                    ArithOp::Sub => self.builder.ins().ssub_overflow(l, r),
                    _ => self.builder.ins().smul_overflow(l, r),
                };
                self.bail_if(overflow, Decline::Overflow);
                value
            }
            ArithOp::Div | ArithOp::Rem => {
                // Checked before the instruction: Cranelift's own `sdiv`
                // traps (a signal, not an error) on both.
                let zero = self.builder.ins().icmp_imm_s(IntCC::Equal, r, 0);
                self.bail_if(zero, Decline::DivisionByZero);
                let is_min = self.builder.ins().icmp_imm_s(IntCC::Equal, l, i64::MIN);
                let is_minus_one = self.builder.ins().icmp_imm_s(IntCC::Equal, r, -1);
                let both = self.builder.ins().band(is_min, is_minus_one);
                self.bail_if(both, Decline::Overflow);
                if op == ArithOp::Div {
                    self.builder.ins().sdiv(l, r)
                } else {
                    self.builder.ins().srem(l, r)
                }
            }
        }
    }

    fn float_arith(&mut self, op: ArithOp, l: Value, r: Value) -> Value {
        match op {
            ArithOp::Add => self.builder.ins().fadd(l, r),
            ArithOp::Sub => self.builder.ins().fsub(l, r),
            ArithOp::Mul => self.builder.ins().fmul(l, r),
            ArithOp::Div => {
                let zero = self.builder.ins().f64const(0.0);
                // `Equal` is ordered: true for 0.0 and -0.0, false for NaN —
                // `b == 0.0` in Rust.
                let is_zero = self.builder.ins().fcmp(FloatCC::Equal, r, zero);
                self.bail_if(is_zero, Decline::DivisionByZero);
                self.builder.ins().fdiv(l, r)
            }
            ArithOp::Rem => {
                let fref = self
                    .module
                    .declare_func_in_func(self.fmod, self.builder.func);
                let call = self.builder.ins().call(fref, &[l, r]);
                self.builder.inst_results(call)[0]
            }
        }
    }
}

fn int_cc(op: CmpOp) -> IntCC {
    match op {
        CmpOp::Eq => IntCC::Equal,
        CmpOp::Ne => IntCC::NotEqual,
        CmpOp::Lt => IntCC::SignedLessThan,
        CmpOp::Le => IntCC::SignedLessThanOrEqual,
        CmpOp::Gt => IntCC::SignedGreaterThan,
        CmpOp::Ge => IntCC::SignedGreaterThanOrEqual,
    }
}

/// Rust's `f64` operators: ordered for `==` and the orderings (false when
/// either side is NaN), unordered-or-not-equal for `!=` (true for NaN).
fn float_cc(op: CmpOp) -> FloatCC {
    match op {
        CmpOp::Eq => FloatCC::Equal,
        CmpOp::Ne => FloatCC::NotEqual,
        CmpOp::Lt => FloatCC::LessThan,
        CmpOp::Le => FloatCC::LessThanOrEqual,
        CmpOp::Gt => FloatCC::GreaterThan,
        CmpOp::Ge => FloatCC::GreaterThanOrEqual,
    }
}

/// The number of IR nodes in a body, to decide whether it is small enough to
/// copy into a caller.
fn body_size(stmts: &[KStmt]) -> usize {
    stmts.iter().map(stmt_size).sum()
}

fn stmt_size(stmt: &KStmt) -> usize {
    1 + match stmt {
        KStmt::Expr(e) | KStmt::Return(e) => expr_size(e),
        KStmt::Nothing(inner) => stmt_size(inner),
        KStmt::If {
            cond,
            then,
            otherwise,
            ..
        } => expr_size(cond) + body_size(then) + body_size(otherwise),
        KStmt::While { cond, body } => expr_size(cond) + body_size(body),
        KStmt::Break => 0,
    }
}

fn expr_size(e: &KExpr) -> usize {
    1 + match e {
        KExpr::Int(_) | KExpr::Float(_) | KExpr::Bool(_) | KExpr::Local(..) => 0,
        KExpr::Arith { left, right, .. }
        | KExpr::Cmp { left, right, .. }
        | KExpr::And(left, right, _)
        | KExpr::Or(left, right, _) => expr_size(left) + expr_size(right),
        KExpr::Neg(inner, _) | KExpr::Not(inner) | KExpr::Assign(_, inner, _) => expr_size(inner),
        KExpr::Select {
            cond,
            then,
            otherwise,
            ..
        } => expr_size(cond) + expr_size(then) + expr_size(otherwise),
        KExpr::Call { args, .. } => args.iter().map(expr_size).sum(),
    }
}
