//! Pattern matching compilation for match expressions.

use crate::ast::expr::{ExprKind, MatchArm, MatchPattern};
use crate::ast::Expr;

use super::chunk::Constant;
use super::compiler::{CompileResult, Compiler};
use super::opcode::Op;

impl Compiler {
    /// Compile a match expression.
    pub fn compile_match(
        &mut self,
        expression: &Expr,
        arms: &[MatchArm],
        line: usize,
    ) -> CompileResult<()> {
        // Evaluate the match subject
        self.compile_expr(expression)?;

        let mut end_jumps = Vec::new();

        for arm in arms {
            // Duplicate the match subject for testing
            self.emit(Op::Dup, line);

            // Compile the pattern test
            let fail_jump = self.compile_pattern(&arm.pattern, line)?;

            // Pop the duplicated subject (pattern matched)
            self.emit(Op::Pop, line);

            // Check guard if present
            let guard_jump = if let Some(ref guard) = arm.guard {
                self.compile_expr(guard)?;
                Some(self.emit_jump(Op::JumpIfFalse(0), line))
            } else {
                None
            };

            // Pop the original subject before evaluating the body
            self.emit(Op::Pop, line);

            // Compile the arm body
            self.compile_expr(&arm.body)?;

            // Jump to end of match
            end_jumps.push(self.emit_jump(Op::Jump(0), line));

            // Patch guard failure — need to push subject back if guard failed
            if let Some(guard_fail) = guard_jump {
                self.patch_jump(guard_fail);
            }

            // Patch pattern failure
            for fj in fail_jump {
                self.patch_jump(fj);
            }
        }

        // Default: if no arm matched, pop the subject and push null
        self.emit(Op::Pop, line); // pop subject
        self.emit(Op::Null, line);

        // Patch all end jumps
        for ej in end_jumps {
            self.patch_jump(ej);
        }

        Ok(())
    }

    /// Compile a single pattern. Returns jump offsets that need to be patched to the "fail" path.
    fn compile_pattern(
        &mut self,
        pattern: &MatchPattern,
        line: usize,
    ) -> CompileResult<Vec<usize>> {
        match pattern {
            MatchPattern::Wildcard => {
                // Always matches — pop the dup'd value
                Ok(vec![])
            }
            MatchPattern::Variable(name) => {
                // Bind the value to the variable — always matches
                self.begin_scope();
                self.add_local(name.clone(), false);
                // The dup'd value becomes the local
                Ok(vec![])
            }
            MatchPattern::Literal(expr_kind) => {
                // Compare with the literal
                self.compile_literal_pattern(expr_kind, line)
            }
            MatchPattern::Typed { name, type_name } => {
                // Check type, then bind
                let _type_idx = self.add_string_constant(type_name);
                self.emit(Op::Dup, line);
                self.emit_constant(Constant::String(type_name.clone()), line);
                // Runtime type check — handled by the VM
                let fail = self.emit_jump(Op::JumpIfFalse(0), line);
                self.begin_scope();
                self.add_local(name.clone(), false);
                Ok(vec![fail])
            }
            MatchPattern::Array { elements, rest } => {
                self.compile_array_pattern(elements, rest.as_deref(), line)
            }
            MatchPattern::Hash { fields, rest } => {
                self.compile_hash_pattern(fields, rest.as_deref(), line)
            }
            MatchPattern::Destructuring { type_name, fields } => {
                // Check type, then destructure
                let mut fails = Vec::new();
                let _type_idx = self.add_string_constant(type_name);
                // Type check would go here
                // Then destructure fields
                for (field_name, sub_pattern) in fields {
                    self.emit(Op::Dup, line);
                    let field_idx = self.add_string_constant(field_name);
                    self.emit(Op::GetProperty(field_idx), line);
                    let mut sub_fails = self.compile_pattern(sub_pattern, line)?;
                    fails.append(&mut sub_fails);
                }
                Ok(fails)
            }
            MatchPattern::And(patterns) => {
                let mut fails = Vec::new();
                for pat in patterns {
                    self.emit(Op::Dup, line);
                    let mut sub_fails = self.compile_pattern(pat, line)?;
                    fails.append(&mut sub_fails);
                    self.emit(Op::Pop, line);
                }
                Ok(fails)
            }
            MatchPattern::Or(patterns) => {
                // Try each pattern; if one succeeds, jump to success
                let mut success_jumps = Vec::new();
                let mut last_fail = Vec::new();

                for (i, pat) in patterns.iter().enumerate() {
                    self.emit(Op::Dup, line);
                    let fails = self.compile_pattern(pat, line)?;

                    if i < patterns.len() - 1 {
                        // If matched, jump to success
                        success_jumps.push(self.emit_jump(Op::Jump(0), line));
                        // Patch fails to try next pattern
                        for f in fails {
                            self.patch_jump(f);
                        }
                        self.emit(Op::Pop, line);
                    } else {
                        last_fail = fails;
                    }
                }

                // Patch success jumps
                for sj in success_jumps {
                    self.patch_jump(sj);
                }

                Ok(last_fail)
            }
        }
    }

