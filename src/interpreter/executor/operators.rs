//! Binary and unary operator evaluation.
//!
//! The rules of `+ - * / %` and of the four orderings are defined once, by
//! the associated functions [`Interpreter::eval_add`] … [`Interpreter::eval_modulo`]
//! and [`Interpreter::compare_ordered`], which read no interpreter state. Both
//! engines call them: the tree-walker from `evaluate_binary_values`, the VM
//! from the slow arm of each operator (`Vm::op_add` … `op_compare_less_equal`)
//! once its inline Int and Float cases do not apply. So a Decimal, an
//! `Array - Array`, an overflow or a division by zero cannot come out
//! differently on the two engines.

use std::cell::RefCell;
use std::rc::Rc;

use rust_decimal::prelude::ToPrimitive;

use crate::ast::*;
use crate::error::RuntimeError;
use crate::interpreter::value::Value;
use crate::span::Span;

use super::{Interpreter, RuntimeResult};

/// An ordering operator, as [`Interpreter::compare_ordered`] applies it.
#[derive(Debug, Clone, Copy)]
pub(crate) enum OrdOp {
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

/// Integer arithmetic that reports overflow instead of wrapping or panicking.
///
/// The release profile does not enable `overflow-checks`, so `+ - *` on `Int`
/// silently **wrapped** in production while panicking in a dev build — the two
/// disagreed, and a quantity or price computed from request data could come out
/// negative with nothing logged. Division was worse: `i64::MIN / -1` panics
/// regardless of profile, because only the zero divisor was checked, and
/// reaching `i64::MIN` takes nothing more than `params["a"] + params["b"]`.
///
/// Every one of these is now an ordinary catchable Soli error.
/// Decimal arithmetic that reports overflow instead of panicking.
///
/// `rust_decimal`'s operators panic on overflow ("Multiplication overflowed").
/// `Money` already used the checked forms; bare `Decimal` did not, so
/// `10000000000.0D * params["qty"]` took the handler down with a panic instead
/// of raising. Same treatment as the Int operators above.
fn decimal_op(
    a: rust_decimal::Decimal,
    op: &str,
    b: rust_decimal::Decimal,
    span: Span,
) -> RuntimeResult<rust_decimal::Decimal> {
    let result = match op {
        "+" => a.checked_add(b),
        "-" => a.checked_sub(b),
        "*" => a.checked_mul(b),
        "/" => a.checked_div(b),
        "%" => a.checked_rem(b),
        _ => None,
    };
    result.ok_or_else(|| RuntimeError::General {
        message: format!("decimal overflow: {a} {op} {b} is out of range for a Decimal"),
        span,
    })
}

pub(crate) fn int_overflow(a: i64, op: &str, b: i64, span: Span) -> RuntimeError {
    RuntimeError::General {
        message: format!(
            "integer overflow: {a} {op} {b} does not fit in an Int (range {}..{})",
            i64::MIN,
            i64::MAX
        ),
        span,
    }
}

/// `-i64::MIN` has no Int either. Unchecked, it came back as `i64::MIN` in a
/// release build and aborted a debug one.
pub(crate) fn int_negate_overflow(n: i64, span: Span) -> RuntimeError {
    RuntimeError::General {
        message: format!(
            "integer overflow: -({n}) does not fit in an Int (range {}..{})",
            i64::MIN,
            i64::MAX
        ),
        span,
    }
}

impl Interpreter {
    pub(crate) fn evaluate_binary(
        &mut self,
        left: &Expr,
        op: BinaryOp,
        right: &Expr,
        span: Span,
    ) -> RuntimeResult<Value> {
        // Shovel must inspect the unevaluated LHS shape to dispatch to HABTM
        // mutators when the LHS is `<model_instance>.<habtm_relation>`.
        if op == BinaryOp::Shovel {
            return self.eval_shovel(left, right, span);
        }

        let left_val = self.evaluate(left)?;
        let right_val = self.evaluate(right)?;

        // Auto-resolve Futures before binary operations
        let left_val = left_val.resolve().map_err(|e| RuntimeError::new(e, span))?;
        let right_val = right_val
            .resolve()
            .map_err(|e| RuntimeError::new(e, span))?;

        self.evaluate_binary_values(&left_val, op, &right_val, span)
    }

