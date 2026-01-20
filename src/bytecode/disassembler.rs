//! Bytecode disassembler for debugging.

use crate::bytecode::chunk::{Chunk, CompiledFunction, Constant};
use crate::bytecode::instruction::OpCode;
use std::fmt::Write;

/// Disassemble a compiled function into human-readable output.
pub fn disassemble_function(function: &CompiledFunction) -> String {
    let mut output = String::new();

    writeln!(
        &mut output,
        "== {} (arity: {}) ==",
        if function.name.is_empty() {
            "<script>"
        } else {
            &function.name
        },
        function.arity
    )
    .unwrap();

    disassemble_chunk(&function.chunk, &mut output);

    // Disassemble nested functions
    for constant in &function.chunk.constants {
        if let Constant::Function(nested) = constant {
            writeln!(&mut output).unwrap();
            output.push_str(&disassemble_function(nested));
        }
    }

    output
}

/// Disassemble a chunk into human-readable output.
pub fn disassemble_chunk(chunk: &Chunk, output: &mut String) {
    let mut offset = 0;

    while offset < chunk.code.len() {
        offset = disassemble_instruction(chunk, offset, output);
    }
}

/// Disassemble a single instruction.
pub fn disassemble_instruction(chunk: &Chunk, offset: usize, output: &mut String) -> usize {
    // Print offset
    write!(output, "{:04} ", offset).unwrap();

    // Print line number (or | if same as previous)
    let line = chunk.get_line(offset);
    if offset > 0 && line == chunk.get_line(offset - 1) {
        write!(output, "   | ").unwrap();
    } else {
        write!(output, "{:4} ", line).unwrap();
    }

    let byte = chunk.code[offset];
    let opcode = match OpCode::from_u8(byte) {
        Some(op) => op,
        None => {
            writeln!(output, "Unknown opcode {}", byte).unwrap();
            return offset + 1;
        }
    };

    match opcode {
        // Simple instructions (no operands)
        OpCode::Null
        | OpCode::True
        | OpCode::False
        | OpCode::Pop
        | OpCode::Dup
        | OpCode::Add
        | OpCode::Subtract
        | OpCode::Multiply
        | OpCode::Divide
        | OpCode::Modulo
        | OpCode::Negate
        | OpCode::Equal
        | OpCode::NotEqual
        | OpCode::Less
        | OpCode::LessEqual
        | OpCode::Greater
        | OpCode::GreaterEqual
        | OpCode::Not
        | OpCode::Return
        | OpCode::CloseUpvalue
        | OpCode::Inherit
        | OpCode::GetThis
        | OpCode::GetSuper
        | OpCode::Index
        | OpCode::IndexSet
        | OpCode::GetIterator
        | OpCode::SpreadArray
        | OpCode::SpreadHash
        | OpCode::Print => {
            writeln!(output, "{:?}", opcode).unwrap();
            offset + 1
        }

        // One byte operand
        OpCode::Call | OpCode::GetUpvalue | OpCode::SetUpvalue => {
            let operand = chunk.code[offset + 1];
            writeln!(output, "{:?} {}", opcode, operand).unwrap();
            offset + 2
        }

        // Two byte operand (constant index or jump offset)
        OpCode::Constant => {
            let idx = chunk.read_u16(offset + 1);
            let constant = &chunk.constants[idx as usize];
            writeln!(output, "{:?} {} ({})", opcode, idx, constant_str(constant)).unwrap();
            offset + 3
        }

        OpCode::GetLocal
        | OpCode::SetLocal
        | OpCode::GetGlobal
        | OpCode::SetGlobal
        | OpCode::DefineGlobal
        | OpCode::GetProperty
        | OpCode::SetProperty
        | OpCode::Class
        | OpCode::Method
        | OpCode::StaticMethod => {
            let idx = chunk.read_u16(offset + 1);
            let name = match &chunk.constants.get(idx as usize) {
                Some(Constant::String(s)) => s.clone(),
                _ => format!("?{}", idx),
            };
            writeln!(output, "{:?} {} ({})", opcode, idx, name).unwrap();
            offset + 3
        }

        OpCode::Jump
        | OpCode::JumpIfFalse
        | OpCode::JumpIfTrue
        | OpCode::JumpIfFalseNoPop
        | OpCode::JumpIfTrueNoPop => {
            let jump = chunk.read_u16(offset + 1) as usize;
            let target = offset + 3 + jump;
            writeln!(output, "{:?} {} -> {}", opcode, jump, target).unwrap();
            offset + 3
        }

        OpCode::Loop => {
            let jump = chunk.read_u16(offset + 1) as usize;
            let target = offset + 3 - jump;
            writeln!(output, "{:?} {} -> {}", opcode, jump, target).unwrap();
            offset + 3
        }

        OpCode::BuildArray | OpCode::BuildHash | OpCode::IteratorNext | OpCode::LoadDefault => {
            let count = chunk.read_u16(offset + 1);
            writeln!(output, "{:?} {}", opcode, count).unwrap();
            offset + 3
        }

        // Closure (variable operands for upvalues)
        OpCode::Closure => {
            let func_idx = chunk.read_u16(offset + 1);
            let function = match &chunk.constants.get(func_idx as usize) {
                Some(Constant::Function(f)) => f,
                _ => {
                    writeln!(output, "{:?} {} (invalid)", opcode, func_idx).unwrap();
                    return offset + 3;
                }
            };

            writeln!(output, "{:?} {} ({})", opcode, func_idx, function.name).unwrap();

            let mut new_offset = offset + 3;
            for _ in 0..function.upvalue_count {
                let is_local = chunk.code[new_offset] != 0;
                let index = chunk.code[new_offset + 1];
                writeln!(
                    output,
                    "{:04}      |                   {} {}",
                    new_offset,
                    if is_local { "local" } else { "upvalue" },
                    index
                )
                .unwrap();
                new_offset += 2;
            }
            new_offset
        }

        // Three byte operand (2 bytes + 1 byte)
        OpCode::Invoke | OpCode::SuperInvoke => {
            let name_idx = chunk.read_u16(offset + 1);
            let arg_count = chunk.code[offset + 3];
            let name = match &chunk.constants.get(name_idx as usize) {
                Some(Constant::String(s)) => s.clone(),
                _ => format!("?{}", name_idx),
            };
            writeln!(
                output,
                "{:?} {} ({}) args={}",
                opcode, name_idx, name, arg_count
            )
            .unwrap();
            offset + 4
        }

        OpCode::New => {
            let class_idx = chunk.read_u16(offset + 1);
            let arg_count = chunk.code[offset + 3];
            let name = match &chunk.constants.get(class_idx as usize) {
                Some(Constant::String(s)) => s.clone(),
                _ => format!("?{}", class_idx),
            };
            writeln!(
                output,
                "{:?} {} ({}) args={}",
                opcode, class_idx, name, arg_count
            )
            .unwrap();
            offset + 4
        }

        OpCode::NativeCall => {
            let native_idx = chunk.read_u16(offset + 1);
            let arg_count = chunk.code[offset + 3];
            writeln!(
                output,
                "{:?} native={} args={}",
                opcode, native_idx, arg_count
            )
            .unwrap();
            offset + 4
        }
    }
}

