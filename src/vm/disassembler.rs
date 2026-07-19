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
        Op::JumpIfParamSupplied(param_index, offset) => out.push_str(&format!(
            "JUMP_IF_PARAM_SUPPLIED p{} {:>5}",
            param_index, offset
        )),
        Op::Call(argc) => out.push_str(&format!("CALL         {:>5}", argc)),
        Op::CallNamed(argc, names_idx) => out.push_str(&format!(
            "CALL_NAMED   {:>5}  {}",
            argc,
            format_constant(chunk.constants.get(*names_idx as usize))
        )),
        Op::NewNamed(argc, names_idx) => out.push_str(&format!(
            "NEW_NAMED    {:>5}  {}",
            argc,
            format_constant(chunk.constants.get(*names_idx as usize))
        )),
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
        Op::PopHandler => out.push_str("POP_HANDLER"),
        Op::RescueJump(offset) => out.push_str(&format!("RESCUE_JUMP  {:>5}", offset)),
        Op::GetIter => out.push_str("GET_ITER"),
        Op::GetIterRange => out.push_str("GET_ITER_RNG"),
        Op::ForIter(offset) => out.push_str(&format!("FOR_ITER     {:>5}", offset)),
        Op::ForIterRange(offset) => out.push_str(&format!("FOR_ITER_RNG {:>5}", offset)),
        Op::Print(n) => out.push_str(&format!("PRINT        {:>5}", n)),
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
        Op::CallSuperInit(argc) => {
            out.push_str(&format!("CALL_SUPER_INIT    argc={}", argc));
        }
        Op::CallSuperMethod(idx, argc) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!(
                "CALL_SUPER_METHOD {:>5} ({}) argc={}",
                idx, name, argc
            ));
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
        Some(Constant::String(s)) => s.clone().to_string(),
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
        Some(Constant::ArgNames(names)) => {
            let rendered: Vec<&str> = names
                .iter()
                .map(|n| n.as_ref().map(|s| s.as_ref()).unwrap_or("_"))
                .collect();
            format!("ArgNames[{}]", rendered.join(", "))
        }
        None => "???".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn proto(name: &str) -> FunctionProto {
        FunctionProto::new(name.to_string())
    }

    /// Build a single-instruction chunk and return only the body (no header).
    /// Each instruction is on line 1.
    fn body_for(ops: Vec<(Op, usize)>) -> String {
        let mut p = proto("test");
        for (op, line) in ops {
            p.chunk.emit(op, line);
        }
        let full = disassemble(&p);
        // Strip the header line ("== test (...) ==\n").
        full.lines().skip(1).collect::<Vec<_>>().join("\n")
    }

    // ---------- header ----------

    #[test]
    fn header_uses_script_label_when_name_empty() {
        let p = proto("");
        let out = disassemble(&p);
        assert!(out.starts_with("== <script> "), "got: {out}");
        assert!(out.contains("arity=0"));
        assert!(out.contains("upvalues=0"));
    }

    #[test]
    fn header_includes_real_name_arity_and_upvalue_count() {
        let mut p = proto("greet");
        p.arity = 2;
        p.upvalue_descriptors
            .push(super::super::upvalue::UpvalueDescriptor {
                is_local: true,
                index: 0,
            });
        let out = disassemble(&p);
        assert!(out.starts_with("== greet "));
        assert!(out.contains("arity=2"));
        assert!(out.contains("upvalues=1"));
    }

    // ---------- offset & line gutter ----------

    #[test]
    fn lines_are_zero_padded_offsets() {
        let body = body_for(vec![(Op::Null, 1), (Op::Pop, 1)]);
        // Each line begins with a 4-digit offset.
        assert!(body.starts_with("0000 "), "got: {body}");
        let second = body.lines().nth(1).unwrap();
        assert!(second.starts_with("0001 "), "got: {second}");
    }

    #[test]
    fn repeated_line_numbers_use_pipe_marker() {
        // First instruction shows the line number, subsequent ones on
        // the same source line show "   |" instead.
        let body = body_for(vec![(Op::Null, 5), (Op::Pop, 5), (Op::True, 6)]);
        let lines: Vec<&str> = body.lines().collect();
        // First on line 5 → shows "   5".
        assert!(lines[0].contains("   5"), "got: {}", lines[0]);
        // Second on line 5 → shows the "   |" marker.
        assert!(lines[1].contains("   |"), "got: {}", lines[1]);
        // Third on line 6 → shows "   6".
        assert!(lines[2].contains("   6"), "got: {}", lines[2]);
    }

    // ---------- simple ops ----------

    #[test]
    fn nullary_ops_render_their_name_in_caps() {
        for (op, expected) in [
            (Op::Null, "NULL"),
            (Op::True, "TRUE"),
            (Op::False, "FALSE"),
            (Op::Pop, "POP"),
            (Op::Dup, "DUP"),
            (Op::Add, "ADD"),
            (Op::Subtract, "SUBTRACT"),
            (Op::Multiply, "MULTIPLY"),
            (Op::Divide, "DIVIDE"),
            (Op::Negate, "NEGATE"),
            (Op::Equal, "EQUAL"),
            (Op::NotEqual, "NOT_EQUAL"),
            (Op::Not, "NOT"),
            (Op::IsNull, "IS_NULL"),
            (Op::NotNull, "NOT_NULL"),
            (Op::Return, "RETURN"),
            (Op::CloseUpvalue, "CLOSE_UPVALUE"),
            (Op::PopNull, "POP_NULL"),
        ] {
            let body = body_for(vec![(op, 1)]);
            assert!(body.contains(expected), "for {op:?}: got body {body:?}");
        }
    }

    // ---------- ops referencing constants ----------

    #[test]
    fn constant_op_renders_int_literal() {
        let mut p = proto("test");
        let idx = p.chunk.add_constant(Constant::Int(42));
        p.chunk.emit(Op::Constant(idx), 1);
        let out = disassemble(&p);
        assert!(out.contains("CONSTANT"));
        assert!(out.contains("(42)"));
    }

    #[test]
    fn constant_op_renders_string_quoted() {
        let mut p = proto("test");
        let idx = p.chunk.add_constant(Constant::String("hi".into()));
        p.chunk.emit(Op::Constant(idx), 1);
        let out = disassemble(&p);
        assert!(out.contains(r#"("hi")"#), "got: {out}");
    }

    #[test]
    fn constant_op_renders_bool_null_decimal() {
        let mut p = proto("test");
        let i_b = p.chunk.add_constant(Constant::Bool(true));
        let i_n = p.chunk.add_constant(Constant::Null);
        let i_d = p.chunk.add_constant(Constant::Decimal("19.99".into()));
        p.chunk.emit(Op::Constant(i_b), 1);
        p.chunk.emit(Op::Constant(i_n), 1);
        p.chunk.emit(Op::Constant(i_d), 1);
        let out = disassemble(&p);
        assert!(out.contains("(true)"));
        assert!(out.contains("(null)"));
        // Decimal renders with a trailing 'D' suffix.
        assert!(out.contains("(19.99D)"), "got: {out}");
    }

    #[test]
    fn symbol_op_uses_colon_prefix_when_constant_is_string() {
        let mut p = proto("test");
        let idx = p.chunk.add_constant(Constant::String("name".into()));
        p.chunk.emit(Op::Symbol(idx), 1);
        let out = disassemble(&p);
        assert!(out.contains("SYMBOL"));
        assert!(out.contains("(:name)"), "got: {out}");
    }

    #[test]
    fn get_global_renders_name_from_constant_pool() {
        let mut p = proto("test");
        let idx = p.chunk.add_constant(Constant::String("my_var".into()));
        p.chunk.emit(Op::GetGlobal(idx), 1);
        let out = disassemble(&p);
        assert!(out.contains("GET_GLOBAL"));
        assert!(out.contains("(my_var)"));
    }

    #[test]
    fn get_global_with_non_string_constant_falls_back_to_question_mark_idx() {
        // constant_string returns "?<idx>" when the slot isn't a String.
        let mut p = proto("test");
        let idx = p.chunk.add_constant(Constant::Int(7));
        p.chunk.emit(Op::GetGlobal(idx), 1);
        let out = disassemble(&p);
        assert!(out.contains(&format!("(?{})", idx)), "got: {out}");
    }

    #[test]
    fn slot_ops_render_index() {
        let body = body_for(vec![
            (Op::GetLocal(3), 1),
            (Op::SetLocal(7), 1),
            (Op::GetUpvalue(2), 1),
            (Op::SetUpvalue(4), 1),
        ]);
        assert!(body.contains("GET_LOCAL"));
        assert!(body.contains("SET_LOCAL"));
        assert!(body.contains("GET_UPVALUE"));
        assert!(body.contains("SET_UPVALUE"));
        // The slot indices appear in the output (right-aligned in a
        // 5-wide column, but we check substring-ish).
        assert!(body.contains("3"));
        assert!(body.contains("7"));
    }

    #[test]
    fn jump_ops_render_offset() {
        let body = body_for(vec![
            (Op::Jump(10), 1),
            (Op::JumpIfFalse(20), 1),
            (Op::Loop(5), 1),
            (Op::JumpIfFalseNoPop(2), 1),
            (Op::JumpIfTrueNoPop(3), 1),
            (Op::NullishJump(4), 1),
        ]);
        assert!(body.contains("JUMP "));
        assert!(body.contains("JUMP_IF_FALSE"));
        assert!(body.contains("LOOP"));
        assert!(body.contains("JUMP_FALSE_NP"));
        assert!(body.contains("JUMP_TRUE_NP"));
        assert!(body.contains("NULLISH_JUMP"));
    }

    // ---------- super-instructions ----------

    #[test]
    fn super_ops_render_in_short_form() {
        let body = body_for(vec![
            (Op::IncrLocalFast(3), 1),
            (Op::EqualLocalLocal(1, 2), 1),
            (Op::NotEqualLocalLocal(1, 2), 1),
            (Op::IsTruthyLocal(0), 1),
            (Op::IsFalsyLocal(0), 1),
        ]);
        assert!(body.contains("INCR_LOCAL_FAST"));
        assert!(body.contains("EQ_LL"));
        assert!(body.contains("NE_LL"));
        assert!(body.contains("IS_TRUTHY_LOCAL"));
        assert!(body.contains("IS_FALSY_LOCAL"));
    }

    #[test]
    fn get_local_property_includes_property_name() {
        let mut p = proto("test");
        let idx = p.chunk.add_constant(Constant::String("foo".into()));
        p.chunk.emit(Op::GetLocalProperty(2, idx), 1);
        let out = disassemble(&p);
        assert!(out.contains("GET_LOCAL_PROP"));
        assert!(out.contains("(foo)"));
    }

    // ---------- nested function recursion ----------

    #[test]
    fn nested_function_appears_after_outer_with_blank_line() {
        let mut outer = proto("outer");
        outer.chunk.emit(Op::Null, 1);
        let mut inner = FunctionProto::new("inner".to_string());
        inner.chunk.emit(Op::Return, 1);
        outer
            .chunk
            .add_constant(Constant::Function(Arc::new(inner)));

        let out = disassemble(&outer);
        // Both function headers must appear.
        assert!(out.contains("== outer "));
        assert!(out.contains("== inner "));
        // The inner header appears AFTER the outer header.
        let outer_pos = out.find("== outer ").unwrap();
        let inner_pos = out.find("== inner ").unwrap();
        assert!(outer_pos < inner_pos, "outer should come first");
    }

    #[test]
    fn function_constant_in_pool_renders_as_fn_name() {
        // A Function constant that is NOT emitted as code still needs to
        // appear in `format_constant` output if referenced via `Op::Constant`.
        let mut p = proto("outer");
        let inner = FunctionProto::new("nested".to_string());
        let idx = p.chunk.add_constant(Constant::Function(Arc::new(inner)));
        p.chunk.emit(Op::Constant(idx), 1);
        let out = disassemble(&p);
        assert!(out.contains("(<fn nested>)"), "got: {out}");
    }

    // ---------- HashKeys constant ----------

    #[test]
    fn hash_keys_constant_renders_with_count() {
        use crate::interpreter::value::HashKey;
        let mut p = proto("test");
        let keys = Arc::new(vec![
            HashKey::String("a".into()),
            HashKey::String("b".into()),
        ]);
        let idx = p.chunk.add_constant(Constant::HashKeys(keys));
        p.chunk.emit(Op::Constant(idx), 1);
        let out = disassemble(&p);
        assert!(out.contains("(HashKeys[2])"), "got: {out}");
    }
}