    fn eval_shovel(&mut self, left: &Expr, right: &Expr, span: Span) -> RuntimeResult<Value> {
        use crate::interpreter::builtins::model::habtm::{
            habtm_add, match_habtm_method, to_singular_method_name,
        };
        // `name << "x"` appends into the variable. Strings are values, so this
        // has to write the binding back; pushing into a clone would not.
        if let ExprKind::Variable(name) = &left.kind {
            let before = self.evaluate(left)?;
            let before = before.resolve().map_err(|e| RuntimeError::new(e, span))?;
            let right_val = self.evaluate(right)?;
            let right_val = right_val
                .resolve()
                .map_err(|e| RuntimeError::new(e, span))?;
            // Only two strings can append; a number skips the call.
            let left_val = if matches!((&before, &right_val), (Value::String(_), Value::String(_)))
            {
                match self.append_to_unchanged_binding(name, before, &right_val, span)? {
                    Ok(updated) => return Ok(updated),
                    Err(left_val) => left_val,
                }
            } else {
                before
            };
            if matches!(left_val, Value::String(_)) {
                if matches!(right_val, Value::String(_)) {
                    // The right-hand side rebound `name`: append to the
                    // string read first, like `name = name + rhs`.
                    let joined = self.evaluate_binary_values(
                        &left_val,
                        crate::ast::BinaryOp::Add,
                        &right_val,
                        span,
                    )?;
                    self.assign_to_target(left, joined.clone(), span)?;
                    return Ok(joined);
                }
                return Err(RuntimeError::type_error(
                    format!("string << expects a string, got {}", right_val.type_name()),
                    span,
                ));
            }
            return self.shovel_array_push(&left_val, right_val, span);
        }

        // Special case: `<instance>.<relation> << <value>` for HABTM and
        // has_many through: associations.
        if let ExprKind::Member { object, name } = &left.kind {
            let obj_val = self.evaluate(object)?;
            let obj_val = obj_val.resolve().map_err(|e| RuntimeError::new(e, span))?;
            if let Value::Instance(inst) = &obj_val {
                let class_name = inst.borrow().class.name.clone();
                let method_name = to_singular_method_name("add", name);
                if let Some(matched) = match_habtm_method(&class_name, &method_name) {
                    let right_val = self.evaluate(right)?;
                    let right_val = right_val
                        .resolve()
                        .map_err(|e| RuntimeError::new(e, span))?;
                    habtm_add(inst, &matched.relation, std::slice::from_ref(&right_val))
                        .map_err(|e| RuntimeError::new(e, span))?;
                    // Re-fetch the relation so the returned value reflects the new state.
                    return self.evaluate(left);
                }
                // `user.teams << team` on a through relation inserts the
                // join record (raw row like HABTM; counter caches bumped).
                // `user.posts << post` / `post.comments << comment` on a
                // has_many (incl. `as:` inverses) stamps the FK — plus the
                // polymorphic type pair — onto the record and saves it.
                if inst.borrow().class.is_model_subclass() {
                    if let Some(relation) =
                        crate::interpreter::builtins::model::get_relation(&class_name, name)
                    {
                        if relation.through.is_some() {
                            let right_val = self.evaluate(right)?;
                            let right_val = right_val
                                .resolve()
                                .map_err(|e| RuntimeError::new(e, span))?;
                            crate::interpreter::builtins::model::habtm::through_add(
                                inst,
                                &class_name,
                                &relation,
                                std::slice::from_ref(&right_val),
                            )
                            .map_err(|e| RuntimeError::new(e, span))?;
                            return self.evaluate(left);
                        }
                        if relation.relation_type
                            == crate::interpreter::builtins::model::RelationType::HasMany
                        {
                            let right_val = self.evaluate(right)?;
                            let right_val = right_val
                                .resolve()
                                .map_err(|e| RuntimeError::new(e, span))?;
                            self.push_to_has_many(inst, &relation, &right_val, span)?;
                            return self.evaluate(left);
                        }
                    }
                }
            }
            // Fall through: evaluate LHS via member access, then push.
            let left_val = self.evaluate(left)?;
            let left_val = left_val.resolve().map_err(|e| RuntimeError::new(e, span))?;
            let right_val = self.evaluate(right)?;
            let right_val = right_val
                .resolve()
                .map_err(|e| RuntimeError::new(e, span))?;
            return self.shovel_array_push(&left_val, right_val, span);
        }

        let left_val = self.evaluate(left)?;
        let left_val = left_val.resolve().map_err(|e| RuntimeError::new(e, span))?;
        let right_val = self.evaluate(right)?;
        let right_val = right_val
            .resolve()
            .map_err(|e| RuntimeError::new(e, span))?;
        self.shovel_array_push(&left_val, right_val, span)
    }

