//! Native typed kernels.
//!
//! A top-level function typed `Int`/`Float`/`Bool` end to end, whose body
//! only does arithmetic, comparisons, branches, loops and calls to other such
//! functions, is compiled to machine code with Cranelift when a script loads:
//!
//! ```soli
//! def fib(n: Int) -> Int
//!   return n if n < 2
//!
//!   fib(n - 1) + fib(n - 2)
//! end
//! ```
//!
//! The function value stays what it was (a `Function` for the tree-walker, a
//! `VmClosure` for the VM); it only *carries* a [`KernelRef`]. Each call tries
//! the kernel first, and the kernel either returns the value or **declines**:
//! on an overflow, a division by zero, the depth limit, a path that falls off
//! the end, an argument of an unexpected type, or a callee that has been
//! rebound since. A declined call is run by the engine exactly as before,
//! which raises exactly the error it always raised — kernels are pure, so
//! running the call a second time changes nothing. The kernel never has to
//! reproduce an error, only to notice one.
//!
//! `SOLI_NATIVE=0` (or `--no-native`) turns the tier off;
//! `SOLI_NATIVE_LOG=1` says what was compiled, what was refused and why, and
//! the first decline of each kernel; `SOLI_NATIVE_LOG=2` adds the IR.

pub mod eligibility;
pub mod ir;
#[cfg(feature = "native")]
mod jit;
#[cfg(feature = "native")]
mod lower;
#[cfg(all(test, feature = "native"))]
mod tests;

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::OnceLock;

use crate::ast::{FunctionDecl, Program};

/// Why a kernel handed a call back to its engine. The code is what generated
/// code stores in `KernelCtx::reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum Decline {
    Overflow = 1,
    DivisionByZero = 2,
    Depth = 3,
    FellThrough = 4,
    /// Not generated code: the arguments did not have the declared types.
    Arguments = 5,
    /// Not generated code: a kernel it calls is no longer what the name is
    /// bound to.
    Rebound = 6,
}

impl Decline {
    fn from_code(code: u32) -> Decline {
        match code {
            1 => Decline::Overflow,
            2 => Decline::DivisionByZero,
            3 => Decline::Depth,
            5 => Decline::Arguments,
            6 => Decline::Rebound,
            _ => Decline::FellThrough,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Decline::Overflow => "integer overflow",
            Decline::DivisionByZero => "division by zero",
            Decline::Depth => "call depth limit",
            Decline::FellThrough => "no value returned",
            Decline::Arguments => "argument of another type",
            Decline::Rebound => "a function it calls is not defined, or was redefined",
        }
    }
}

static DISABLED_BY_FLAG: AtomicBool = AtomicBool::new(false);

/// Whether kernels are compiled at all: on unless `SOLI_NATIVE` says
/// `0`/`off`/`false`/`no`, or [`set_enabled`] turned them off.
pub fn enabled() -> bool {
    if !cfg!(feature = "native") || DISABLED_BY_FLAG.load(Ordering::Relaxed) {
        return false;
    }
    static FROM_ENV: OnceLock<bool> = OnceLock::new();
    *FROM_ENV.get_or_init(|| {
        !matches!(
            std::env::var("SOLI_NATIVE")
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
                .as_str(),
            "0" | "off" | "false" | "no"
        )
    })
}

/// `--no-native`, and the tests' way to compare against the engines alone.
pub fn set_enabled(on: bool) {
    DISABLED_BY_FLAG.store(!on, Ordering::Relaxed);
}

/// `SOLI_NATIVE_LOG`: 0 silent, 1 decisions, 2 decisions and IR.
pub fn log_level() -> u8 {
    static LEVEL: AtomicU8 = AtomicU8::new(u8::MAX);
    let level = LEVEL.load(Ordering::Relaxed);
    if level != u8::MAX {
        return level;
    }
    let parsed = std::env::var("SOLI_NATIVE_LOG")
        .ok()
        .and_then(|v| v.trim().parse::<u8>().ok())
        .unwrap_or(0);
    LEVEL.store(parsed, Ordering::Relaxed);
    parsed
}

/// The identity of one kernel, cheap to compare: the set's address and the
/// kernel's index in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelId(usize, u32);

#[cfg(not(feature = "native"))]
pub use disabled_impl::*;
#[cfg(feature = "native")]
pub use enabled_impl::*;

#[cfg(feature = "native")]
mod enabled_impl {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::eligibility::MAX_ARGS;
    use super::ir::Ty;
    use super::jit::{Entry, Jit, KernelCtx};
    use super::*;
    use crate::interpreter::value::Value;

