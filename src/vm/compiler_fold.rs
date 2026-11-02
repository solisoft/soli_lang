//! Constant folding: arithmetic on number and string literals, worked out once
//! at compile time.
//!
//! `60 * 60 * 24` compiled to three `Constant`s and two `Multiply`s, run on
//! every evaluation, and `-1` to a `Constant` and a `Negate`. [`fold`] computes
//! the value the VM's own arithmetic opcodes would produce, with the same
//! operations, and only where they succeed. An integer overflow, a division by
//! zero or an operand that is not a literal leaves the expression to be
//! computed, and to raise, at run time as written.

use crate::ast::expr::{BinaryOp, Expr, ExprKind, UnaryOp};

use super::chunk::Constant;
use super::compiler::Compiler;

impl Compiler {
    /// The constant `expr` folds to ([`fold`]). Not under line coverage,
    /// where each operand's line is marked as it is compiled: a literal
    /// product written over two lines would lose the second one's mark.
    pub(crate) fn folded(&self, expr: &Expr) -> Option<Constant> {
        if self.coverage && self.proto.source_path.is_some() {
            return None;
        }
        fold(expr)
    }
}

/// The most nodes one attempt looks at. In `x + 1 + 2 + …`, with the variable
/// at the far left, every `+` would otherwise walk the whole chain below it.
const BUDGET: usize = 64;

/// A literal operand, as folding computes with it.
enum Literal {
    Int(i64),
    Float(f64),
    Str(String),
}

impl Literal {
    fn into_constant(self) -> Constant {
        match self {
            Literal::Int(n) => Constant::Int(n),
            Literal::Float(f) => Constant::Float(f),
            Literal::Str(s) => Constant::String(s.into()),
        }
    }

    /// A number as a mixed `Int`/`Float` operation takes it: the VM converts
    /// an `Int` operand with `as f64`.
    fn as_float(&self) -> Option<f64> {
        match self {
            Literal::Float(f) => Some(*f),
            Literal::Int(n) => Some(*n as f64),
            Literal::Str(_) => None,
        }
    }
}

/// The constant a binary or unary expression folds to: an operator applied to
/// literals, or to literal sub-expressions that fold themselves. `None` for
/// anything that has to run.
pub(super) fn fold(expr: &Expr) -> Option<Constant> {
    match &expr.kind {
        ExprKind::Binary { .. } | ExprKind::Unary { .. } => {
            let mut budget = BUDGET;
            literal(expr, &mut budget).map(Literal::into_constant)
        }
        _ => None,
    }
}

fn literal(expr: &Expr, budget: &mut usize) -> Option<Literal> {
    *budget = budget.checked_sub(1)?;
    match &expr.kind {
        ExprKind::IntLiteral(n) => Some(Literal::Int(*n)),
        ExprKind::FloatLiteral(f) => Some(Literal::Float(*f)),
        ExprKind::StringLiteral(s) => Some(Literal::Str(s.clone())),
        ExprKind::Grouping(inner) => literal(inner, budget),
        ExprKind::Unary {
            operator: UnaryOp::Negate,
            operand,
        } => match literal(operand, budget)? {
            // `Op::Negate`: an `Int` with no negation (`i64::MIN`) raises.
            Literal::Int(n) => n.checked_neg().map(Literal::Int),
            Literal::Float(f) => Some(Literal::Float(-f)),
            Literal::Str(_) => None,
        },
        ExprKind::Binary {
            left,
            operator,
            right,
        } => {
            // The right operand first: in a chain leaning left it is the
            // leaf, and the first one that is not a literal ends the attempt.
            let right = literal(right, budget)?;
            let left = literal(left, budget)?;
            binary(left, *operator, right)
        }
        _ => None,
    }
}

