//! Array literals made only of literals, compiled from one constant.
//!
//! `["draft", "sent"]` is one `Op::LiteralArray` over a `Constant::LiteralList`
//! instead of a `Constant` per element and an `Op::Array`, and
//! `["draft", "sent"].includes?(status)` — the membership test the style guide
//! recommends, also spelled `include?`, `contains` and `contains?` — is the
//! argument followed by one `Op::LiteralListIncludes`, with no array built and
//! no method dispatched. `literal_list.rs` holds the run-time half.
//!
//! An element is a literal when it is a string, integer, float, boolean, nil
//! or symbol literal, one in parentheses, or literal arithmetic that folds
//! (`-1`, `60 * 60`, see `compiler_fold.rs`). Anything else — a variable, a
//! call, an interpolated string, a decimal, a nested array, a spread — leaves
//! the whole array on the per-element path. Literals cannot fail or have side
//! effects, so not evaluating them one by one is unobservable; under line
//! coverage their lines are still marked, in the order the per-element path
//! marks them.

use std::sync::Arc;

use crate::ast::expr::{Argument, ExprKind};
use crate::ast::Expr;

use super::chunk::Constant;
use super::compiler::{CompileResult, Compiler};
use super::literal_list::{ListLiteral, LiteralList};
use super::method_table::{resolve_method_id, METHOD_UNKNOWN};
use super::opcode::{Op, NO_USER_METHOD};

/// The names `Array#includes?` answers to, on both engines.
const MEMBERSHIP_METHODS: [&str; 4] = ["includes?", "include?", "contains", "contains?"];

impl Compiler {
    /// `[<literals>]`, one or more of them: one `LiteralArray`. `false`,
    /// with nothing emitted, for any other array literal (`[]` is a single
    /// `Array(0)` already).
    pub(super) fn try_compile_literal_array(&mut self, elements: &[Expr], line: usize) -> bool {
        if elements.is_empty() {
            return false;
        }
        let Some(list) = self.literal_list(elements) else {
            return false;
        };
        // The array's own line was marked by `compile_expr`.
        for element in elements {
            self.cover_literal_lines(element);
        }
        let list_idx = self.add_constant(Constant::LiteralList(Arc::new(list)));
        self.emit(Op::LiteralArray(list_idx), line);
        true
    }

    /// `[<literals>].includes?(x)` — or `include?`, `contains`, `contains?` —
    /// with one positional argument: the argument, then one
    /// `LiteralListIncludes`. `false`, with nothing emitted, for any other
    /// call.
    ///
    /// The op keeps the user-method gate of the call it replaces: a name
    /// with no method id compiles to `CallMethod`, which lets a user-defined
    /// Array method of that name take the call; one with an id
    /// (`include?`, `contains`) to `CallMethodById`, which does not.
    pub(super) fn try_compile_literal_membership(
        &mut self,
        object: &Expr,
        name: &str,
        arguments: &[Argument],
        line: usize,
    ) -> CompileResult<bool> {
        if !MEMBERSHIP_METHODS.contains(&name) {
            return Ok(false);
        }
        let [Argument::Positional(needle)] = arguments else {
            return Ok(false);
        };
        let Some(elements) = array_literal(object) else {
            return Ok(false);
        };
        let Some(list) = self.literal_list(elements) else {
            return Ok(false);
        };
        // The receiver is evaluated first, as before: under coverage its
        // lines are marked ahead of the argument's.
        self.cover_literal_lines(object);
        self.compile_expr(needle)?;
        let list_idx = self.add_constant(Constant::LiteralList(Arc::new(list)));
        let user_method = if resolve_method_id(name) == METHOD_UNKNOWN {
            self.add_string_constant(name)
        } else {
            NO_USER_METHOD
        };
        self.emit(Op::LiteralListIncludes(list_idx, user_method), line);
        Ok(true)
    }