    /// `owner.posts << post` (has_many, incl. `as:` inverses): stamp the FK
    /// — plus the polymorphic type pair — onto each pushed record and save
    /// it through the regular persist path (validations, callbacks, counter
    /// caches, dirty tracking). Accepts a model instance or an array of
    /// them; a save that fails (validation veto) raises so the push can't
    /// silently no-op.
    fn push_to_has_many(
        &mut self,
        owner: &std::rc::Rc<std::cell::RefCell<crate::interpreter::value::Instance>>,
        relation: &crate::interpreter::builtins::model::RelationDef,
        right: &Value,
        span: Span,
    ) -> RuntimeResult<()> {
        let owner_key = match owner.borrow().get("_key") {
            Some(Value::String(s)) => s.to_string(),
            Some(Value::Int(n)) => n.to_string(),
            _ => {
                return Err(RuntimeError::new(
                    format!(
                        "cannot push to \"{}\": save the owner record first",
                        relation.name
                    ),
                    span,
                ))
            }
        };

        let children: Vec<Value> = match right {
            Value::Array(arr) => arr.borrow().clone(),
            other => vec![other.clone()],
        };

        for child in children {
            let Value::Instance(child_inst) = &child else {
                return Err(RuntimeError::new(
                    format!(
                        "\"{}\" << expects a model instance (or an array of them), got {} — \
create the record first, or use {}.create({{...}})",
                        relation.name,
                        child.type_name(),
                        relation.name
                    ),
                    span,
                ));
            };
            {
                let mut child_mut = child_inst.borrow_mut();
                child_mut.set(
                    relation.foreign_key.clone(),
                    Value::String(owner_key.as_str().into()),
                );
                if let (Some(field), Some(value)) = (
                    &relation.polymorphic_type_field,
                    &relation.polymorphic_type_value,
                ) {
                    child_mut.set(field.clone(), Value::String(value.as_str().into()));
                }
            }
            let result = self.save_model_instance(&child, span)?;
            if matches!(result, Value::Bool(false)) {
                return Err(RuntimeError::new(
                    format!(
                        "\"{}\" << failed: the record did not save (check its _errors)",
                        relation.name
                    ),
                    span,
                ));
            }
        }
        Ok(())
    }

    fn shovel_array_push(&self, left: &Value, right: Value, span: Span) -> RuntimeResult<Value> {
        match left {
            Value::Array(arr) => {
                arr.borrow_mut().push(right);
                Ok(left.clone())
            }
            other => Err(RuntimeError::type_error(
                format!("<< expects an array on the left, got {}", other.type_name()),
                span,
            )),
        }
    }