    struct KernelInfo {
        name: String,
        params: Vec<Ty>,
        ret: Ty,
        /// This kernel and every kernel it can reach, by name and index: all
        /// must still be bound to themselves for the machine code (which
        /// calls them directly) to mean what the source means.
        deps: Vec<(String, u32)>,
        entry: Entry,
        decline_logged: AtomicBool,
    }

    struct Inner {
        kernels: Vec<KernelInfo>,
        /// `FunctionDecl` address → kernel index.
        by_decl: HashMap<usize, u32>,
        _jit: Jit,
    }

    /// The kernels compiled for one program.
    #[derive(Clone)]
    pub struct KernelSet {
        inner: Arc<Inner>,
    }

    impl std::fmt::Debug for KernelSet {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "KernelSet({} kernels)", self.inner.kernels.len())
        }
    }

    /// A handle on one kernel, carried by the function value it accelerates.
    #[derive(Clone)]
    pub struct KernelRef {
        inner: Arc<Inner>,
        idx: u32,
    }

    impl std::fmt::Debug for KernelRef {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "KernelRef({})",
                self.inner.kernels[self.idx as usize].name
            )
        }
    }

    impl KernelSet {
        /// The kernel built from this exact declaration, if any. Keyed by
        /// address, so a nested `def` or a method of the same name never
        /// matches: only the declaration that was analysed does.
        pub fn for_decl(&self, decl: &FunctionDecl) -> Option<KernelRef> {
            let key = decl as *const FunctionDecl as usize;
            self.inner.by_decl.get(&key).map(|idx| KernelRef {
                inner: self.inner.clone(),
                idx: *idx,
            })
        }

        pub fn len(&self) -> usize {
            self.inner.kernels.len()
        }

        pub fn is_empty(&self) -> bool {
            self.inner.kernels.is_empty()
        }
    }

    impl KernelRef {
        pub fn id(&self) -> KernelId {
            KernelId(Arc::as_ptr(&self.inner) as usize, self.idx)
        }

        pub fn name(&self) -> &str {
            &self.inner.kernels[self.idx as usize].name
        }

        /// Run the kernel on `args`. `None` means *declined*: the caller runs
        /// the call itself, as if there were no kernel.
        ///
        /// `depth` is the caller's current call depth, counted as its engine
        /// counts it (the depth check is the engine's own, moved inside).
        /// `bound` answers which kernel, if any, a global name is bound to
        /// right now.
        #[inline]
        pub fn try_call(
            &self,
            args: &[Value],
            depth: usize,
            bound: impl Fn(&str) -> Option<KernelId>,
        ) -> Option<Value> {
            let info = &self.inner.kernels[self.idx as usize];
            if args.len() != info.params.len() || args.len() > MAX_ARGS {
                return self.declined(Decline::Arguments);
            }
            let mut slots = [0u64; MAX_ARGS];
            for (i, (arg, ty)) in args.iter().zip(&info.params).enumerate() {
                slots[i] = match (arg, ty) {
                    (Value::Int(n), Ty::Int) => *n as u64,
                    (Value::Float(f), Ty::Float) => f.to_bits(),
                    (Value::Bool(b), Ty::Bool) => *b as u64,
                    _ => return self.declined(Decline::Arguments),
                };
            }
            let set = Arc::as_ptr(&self.inner) as usize;
            for (name, idx) in &info.deps {
                if bound(name) != Some(KernelId(set, *idx)) {
                    return self.declined(Decline::Rebound);
                }
            }

            let mut ctx = KernelCtx::default();
            let mut out = 0u64;
            // SAFETY: `entry` is a finalized trampoline of this set, which
            // `self.inner` keeps alive. It reads `info.params.len()` slots
            // from `slots` (checked above to fit), writes 8 bytes at `out`,
            // and writes `ctx` through the pointer; all three outlive the
            // call.
            unsafe {
                (info.entry)(&mut ctx, depth as i64, slots.as_ptr(), &mut out);
            }
            if ctx.status != 0 {
                return self.declined(Decline::from_code(ctx.reason));
            }
            Some(match info.ret {
                Ty::Int => Value::Int(out as i64),
                Ty::Float => Value::Float(f64::from_bits(out)),
                Ty::Bool => Value::Bool(out & 0xff != 0),
            })
        }

        #[cold]
        fn declined(&self, why: Decline) -> Option<Value> {
            let info = &self.inner.kernels[self.idx as usize];
            if log_level() >= 1 && !info.decline_logged.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "native: {} declined ({}) — running it interpreted (logged once)",
                    info.name,
                    why.describe()
                );
            }
            None
        }
    }

    /// Analyse `program` and compile its kernels. `None` when the tier is off,
    /// when nothing qualifies, or when the host cannot run generated code.
    pub fn analyze_and_compile(
        program: &Program,
        is_builtin: &dyn Fn(&str) -> bool,
    ) -> Option<KernelSet> {
        if !enabled() {
            return None;
        }
        let level = log_level();
        let analysis = super::eligibility::analyze(program, is_builtin);
        if level >= 1 {
            for refusal in &analysis.refusals {
                eprintln!(
                    "native: {} (line {}) refused — {}",
                    refusal.name, refusal.line, refusal.reason
                );
            }
        }
        let funcs = analysis.funcs;
        let decls = analysis.decls;
        if funcs.is_empty() {
            return None;
        }

        let max_depth = crate::interpreter::executor::MAX_CALL_DEPTH as i64;
        let compiled = match super::jit::compile(&funcs, max_depth, level >= 2) {
            Ok(compiled) => compiled,
            Err(why) => {
                if level >= 1 {
                    eprintln!("native: no kernels — {why}");
                }
                return None;
            }
        };

        let deps = transitive_deps(&funcs);
        let mut kernels = Vec::with_capacity(funcs.len());
        let mut by_decl = HashMap::with_capacity(funcs.len());
        for (i, func) in funcs.iter().enumerate() {
            if level >= 1 {
                let params: Vec<&str> = func.params.iter().map(|t| t.name()).collect();
                eprintln!(
                    "native: {}({}) -> {} compiled (line {})",
                    func.name,
                    params.join(", "),
                    func.ret.name(),
                    func.line
                );
            }
            if level >= 2 {
                eprintln!("{}", compiled.ir[i]);
            }
            by_decl.insert(decls[i] as *const FunctionDecl as usize, i as u32);
            kernels.push(KernelInfo {
                name: func.name.clone(),
                params: func.params.clone(),
                ret: func.ret,
                deps: deps[i]
                    .iter()
                    .map(|d| (funcs[*d as usize].name.clone(), *d))
                    .collect(),
                entry: compiled.entries[i],
                decline_logged: AtomicBool::new(false),
            });
        }
        if level >= 1 {
            eprintln!(
                "native: {} kernel(s) compiled in {:.2} ms",
                kernels.len(),
                compiled.elapsed_ms
            );
        }
        Some(KernelSet {
            inner: Arc::new(Inner {
                kernels,
                by_decl,
                _jit: compiled.jit,
            }),
        })
    }

    fn transitive_deps(funcs: &[super::ir::KFunc]) -> Vec<Vec<u32>> {
        (0..funcs.len())
            .map(|start| {
                let mut seen = vec![false; funcs.len()];
                let mut stack = vec![start as u32];
                let mut out = Vec::new();
                while let Some(k) = stack.pop() {
                    if std::mem::replace(&mut seen[k as usize], true) {
                        continue;
                    }
                    out.push(k);
                    stack.extend(funcs[k as usize].callees.iter().copied());
                }
                out
            })
            .collect()
    }
}

