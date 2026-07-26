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
/// Almost nothing does any more. The compiled form keeps the match subject in a
/// real local slot, so a pattern can bind by putting the value in the slot above
/// it, and `compile_pattern_in_slot` recurses for nested sub-patterns by parking
/// each extracted value in a slot of its own.
///
/// What is left needs machinery that does not exist yet rather than a different
/// arrangement of what does:
///
/// * `{a: x, ...rest}` has to *build* the leftover hash — every other rest form
///   is a slice of something that already exists.
/// * `Destructuring` (`Type { field }`) has no compiled form yet.
/// * `And`/`Or` are unreachable: no parser path constructs them. See
///   tasks/todo/match-and-or-patterns-are-unreachable.md.
///
/// The historical hazard this gate guarded against is worth keeping in view: an
/// early version interleaved each sub-pattern's stack traffic with the arm's own
/// and popped one value too many, corrupting the value stack for whatever
/// followed the match. `compile_pattern_in_slot` answers that by reporting, per
/// failure jump, exactly how many values are live when it is taken.
fn pattern_needs_interpreter(pattern: &MatchPattern) -> bool {
    match pattern {
        MatchPattern::Wildcard
        | MatchPattern::Literal(_)
        | MatchPattern::Variable(_)
        | MatchPattern::Typed { .. } => false,
        // Nested sub-patterns are compiled by recursion, so a composite is
        // supported exactly when its parts are.
        MatchPattern::Array { elements, .. } => elements.iter().any(pattern_needs_interpreter),
        MatchPattern::Hash { fields, rest } => {
            rest.is_some() || fields.iter().any(|(_, p)| pattern_needs_interpreter(p))
        }
        MatchPattern::EnumVariant { bindings, .. } => {
            bindings.iter().any(pattern_needs_interpreter)
        }
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
            // Compile the pattern against the subject's slot. `fails` pairs each
            // failure jump with how many values are live on the stack at that
            // point — nested patterns bind before their inner tests run, so a
            // single "how much to clean up" number no longer covers every exit.
            let (fails, bindings) =
                self.compile_pattern_in_slot(&arm.pattern, subject_slot, line)?;

            // The guard sees the bindings, which is the point of `n if n > 0`.
            let guard_fail = if let Some(ref guard) = arm.guard {
                self.compile_expr(guard)?;
                Some(self.emit_jump(Op::JumpIfFalse(0), line))
            } else {
                None
            };

            self.compile_expr(&arm.body)?;

            // Collapse [subject, …bindings, result] down to [result].
            self.emit(Op::SetLocal(subject_slot), line);
            self.emit(Op::Pop, line);
            for _ in 0..bindings {
                self.emit(Op::Pop, line);
            }
            end_jumps.push(self.emit_jump(Op::Jump(0), line));

            // Past here is failure. The bindings are not in scope on any of
            // those paths, so drop the compiler's record of them before
            // emitting the cleanup, or the next arm's slots are numbered high.
            for _ in 0..bindings {
                self.locals.pop();
            }

            let mut converge = Vec::new();
            if let Some(gf) = guard_fail {
                self.patch_jump(gf);
                for _ in 0..bindings {
                    self.emit(Op::Pop, line);
                }
                converge.push(self.emit_jump(Op::Jump(0), line));
            }
            for (jump, live) in fails {
                self.patch_jump(jump);
                for _ in 0..live {
                    self.emit(Op::Pop, line);
                }
                converge.push(self.emit_jump(Op::Jump(0), line));
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

    /// Compile `pattern` as a test against the value already sitting in `slot`.
    ///
    /// Returns each failure jump paired with **how many values are live on the
    /// stack when it is taken**, plus how many the pattern pushed on success.
    ///
    /// That per-jump count is what makes nesting possible. Every non-nested
    /// kind runs all its tests before pushing anything, so one number would do;
    /// a nested sub-pattern is tested only after its container has extracted
    /// and bound the value it lives in, so an inner failure unwinds with outer
    /// bindings already on the stack. Collapsing those into a single count is
    /// how this compilation corrupted the value stack the first time.
    ///
    /// Everything the pattern leaves is a local, whether a user binding or an
    /// internal temporary holding an extracted field — the arm pops them all
    /// the same way, so they do not need telling apart.
    fn compile_pattern_in_slot(
        &mut self,
        pattern: &MatchPattern,
        slot: u16,
        line: usize,
    ) -> CompileResult<(Vec<(usize, usize)>, usize)> {
        let mut fails: Vec<(usize, usize)> = Vec::new();
        let mut live: usize = 0;

        match pattern {
            // Always matches, pushes nothing.
            MatchPattern::Wildcard => {}

            // Always matches; the value becomes the binding.
            MatchPattern::Variable(name) => {
                self.emit(Op::GetLocal(slot), line);
                self.add_local(name.clone(), false);
                live += 1;
            }

            // `Equal` consumes the copy, `JumpIfFalse` the boolean.
            MatchPattern::Literal(expr_kind) => {
                self.emit(Op::GetLocal(slot), line);
                for j in self.compile_literal_pattern(expr_kind, line)? {
                    fails.push((j, live));
                }
            }

            // `MatchType` peeks, so on failure its copy is still there; on
            // success that same copy is the binding.
            MatchPattern::Typed { name, type_name } => {
                self.emit(Op::GetLocal(slot), line);
                let idx = self.add_string_constant(type_name);
                let f = self.emit_jump(Op::MatchType(idx, 0), line);
                fails.push((f, live + 1));
                self.add_local(name.clone(), false);
                live += 1;
            }

            MatchPattern::Array { elements, rest } => {
                self.emit(Op::GetLocal(slot), line);
                let ty = self.add_string_constant("Array");
                let f = self.emit_jump(Op::MatchType(ty, 0), line);
                fails.push((f, live + 1));
                self.emit(Op::Pop, line);

                // Without `...rest` the length must match exactly; with it the
                // named elements are a prefix. Same rule as the tree-walker.
                self.emit(Op::GetLocal(slot), line);
                let len_idx = self.add_string_constant("length");
                self.emit(Op::GetProperty(len_idx), line);
                self.emit_constant(Constant::Int(elements.len() as i64), line);
                self.emit(
                    if rest.is_some() {
                        Op::GreaterEqual
                    } else {
                        Op::Equal
                    },
                    line,
                );
                let f = self.emit_jump(Op::JumpIfFalse(0), line);
                fails.push((f, live));

                for (i, elem) in elements.iter().enumerate() {
                    if matches!(elem, MatchPattern::Wildcard) {
                        continue;
                    }
                    self.emit(Op::GetLocal(slot), line);
                    self.emit_constant(Constant::Int(i as i64), line);
                    self.emit(Op::GetIndex, line);
                    live += self.bind_or_recurse(elem, line, live, &mut fails)?;
                }

                if let Some(rest_name) = rest {
                    self.emit(Op::GetLocal(slot), line);
                    self.emit_constant(Constant::Int(elements.len() as i64), line);
                    let name_idx = self.add_string_constant("slice");
                    let mid = super::method_table::resolve_method_id("slice");
                    if mid != super::method_table::METHOD_UNKNOWN {
                        self.emit(Op::CallMethodById(name_idx, 1, mid), line);
                    } else {
                        self.emit(Op::CallMethod(name_idx, 1), line);
                    }
                    self.add_local(rest_name.clone(), false);
                    live += 1;
                }
            }

            MatchPattern::Hash { fields, rest } => {
                // `...rest` would have to build the leftover hash; not yet.
                if rest.is_some() {
                    return Err(CompileError::new(
                        "this match pattern is not yet supported by the bytecode VM",
                        crate::span::Span::new(0, 0, line, 0),
                    ));
                }
                self.emit(Op::GetLocal(slot), line);
                let ty = self.add_string_constant("Hash");
                let f = self.emit_jump(Op::MatchType(ty, 0), line);
                fails.push((f, live + 1));
                self.emit(Op::Pop, line);

                // Every key must be present before anything is read out.
                for (field, _) in fields {
                    let key = self.add_string_constant(field);
                    self.emit(Op::HashHasKeyLocalConst(slot, key), line);
                    let f = self.emit_jump(Op::JumpIfFalse(0), line);
                    fails.push((f, live));
                }
                for (field, sub) in fields {
                    if matches!(sub, MatchPattern::Wildcard) {
                        continue;
                    }
                    let key = self.add_string_constant(field);
                    self.emit(Op::HashGetLocalConst(slot, key), line);
                    live += self.bind_or_recurse(sub, line, live, &mut fails)?;
                }
            }

            MatchPattern::EnumVariant {
                enum_name,
                variant_name,
                bindings: payload,
            } => {
                self.emit(Op::GetLocal(slot), line);
                let ty = self.add_string_constant(enum_name);
                let f = self.emit_jump(Op::MatchType(ty, 0), line);
                fails.push((f, live + 1));
                self.emit(Op::Pop, line);

                let tag = self.add_string_constant("__variant");
                self.emit(Op::GetLocal(slot), line);
                self.emit(Op::GetProperty(tag), line);
                let want = self.add_string_constant(variant_name);
                self.emit(Op::Constant(want), line);
                self.emit(Op::Equal, line);
                let f = self.emit_jump(Op::JumpIfFalse(0), line);
                fails.push((f, live));

                let variant_idx = self.add_string_constant(variant_name);
                for (i, b) in payload.iter().enumerate() {
                    if matches!(b, MatchPattern::Wildcard) {
                        continue;
                    }
                    self.emit(Op::EnumPayload(slot, variant_idx, i as u8), line);
                    live += self.bind_or_recurse(b, line, live, &mut fails)?;
                }
            }

            other => {
                return Err(CompileError::new(
                    format!(
                        "this match pattern is not yet supported by the bytecode VM: {other:?}"
                    ),
                    crate::span::Span::new(0, 0, line, 0),
                ));
            }
        }

        Ok((fails, live))
    }

    /// The extracted value is on top of the stack. Either bind it directly, or
    /// park it in a slot and recurse into the sub-pattern.
    ///
    /// Returns how many locals this added. Sub-pattern failures are recorded
    /// with the outer `live` count added, because those values are still on the
    /// stack when the inner test jumps.
    fn bind_or_recurse(
        &mut self,
        sub: &MatchPattern,
        line: usize,
        live: usize,
        fails: &mut Vec<(usize, usize)>,
    ) -> CompileResult<usize> {
        if let MatchPattern::Variable(name) = sub {
            self.add_local(name.clone(), false);
            return Ok(1);
        }
        // Anything else needs a slot of its own to be tested against.
        self.add_local(String::new(), false);
        let sub_slot = (self.locals.len() - 1) as u16;
        let (sub_fails, sub_live) = self.compile_pattern_in_slot(sub, sub_slot, line)?;
        for (j, inner) in sub_fails {
            fails.push((j, live + 1 + inner));
        }
        Ok(1 + sub_live)
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
