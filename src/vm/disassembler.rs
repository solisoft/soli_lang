//! Bytecode disassembler for debug output.

use super::chunk::{Chunk, Constant, FunctionProto};
use super::opcode::Op;

/// Disassemble a function prototype to a human-readable string.
pub fn disassemble(proto: &FunctionProto) -> String {
    let mut out = String::new();
    let name = if proto.name.is_empty() {
        "<script>"
    } else {
        &proto.name
    };
    out.push_str(&format!(
        "== {} (arity={}, upvalues={}) ==\n",
        name,
        proto.arity,
        proto.upvalue_descriptors.len()
    ));
    disassemble_chunk(&proto.chunk, &mut out);

    // Recursively disassemble nested functions
    for constant in &proto.chunk.constants {
        if let Constant::Function(nested) = constant {
            out.push('\n');
            out.push_str(&disassemble(nested));
        }
    }
    out
}

fn disassemble_chunk(chunk: &Chunk, out: &mut String) {
    for (offset, op) in chunk.code.iter().enumerate() {
        let line = chunk.lines.get(offset).copied().unwrap_or(0);
        let line_str = if offset > 0 && chunk.lines.get(offset - 1).copied() == Some(line) {
            "   |".to_string()
        } else {
            format!("{:4}", line)
        };
        out.push_str(&format!("{:04} {} ", offset, line_str));
        disassemble_op(op, chunk, out);
        out.push('\n');
    }
}