/// Convert a constant to a display string.
fn constant_str(constant: &Constant) -> String {
    match constant {
        Constant::Int(n) => format!("{}", n),
        Constant::Float(n) => format!("{}", n),
        Constant::String(s) => {
            if s.len() > 20 {
                format!("\"{}...\"", &s[..20])
            } else {
                format!("\"{}\"", s)
            }
        }
        Constant::Function(f) => format!("<fn {}>", f.name),
        Constant::Class(c) => format!("<class {}>", c.name),
        Constant::Null => format!("null"),
    }
}

/// Print disassembly to stdout.
pub fn print_disassembly(function: &CompiledFunction) {
    print!("{}", disassemble_function(function));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::compiler::Compiler;

    fn disassemble_source(source: &str) -> String {
        let tokens = crate::lexer::Scanner::new(source).scan_tokens().unwrap();
        let program = crate::parser::Parser::new(tokens).parse().unwrap();
        let mut compiler = Compiler::new();
        let function = compiler.compile(&program).unwrap();
        disassemble_function(&function)
    }

    #[test]
    fn test_disassemble_simple() {
        let output = disassemble_source("let x = 42;");
        assert!(output.contains("Constant"));
        assert!(output.contains("DefineGlobal"));
    }

    #[test]
    fn test_disassemble_function() {
        let output = disassemble_source("fn add(a: Int, b: Int) -> Int { return a + b; }");
        assert!(output.contains("add"));
        assert!(output.contains("GetLocal"));
        assert!(output.contains("Add"));
        assert!(output.contains("Return"));
    }
}