    /// Every element as a literal, or `None` if one is not.
    fn literal_list(&self, elements: &[Expr]) -> Option<LiteralList> {
        let items = elements
            .iter()
            .map(|element| self.list_literal(element))
            .collect::<Option<Vec<_>>>()?;
        Some(LiteralList { items })
    }

    /// The literal `expr` is, as the value compiling it would push.
    fn list_literal(&self, expr: &Expr) -> Option<ListLiteral> {
        Some(match &expr.kind {
            ExprKind::IntLiteral(n) => ListLiteral::Int(*n),
            ExprKind::FloatLiteral(f) => ListLiteral::Float(*f),
            ExprKind::StringLiteral(s) => ListLiteral::String(s.as_str().into()),
            ExprKind::Symbol(s) => ListLiteral::Symbol(s.as_str().into()),
            ExprKind::BoolLiteral(b) => ListLiteral::Bool(*b),
            ExprKind::Null => ListLiteral::Null,
            ExprKind::Grouping(inner) => return self.list_literal(inner),
            // What `compile_expr` emits for them when they fold; off under
            // coverage, as folding is.
            ExprKind::Binary { .. } | ExprKind::Unary { .. } => match self.folded(expr)? {
                Constant::Int(n) => ListLiteral::Int(n),
                Constant::Float(f) => ListLiteral::Float(f),
                Constant::String(s) => ListLiteral::String(s),
                _ => return None,
            },
            _ => return None,
        })
    }

    /// Mark the lines of a literal (or of an array of them) as compiling it
    /// would: `compile_expr` marks each expression's line before its parts'.
    fn cover_literal_lines(&mut self, expr: &Expr) {
        self.cover_line(expr.span.line as usize);
        match &expr.kind {
            ExprKind::Grouping(inner) => self.cover_literal_lines(inner),
            ExprKind::Array(elements) => {
                for element in elements {
                    self.cover_literal_lines(element);
                }
            }
            _ => {}
        }
    }
}

