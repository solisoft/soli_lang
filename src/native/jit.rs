//! The JIT module that owns the machine code, and the per-kernel entry
//! trampolines the engines call through.
//!
//! All of the `unsafe` of the native tier lives here: building a callable
//! from a code pointer, and handing raw pointers to generated code.

use web_time::Instant;

use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlagsData, UserFuncName};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, FuncId, Linkage, Module};

use super::ir::{KFunc, Ty};
use super::lower::{kernel_signature, Lowerer};

/// What a kernel reports besides its value. Generated code writes it through
/// the pointer each kernel receives first; `lower::CTX_STATUS` and
/// `lower::CTX_REASON` are the field offsets.
#[repr(C)]
#[derive(Default)]
pub struct KernelCtx {
    /// 0: the value is good. 1: declined.
    pub status: u32,
    /// A [`super::Decline`] code when declined.
    pub reason: u32,
}

/// `t(ctx, depth, args, out)`: loads the arguments from `args` (one 8-byte
/// slot each), calls the kernel, and stores its value at `out`.
pub type Entry = unsafe extern "C" fn(*mut KernelCtx, i64, *const u64, *mut u64);

/// The machine code of one kernel set. Freed when the set is dropped.
pub struct Jit {
    module: Option<JITModule>,
}

// SAFETY: once `finalize_definitions` has run the module is never touched
// again until `Drop`, and the code it owns is immutable. Sharing it between
// threads (the compiled-module cache is process-wide) only shares read-only
// executable memory.
unsafe impl Send for Jit {}
unsafe impl Sync for Jit {}

impl Drop for Jit {
    fn drop(&mut self) {
        if let Some(module) = self.module.take() {
            // SAFETY: the last `KernelRef` into this set is gone (the set
            // owns the `Jit`), so no entry pointer can be called again.
            unsafe { module.free_memory() };
        }
    }
}

/// `%` on two Floats, called from generated code: Rust's `f64 %`, which is
/// what both engines compute (NaN for a zero divisor, no error).
extern "C" fn soli_native_fmod(a: f64, b: f64) -> f64 {
    a % b
}

pub struct Compiled {
    pub jit: Jit,
    pub entries: Vec<Entry>,
    /// Compile time of the whole set, for the log.
    pub elapsed_ms: f64,
    /// Textual IR per kernel, kept only when the log asks for it.
    pub ir: Vec<String>,
}