#[cfg(not(feature = "native"))]
mod disabled_impl {
    use super::*;
    use crate::interpreter::value::Value;

    /// Without the `native` feature no kernel exists; these types only keep
    /// the call sites compiling.
    #[derive(Debug, Clone)]
    pub struct KernelSet(std::convert::Infallible);

    #[derive(Debug, Clone)]
    pub struct KernelRef(std::convert::Infallible);

    impl KernelSet {
        pub fn for_decl(&self, _decl: &FunctionDecl) -> Option<KernelRef> {
            match self.0 {}
        }
        pub fn len(&self) -> usize {
            match self.0 {}
        }
        pub fn is_empty(&self) -> bool {
            match self.0 {}
        }
    }

    impl KernelRef {
        pub fn id(&self) -> KernelId {
            match self.0 {}
        }
        pub fn name(&self) -> &str {
            match self.0 {}
        }
        pub fn try_call(
            &self,
            _args: &[Value],
            _depth: usize,
            _bound: impl Fn(&str) -> Option<KernelId>,
        ) -> Option<Value> {
            match self.0 {}
        }
    }

    pub fn analyze_and_compile(
        _program: &Program,
        _is_builtin: &dyn Fn(&str) -> bool,
    ) -> Option<KernelSet> {
        None
    }
}