/// The elements of an array literal, parentheses aside.
fn array_literal(expr: &Expr) -> Option<&[Expr]> {
    match &expr.kind {
        ExprKind::Array(elements) => Some(elements),
        ExprKind::Grouping(inner) => array_literal(inner),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::stmt::{Program, StmtKind};
    use crate::lexer::Scanner;
    use crate::parser::Parser;
    use crate::vm::chunk::FunctionProto;
    use crate::vm::compiler::FunctionType;

    fn parse(src: &str) -> Program {
        let tokens = Scanner::new(src).scan_tokens().expect("lex");
        Parser::new(tokens).parse().expect("parse")
    }

    /// The compiled `def f(s) <body> end`.
    fn proto_of(body: &str) -> Arc<FunctionProto> {
        let program = parse(&format!("def f(s)\n  {body}\nend\n"));
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

    fn has(proto: &FunctionProto, wanted: impl Fn(&Op) -> bool) -> bool {
        proto.chunk.code.iter().any(wanted)
    }

    /// The list a literal-list op of `proto` reads, and its user-method name.
    fn membership_of(proto: &FunctionProto) -> (Vec<ListLiteral>, Option<String>) {
        let (list_idx, user_method) = proto
            .chunk
            .code
            .iter()
            .find_map(|op| match op {
                Op::LiteralListIncludes(list, user_method) => Some((*list, *user_method)),
                _ => None,
            })
            .expect("a LiteralListIncludes");
        let Constant::LiteralList(list) = &proto.chunk.constants[list_idx as usize] else {
            panic!("the op reads a LiteralList constant");
        };
        let name = (user_method != NO_USER_METHOD).then(|| {
            match &proto.chunk.constants[user_method as usize] {
                Constant::String(name) => name.to_string(),
                other => panic!("the method name is a string constant, got {other:?}"),
            }
        });
        (list.items.clone(), name)
    }

    fn strings(items: &[&str]) -> Vec<ListLiteral> {
        items
            .iter()
            .map(|s| ListLiteral::String((*s).into()))
            .collect()
    }

    #[test]
    fn membership_in_a_literal_list_is_one_op() {
        let proto = proto_of("[\"up\", \"late\", \"overdue\"].includes?(s)");
        assert!(
            matches!(
                proto.chunk.code.as_slice(),
                [Op::GetLocal(1), Op::LiteralListIncludes(..), Op::Return, ..]
            ),
            "got: {:?}",
            proto.chunk.code
        );
        assert!(!has(&proto, |op| matches!(
            op,
            Op::Array(_) | Op::Constant(_) | Op::CallMethod(..) | Op::CallMethodById(..)
        )));
        let (items, name) = membership_of(&proto);
        assert_eq!(items, strings(&["up", "late", "overdue"]));
        assert_eq!(name.as_deref(), Some("includes?"));
    }

    /// `includes?` and `contains?` keep `CallMethod`'s user-method gate;
    /// `include?` and `contains` have a method id, compile to
    /// `CallMethodById`, and have none.
    #[test]
    fn every_membership_name_compiles_to_the_op_with_its_call_gate() {
        for (method, gate) in [
            ("includes?", Some("includes?")),
            ("contains?", Some("contains?")),
            ("include?", None),
            ("contains", None),
        ] {
            let proto = proto_of(&format!("[:a, 1, 2.5, true, nil].{method}(s)"));
            let (items, name) = membership_of(&proto);
            assert_eq!(
                items,
                vec![
                    ListLiteral::Symbol("a".into()),
                    ListLiteral::Int(1),
                    ListLiteral::Float(2.5),
                    ListLiteral::Bool(true),
                    ListLiteral::Null,
                ],
                "{method}"
            );
            assert_eq!(name.as_deref(), gate, "{method}");
        }
    }

    #[test]
    fn folded_parenthesized_and_word_array_elements_are_literals() {
        let (items, _) = membership_of(&proto_of("[-1, (2), 60 * 60].includes?(s)"));
        assert_eq!(
            items,
            vec![
                ListLiteral::Int(-1),
                ListLiteral::Int(2),
                ListLiteral::Int(3600)
            ]
        );
        let (items, _) = membership_of(&proto_of("%w[draft sent].include?(s)"));
        assert_eq!(items, strings(&["draft", "sent"]));
        let (items, _) = membership_of(&proto_of("([\"a\"]).contains(s)"));
        assert_eq!(items, strings(&["a"]));
        let (items, _) = membership_of(&proto_of("[].includes?(s)"));
        assert!(items.is_empty());
    }

    /// Only a membership call with one positional argument on a list made of
    /// literals takes the op; the rest compile as before.
    #[test]
    fn other_shapes_keep_the_call() {
        for body in [
            "[\"up\", s].includes?(s)",
            "[\"up\", \"#{s}\"].includes?(s)",
            "[1.5D].includes?(s)",
            "[[1], 2].includes?(s)",
            "[\"up\"].includes?(s, s)",
            "[\"up\"].index_of(s)",
            "[\"up\"].first()",
            "x = [\"up\"]\n  x.includes?(s)",
        ] {
            let proto = proto_of(body);
            assert!(
                !has(&proto, |op| matches!(op, Op::LiteralListIncludes(..))),
                "{body}: {:?}",
                proto.chunk.code
            );
            assert!(
                has(&proto, |op| matches!(
                    op,
                    Op::CallMethod(..) | Op::CallMethodById(..)
                )),
                "{body}: {:?}",
                proto.chunk.code
            );
        }
    }

    #[test]
    fn an_all_literal_array_is_one_op() {
        let proto = proto_of("[\"a\", :b, 1, 2.5, true, nil, -3]");
        let code = &proto.chunk.code;
        assert!(
            matches!(code.as_slice(), [Op::LiteralArray(_), Op::Return, ..]),
            "got: {code:?}"
        );
        let Some(Op::LiteralArray(list_idx)) = code.first() else {
            unreachable!();
        };
        let Constant::LiteralList(list) = &proto.chunk.constants[*list_idx as usize] else {
            panic!("LiteralArray reads a LiteralList constant");
        };
        assert_eq!(
            list.items,
            vec![
                ListLiteral::String("a".into()),
                ListLiteral::Symbol("b".into()),
                ListLiteral::Int(1),
                ListLiteral::Float(2.5),
                ListLiteral::Bool(true),
                ListLiteral::Null,
                ListLiteral::Int(-3),
            ]
        );
        // Nested: each inner literal is one op, the outer array is built.
        let nested = proto_of("[[1, 2], [\"x\"]]");
        assert!(
            matches!(
                nested.chunk.code.as_slice(),
                [
                    Op::LiteralArray(_),
                    Op::LiteralArray(_),
                    Op::Array(2),
                    Op::Return,
                    ..
                ]
            ),
            "got: {:?}",
            nested.chunk.code
        );
        // Empty, or with an element that is not a literal: as before.
        let empty = proto_of("[]");
        assert!(matches!(
            empty.chunk.code.as_slice(),
            [Op::Array(0), Op::Return, ..]
        ));
        let mixed = proto_of("[\"a\", s]");
        assert!(has(&mixed, |op| matches!(op, Op::Array(2))));
        assert!(!has(&mixed, |op| matches!(op, Op::LiteralArray(_))));
    }

    /// `src` (which assigns `result`) on the tree-walker and on the VM,
    /// each result shown.
    fn on_both_engines(src: &str) -> (String, String) {
        let program = parse(src);
        let mut interpreter = crate::interpreter::Interpreter::new();
        let tree = match interpreter.interpret(&program) {
            Ok(()) => interpreter
                .environment
                .borrow()
                .get("result")
                .map(|value| value.to_string())
                .unwrap_or_else(|| "no result".to_string()),
            Err(err) => format!("error: {err}"),
        };
        let module = Compiler::compile(&program).expect("compile");
        let seed = crate::interpreter::Interpreter::new();
        let mut vm = crate::vm::Vm::new();
        for (name, value) in seed.environment.borrow().get_all_bindings() {
            vm.globals.insert(name, value);
        }
        let on_vm = match vm.execute(&module.main) {
            Ok(_) => vm
                .globals
                .get("result")
                .map(|value| value.to_string())
                .unwrap_or_else(|| "no result".to_string()),
            Err(err) => format!("error: {err}"),
        };
        (tree, on_vm)
    }

    /// Membership answers as `Array#includes?` does, scalar needles and the
    /// others (a class equals the string of its name; arrays compare by
    /// value), in a function, a method and at top level.
    #[test]
    fn membership_answers_as_array_includes_on_both_engines() {
        let src = r#"class Account
end

class Ticket
  status: String

  new(status: String)
    @status = status
  end

  def open?
    ["up", "late", "overdue"].includes?(@status)
  end
end

def check(s)
  [
    ["up", "late"].includes?(s),
    ["up", "late"].include?(s),
    ["up", "late"].contains(s),
    ["up", "late"].contains?(s),
    [1, 2, 3].includes?(s),
    [1.5, 2.0].includes?(s),
    [:up, :late].includes?(s),
    [true, nil].includes?(s),
    [-1, 60 * 60].includes?(s),
    [].includes?(s)
  ]
end

result = [
  check("late"), check("pending"), check(2), check(2.0), check(1.5), check(:up),
  check(nil), check(false), check(-1), check(3600), check([1]),
  ["Account", "User"].includes?(Account), [:Account].includes?(Account),
  [[1], 2].includes?([1]), %w[a b].includes?("b"),
  Ticket.new("late").open?, Ticket.new("closed").open?
]
"#;
        let (tree, on_vm) = on_both_engines(src);
        assert!(!tree.starts_with("error"), "tree-walker: {tree}");
        assert_eq!(tree, on_vm);
    }

    /// `LiteralArray` builds a new array on every evaluation: changing one
    /// leaves the next alone.
    #[test]
    fn a_literal_array_is_a_new_array_each_time() {
        let src = r#"def make()
  ["a", :b, 1]
end

first = make()
first.push("pushed")
first[0] = "changed"
second = make()
rows = []
i = 0
while i < 3
  row = [0, 0]
  row[0] = i
  rows.push(row)
  i += 1
end
result = [first, second, rows, make() == make()]
"#;
        let (tree, on_vm) = on_both_engines(src);
        assert_eq!(
            tree,
            "[[changed, :b, 1, pushed], [a, :b, 1], [[0, 0], [1, 0], [2, 0]], true]"
        );
        assert_eq!(tree, on_vm);
    }

    /// A user-defined Array method takes a literal's membership call
    /// exactly when it takes the same call on an array in a variable:
    /// through `CallMethod`'s gate for `includes?` and `contains?`, never
    /// for `include?` and `contains` (`CallMethodById`).
    ///
    /// The methods are registered on the tree-walker, as an app does: a VM
    /// closure cannot be defined on a primitive class.
    #[test]
    fn a_user_method_takes_the_call_where_the_call_it_replaces_would_let_it() {
        let mut interpreter = crate::interpreter::Interpreter::new();
        interpreter
            .interpret(&parse(
                "Array.define_method(\"includes?\", fn(x) { \"user includes?\" })\n\
                 Array.define_method(\"contains?\", fn(x) { \"user contains?\" })\n\
                 Array.define_method(\"include?\", fn(x) { \"user include?\" })\n\
                 Array.define_method(\"contains\", fn(x) { \"user contains\" })\n",
            ))
            .expect("define the methods");
        let program = parse(
            "xs = [\"a\", \"b\"]\n\
             literal = [[\"a\", \"b\"].includes?(\"a\"), [\"a\", \"b\"].contains?(\"a\"), [\"a\", \"b\"].include?(\"a\"), [\"a\", \"b\"].contains(\"a\")]\n\
             variable = [xs.includes?(\"a\"), xs.contains?(\"a\"), xs.include?(\"a\"), xs.contains(\"a\")]\n",
        );
        let module = Compiler::compile(&program).expect("compile");
        let mut vm = crate::vm::Vm::new();
        vm.execute(&module.main).expect("run on the VM");
        let literal = vm.globals.get("literal").expect("literal").to_string();
        let variable = vm.globals.get("variable").expect("variable").to_string();
        assert_eq!(literal, variable);
        assert_eq!(literal, "[user includes?, user contains?, true, true]");
    }

    /// Under line coverage, every line of the literal is still marked, ahead
    /// of the op, as the per-element path marked them.
    #[test]
    fn coverage_still_marks_each_line_of_the_literal() {
        let program =
            parse("[\n  \"up\",\n  \"late\"\n].includes?(status)\nrows = [\n  1,\n  2\n]\n");
        let mut compiler = Compiler::new(FunctionType::Script, String::new());
        compiler.coverage = true;
        compiler.proto.source_path = Some(Arc::new(std::path::PathBuf::from("t.sl")));
        for statement in &program.statements {
            let StmtKind::Expression(expr) = &statement.kind else {
                panic!("expression statements");
            };
            compiler.compile_expr(expr).expect("compile");
        }
        let code = &compiler.proto.chunk.code;
        let first = |wanted: Op| code.iter().position(|op| *op == wanted);
        let includes = code
            .iter()
            .position(|op| matches!(op, Op::LiteralListIncludes(..)))
            .expect("the membership op");
        let array = code
            .iter()
            .position(|op| matches!(op, Op::LiteralArray(_)))
            .expect("the array op");
        for line in [1, 2, 3] {
            let at = first(Op::CoverLine(line)).expect("marked");
            assert!(at < includes, "line {line}: {code:?}");
        }
        for line in [6, 7] {
            let at = first(Op::CoverLine(line)).expect("marked");
            assert!(includes < at && at < array, "line {line}: {code:?}");
        }
    }
}