/// Compile every kernel of the set. `Err` carries why the host refused
/// (unsupported CPU, no executable memory): the caller then runs without
/// kernels rather than failing the program.
pub fn compile(funcs: &[KFunc], max_depth: i64, keep_ir: bool) -> Result<Compiled, String> {
    let started = Instant::now();

    let mut flags = settings::builder();
    let set = |flags: &mut settings::Builder, name: &str, value: &str| {
        flags
            .set(name, value)
            .map_err(|e| format!("cranelift flag {name}: {e}"))
    };
    set(&mut flags, "opt_level", "speed")?;
    // The two flags `JITBuilder::with_flags` sets, for the same reasons: JIT
    // memory can land anywhere relative to the symbols it calls.
    set(&mut flags, "use_colocated_libcalls", "false")?;
    set(
        &mut flags,
        "is_pic",
        if cfg!(target_arch = "x86_64") {
            "true"
        } else {
            "false"
        },
    )?;
    // `JITBuilder::with_flags` panics on an unsupported host; this does not.
    let isa = cranelift_native::builder()
        .map_err(|e| format!("host not supported by Cranelift: {e}"))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| format!("cranelift ISA: {e}"))?;

    let mut builder = JITBuilder::with_isa(isa, default_libcall_names());
    builder.symbol("soli_native_fmod", soli_native_fmod as *const u8);
    let mut module = JITModule::new(builder);

    let mut fmod_sig = module.make_signature();
    fmod_sig.params.push(AbiParam::new(types::F64));
    fmod_sig.params.push(AbiParam::new(types::F64));
    fmod_sig.returns.push(AbiParam::new(types::F64));
    let fmod = module
        .declare_function("soli_native_fmod", Linkage::Import, &fmod_sig)
        .map_err(|e| e.to_string())?;

    let mut func_ids: Vec<FuncId> = Vec::with_capacity(funcs.len());
    for (i, func) in funcs.iter().enumerate() {
        let sig = kernel_signature(&module, func);
        let id = module
            .declare_function(&format!("k{i}_{}", func.name), Linkage::Local, &sig)
            .map_err(|e| e.to_string())?;
        func_ids.push(id);
    }

    let mut ctx = Context::new();
    let mut fb_ctx = FunctionBuilderContext::new();
    let mut ir = Vec::new();

    for (i, func) in funcs.iter().enumerate() {
        ctx.func.signature = kernel_signature(&module, func);
        ctx.func.name = UserFuncName::user(0, func_ids[i].as_u32());
        {
            let builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let builder = Lowerer::new(
                builder,
                &mut module,
                funcs,
                &func_ids,
                fmod,
                max_depth,
                func,
            )
            .lower();
            builder.finalize(module.target_config());
        }
        if keep_ir {
            ir.push(format!("{}", ctx.func.display()));
        }
        module
            .define_function(func_ids[i], &mut ctx)
            .map_err(|e| format!("kernel `{}`: {e:?}", func.name))?;

        module.clear_context(&mut ctx);
    }

    // One trampoline per kernel, so Rust calls every kernel through a single
    // function type whatever its parameters.
    let ptr = module.target_config().pointer_type();
    let mut tramp_ids = Vec::with_capacity(funcs.len());
    for (i, func) in funcs.iter().enumerate() {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(ptr));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(ptr));
        sig.params.push(AbiParam::new(ptr));
        let id = module
            .declare_function(&format!("t{i}_{}", func.name), Linkage::Local, &sig)
            .map_err(|e| e.to_string())?;
        ctx.func.signature = sig;
        ctx.func.name = UserFuncName::user(1, id.as_u32());
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let p = b.block_params(entry).to_vec();
            let (kctx, depth, args, out) = (p[0], p[1], p[2], p[3]);
            let mut call_args = vec![kctx, depth];
            for (slot, ty) in func.params.iter().enumerate() {
                let offset = (slot * 8) as i32;
                let v = match ty {
                    Ty::Int => b
                        .ins()
                        .load(types::I64, MemFlagsData::trusted(), args, offset),
                    Ty::Float => b
                        .ins()
                        .load(types::F64, MemFlagsData::trusted(), args, offset),
                    Ty::Bool => {
                        let wide = b
                            .ins()
                            .load(types::I64, MemFlagsData::trusted(), args, offset);
                        b.ins().ireduce(types::I8, wide)
                    }
                };
                call_args.push(v);
            }
            let callee = module.declare_func_in_func(func_ids[i], b.func);
            let call = b.ins().call(callee, &call_args);
            let result = b.inst_results(call)[0];
            let stored = if func.ret == Ty::Bool {
                b.ins().uextend(types::I64, result)
            } else {
                result
            };
            b.ins().store(MemFlagsData::trusted(), stored, out, 0);
            b.ins().return_(&[]);
            b.seal_all_blocks();
            b.finalize(module.target_config());
        }
        module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("trampoline `{}`: {e:?}", func.name))?;
        module.clear_context(&mut ctx);
        tramp_ids.push(id);
    }

    module
        .finalize_definitions()
        .map_err(|e| format!("finalizing native code: {e}"))?;

    let entries = tramp_ids
        .iter()
        .map(|id| {
            let code = module.get_finalized_function(*id);
            // SAFETY: `code` is the finalized trampoline defined above with
            // the signature (ptr, i64, ptr, ptr) -> (), in the host's default
            // calling convention, which is `extern "C"`.
            unsafe { std::mem::transmute::<*const u8, Entry>(code) }
        })
        .collect();

    Ok(Compiled {
        jit: Jit {
            module: Some(module),
        },
        entries,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        ir,
    })
}
