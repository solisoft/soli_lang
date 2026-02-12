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
        Op::Hash(n) => out.push_str(&format!("HASH         {:>5}", n)),
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
        Op::TryBegin(catch, finally) => {
            out.push_str(&format!("TRY_BEGIN    c:{} f:{}", catch, finally));
        }
        Op::TryEnd => out.push_str("TRY_END"),
        Op::Throw => out.push_str("THROW"),
        Op::GetIter => out.push_str("GET_ITER"),
        Op::ForIter(offset) => out.push_str(&format!("FOR_ITER     {:>5}", offset)),
        Op::Print(n) => out.push_str(&format!("PRINT        {:>5}", n)),
        Op::NamedArg(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("NAMED_ARG    {:>5} ({})", idx, name));
        }
        Op::Import(idx) => {
            let name = constant_string(chunk, *idx);
            out.push_str(&format!("IMPORT       {:>5} ({})", idx, name));
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
        None => "???".to_string(),
    }
}