/// `left op right` as the VM's `Add`, `Subtract`, `Multiply`, `Divide` and
/// `Modulo` compute it, or `None` where they raise or the pair is not one
/// folded here.
fn binary(left: Literal, op: BinaryOp, right: Literal) -> Option<Literal> {
    use Literal::{Float, Int, Str};
    match (left, op, right) {
        (Int(x), BinaryOp::Add, Int(y)) => x.checked_add(y).map(Int),
        (Int(x), BinaryOp::Subtract, Int(y)) => x.checked_sub(y).map(Int),
        (Int(x), BinaryOp::Multiply, Int(y)) => x.checked_mul(y).map(Int),
        // Integer division truncates; a zero divisor and `MIN / -1` raise.
        (Int(x), BinaryOp::Divide, Int(y)) if y != 0 => x.checked_div(y).map(Int),
        (Int(x), BinaryOp::Modulo, Int(y)) if y != 0 => x.checked_rem(y).map(Int),
        (Float(x), BinaryOp::Divide, Float(y)) if y != 0.0 => Some(Float(x / y)),
        (Str(x), BinaryOp::Add, Str(y)) => Some(Str(x + &y)),
        (left, op @ (BinaryOp::Add | BinaryOp::Subtract | BinaryOp::Multiply), right) => {
            let (x, y) = (left.as_float()?, right.as_float()?);
            Some(Float(match op {
                BinaryOp::Add => x + y,
                BinaryOp::Subtract => x - y,
                _ => x * y,
            }))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::stmt::{Program, StmtKind};
    use crate::interpreter::value::Value;
    use crate::lexer::Scanner;
    use crate::parser::Parser;
    use crate::vm::chunk::FunctionProto;
    use crate::vm::compiler::Compiler;
    use crate::vm::opcode::Op;
    use std::sync::Arc;

    fn parse(src: &str) -> Program {
        let tokens = Scanner::new(src).scan_tokens().expect("lex");
        Parser::new(tokens).parse().expect("parse")
    }

    /// What the right-hand side of `x = <src>` folds to, shown.
    fn folded(src: &str) -> Option<String> {
        let program = parse(&format!("x = {src}\n"));
        let StmtKind::Expression(statement) = &program.statements[0].kind else {
            panic!("expected an expression statement");
        };
        let ExprKind::Assign { value, .. } = &statement.kind else {
            panic!("expected an assignment");
        };
        fold(value).map(|constant| match constant {
            Constant::Int(n) => format!("int {n}"),
            Constant::Float(f) => format!("float {f:?}"),
            Constant::String(s) => format!("string {s:?}"),
            other => panic!("unexpected constant {other:?}"),
        })
    }

    /// The compiled `def f() <body> end`.
    fn proto_of(body: &str) -> Arc<FunctionProto> {
        let program = parse(&format!("def f()\n  {body}\nend\n"));
        let module = Compiler::compile(&program).expect("compile");
        module
            .main
            .chunk
            .constants
            .iter()
            .find_map(|constant| match constant {
                Constant::Function(inner) if inner.name == "f" => Some(inner.clone()),
                _ => None,
            })
            .expect("the proto of f")
    }

    /// The integer constants of `proto`'s pool.
    fn int_constants(proto: &FunctionProto) -> Vec<i64> {
        proto
            .chunk
            .constants
            .iter()
            .filter_map(|constant| match constant {
                Constant::Int(n) => Some(*n),
                _ => None,
            })
            .collect()
    }

    /// `result = <src>` run on the VM, shown.
    fn run(src: &str) -> String {
        let program = parse(&format!("result = {src}\n"));
        let module = Compiler::compile(&program).expect("compile");
        let interpreter = crate::interpreter::Interpreter::new();
        let mut vm = crate::vm::Vm::new();
        for (name, value) in interpreter.environment.borrow().get_all_bindings() {
            vm.globals.insert(name, value);
        }
        if let Err(err) = vm.execute(&module.main) {
            return format!("error: {err}");
        }
        let result: &Value = vm.globals.get("result").expect("a result global");
        result.to_string()
    }

    #[test]
    fn integer_arithmetic_folds() {
        assert_eq!(folded("60 * 60 * 24").as_deref(), Some("int 86400"));
        assert_eq!(folded("10 - 2 - 3").as_deref(), Some("int 5"));
        assert_eq!(folded("2 * 3 + 4 * 5").as_deref(), Some("int 26"));
        assert_eq!(folded("(1 + 2) * 3").as_deref(), Some("int 9"));
        assert_eq!(folded("-7 / 2").as_deref(), Some("int -3"));
        assert_eq!(folded("-7 % 3").as_deref(), Some("int -1"));
        assert_eq!(folded("-5").as_deref(), Some("int -5"));
        assert_eq!(folded("2 * -3").as_deref(), Some("int -6"));
    }

    #[test]
    fn float_and_mixed_arithmetic_folds_as_the_vm_computes_it() {
        assert_eq!(
            folded("0.1 + 0.2").as_deref(),
            Some("float 0.30000000000000004")
        );
        assert_eq!(folded("1 + 2.5").as_deref(), Some("float 3.5"));
        assert_eq!(folded("2.0 * 3").as_deref(), Some("float 6.0"));
        assert_eq!(folded("7.5 / 2.5").as_deref(), Some("float 3.0"));
        assert_eq!(folded("-2.5").as_deref(), Some("float -2.5"));
    }

    #[test]
    fn string_concatenation_folds() {
        assert_eq!(
            folded("\"ab\" + \"cd\"").as_deref(),
            Some("string \"abcd\"")
        );
    }

    /// What raises at run time is left to run, and raise there.
    #[test]
    fn overflow_and_division_by_zero_are_not_folded() {
        assert_eq!(folded("9223372036854775807 + 1"), None);
        assert_eq!(folded("1 / 0"), None);
        assert_eq!(folded("1 % 0"), None);
        assert_eq!(folded("1.0 / 0.0"), None);
        assert_eq!(folded("-9223372036854775807 - 2"), None);
        assert!(run("9223372036854775807 + 1").starts_with("error:"));
        assert!(run("1 / 0").starts_with("error:"));
    }

    #[test]
    fn non_literal_or_unfolded_operations_are_left_alone() {
        assert_eq!(folded("x * 60"), None);
        assert_eq!(folded("\"a\" + 1"), None);
        assert_eq!(folded("1 < 2"), None);
        assert_eq!(folded("1.5 % 1.0"), None);
        assert_eq!(folded("7 / 2.0"), None);
        assert_eq!(folded("[1] + [2]"), None);
    }

    #[test]
    fn a_folded_expression_compiles_to_one_constant() {
        let whole = proto_of("60 * 60 * 24");
        let code = &whole.chunk.code;
        assert!(
            matches!(code.as_slice(), [Op::Constant(_), Op::Return, ..]),
            "got: {code:?}"
        );
        assert_eq!(int_constants(&whole), [86400]);

        // `x * (60 * 60)`: the literal product is one constant, the variable
        // is still multiplied at run time.
        let partly = proto_of("x = 7\n  x * (60 * 60)");
        assert_eq!(int_constants(&partly), [7, 3600]);
    }

    #[test]
    fn folded_values_are_what_the_vm_computes() {
        assert_eq!(run("60 * 60 * 24"), "86400");
        assert_eq!(run("0.1 + 0.2"), "0.30000000000000004");
        assert_eq!(run("\"ab\" + \"cd\""), "abcd");
        assert_eq!(run("-7 / 2"), "-3");
        assert_eq!(run("2.0 * 3"), "6");
    }
}
