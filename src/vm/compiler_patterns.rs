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
        // Wildcard and literal have a proven stack effect; `Variable` binds the
        // subject to a name, which `compile_match` now models by keeping the
        // subject in a real local slot (see there).
        MatchPattern::Wildcard
        | MatchPattern::Literal(_)
        | MatchPattern::Variable(_)
        | MatchPattern::Typed { .. } => false,
        // `[a, b]` / `[_, b]` — a fixed-length destructure whose parts only
        // bind or ignore. Provable shape: every test runs before any binding is
        // pushed, so a failing arm never unwinds a half-built set of bindings.
        // `...rest` needs a slice, and a nested or literal sub-pattern needs
        // recursion this does not have yet, so both still defer.
        MatchPattern::Array { elements, rest } => {
            rest.is_some()
                || !elements
                    .iter()
                    .all(|e| matches!(e, MatchPattern::Variable(_) | MatchPattern::Wildcard))
        }
        // `{name: n, age: a}` — same bounded shape as the array form: every key
        // test runs before any binding is pushed. `...rest` needs to build the
        // leftover hash, which this does not do yet.
        MatchPattern::Hash { fields, rest } => {
            rest.is_some()
                || !fields
                    .iter()
                    .all(|(_, p)| matches!(p, MatchPattern::Variable(_) | MatchPattern::Wildcard))
        }
        // `Status.Active` / `Status.Pending(r)` — class name and `__variant`
        // tag are both checked before any payload is bound, so the shape is the
        // same provable one as the array and hash forms. A nested sub-pattern
        // in the payload still defers.
        MatchPattern::EnumVariant { bindings, .. } => !bindings
            .iter()
            .all(|b| matches!(b, MatchPattern::Variable(_) | MatchPattern::Wildcard)),
        // Composite patterns: unbalanced Dup/Pop against literal sub-patterns.
        MatchPattern::Destructuring { .. } | MatchPattern::And(_) | MatchPattern::Or(_) => true,
    }
}