    fn compile_literal_pattern(
        &mut self,
        expr_kind: &ExprKind,
        line: usize,
    ) -> CompileResult<Vec<usize>> {
        match expr_kind {
            ExprKind::IntLiteral(n) => {
                self.emit_constant(Constant::Int(*n), line);
            }
            ExprKind::FloatLiteral(n) => {
                self.emit_constant(Constant::Float(*n), line);
            }
            ExprKind::DecimalLiteral(s) => {
                self.emit_constant(Constant::Decimal(s.clone()), line);
            }
            ExprKind::StringLiteral(s) => {
                self.emit_constant(Constant::String(s.clone()), line);
            }
            ExprKind::BoolLiteral(b) => {
                self.emit(if *b { Op::True } else { Op::False }, line);
            }
            ExprKind::Null => {
                self.emit(Op::Null, line);
            }
            _ => {
                // Other expression kinds aren't valid literal patterns
                self.emit(Op::Null, line);
            }
        }
        self.emit(Op::Equal, line);
        let fail = self.emit_jump(Op::JumpIfFalse(0), line);
        Ok(vec![fail])
    }

    fn compile_array_pattern(
        &mut self,
        elements: &[MatchPattern],
        rest: Option<&str>,
        line: usize,
    ) -> CompileResult<Vec<usize>> {
        let mut fails = Vec::new();

        // Check length (at least N elements)
        // We'd need a length check opcode or runtime call
        // For now, we'll compile element-by-element checks

        for (i, elem) in elements.iter().enumerate() {
            self.emit(Op::Dup, line);
            self.emit_constant(Constant::Int(i as i64), line);
            self.emit(Op::GetIndex, line);
            let mut sub_fails = self.compile_pattern(elem, line)?;
            fails.append(&mut sub_fails);
            self.emit(Op::Pop, line);
        }

        if let Some(rest_name) = rest {
            // Bind the rest of the array to a variable
            // This would need a slice operation
            self.begin_scope();
            self.add_local(rest_name.to_string(), false);
        }

        Ok(fails)
    }

    fn compile_hash_pattern(
        &mut self,
        fields: &[(String, MatchPattern)],
        rest: Option<&str>,
        line: usize,
    ) -> CompileResult<Vec<usize>> {
        let mut fails = Vec::new();

        for (field_name, sub_pattern) in fields {
            self.emit(Op::Dup, line);
            let _key_idx = self.add_string_constant(field_name);
            self.emit_constant(Constant::String(field_name.clone()), line);
            self.emit(Op::GetIndex, line);
            let mut sub_fails = self.compile_pattern(sub_pattern, line)?;
            fails.append(&mut sub_fails);
            self.emit(Op::Pop, line);
        }

        if let Some(rest_name) = rest {
            self.begin_scope();
            self.add_local(rest_name.to_string(), false);
        }

        Ok(fails)
    }
}
