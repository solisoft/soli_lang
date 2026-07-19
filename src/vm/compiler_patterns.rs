//! Pattern matching compilation for match expressions.

use crate::ast::expr::{ExprKind, MatchArm, MatchPattern};
use crate::ast::Expr;
use crate::error::CompileError;

use super::chunk::Constant;
use super::compiler::{CompileResult, Compiler};
use super::opcode::Op;

/// Whether a pattern must run on the tree-walking interpreter rather than
/// compile to bytecode.
///
/// Two reasons land a pattern here. **Binding** patterns (`x`, `[a, b]`,
/// `{k: v}`, enum variants) alias the subject's stack slot, which the VM's
/// compilation does not yet model. **Composite** patterns (array, hash, and/or)
/// interleave their own `Dup`/`Pop` with each sub-pattern's, and the two only
/// balance when every sub-pattern leaves the duplicated subject in place — which
/// a literal sub-pattern does not (its `Equal` consumes it). That mismatch
/// silently popped one value too many, corrupting the value stack for whatever
/// followed the match; see [`Compiler::compile_pattern`] for the contract that
/// now makes dup-consumption explicit for the kinds the VM does compile.
///
/// Only wildcard and literal patterns compile — both have a proven stack effect.
fn pattern_needs_interpreter(pattern: &MatchPattern) -> bool {
    match pattern {
        MatchPattern::Wildcard | MatchPattern::Literal(_) => false,
        MatchPattern::Variable(_) | MatchPattern::Typed { .. } => true,
        MatchPattern::EnumVariant { .. } => true,
        // Composite patterns: unbalanced Dup/Pop against literal sub-patterns.
        MatchPattern::Array { .. }
        | MatchPattern::Hash { .. }
        | MatchPattern::Destructuring { .. }
        | MatchPattern::And(_)
        | MatchPattern::Or(_) => true,
    }
}

impl Compiler {
    /// Compile a match expression.
    pub fn compile_match(
        &mut self,
        expression: &Expr,
        arms: &[MatchArm],
        line: usize,
    ) -> CompileResult<()> {
        // Patterns the VM cannot compile with a proven stack effect run on the
        // tree-walking interpreter instead (see `pattern_needs_interpreter`).
        // Failing compilation here is what routes them there; the alternative
        // was miscompiled bytecode that corrupted the value stack.
        if let Some(arm) = arms.iter().find(|a| pattern_needs_interpreter(&a.pattern)) {
            return Err(CompileError::new(
                "this match pattern is not yet supported by the bytecode VM",
                arm.body.span,
            ));
        }

        // Evaluate the match subject
        self.compile_expr(expression)?;

        let mut end_jumps = Vec::new();

        for arm in arms {
            // Duplicate the match subject for testing
            self.emit(Op::Dup, line);

            // Compile the pattern test. `consumed_dup` says whether the test
            // itself used up the duplicate — a literal's `Equal` does, a
            // wildcard emits nothing at all — so exactly one `Pop` happens per
            // arm. Popping unconditionally (the old behavior) removed the
            // *subject* after a literal test, leaving the whole expression one
            // slot short and silently corrupting later stack-relative reads
            // such as a `catch` binding.
            let (fail_jump, consumed_dup) = self.compile_pattern(&arm.pattern, line)?;
            if !consumed_dup {
                self.emit(Op::Pop, line);
            }

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

    /// Compile a single pattern test.
    ///
    /// Returns the jump offsets to patch to the "fail" path, and whether the
    /// test consumed the duplicated subject that `compile_match` pushed for it.
    /// That flag is the stack contract between the two: the caller emits a `Pop`
    /// only when the pattern left the duplicate in place, so exactly one value
    /// is removed either way, on both the success and the fail path.
    ///
    /// Only the kinds `pattern_needs_interpreter` admits reach here; everything
    /// else has already failed compilation and runs on the tree-walker.
    fn compile_pattern(
        &mut self,
        pattern: &MatchPattern,
        line: usize,
    ) -> CompileResult<(Vec<usize>, bool)> {
        match pattern {
            // Always matches and emits nothing, so the duplicate is untouched.
            MatchPattern::Wildcard => Ok((vec![], false)),
            // `Equal` pops the duplicate and the literal; `JumpIfFalse` then pops
            // the boolean — so the duplicate is gone down both paths.
            MatchPattern::Literal(expr_kind) => {
                Ok((self.compile_literal_pattern(expr_kind, line)?, true))
            }
            _ => Err(CompileError::new(
                "this match pattern is not yet supported by the bytecode VM",
                crate::span::Span::new(0, 0, line, 0),
            )),
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
                self.emit_constant(Constant::String(s.clone().into()), line);
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
}