/// Does this pattern bind a name?
fn is_binding_pattern(pattern: &MatchPattern) -> bool {
    matches!(
        pattern,
        MatchPattern::Variable(_)
            | MatchPattern::Typed { .. }
            | MatchPattern::Array { .. }
            | MatchPattern::Hash { .. }
            | MatchPattern::EnumVariant { .. }
    )
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

        // Binding needs a slot, and a slot is only meaningful when the value
        // stack is at the locals baseline. Mid-expression — `out.push(match x
        // { … })` — there are temporaries below the top, so `add_local` would
        // name a position that is not where the subject is. Same gate the
        // comprehension compiler uses, for the same reason.
        if self.stack_height != self.locals.len() {
            return self.compile_match_stackwise(expression, arms, line);
        }

        // The subject lives in a real local slot for the whole match, rather
        // than as an anonymous stack value. That is what lets a `Variable`
        // pattern bind it: the binding is another local directly above, and
        // the guard and body resolve both like any other local.
        //
        // It also gives each arm a place to put its result. An arm ends holding
        // `[subject, (binding,) result]` and has to leave `[result]`, which
        // means removing values from *underneath* the top — impossible for a
        // stack machine directly. `SetLocal` is the lever: its stack effect is
        // 0, it writes the slot and leaves the value on top, so
        // `SetLocal(subject) + Pop…` collapses the frame down to the result.
        self.compile_expr(expression)?;
        self.add_local(String::new(), false);
        let subject_slot = (self.locals.len() - 1) as u16;

        let mut end_jumps = Vec::new();

        for arm in arms {
            // A fresh copy of the subject for this arm's test or binding.
            self.emit(Op::GetLocal(subject_slot), line);

            let mut fail_jumps = Vec::new();
            // How many locals this arm's pattern pushed. The collapse pops that
            // many; a failing guard drops them again.
            let mut bindings: usize = 0;
            // Fail jumps, split by what the stack looks like when they are
            // taken. `peeked_fails` are tests that left the subject copy in
            // place (they peek); `fail_jumps` are tests that already consumed
            // it. Mixing the two is how the original pattern compilation
            // corrupted the value stack.
            let mut peeked_fails: Vec<usize> = Vec::new();
            match &arm.pattern {
                // Always matches; the copy is dead weight.
                MatchPattern::Wildcard => {
                    self.emit(Op::Pop, line);
                }
                // `Equal` consumes the copy, `JumpIfFalse` the boolean.
                MatchPattern::Literal(expr_kind) => {
                    fail_jumps = self.compile_literal_pattern(expr_kind, line)?;
                }
                // Always matches, and the copy *is* the binding.
                MatchPattern::Variable(name) => {
                    self.add_local(name.clone(), false);
                    bindings += 1;
                }
                // `[a, b]`: check it is an array of the right length, then read
                // each element out of the subject's slot and bind it. Both
                // tests run before any binding is pushed.
                MatchPattern::Array { elements, .. } => {
                    let ty = self.add_string_constant("Array");
                    let type_fail = self.emit_jump(Op::MatchType(ty, 0), line);
                    self.emit(Op::Pop, line); // the copy; elements come from the slot

                    self.emit(Op::GetLocal(subject_slot), line);
                    let len_idx = self.add_string_constant("length");
                    self.emit(Op::GetProperty(len_idx), line);
                    self.emit_constant(Constant::Int(elements.len() as i64), line);
                    self.emit(Op::Equal, line);
                    let len_fail = self.emit_jump(Op::JumpIfFalse(0), line);

                    for (i, elem) in elements.iter().enumerate() {
                        if let MatchPattern::Variable(elem_name) = elem {
                            self.emit(Op::GetLocal(subject_slot), line);
                            self.emit_constant(Constant::Int(i as i64), line);
                            self.emit(Op::GetIndex, line);
                            self.add_local(elem_name.clone(), false);
                            bindings += 1;
                        }
                    }
                    peeked_fails.push(type_fail);
                    fail_jumps.push(len_fail);
                }
                // `Status.Pending(r)`: the class name identifies the enum, the
                // `__variant` field identifies the case, and the payload is
                // bound positionally. Both tests run before any binding.
                MatchPattern::EnumVariant {
                    enum_name,
                    variant_name,
                    bindings: payload,
                } => {
                    let ty = self.add_string_constant(enum_name);
                    peeked_fails.push(self.emit_jump(Op::MatchType(ty, 0), line));
                    self.emit(Op::Pop, line); // the copy; fields come from the slot

                    let tag = self.add_string_constant("__variant");
                    self.emit(Op::GetLocal(subject_slot), line);
                    self.emit(Op::GetProperty(tag), line);
                    let want = self.add_string_constant(variant_name);
                    self.emit(Op::Constant(want), line);
                    self.emit(Op::Equal, line);
                    fail_jumps.push(self.emit_jump(Op::JumpIfFalse(0), line));

                    let variant_idx = self.add_string_constant(variant_name);
                    for (i, b) in payload.iter().enumerate() {
                        if let MatchPattern::Variable(bind_name) = b {
                            self.emit(Op::EnumPayload(subject_slot, variant_idx, i as u8), line);
                            self.add_local(bind_name.clone(), false);
                            bindings += 1;
                        }
                    }
                }
                // `{name: n}`: check it is a hash that has every named key,
                // then read those keys out of the subject's slot and bind them.
                // Every key test runs before any binding is pushed.
                MatchPattern::Hash { fields, .. } => {
                    let ty = self.add_string_constant("Hash");
                    peeked_fails.push(self.emit_jump(Op::MatchType(ty, 0), line));
                    self.emit(Op::Pop, line); // the copy; fields come from the slot

                    for (field, _) in fields {
                        let key = self.add_string_constant(field);
                        self.emit(Op::HashHasKeyLocalConst(subject_slot, key), line);
                        fail_jumps.push(self.emit_jump(Op::JumpIfFalse(0), line));
                    }
                    for (field, sub) in fields {
                        if let MatchPattern::Variable(bind_name) = sub {
                            let key = self.add_string_constant(field);
                            self.emit(Op::HashGetLocalConst(subject_slot, key), line);
                            self.add_local(bind_name.clone(), false);
                            bindings += 1;
                        }
                    }
                }
                // Tests the copy's type without consuming it, then binds it.
                MatchPattern::Typed { name, type_name } => {
                    let idx = self.add_string_constant(type_name);
                    peeked_fails.push(self.emit_jump(Op::MatchType(idx, 0), line));
                    self.add_local(name.clone(), false);
                    bindings += 1;
                }
                other => {
                    return Err(CompileError::new(
                        format!("unsupported match pattern reached the compiler: {other:?}"),
                        arm.body.span,
                    ));
                }
            }

            // The guard sees the binding, which is the point of `n if n > 0`.
            let guard_jump = if let Some(ref guard) = arm.guard {
                self.compile_expr(guard)?;
                Some(self.emit_jump(Op::JumpIfFalse(0), line))
            } else {
                None
            };

            self.compile_expr(&arm.body)?;

            // Collapse [subject, (binding,) result] down to [result].
            self.emit(Op::SetLocal(subject_slot), line);
            self.emit(Op::Pop, line);
            for _ in 0..bindings {
                self.emit(Op::Pop, line);
            }
            end_jumps.push(self.emit_jump(Op::Jump(0), line));

            // Everything below is the fail path, where the binding is not in
            // scope — drop it from the compiler's view before emitting it, or
            // the next arm's slots are numbered one too high.
            for _ in 0..bindings {
                self.locals.pop();
            }
            // Each fail path cleans up only what it actually left behind.
            //
            //   guard failed          bindings are on the stack
            //   array type test       its peeked copy is on the stack
            //   array length test     the copy was already popped
            //   Typed type test       its peeked copy is on the stack
            //
            // so they cannot share one label, and mixing them was how the
            // original compilation corrupted the stack.
            let mut converge = Vec::new();
            // A guard that failed still has the arm's bindings on the stack.
            if let Some(guard_fail) = guard_jump {
                self.patch_jump(guard_fail);
                for _ in 0..bindings {
                    self.emit(Op::Pop, line);
                }
                converge.push(self.emit_jump(Op::Jump(0), line));
            }
            // Tests that peeked: drop the copy, then fall into the clean group.
            if !peeked_fails.is_empty() {
                for fj in std::mem::take(&mut peeked_fails) {
                    self.patch_jump(fj);
                }
                self.emit(Op::Pop, line);
            }
            // Tests that already consumed the copy.
            for fj in std::mem::take(&mut fail_jumps) {
                self.patch_jump(fj);
            }
            for c in converge {
                self.patch_jump(c);
            }
        }

        // No arm matched. The tree-walker raises "no pattern matched the
        // value"; this engine used to push null and carry on, so a match that
        // fell through every arm failed loudly under `soli test` and silently
        // produced null under `soli serve`. Raise here too.
        //
        // Emitted as a throw of that message rather than a new opcode: it
        // surfaces as an error either way, and code that relied on the null
        // cannot exist — it would already have been failing in the interpreter.
        let msg = self.add_string_constant("Type error: no pattern matched the value");
        self.emit(Op::Constant(msg), line);
        self.emit(Op::Throw, line);

        for ej in end_jumps {
            self.patch_jump(ej);
        }

        // The subject's slot now holds the match's value, which is exactly what
        // an expression leaves behind — so drop the compiler's record of the
        // slot without emitting a pop for it.
        self.locals.pop();

        Ok(())
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

    /// The original stack-only compilation: the subject stays an anonymous
    /// stack value and every arm `Dup`s it.
    ///
    /// Used when the match sits mid-expression — `out.push(match x { … })`
    /// — where there are temporaries below the top and `add_local` would
    /// hand out a slot that is not where the value actually is. It cannot
    /// support a binding for exactly that reason, so `Variable` arms fall
    /// back to the interpreter from here; at a clean stack position
    /// `compile_match` uses the slot-based path instead and compiles them.
    fn compile_match_stackwise(
        &mut self,
        expression: &Expr,
        arms: &[MatchArm],
        line: usize,
    ) -> CompileResult<()> {
        // Patterns the VM cannot compile with a proven stack effect run on the
        // tree-walking interpreter instead (see `pattern_needs_interpreter`).
        // Failing compilation here is what routes them there; the alternative
        // was miscompiled bytecode that corrupted the value stack.
        if let Some(arm) = arms
            .iter()
            .find(|a| pattern_needs_interpreter(&a.pattern) || is_binding_pattern(&a.pattern))
        {
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

        // Default: no arm matched — raise, as the tree-walker does, rather
        // than yielding null (see the slot-based path for why).
        self.emit(Op::Pop, line); // pop subject
        let msg = self.add_string_constant("Type error: no pattern matched the value");
        self.emit(Op::Constant(msg), line);
        self.emit(Op::Throw, line);

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
}