    /// Evaluate a binary operation on already-evaluated values.
    pub(crate) fn evaluate_binary_values(
        &self,
        left_val: &Value,
        op: BinaryOp,
        right_val: &Value,
        span: Span,
    ) -> RuntimeResult<Value> {
        let ordered = |op: OrdOp| Self::compare_ordered(left_val, op, right_val, span);
        match op {
            BinaryOp::Add => Self::eval_add(left_val, right_val, span),
            BinaryOp::Subtract => Self::eval_subtract(left_val, right_val, span),
            BinaryOp::Multiply => Self::eval_multiply(left_val, right_val, span),
            BinaryOp::Divide => Self::eval_divide(left_val, right_val, span),
            BinaryOp::Modulo => Self::eval_modulo(left_val, right_val, span),
            BinaryOp::Equal => Ok(Value::Bool(crate::interpreter::value::enum_aware_equal(
                left_val, right_val,
            ))),
            BinaryOp::NotEqual => Ok(Value::Bool(!crate::interpreter::value::enum_aware_equal(
                left_val, right_val,
            ))),
            BinaryOp::Less => ordered(OrdOp::Less).map(Value::Bool),
            BinaryOp::LessEqual => ordered(OrdOp::LessEqual).map(Value::Bool),
            BinaryOp::Greater => ordered(OrdOp::Greater).map(Value::Bool),
            BinaryOp::GreaterEqual => ordered(OrdOp::GreaterEqual).map(Value::Bool),
            BinaryOp::Range => self.eval_range(left_val, right_val, span),
            BinaryOp::Shovel => match left_val {
                Value::Array(arr) => {
                    arr.borrow_mut().push(right_val.clone());
                    Ok(left_val.clone())
                }
                _ => Err(RuntimeError::type_error(
                    format!(
                        "<< expects an array on the left, got {}",
                        left_val.type_name()
                    ),
                    span,
                )),
            },
        }
    }

    fn eval_range(&self, left: &Value, right: &Value, span: Span) -> RuntimeResult<Value> {
        match (left, right) {
            (Value::Int(start), Value::Int(end)) => {
                crate::interpreter::limits::check_range(*start, *end, "range (..)")
                    .map_err(|message| RuntimeError::General { message, span })?;
                let arr: Vec<Value> = (*start..*end).map(Value::Int).collect();
                Ok(Value::Array(Rc::new(RefCell::new(arr))))
            }
            _ => Err(RuntimeError::type_error(
                format!(
                    "range (..) expects two integers, got {} and {}",
                    left.type_name(),
                    right.type_name()
                ),
                span,
            )),
        }
    }