fn disassemble_op(op: &Op, chunk: &Chunk, out: &mut String) {
    match op {
        Op::Constant(idx) => {
            let val = chunk.constants.get(*idx as usize);
            out.push_str(&format!("CONSTANT {:>5} ({})", idx, format_constant(val)));
        }
        Op::Symbol(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("SYMBOL       {:>5} (:{})", idx, name));
        }
        Op::Null => out.push_str("NULL"),
        Op::True => out.push_str("TRUE"),
        Op::False => out.push_str("FALSE"),
        Op::Pop => out.push_str("POP"),
        Op::Dup => out.push_str("DUP"),
        Op::GetLocal(slot) => out.push_str(&format!("GET_LOCAL    {:>5}", slot)),
        Op::SetLocal(slot) => out.push_str(&format!("SET_LOCAL    {:>5}", slot)),
        Op::GetGlobal(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("GET_GLOBAL   {:>5} ({})", idx, name));
        }
        Op::SetGlobal(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("SET_GLOBAL   {:>5} ({})", idx, name));
        }
        Op::DefineGlobal(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("DEF_GLOBAL   {:>5} ({})", idx, name));
        }
        Op::GetUpvalue(idx) => out.push_str(&format!("GET_UPVALUE  {:>5}", idx)),
        Op::SetUpvalue(idx) => out.push_str(&format!("SET_UPVALUE  {:>5}", idx)),
        Op::CloseUpvalue => out.push_str("CLOSE_UPVALUE"),
        Op::Add => out.push_str("ADD"),
        Op::Subtract => out.push_str("SUBTRACT"),
        Op::Multiply => out.push_str("MULTIPLY"),
        Op::Divide => out.push_str("DIVIDE"),
        Op::Modulo => out.push_str("MODULO"),
        Op::Negate => out.push_str("NEGATE"),
        Op::Equal => out.push_str("EQUAL"),
        Op::NotEqual => out.push_str("NOT_EQUAL"),
        Op::Less => out.push_str("LESS"),
        Op::LessEqual => out.push_str("LESS_EQUAL"),
        Op::Greater => out.push_str("GREATER"),
        Op::GreaterEqual => out.push_str("GREATER_EQUAL"),
        Op::Not => out.push_str("NOT"),
        Op::Jump(offset) => out.push_str(&format!("JUMP         {:>5}", offset)),
        Op::JumpIfFalse(offset) => out.push_str(&format!("JUMP_IF_FALSE {:>4}", offset)),
        Op::Loop(offset) => out.push_str(&format!("LOOP         {:>5}", offset)),
        Op::JumpIfFalseNoPop(offset) => {
            out.push_str(&format!("JUMP_FALSE_NP {:>4}", offset));
        }
        Op::JumpIfTrueNoPop(offset) => {
            out.push_str(&format!("JUMP_TRUE_NP  {:>4}", offset));
        }
        Op::NullishJump(offset) => out.push_str(&format!("NULLISH_JUMP {:>5}", offset)),
        Op::Call(argc) => out.push_str(&format!("CALL         {:>5}", argc)),
        Op::Closure(idx) => {
            let val = chunk.constants.get(*idx as usize);
            out.push_str(&format!(
                "CLOSURE      {:>5} ({})",
                idx,
                format_constant(val)
            ));
        }
        Op::Return => out.push_str("RETURN"),
        Op::Array(n) => out.push_str(&format!("ARRAY        {:>5}", n)),
        Op::ArrayPush => out.push_str("ARRAY_PUSH"),
        Op::Hash(n) => out.push_str(&format!("HASH         {:>5}", n)),
        Op::HashWithKeys(idx, n) => out.push_str(&format!("HASH_W_KEYS  k={:>3} n={:>3}", idx, n)),
        Op::Range => out.push_str("RANGE"),
        Op::GetIndex => out.push_str("GET_INDEX"),
        Op::SetIndex => out.push_str("SET_INDEX"),
        Op::BuildString(n) => out.push_str(&format!("BUILD_STRING {:>5}", n)),
        Op::Spread => out.push_str("SPREAD"),
        Op::GetProperty(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("GET_PROPERTY {:>5} ({})", idx, name));
        }
        Op::SetProperty(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("SET_PROPERTY {:>5} ({})", idx, name));
        }
        Op::Class(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("CLASS        {:>5} ({})", idx, name));
        }
        Op::Inherit => out.push_str("INHERIT"),
        Op::Method(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("METHOD       {:>5} ({})", idx, name));
        }
        Op::StaticMethod(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("STATIC_METH  {:>5} ({})", idx, name));
        }
        Op::New(argc) => out.push_str(&format!("NEW          {:>5}", argc)),
        Op::GetThis => out.push_str("GET_THIS"),
        Op::GetSuper(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("GET_SUPER    {:>5} ({})", idx, name));
        }
        Op::Field(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("FIELD        {:>5} ({})", idx, name));
        }
        Op::StaticField(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("STATIC_FIELD {:>5} ({})", idx, name));
        }
        Op::ConstField(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("CONST_FIELD  {:>5} ({})", idx, name));
        }
        Op::StaticConstField(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("ST_CONST_FLD {:>5} ({})", idx, name));
        }
        Op::TryBegin(catch, finally) => {
            out.push_str(&format!("TRY_BEGIN    c:{} f:{}", catch, finally));
        }
        Op::TryEnd => out.push_str("TRY_END"),
        Op::Throw => out.push_str("THROW"),
        Op::CatchMatch(idx, offset) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!(
                "CATCH_MATCH  {:>5} ({}) jump:{}",
                idx, name, offset
            ));
        }
        Op::Rethrow => out.push_str("RETHROW"),
        Op::GetIter => out.push_str("GET_ITER"),
        Op::GetIterRange => out.push_str("GET_ITER_RNG"),
        Op::ForIter(offset) => out.push_str(&format!("FOR_ITER     {:>5}", offset)),
        Op::ForIterRange(offset) => out.push_str(&format!("FOR_ITER_RNG {:>5}", offset)),
        Op::Print(n) => out.push_str(&format!("PRINT        {:>5}", n)),
        Op::NamedArg(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("NAMED_ARG    {:>5} ({})", idx, name));
        }
        Op::Import(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("IMPORT       {:>5} ({})", idx, name));
        }
        Op::JsonParse => out.push_str("JSON_PARSE"),
        Op::JsonStringify => out.push_str("JSON_STRINGIFY"),
        Op::IncrLocal(slot) => out.push_str(&format!("INCR_LOCAL   {:>5}", slot)),
        Op::DecrLocal(slot) => out.push_str(&format!("DECR_LOCAL   {:>5}", slot)),
        Op::AddLocalLocal(a, b) => out.push_str(&format!("ADD_LL       {:>3},{:>3}", a, b)),
        Op::LessEqualLocalLocal(a, b) => out.push_str(&format!("LE_LL        {:>3},{:>3}", a, b)),
        Op::AddLocalConst(slot, cidx) => {
            out.push_str(&format!("ADD_LC       {:>3},{:>3}", slot, cidx))
        }
        Op::SetLocalPop(slot) => out.push_str(&format!("SET_LOCAL_POP {:>4}", slot)),
        Op::TestLessEqualJump(offset) => out.push_str(&format!("TEST_LE_JUMP {:>5}", offset)),
        Op::TestLessJump(offset) => out.push_str(&format!("TEST_LT_JUMP {:>5}", offset)),
        Op::CallGlobal(idx, argc) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("CALL_GLOBAL  {:>5} ({}) argc={}", idx, name, argc));
        }
        Op::Nop => out.push_str("NOP"),
        Op::CallMethod(idx, argc) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("CALL_METHOD  {:>5} ({}) argc={}", idx, name, argc));
        }
        Op::CallMethodById(idx, argc, mid) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!(
                "CALL_MID     {:>5} ({}) argc={} mid={}",
                idx, name, argc, mid
            ));
        }
        Op::HashGetConst(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("HASH_GET_C   {:>5} ({})", idx, name));
        }
        Op::HashHasKeyConst(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("HASH_HAS_C   {:>5} ({})", idx, name));
        }
        Op::HashDeleteConst(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("HASH_DEL_C   {:>5} ({})", idx, name));
        }
        Op::HashSetConst(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("HASH_SET_C   {:>5} ({})", idx, name));
        }
        Op::HashGetLocalConst(slot, idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("HGET_L_C     slot={} key={}", slot, name));
        }
        Op::HashHasKeyLocalConst(slot, idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("HHAS_L_C     slot={} key={}", slot, name));
        }
        Op::HashDeleteLocalConst(slot, idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("HDEL_L_C     slot={} key={}", slot, name));
        }
        Op::HashSetLocalConst(slot, idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("HSET_L_C     slot={} key={}", slot, name));
        }
        Op::HashGetGlobalConst(gidx, kidx) => {
            let g = constant_string(chunk, *gidx);
            let k = constant_string(chunk, *kidx);
            out.push_str(&format!("HGET_G_C     {:>5} ({}) key={}", gidx, g, k));
        }
        Op::HashHasKeyGlobalConst(gidx, kidx) => {
            let g = constant_string(chunk, *gidx);
            let k = constant_string(chunk, *kidx);
            out.push_str(&format!("HHAS_G_C     {:>5} ({}) key={}", gidx, g, k));
        }
        Op::HashDeleteGlobalConst(gidx, kidx) => {
            let g = constant_string(chunk, *gidx);
            let k = constant_string(chunk, *kidx);
            out.push_str(&format!("HDEL_G_C     {:>5} ({}) key={}", gidx, g, k));
        }
        Op::HashSetGlobalConst(gidx, kidx) => {
            let g = constant_string(chunk, *gidx);
            let k = constant_string(chunk, *kidx);
            out.push_str(&format!("HSET_G_C     {:>5} ({}) key={}", gidx, g, k));
        }
        Op::SubLocalLocal(a, b) => out.push_str(&format!("SUB_LL       {:>3},{:>3}", a, b)),
        Op::MulLocalLocal(a, b) => out.push_str(&format!("MUL_LL       {:>3},{:>3}", a, b)),
        Op::DivLocalLocal(a, b) => out.push_str(&format!("DIV_LL       {:>3},{:>3}", a, b)),
        Op::ModLocalLocal(a, b) => out.push_str(&format!("MOD_LL       {:>3},{:>3}", a, b)),
        Op::SubLocalConst(slot, cidx) => {
            out.push_str(&format!("SUB_LC       {:>3},{:>3}", slot, cidx))
        }
        Op::MulLocalConst(slot, cidx) => {
            out.push_str(&format!("MUL_LC       {:>3},{:>3}", slot, cidx))
        }
        Op::DivLocalConst(slot, cidx) => {
            out.push_str(&format!("DIV_LC       {:>3},{:>3}", slot, cidx))
        }
        Op::GetLocal2(a, b) => out.push_str(&format!("GET_LOCAL_2  {:>3},{:>3}", a, b)),
        Op::LessLocalLocal(a, b) => out.push_str(&format!("LESS_LL      {:>3},{:>3}", a, b)),
        Op::GreaterLocalLocal(a, b) => out.push_str(&format!("GREATER_LL   {:>3},{:>3}", a, b)),
        Op::NotEqualLocalConst(slot, cidx) => {
            out.push_str(&format!("NE_LC        {:>3},{:>3}", slot, cidx))
        }
        Op::EqualLocalConst(slot, cidx) => {
            out.push_str(&format!("EQ_LC        {:>3},{:>3}", slot, cidx))
        }
        Op::TestGreaterJump(offset) => out.push_str(&format!("TEST_GT_JUMP {:>5}", offset)),
        Op::TestGreaterEqualJump(offset) => out.push_str(&format!("TEST_GE_JUMP {:>5}", offset)),
        Op::TestNotEqualJump(offset) => out.push_str(&format!("TEST_NE_JUMP {:>5}", offset)),
        Op::IsNull => out.push_str("IS_NULL"),
        Op::NotNull => out.push_str("NOT_NULL"),
        Op::JumpIfNull(offset) => out.push_str(&format!("JUMP_IF_NULL {:>5}", offset)),
        Op::JumpIfNotNull(offset) => out.push_str(&format!("JUMP_IF_NOT_NULL {:>4}", offset)),
        Op::IsTruthyLocal(slot) => out.push_str(&format!("IS_TRUTHY_LOCAL {:>3}", slot)),
        Op::IsFalsyLocal(slot) => out.push_str(&format!("IS_FALSY_LOCAL {:>3}", slot)),
        Op::AddLocalInt(slot, n) => out.push_str(&format!("ADD_LOCAL_INT {:>3},{:>3}", slot, n)),
        Op::IncrLocalFast(slot) => out.push_str(&format!("INCR_LOCAL_FAST {:>2}", slot)),
        Op::GetAndNullLocal(slot) => out.push_str(&format!("GET_AND_NULL_LOCAL {:>2}", slot)),
        Op::IsZeroLocal(slot) => out.push_str(&format!("IS_ZERO_LOCAL  {:>3}", slot)),
        Op::NotZeroLocal(slot) => out.push_str(&format!("NOT_ZERO_LOCAL {:>3}", slot)),
        Op::GetAndIncrLocal(slot) => out.push_str(&format!("GET_AND_INCR_LOCAL {:>2}", slot)),
        Op::GetAndDecrLocal(slot) => out.push_str(&format!("GET_AND_DECR_LOCAL {:>2}", slot)),
        Op::SwapSetLocal(slot) => out.push_str(&format!("SWAP_SET_LOCAL   {:>3}", slot)),
        Op::GetGlobalNullCheck(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("GET_GLOBAL_NULL  {:>5} ({})", idx, name))
        }
        Op::GetGlobalCall(idx, argc) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!(
                "GET_GLOBAL_CALL  {:>5} ({}) argc={}",
                idx, name, argc
            ))
        }
        Op::NotLocal(slot) => out.push_str(&format!("NOT_LOCAL        {:>3}", slot)),
        Op::NegateLocal(slot) => out.push_str(&format!("NEGATE_LOCAL     {:>3}", slot)),
        Op::EqualLocalLocal(a, b) => out.push_str(&format!("EQ_LL            {:>3},{:>3}", a, b)),
        Op::NotEqualLocalLocal(a, b) => {
            out.push_str(&format!("NE_LL            {:>3},{:>3}", a, b))
        }
        Op::PopNull => out.push_str("POP_NULL"),
        Op::DupN(n) => out.push_str(&format!("DUP_N            {:>3}", n)),
        Op::GetLocalProperty(slot, idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!(
                "GET_LOCAL_PROP   {:>3},{:>3} ({})",
                slot, idx, name
            ))
        }
        Op::GetLocalIndex(slot, idx_slot) => {
            out.push_str(&format!("GET_LOCAL_INDEX  {:>3},{:>3}", slot, idx_slot))
        }
    }
}

fn constant_string(chunk: &Chunk, idx: u16) -> String {
    match chunk.constants.get(idx as usize) {
        Some(Constant::String(s)) => s.clone(),
        _ => format!("?{}", idx),
    }
}

fn format_constant(val: Option<&Constant>) -> String {
    match val {
        Some(Constant::Int(n)) => format!("{}", n),
        Some(Constant::Float(n)) => format!("{}", n),
        Some(Constant::Decimal(s)) => format!("{}D", s),
        Some(Constant::String(s)) => format!("\"{}\"", s),
        Some(Constant::Bool(b)) => format!("{}", b),
        Some(Constant::Null) => "null".to_string(),
        Some(Constant::Function(f)) => format!("<fn {}>", f.name),
        Some(Constant::HashKeys(ks)) => format!("HashKeys[{}]", ks.len()),
        None => "???".to_string(),
    }
}