    /// `left + right`, on both engines (see the module docs).
    pub(crate) fn eval_add(left: &Value, right: &Value, span: Span) -> RuntimeResult<Value> {
        match (left, right) {
            (Value::Array(a), Value::Array(b)) => {
                let a = a.borrow();
                let b = b.borrow();
                let mut result = Vec::with_capacity(a.len() + b.len());
                result.extend_from_slice(&a);
                result.extend_from_slice(&b);
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            (Value::Int(a), Value::Int(b)) => match a.checked_add(*b) {
                Some(sum) => Ok(Value::Int(sum)),
                None => Err(int_overflow(*a, "+", *b, span)),
            },
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
            (Value::Decimal(a), Value::Decimal(b)) => {
                use crate::interpreter::value::DecimalValue;
                Ok(Value::Decimal(DecimalValue(
                    decimal_op(a.0, "+", b.0, span)?,
                    std::cmp::max(a.1, b.1),
                )))
            }
            (Value::Decimal(a), Value::Int(b)) => {
                use crate::interpreter::value::DecimalValue;
                let b_dec = rust_decimal::Decimal::from(*b);
                Ok(Value::Decimal(DecimalValue(
                    decimal_op(a.0, "+", b_dec, span)?,
                    a.1,
                )))
            }
            (Value::Int(b), Value::Decimal(a)) => {
                use crate::interpreter::value::DecimalValue;
                let b_dec = rust_decimal::Decimal::from(*b);
                Ok(Value::Decimal(DecimalValue(
                    decimal_op(b_dec, "+", a.0, span)?,
                    a.1,
                )))
            }
            (Value::Decimal(a), Value::Float(b)) => {
                Ok(Value::Float(a.value().to_f64().unwrap_or(0.0) + b))
            }
            (Value::Float(a), Value::Decimal(b)) => {
                Ok(Value::Float(a + b.value().to_f64().unwrap_or(0.0)))
            }
            // eco_format! writes straight into a SoliStr (inline for short
            // results) — no intermediate String allocation + copy.
            (Value::String(a), Value::String(b)) => {
                Ok(Value::String(ecow::eco_format!("{}{}", a, b)))
            }
            // A string and anything else: the other side as `Display` writes
            // it, built like an interpolation (itoa/ryu into a sized buffer).
            (Value::String(_), _) | (_, Value::String(_)) => Ok(Value::String(
                crate::interpreter::value::concat_display(left, right),
            )),
            _ => Err(RuntimeError::type_error(
                format!("Cannot add {} and {}", left.type_name(), right.type_name()),
                span,
            )),
        }
    }

    /// `left - right`, on both engines. An array minus an array keeps the
    /// elements of `left` equal to none of `right`'s, by `PartialEq`: an
    /// instance (an enum payload variant too) only matches itself.
    pub(crate) fn eval_subtract(left: &Value, right: &Value, span: Span) -> RuntimeResult<Value> {
        match (left, right) {
            (Value::Array(arr), Value::Array(to_remove)) => {
                let arr = arr.borrow();
                let to_remove = to_remove.borrow();
                let result: Vec<Value> = arr
                    .iter()
                    .filter(|item| !to_remove.contains(item))
                    .cloned()
                    .collect();
                Ok(Value::Array(Rc::new(RefCell::new(result))))
            }
            (Value::Int(a), Value::Int(b)) => match a.checked_sub(*b) {
                Some(diff) => Ok(Value::Int(diff)),
                None => Err(int_overflow(*a, "-", *b, span)),
            },
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a - *b as f64)),
            (Value::Decimal(a), Value::Decimal(b)) => {
                use crate::interpreter::value::DecimalValue;
                Ok(Value::Decimal(DecimalValue(
                    decimal_op(a.0, "-", b.0, span)?,
                    std::cmp::max(a.1, b.1),
                )))
            }
            (Value::Decimal(a), Value::Int(b)) => {
                use crate::interpreter::value::DecimalValue;
                let b_dec = rust_decimal::Decimal::from(*b);
                Ok(Value::Decimal(DecimalValue(
                    decimal_op(a.0, "-", b_dec, span)?,
                    a.1,
                )))
            }
            (Value::Int(b), Value::Decimal(a)) => {
                use crate::interpreter::value::DecimalValue;
                let b_dec = rust_decimal::Decimal::from(*b);
                Ok(Value::Decimal(DecimalValue(
                    decimal_op(b_dec, "-", a.0, span)?,
                    a.1,
                )))
            }
            (Value::Decimal(a), Value::Float(b)) => {
                Ok(Value::Float(a.value().to_f64().unwrap_or(0.0) - b))
            }
            (Value::Float(a), Value::Decimal(b)) => {
                Ok(Value::Float(a - b.value().to_f64().unwrap_or(0.0)))
            }
            _ => Err(RuntimeError::type_error(
                format!(
                    "Cannot subtract {} from {}",
                    right.type_name(),
                    left.type_name()
                ),
                span,
            )),
        }
    }

    /// `left * right`, on both engines.
    pub(crate) fn eval_multiply(left: &Value, right: &Value, span: Span) -> RuntimeResult<Value> {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => match a.checked_mul(*b) {
                Some(product) => Ok(Value::Int(product)),
                None => Err(int_overflow(*a, "*", *b, span)),
            },
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a * *b as f64)),
            (Value::Decimal(a), Value::Decimal(b)) => {
                use crate::interpreter::value::DecimalValue;
                Ok(Value::Decimal(DecimalValue(
                    decimal_op(a.0, "*", b.0, span)?,
                    a.1 + b.1,
                )))
            }
            (Value::Decimal(a), Value::Int(b)) => {
                use crate::interpreter::value::DecimalValue;
                let b_dec = rust_decimal::Decimal::from(*b);
                Ok(Value::Decimal(DecimalValue(
                    decimal_op(a.0, "*", b_dec, span)?,
                    a.1,
                )))
            }
            (Value::Int(b), Value::Decimal(a)) => {
                use crate::interpreter::value::DecimalValue;
                let b_dec = rust_decimal::Decimal::from(*b);
                Ok(Value::Decimal(DecimalValue(
                    decimal_op(b_dec, "*", a.0, span)?,
                    a.1,
                )))
            }
            (Value::Decimal(a), Value::Float(b)) => {
                Ok(Value::Float(a.value().to_f64().unwrap_or(0.0) * b))
            }
            (Value::Float(a), Value::Decimal(b)) => {
                Ok(Value::Float(a * b.value().to_f64().unwrap_or(0.0)))
            }
            (Value::String(s), Value::Int(n)) | (Value::Int(n), Value::String(s)) => {
                // `*n as usize` turned a negative count into `usize::MAX` and a
                // large one into a multi-gigabyte allocation; both took the
                // process down rather than the request.
                let repeated = crate::interpreter::limits::repeat_string(s, *n, "string * count")
                    .map_err(|message| RuntimeError::General { message, span })?;
                Ok(Value::String(repeated.into()))
            }
            _ => Err(RuntimeError::type_error(
                format!(
                    "Cannot multiply {} and {}",
                    left.type_name(),
                    right.type_name()
                ),
                span,
            )),
        }
    }

    /// `left / right`, on both engines. A zero divisor raises only between
    /// numbers; any other pair is the type error.
    pub(crate) fn eval_divide(left: &Value, right: &Value, span: Span) -> RuntimeResult<Value> {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    // `i64::MIN / -1` has no representable result and panics
                    // even with overflow checks off.
                    match a.checked_div(*b) {
                        Some(quotient) => Ok(Value::Int(quotient)),
                        None => Err(int_overflow(*a, "/", *b, span)),
                    }
                }
            }
            (Value::Float(a), Value::Float(b)) => {
                if *b == 0.0 {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    Ok(Value::Float(a / b))
                }
            }
            (Value::Int(a), Value::Float(b)) => {
                if *b == 0.0 {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    Ok(Value::Float(*a as f64 / b))
                }
            }
            (Value::Float(a), Value::Int(b)) => {
                if *b == 0 {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    Ok(Value::Float(a / *b as f64))
                }
            }
            (Value::Decimal(a), Value::Decimal(b)) => {
                use crate::interpreter::value::DecimalValue;
                if b.0.is_zero() {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    Ok(Value::Decimal(DecimalValue(
                        decimal_op(a.0, "/", b.0, span)?,
                        std::cmp::max(a.1, b.1),
                    )))
                }
            }
            (Value::Decimal(a), Value::Int(b)) => {
                use crate::interpreter::value::DecimalValue;
                if *b == 0 {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    let b_dec = rust_decimal::Decimal::from(*b);
                    Ok(Value::Decimal(DecimalValue(
                        decimal_op(a.0, "/", b_dec, span)?,
                        a.1,
                    )))
                }
            }
            (Value::Int(b), Value::Decimal(a)) => {
                use crate::interpreter::value::DecimalValue;
                if a.0.is_zero() {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    let b_dec = rust_decimal::Decimal::from(*b);
                    Ok(Value::Decimal(DecimalValue(
                        decimal_op(b_dec, "/", a.0, span)?,
                        a.1,
                    )))
                }
            }
            _ => Err(RuntimeError::type_error(
                format!(
                    "Cannot divide {} by {}",
                    left.type_name(),
                    right.type_name()
                ),
                span,
            )),
        }
    }

    /// `left % right`, on both engines. A Float remainder by zero, Int or
    /// Float, is NaN, as the native kernels compute it.
    pub(crate) fn eval_modulo(left: &Value, right: &Value, span: Span) -> RuntimeResult<Value> {
        match (left, right) {
            (Value::Int(a), Value::Int(b)) => {
                if *b == 0 {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    match a.checked_rem(*b) {
                        Some(remainder) => Ok(Value::Int(remainder)),
                        None => Err(int_overflow(*a, "%", *b, span)),
                    }
                }
            }
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a % b)),
            (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 % b)),
            (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a % *b as f64)),
            (Value::Decimal(a), Value::Decimal(b)) => {
                use crate::interpreter::value::DecimalValue;
                if b.0.is_zero() {
                    Err(RuntimeError::division_by_zero(span))
                } else {
                    // Checked: rust_decimal's `%` panics where `checked_rem`
                    // gives up ("Division overflowed").
                    Ok(Value::Decimal(DecimalValue(
                        decimal_op(a.0, "%", b.0, span)?,
                        std::cmp::max(a.1, b.1),
                    )))
                }
            }
            _ => Err(RuntimeError::type_error(
                format!(
                    "Cannot modulo {} by {}",
                    left.type_name(),
                    right.type_name()
                ),
                span,
            )),
        }
    }

    /// `left <op> right` for `<`, `<=`, `>` and `>=`, on both engines: the VM's
    /// `op_compare_less` / `op_compare_less_equal` call it for whatever their
    /// inline Int and Float arms leave.
    ///
    /// Ints compare exactly (past 2^53 two different Ints round to the same
    /// f64), strings byte by byte, DateTimes by their nanosecond timestamp,
    /// exactly too (as `f64` it rounds to 256 ns, so `a < b`, `a == b` and
    /// `a > b` could all be false). An Int meeting a Float, and a Decimal
    /// meeting a Decimal, an Int or a Float, compare as `f64`. A NaN orders
    /// with nothing, so every comparison with one is false.
    pub(crate) fn compare_ordered(
        left: &Value,
        op: OrdOp,
        right: &Value,
        span: Span,
    ) -> RuntimeResult<bool> {
        let ordering = match (left, right) {
            (Value::Int(a), Value::Int(b)) => Some(a.cmp(b)),
            (Value::Float(a), Value::Float(b)) => a.partial_cmp(b),
            (Value::Int(a), Value::Float(b)) => (*a as f64).partial_cmp(b),
            (Value::Float(a), Value::Int(b)) => a.partial_cmp(&(*b as f64)),
            (Value::Decimal(a), Value::Decimal(b)) => a.to_f64().partial_cmp(&b.to_f64()),
            (Value::Decimal(a), Value::Int(b)) => a.to_f64().partial_cmp(&(*b as f64)),
            (Value::Int(a), Value::Decimal(b)) => (*a as f64).partial_cmp(&b.to_f64()),
            (Value::Decimal(a), Value::Float(b)) => a.to_f64().partial_cmp(b),
            (Value::Float(a), Value::Decimal(b)) => a.partial_cmp(&b.to_f64()),
            (Value::String(a), Value::String(b)) => Some(a.as_str().cmp(b.as_str())),
            _ => match (left.datetime_ts(), right.datetime_ts()) {
                (Some(a), Some(b)) => Some(a.cmp(&b)),
                _ => {
                    return Err(RuntimeError::type_error(
                        format!(
                            "Cannot compare {} and {}",
                            left.type_name(),
                            right.type_name()
                        ),
                        span,
                    ))
                }
            },
        };
        Ok(ordering.is_some_and(|ordering| match op {
            OrdOp::Less => ordering.is_lt(),
            OrdOp::LessEqual => ordering.is_le(),
            OrdOp::Greater => ordering.is_gt(),
            OrdOp::GreaterEqual => ordering.is_ge(),
        }))
    }

    pub(crate) fn evaluate_unary(
        &mut self,
        op: UnaryOp,
        operand: &Expr,
        span: Span,
    ) -> RuntimeResult<Value> {
        let val = self.evaluate(operand)?;

        match op {
            UnaryOp::Negate => match val {
                Value::Int(n) => n
                    .checked_neg()
                    .map(Value::Int)
                    .ok_or_else(|| int_negate_overflow(n, span)),
                Value::Float(n) => Ok(Value::Float(-n)),
                Value::Decimal(n) => {
                    use crate::interpreter::value::DecimalValue;
                    Ok(Value::Decimal(DecimalValue(-n.0, n.1)))
                }
                _ => Err(RuntimeError::type_error(
                    format!("Cannot negate {}", val.type_name()),
                    span,
                )),
            },
            UnaryOp::Not => Ok(Value::Bool(!val.is_truthy())),
        }
    }

    pub(crate) fn evaluate_logical_and(
        &mut self,
        left: &Expr,
        right: &Expr,
    ) -> RuntimeResult<Value> {
        let left_val = self.evaluate(left)?;
        if !left_val.is_truthy() {
            Ok(left_val)
        } else {
            self.evaluate(right)
        }
    }

    pub(crate) fn evaluate_logical_or(
        &mut self,
        left: &Expr,
        right: &Expr,
    ) -> RuntimeResult<Value> {
        let left_val = self.evaluate(left)?;
        if left_val.is_truthy() {
            Ok(left_val)
        } else {
            self.evaluate(right)
        }
    }

    pub(crate) fn evaluate_nullish_coalescing(
        &mut self,
        left: &Expr,
        right: &Expr,
    ) -> RuntimeResult<Value> {
        let left_val = self.evaluate(left)?;
        if matches!(left_val, Value::Null) {
            self.evaluate(right)
        } else {
            Ok(left_val)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::value::DecimalValue;

    /// `<`, `<=`, `>` and `>=` of `left` against `right`.
    fn order(left: &Value, right: &Value) -> [bool; 4] {
        [
            OrdOp::Less,
            OrdOp::LessEqual,
            OrdOp::Greater,
            OrdOp::GreaterEqual,
        ]
        .map(|op| {
            Interpreter::compare_ordered(left, op, right, Span::default()).expect("comparable")
        })
    }

    /// Two instants 10 ns apart, 2026-01-01T00:00:00.000000100Z and …110Z.
    /// As `f64` nanoseconds both round to the same value (the step is 256 ns
    /// at this epoch), so the old comparator found `early < late` false and
    /// `early >= late` true while `==` said they differ. The VM always
    /// compared the `i64`.
    #[test]
    fn datetime_ordering_is_exact() {
        let early = Value::DateTime(1_767_225_600_000_000_100, false);
        let late = Value::DateTime(1_767_225_600_000_000_110, false);
        assert_eq!(order(&early, &late), [true, true, false, false]);
        assert_eq!(order(&late, &early), [false, false, true, true]);
        assert_eq!(order(&early, &early), [false, true, false, true]);
    }

    #[test]
    fn nan_compares_false_and_mismatches_raise() {
        let nan = Value::Float(f64::NAN);
        assert_eq!(order(&nan, &Value::Float(1.0)), [false; 4]);
        assert_eq!(order(&Value::Int(1), &nan), [false; 4]);
        let one = Value::Decimal(DecimalValue(rust_decimal::Decimal::ONE, 0));
        assert_eq!(order(&one, &nan), [false; 4]);
        assert_eq!(order(&one, &Value::Int(1)), [false, true, false, true]);
        // 2^53 and 2^53 + 1 are one f64; as Ints they still order.
        let (low, high) = (Value::Int(1 << 53), Value::Int((1 << 53) + 1));
        assert_eq!(order(&low, &high), [true, true, false, false]);
        let (a, b) = (Value::String("a".into()), Value::String("b".into()));
        assert_eq!(order(&a, &b), [true, true, false, false]);

        let array = Value::Array(Rc::new(RefCell::new(vec![Value::Int(1)])));
        let err =
            Interpreter::compare_ordered(&array, OrdOp::Less, &Value::Int(1), Span::default())
                .expect_err("an array does not order against an int");
        assert!(
            err.to_string().contains("Cannot compare array and int"),
            "{err}"
        );
    }
}
