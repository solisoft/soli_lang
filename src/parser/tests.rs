//! Parser tests.

#[cfg(test)]
mod parser_tests {
    use crate::ast::expr::Argument;
    use crate::ast::*;
    use crate::lexer::Scanner;
    use crate::parser::Parser;

    fn parse_expr(source: &str) -> Expr {
        let tokens = Scanner::new(source).scan_tokens().unwrap();
        let mut parser = Parser::new(tokens);
        let program = parser.parse().unwrap();
        match program.statements.into_iter().next().unwrap().kind {
            StmtKind::Expression(expr) => expr,
            _ => panic!("Expected expression statement"),
        }
    }

    #[test]
    fn test_binary_expr() {
        let expr = parse_expr("1 + 2;");
        match expr.kind {
            ExprKind::Binary { operator, .. } => assert_eq!(operator, BinaryOp::Add),
            _ => panic!("Expected binary expression"),
        }
    }

    #[test]
    fn test_precedence() {
        // 1 + 2 * 3 should parse as 1 + (2 * 3)
        let expr = parse_expr("1 + 2 * 3;");
        match expr.kind {
            ExprKind::Binary {
                operator: BinaryOp::Add,
                right,
                ..
            } => match right.kind {
                ExprKind::Binary {
                    operator: BinaryOp::Multiply,
                    ..
                } => {}
                _ => panic!("Expected multiply on right"),
            },
            _ => panic!("Expected add at top"),
        }
    }

    #[test]
    fn test_pipeline() {
        let expr = parse_expr("x |> foo();");
        match expr.kind {
            ExprKind::Pipeline { .. } => {}
            _ => panic!("Expected pipeline expression"),
        }
    }

    #[test]
    fn test_call() {
        let expr = parse_expr("foo(1, 2);");
        match expr.kind {
            ExprKind::Call { arguments, .. } => assert_eq!(arguments.len(), 2),
            _ => panic!("Expected call expression"),
        }
    }

    #[test]
    fn test_match_literal() {
        let expr = parse_expr("match x { 42 => \"answer\" };");
        match expr.kind {
            ExprKind::Match { arms, .. } => {
                assert_eq!(arms.len(), 1);
                match &arms[0].pattern {
                    MatchPattern::Literal(ExprKind::IntLiteral(42)) => {}
                    _ => panic!("Expected literal 42 pattern"),
                }
            }
            _ => panic!("Expected match expression"),
        }
    }

    #[test]
    fn test_match_wildcard() {
        let expr = parse_expr("match x { _ => \"default\" };");
        match expr.kind {
            ExprKind::Match { arms, .. } => {
                assert_eq!(arms.len(), 1);
                match &arms[0].pattern {
                    MatchPattern::Wildcard => {}
                    _ => panic!("Expected wildcard pattern"),
                }
            }
            _ => panic!("Expected match expression"),
        }
    }

    #[test]
    fn test_match_variable() {
        let expr = parse_expr("match x { n => n };");
        match expr.kind {
            ExprKind::Match { arms, .. } => {
                assert_eq!(arms.len(), 1);
                match &arms[0].pattern {
                    MatchPattern::Variable(name) => assert_eq!(name, "n"),
                    _ => panic!("Expected variable pattern"),
                }
            }
            _ => panic!("Expected match expression"),
        }
    }

    #[test]
    fn test_match_multiple_arms() {
        let expr = parse_expr("match x { 1 => \"one\", 2 => \"two\", _ => \"many\" };");
        match expr.kind {
            ExprKind::Match { arms, .. } => {
                assert_eq!(arms.len(), 3);
            }
            _ => panic!("Expected match expression"),
        }
    }

    #[test]
    fn test_match_guard() {
        let expr = parse_expr("match n { n if n > 0 => \"positive\" };");
        match expr.kind {
            ExprKind::Match { arms, .. } => {
                assert_eq!(arms.len(), 1);
                assert!(arms[0].guard.is_some());
            }
            _ => panic!("Expected match expression"),
        }
    }

    #[test]
    fn test_match_array_pattern() {
        let expr = parse_expr("match arr { [a, b] => a + b };");
        match expr.kind {
            ExprKind::Match { arms, .. } => {
                assert_eq!(arms.len(), 1);
                match &arms[0].pattern {
                    MatchPattern::Array {
                        elements,
                        rest: None,
                    } => {
                        assert_eq!(elements.len(), 2);
                    }
                    _ => panic!("Expected array pattern"),
                }
            }
            _ => panic!("Expected match expression"),
        }
    }

    #[test]
    fn test_not_keyword() {
        let expr = parse_expr("not true;");
        match expr.kind {
            ExprKind::Unary {
                operator: UnaryOp::Not,
                ..
            } => {}
            _ => panic!("Expected unary not expression"),
        }
    }

    fn parse_stmt(source: &str) -> StmtKind {
        let tokens = Scanner::new(source).scan_tokens().unwrap();
        let mut parser = Parser::new(tokens);
        let program = parser.parse().unwrap();
        program.statements.into_iter().next().unwrap().kind
    }

    #[test]
    fn test_fn_no_parens() {
        match parse_stmt("fn demo { 1 }") {
            StmtKind::Function(f) => {
                assert_eq!(f.name, "demo");
                assert!(f.params.is_empty());
            }
            other => panic!("Expected function, got {:?}", other),
        }
    }

    #[test]
    fn test_fn_empty_parens() {
        match parse_stmt("fn demo() { 1 }") {
            StmtKind::Function(f) => {
                assert_eq!(f.name, "demo");
                assert!(f.params.is_empty());
            }
            other => panic!("Expected function, got {:?}", other),
        }
    }

    #[test]
    fn test_fn_with_params() {
        match parse_stmt("fn add(a, b) { a + b }") {
            StmtKind::Function(f) => {
                assert_eq!(f.name, "add");
                assert_eq!(f.params.len(), 2);
                assert_eq!(f.params[0].name, "a");
                assert_eq!(f.params[1].name, "b");
            }
            other => panic!("Expected function, got {:?}", other),
        }
    }

    #[test]
    fn test_fn_with_end() {
        match parse_stmt("fn demo() end") {
            StmtKind::Function(f) => {
                assert_eq!(f.name, "demo");
                assert!(f.params.is_empty());
                assert!(f.body.is_empty());
            }
            other => panic!("Expected function, got {:?}", other),
        }
    }

    #[test]
    fn test_fn_with_params_and_end() {
        match parse_stmt("fn add(a, b) end") {
            StmtKind::Function(f) => {
                assert_eq!(f.name, "add");
                assert_eq!(f.params.len(), 2);
                assert!(f.body.is_empty());
            }
            other => panic!("Expected function, got {:?}", other),
        }
    }

    #[test]
    fn test_if_with_end() {
        match parse_stmt("if true end") {
            StmtKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                if let StmtKind::Block(stmts) = &then_branch.kind {
                    assert!(stmts.is_empty());
                } else {
                    panic!("Expected empty block in then branch");
                }
                assert!(else_branch.is_none());
            }
            other => panic!("Expected if, got {:?}", other),
        }
    }

    #[test]
    fn test_if_else_with_end() {
        match parse_stmt("if true end else { 1 }") {
            StmtKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                if let StmtKind::Block(stmts) = &then_branch.kind {
                    assert!(stmts.is_empty());
                } else {
                    panic!("Expected empty block in then branch");
                }
                assert!(else_branch.is_some());
            }
            other => panic!("Expected if, got {:?}", other),
        }
    }

    #[test]
    fn test_if_elsif_else_with_end() {
        match parse_stmt("if true end elsif false end else end") {
            StmtKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                if let StmtKind::Block(stmts) = &then_branch.kind {
                    assert!(stmts.is_empty());
                } else {
                    panic!("Expected empty block in then branch");
                }
                assert!(else_branch.is_some());
            }
            other => panic!("Expected if, got {:?}", other),
        }
    }

    #[test]
    fn test_while_with_end() {
        match parse_stmt("while true end") {
            StmtKind::While { body, .. } => {
                if let StmtKind::Block(stmts) = &body.kind {
                    assert!(stmts.is_empty());
                } else {
                    panic!("Expected empty block in while body");
                }
            }
            other => panic!("Expected while, got {:?}", other),
        }
    }

    #[test]
    fn test_for_with_end() {
        match parse_stmt("for x in items end") {
            StmtKind::For { body, .. } => {
                if let StmtKind::Block(stmts) = &body.kind {
                    assert!(stmts.is_empty());
                } else {
                    panic!("Expected empty block in for body");
                }
            }
            other => panic!("Expected for, got {:?}", other),
        }
    }

    // === Helpers ===

    fn parse_stmts(source: &str) -> Vec<StmtKind> {
        let tokens = Scanner::new(source).scan_tokens().unwrap();
        let mut parser = Parser::new(tokens);
        let program = parser.parse().unwrap();
        program.statements.into_iter().map(|s| s.kind).collect()
    }

    /// Extract the lambda from a let initializer
    fn parse_lambda_from_let(source: &str) -> (Vec<crate::ast::stmt::Parameter>, Vec<Stmt>) {
        let stmts = parse_stmts(source);
        match stmts.into_iter().next().unwrap() {
            StmtKind::Let { initializer, .. } => match initializer.unwrap().kind {
                ExprKind::Lambda { params, body, .. } => (params, body),
                other => panic!("Expected lambda, got {:?}", other),
            },
            other => panic!("Expected let, got {:?}", other),
        }
    }

    /// Extract function body from a single function declaration
    fn parse_fn_body(source: &str) -> Vec<Stmt> {
        match parse_stmts(source).into_iter().next().unwrap() {
            StmtKind::Function(f) => f.body,
            other => panic!("Expected function, got {:?}", other),
        }
    }

    #[allow(dead_code)]
    fn parse_should_fail(source: &str) -> bool {
        let tokens = Scanner::new(source).scan_tokens().unwrap();
        let mut parser = Parser::new(tokens);
        parser.parse().is_err()
    }

    // ================================================================
    // End-block tests: function bodies
    // ================================================================

    #[test]
    fn test_fn_end_with_body() {
        let body = parse_fn_body("fn foo()\n  1 + 2\nend");
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn test_fn_end_multiple_stmts() {
        let body = parse_fn_body("fn foo()\n  let x = 1\n  let y = 2\n  x + y\nend");
        assert_eq!(body.len(), 3);
    }

    #[test]
    fn test_fn_end_with_return_hash() {
        // Hash literal inside end-block function body
        let body = parse_fn_body("fn foo()\n  {\"status\": 200}\nend");
        assert_eq!(body.len(), 1);
        match &body[0].kind {
            StmtKind::Expression(expr) => assert!(matches!(expr.kind, ExprKind::Hash(_))),
            other => panic!("Expected hash expression, got {:?}", other),
        }
    }

    #[test]
    fn test_fn_end_return_hash_with_body() {
        // Multiple stmts + hash return
        let body = parse_fn_body("fn foo(req)\n  let x = 1\n  {\"status\": 200, \"body\": x}\nend");
        assert_eq!(body.len(), 2);
    }

    #[test]
    fn test_fn_brace_body_still_works() {
        // Ensure { } function body still works
        let body = parse_fn_body("fn foo() { 1 + 2 }");
        assert_eq!(body.len(), 1);
    }

    // ================================================================
    // End-block tests: for loops
    // ================================================================

    #[test]
    fn test_for_end_with_body() {
        let body = parse_fn_body("fn foo()\n  for x in items\n    print(x)\n  end\nend");
        assert_eq!(body.len(), 1);
        assert!(matches!(&body[0].kind, StmtKind::For { .. }));
    }

    #[test]
    fn test_for_end_then_more_stmts() {
        let body = parse_fn_body("fn foo()\n  for x in items\n    x\n  end\n  42\nend");
        assert_eq!(body.len(), 2, "for + trailing expression");
        assert!(matches!(&body[0].kind, StmtKind::For { .. }));
    }

    #[test]
    fn test_for_brace_in_end_fn() {
        // Brace-delimited for inside end-delimited function
        let body = parse_fn_body("fn foo()\n  for x in items { print(x) }\n  42\nend");
        assert_eq!(body.len(), 2);
        assert!(matches!(&body[0].kind, StmtKind::For { .. }));
    }

    #[test]
    fn test_nested_for_end() {
        let body = parse_fn_body(
            "fn foo()\n  for x in xs\n    for y in ys\n      x + y\n    end\n  end\nend",
        );
        assert_eq!(body.len(), 1);
        match &body[0].kind {
            StmtKind::For { body, .. } => match &body.kind {
                StmtKind::Block(stmts) => {
                    assert_eq!(stmts.len(), 1);
                    assert!(matches!(&stmts[0].kind, StmtKind::For { .. }));
                }
                other => panic!("Expected block, got {:?}", other),
            },
            other => panic!("Expected for, got {:?}", other),
        }
    }

    #[test]
    fn test_for_end_with_hash_body() {
        // Hash literal in for loop body shouldn't confuse block detection
        let body = parse_fn_body("fn foo()\n  for x in items\n    {\"key\": x}\n  end\nend");
        assert_eq!(body.len(), 1);
    }

    // ================================================================
    // End-block tests: while loops
    // ================================================================

    #[test]
    fn test_while_end_with_body() {
        let body = parse_fn_body("fn foo()\n  while true\n    1\n  end\nend");
        assert_eq!(body.len(), 1);
        assert!(matches!(&body[0].kind, StmtKind::While { .. }));
    }

    #[test]
    fn test_while_end_then_more_stmts() {
        let body = parse_fn_body("fn foo()\n  while true\n    1\n  end\n  42\nend");
        assert_eq!(body.len(), 2);
        assert!(matches!(&body[0].kind, StmtKind::While { .. }));
    }

    #[test]
    fn test_while_brace_in_end_fn() {
        let body = parse_fn_body("fn foo()\n  while true { break }\n  42\nend");
        assert_eq!(body.len(), 2);
    }

    // ================================================================
    // End-block tests: if/else/elsif
    // ================================================================

    #[test]
    fn test_if_end_with_body() {
        let body = parse_fn_body("fn foo()\n  if true\n    1\n  end\nend");
        assert_eq!(body.len(), 1);
        assert!(matches!(&body[0].kind, StmtKind::If { .. }));
    }

    #[test]
    fn test_if_end_then_more_stmts() {
        let body = parse_fn_body("fn foo()\n  if true\n    1\n  end\n  42\nend");
        assert_eq!(body.len(), 2);
    }

    #[test]
    fn test_if_else_multiline_end() {
        // Multi-line else consumes its own end
        let body = parse_fn_body("fn foo()\n  if true\n    1\n  else\n    2\n  end\n  42\nend");
        assert_eq!(body.len(), 2, "if/else + trailing expression");
        match &body[0].kind {
            StmtKind::If { else_branch, .. } => assert!(else_branch.is_some()),
            other => panic!("Expected if, got {:?}", other),
        }
    }

    #[test]
    fn test_if_elsif_multiline_end() {
        let body = parse_fn_body(
            "fn foo(x)\n  if x == 1\n    \"a\"\n  elsif x == 2\n    \"b\"\n  end\n  42\nend",
        );
        assert_eq!(body.len(), 2);
    }

    #[test]
    fn test_if_elsif_else_multiline_end() {
        let body = parse_fn_body(
            "fn foo(x)\n  if x == 1\n    \"a\"\n  elsif x == 2\n    \"b\"\n  else\n    \"c\"\n  end\n  42\nend",
        );
        assert_eq!(body.len(), 2);
        match &body[0].kind {
            StmtKind::If { else_branch, .. } => assert!(else_branch.is_some()),
            other => panic!("Expected if, got {:?}", other),
        }
    }

    #[test]
    fn test_oneliner_if_consumes_end() {
        // Standalone if (no elsif/else) always consumes the next end
        // So the function also needs its own end
        let body = parse_fn_body("fn foo(x)\n  if x == 1 return \"a\";\n  end\nend");
        assert_eq!(body.len(), 1);
        assert!(matches!(&body[0].kind, StmtKind::If { .. }));
    }

    #[test]
    fn test_oneliner_if_elsif_else_no_end() {
        // One-liner branches — single end closes the function
        let body = parse_fn_body(
            "fn foo(x)\n  if x == 1 return \"a\";\n  elsif x == 2 return \"b\";\n  else return \"c\";\nend",
        );
        assert_eq!(body.len(), 1, "function should have one if statement");
        match &body[0].kind {
            StmtKind::If { else_branch, .. } => {
                assert!(else_branch.is_some(), "should have else/elsif chain");
            }
            other => panic!("Expected if, got {:?}", other),
        }
    }

    #[test]
    fn test_oneliner_if_with_semicolons_end() {
        // One-liner if with explicit end: if (cond) stmt; stmt end
        match parse_stmt("if (x >= 16) h4 = 1; x = x - 16 end") {
            StmtKind::If { then_branch, .. } => match &then_branch.kind {
                StmtKind::Block(stmts) => assert_eq!(stmts.len(), 2),
                other => panic!("Expected block, got {:?}", other),
            },
            other => panic!("Expected if, got {:?}", other),
        }
    }

    #[test]
    fn test_nested_if_else_end_both_levels() {
        // Nested if/else each with their own end
        let body = parse_fn_body(
            "fn foo()\n  if true\n    if false\n      1\n    else\n      2\n    end\n  end\n  42\nend",
        );
        assert_eq!(body.len(), 2, "function: outer_if + 42");
        // Inner if should have else branch
        match &body[0].kind {
            StmtKind::If { then_branch, .. } => match &then_branch.kind {
                StmtKind::Block(stmts) => {
                    assert_eq!(stmts.len(), 1);
                    match &stmts[0].kind {
                        StmtKind::If { else_branch, .. } => {
                            assert!(else_branch.is_some(), "inner if should have else");
                        }
                        other => panic!("Expected inner if, got {:?}", other),
                    }
                }
                other => panic!("Expected block, got {:?}", other),
            },
            other => panic!("Expected outer if, got {:?}", other),
        }
    }

    #[test]
    fn test_if_with_return_hash_end() {
        // if body returns a hash, then end closes the if
        let body = parse_fn_body(
            "fn foo(data)\n  if data == null\n    return {\"error\": \"missing\"}\n  end\n  42\nend",
        );
        assert_eq!(body.len(), 2, "if + trailing expression");
    }

    // ================================================================
    // End-block tests: mixed brace and end styles
    // ================================================================

    #[test]
    fn test_brace_if_in_end_fn() {
        let body = parse_fn_body("fn foo()\n  if true { 1 }\n  42\nend");
        assert_eq!(body.len(), 2);
    }

    #[test]
    fn test_brace_if_else_in_end_fn() {
        let body = parse_fn_body("fn foo()\n  if true { 1 } else { 2 }\n  42\nend");
        assert_eq!(body.len(), 2);
    }

    #[test]
    fn test_mixed_for_brace_if_end() {
        // for uses braces, function uses end
        let body = parse_fn_body(
            "fn foo()\n  for x in items {\n    if x > 0\n      print(x)\n    end\n  }\n  42\nend",
        );
        assert_eq!(body.len(), 2);
    }

    // ================================================================
    // End-block tests: class bodies
    // ================================================================

    #[test]
    fn test_class_with_end() {
        match parse_stmt("class Foo end") {
            StmtKind::Class(c) => {
                assert_eq!(c.name, "Foo");
                assert!(c.methods.is_empty());
                assert!(c.fields.is_empty());
            }
            other => panic!("Expected class, got {:?}", other),
        }
    }

    #[test]
    fn test_class_with_method_end() {
        let stmts = parse_stmts("class Foo\n  fn bar()\n    42\n  end\nend");
        match &stmts[0] {
            StmtKind::Class(c) => {
                assert_eq!(c.name, "Foo");
                assert_eq!(c.methods.len(), 1);
                assert_eq!(c.methods[0].name, "bar");
            }
            other => panic!("Expected class, got {:?}", other),
        }
    }

    #[test]
    fn test_class_less_than_extends() {
        match parse_stmt("class Child < Parent { }") {
            StmtKind::Class(c) => {
                assert_eq!(c.name, "Child");
                assert_eq!(c.superclass, Some("Parent".to_string()));
            }
            other => panic!("Expected class, got {:?}", other),
        }
    }

    #[test]
    fn test_class_method_named_new() {
        let stmts = parse_stmts("class Foo\n  def new(req)\n    42\n  end\nend");
        match &stmts[0] {
            StmtKind::Class(c) => {
                assert_eq!(c.name, "Foo");
                assert_eq!(c.methods.len(), 1);
                assert_eq!(c.methods[0].name, "new");
            }
            other => panic!("Expected class, got {:?}", other),
        }
    }

    #[test]
    fn test_class_method_named_new_with_body() {
        let stmts = parse_stmts(
            "class MyController\n  def new(req)\n    return 1\n  end\n  def index(req)\n    return 2\n  end\nend",
        );
        match &stmts[0] {
            StmtKind::Class(c) => {
                assert_eq!(c.name, "MyController");
                assert_eq!(c.methods.len(), 2);
                assert_eq!(c.methods[0].name, "new");
                assert_eq!(c.methods[1].name, "index");
            }
            other => panic!("Expected class, got {:?}", other),
        }
    }

    #[test]
    fn test_class_method_named_new_brace_style() {
        let stmts = parse_stmts("class Foo { fn new(x) { return x } }");
        match &stmts[0] {
            StmtKind::Class(c) => {
                assert_eq!(c.name, "Foo");
                assert_eq!(c.methods.len(), 1);
                assert_eq!(c.methods[0].name, "new");
                assert_eq!(c.methods[0].params.len(), 1);
            }
            other => panic!("Expected class, got {:?}", other),
        }
    }

    #[test]
    fn test_class_method_named_match() {
        let stmts = parse_stmts("class Foo { fn match(pattern) { return true } }");
        match &stmts[0] {
            StmtKind::Class(c) => {
                assert_eq!(c.name, "Foo");
                assert_eq!(c.methods.len(), 1);
                assert_eq!(c.methods[0].name, "match");
            }
            other => panic!("Expected class, got {:?}", other),
        }
    }

    #[test]
    fn test_match_as_method_call() {
        // name.match("pattern") should parse as a method call, not a match expression
        let expr = parse_expr("name.match(\"^[a-z]+$\")");
        match &expr.kind {
            ExprKind::Call { callee, arguments, .. } => {
                assert_eq!(arguments.len(), 1);
                match &callee.kind {
                    ExprKind::Member { name, .. } => {
                        assert_eq!(name, "match");
                    }
                    other => panic!("Expected member access, got {:?}", other),
                }
            }
            other => panic!("Expected call, got {:?}", other),
        }
    }

    // ================================================================
    // Inline lambda tests: fn(params) expr
    // ================================================================

    #[test]
    fn test_inline_lambda_in_call() {
        let expr = parse_expr("items.find(fn(x) x == 5)");
        match &expr.kind {
            ExprKind::Call { arguments, .. } => {
                assert_eq!(arguments.len(), 1);
                match &arguments[0] {
                    Argument::Positional(arg_expr) => {
                        let (params, body) = match &arg_expr.kind {
                            ExprKind::Lambda { params, body, .. } => (params, body),
                            other => panic!("Expected lambda, got {:?}", other),
                        };
                        assert_eq!(params.len(), 1);
                        assert_eq!(params[0].name, "x");
                        assert_eq!(body.len(), 1);
                    }
                    other => panic!("Expected positional arg, got {:?}", other),
                }
            }
            other => panic!("Expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_inline_lambda_multiple_args() {
        // Lambda as one of several arguments
        let expr = parse_expr("items.map(fn(x) x * 2, 10)");
        match &expr.kind {
            ExprKind::Call { arguments, .. } => {
                assert_eq!(arguments.len(), 2);
                match &arguments[0] {
                    Argument::Positional(arg) => {
                        assert!(matches!(&arg.kind, ExprKind::Lambda { .. }));
                    }
                    other => panic!("Expected positional, got {:?}", other),
                }
            }
            other => panic!("Expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_inline_lambda_chained_calls() {
        // items.map(fn(x) x * 2).filter(fn(y) y > 5)
        let expr = parse_expr("items.map(fn(x) x * 2).filter(fn(y) y > 5)");
        // Should parse as chained method calls
        match &expr.kind {
            ExprKind::Call {
                callee, arguments, ..
            } => {
                // Outer call is .filter(fn(y) y > 5)
                assert_eq!(arguments.len(), 1);
                match &arguments[0] {
                    Argument::Positional(arg) => match &arg.kind {
                        ExprKind::Lambda { params, .. } => {
                            assert_eq!(params[0].name, "y");
                        }
                        other => panic!("Expected lambda in filter, got {:?}", other),
                    },
                    other => panic!("Expected positional, got {:?}", other),
                }
                // callee should be items.map(fn(x) x * 2).filter
                match &callee.kind {
                    ExprKind::Member { name, .. } => assert_eq!(name, "filter"),
                    other => panic!("Expected member access, got {:?}", other),
                }
            }
            other => panic!("Expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_inline_lambda_in_array() {
        let expr = parse_expr("[fn(x) x + 1, fn(y) y * 2]");
        match &expr.kind {
            ExprKind::Array(elements) => {
                assert_eq!(elements.len(), 2);
                assert!(matches!(&elements[0].kind, ExprKind::Lambda { .. }));
                assert!(matches!(&elements[1].kind, ExprKind::Lambda { .. }));
            }
            other => panic!("Expected array, got {:?}", other),
        }
    }

    #[test]
    fn test_inline_lambda_nested_call() {
        // Lambda inside a nested function call
        let expr = parse_expr("outer(inner(fn(x) x))");
        match &expr.kind {
            ExprKind::Call { arguments, .. } => {
                assert_eq!(arguments.len(), 1);
                match &arguments[0] {
                    Argument::Positional(inner_call) => match &inner_call.kind {
                        ExprKind::Call {
                            arguments: inner_args,
                            ..
                        } => {
                            assert_eq!(inner_args.len(), 1);
                            match &inner_args[0] {
                                Argument::Positional(arg) => {
                                    assert!(matches!(&arg.kind, ExprKind::Lambda { .. }));
                                }
                                other => panic!("Expected positional, got {:?}", other),
                            }
                        }
                        other => panic!("Expected inner call, got {:?}", other),
                    },
                    other => panic!("Expected positional, got {:?}", other),
                }
            }
            other => panic!("Expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_inline_lambda_with_comparison() {
        // Lambda body is a comparison expression
        let expr = parse_expr("list.find(fn(item) item == target)");
        match &expr.kind {
            ExprKind::Call { arguments, .. } => match &arguments[0] {
                Argument::Positional(arg) => match &arg.kind {
                    ExprKind::Lambda { body, .. } => {
                        assert_eq!(body.len(), 1);
                        match &body[0].kind {
                            StmtKind::Expression(e) => {
                                assert!(matches!(
                                    &e.kind,
                                    ExprKind::Binary {
                                        operator: BinaryOp::Equal,
                                        ..
                                    }
                                ));
                            }
                            other => panic!("Expected expression, got {:?}", other),
                        }
                    }
                    other => panic!("Expected lambda, got {:?}", other),
                },
                other => panic!("Expected positional, got {:?}", other),
            },
            other => panic!("Expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_inline_lambda_with_arithmetic() {
        let expr = parse_expr("items.map(fn(x) x * 2 + 1)");
        match &expr.kind {
            ExprKind::Call { arguments, .. } => match &arguments[0] {
                Argument::Positional(arg) => match &arg.kind {
                    ExprKind::Lambda { body, .. } => {
                        assert_eq!(body.len(), 1);
                    }
                    other => panic!("Expected lambda, got {:?}", other),
                },
                other => panic!("Expected positional, got {:?}", other),
            },
            other => panic!("Expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_inline_lambda_multiparams() {
        let expr = parse_expr("items.reduce(fn(acc, x) acc + x)");
        match &expr.kind {
            ExprKind::Call { arguments, .. } => match &arguments[0] {
                Argument::Positional(arg) => match &arg.kind {
                    ExprKind::Lambda { params, .. } => {
                        assert_eq!(params.len(), 2);
                        assert_eq!(params[0].name, "acc");
                        assert_eq!(params[1].name, "x");
                    }
                    other => panic!("Expected lambda, got {:?}", other),
                },
                other => panic!("Expected positional, got {:?}", other),
            },
            other => panic!("Expected call, got {:?}", other),
        }
    }

    // ================================================================
    // Lambda with brace body
    // ================================================================

    #[test]
    fn test_lambda_with_brace_body() {
        let (params, body) = parse_lambda_from_let("let f = fn(x) { x + 1 }");
        assert_eq!(params.len(), 1);
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn test_lambda_brace_multi_stmt() {
        let (_, body) = parse_lambda_from_let("let f = fn(x) { let y = x + 1; y * 2 }");
        assert_eq!(body.len(), 2);
    }

    // ================================================================
    // Lambda with end body
    // ================================================================

    #[test]
    fn test_lambda_with_end_body() {
        let (_, body) = parse_lambda_from_let("let f = fn(x)\n  let y = x + 1\n  y\nend");
        assert_eq!(body.len(), 2);
    }

    #[test]
    fn test_lambda_end_empty() {
        let (_, body) = parse_lambda_from_let("let f = fn(x) end");
        assert_eq!(body.len(), 0);
    }

    #[test]
    fn test_lambda_end_single_expr() {
        let (_, body) = parse_lambda_from_let("let f = fn(x)\n  x + 1\nend");
        assert_eq!(body.len(), 1);
    }

    // ================================================================
    // Pipe-style lambdas: |x| expr
    // ================================================================

    #[test]
    fn test_pipe_lambda_inline_in_call() {
        let expr = parse_expr("items.map(|x| x + 1)");
        match &expr.kind {
            ExprKind::Call { arguments, .. } => match &arguments[0] {
                Argument::Positional(arg) => match &arg.kind {
                    ExprKind::Lambda { params, body, .. } => {
                        assert_eq!(params.len(), 1);
                        assert_eq!(params[0].name, "x");
                        assert_eq!(body.len(), 1);
                    }
                    other => panic!("Expected lambda, got {:?}", other),
                },
                other => panic!("Expected positional, got {:?}", other),
            },
            other => panic!("Expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_pipe_lambda_in_array() {
        let expr = parse_expr("[|x| x, |y| y]");
        match &expr.kind {
            ExprKind::Array(elements) => {
                assert_eq!(elements.len(), 2);
                assert!(matches!(&elements[0].kind, ExprKind::Lambda { .. }));
                assert!(matches!(&elements[1].kind, ExprKind::Lambda { .. }));
            }
            other => panic!("Expected array, got {:?}", other),
        }
    }

    #[test]
    fn test_pipe_lambda_multi_params() {
        let expr = parse_expr("items.reduce(|acc, x| acc + x)");
        match &expr.kind {
            ExprKind::Call { arguments, .. } => match &arguments[0] {
                Argument::Positional(arg) => match &arg.kind {
                    ExprKind::Lambda { params, .. } => {
                        assert_eq!(params.len(), 2);
                    }
                    other => panic!("Expected lambda, got {:?}", other),
                },
                other => panic!("Expected positional, got {:?}", other),
            },
            other => panic!("Expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_empty_pipe_lambda_inline() {
        let expr = parse_expr("run(|| 42)");
        match &expr.kind {
            ExprKind::Call { arguments, .. } => match &arguments[0] {
                Argument::Positional(arg) => match &arg.kind {
                    ExprKind::Lambda { params, body, .. } => {
                        assert_eq!(params.len(), 0);
                        assert_eq!(body.len(), 1);
                    }
                    other => panic!("Expected lambda, got {:?}", other),
                },
                other => panic!("Expected positional, got {:?}", other),
            },
            other => panic!("Expected call, got {:?}", other),
        }
    }

    // ================================================================
    // Hash literal vs block disambiguation
    // ================================================================

    #[test]
    fn test_hash_literal_string_key() {
        let expr = parse_expr("{\"key\": 1}");
        assert!(matches!(expr.kind, ExprKind::Hash(_)));
    }

    #[test]
    fn test_hash_literal_ident_key() {
        let expr = parse_expr("{name: \"value\"}");
        assert!(matches!(expr.kind, ExprKind::Hash(_)));
    }

    #[test]
    fn test_hash_literal_int_key() {
        let expr = parse_expr("{42: \"answer\"}");
        assert!(matches!(expr.kind, ExprKind::Hash(_)));
    }

    #[test]
    fn test_empty_hash_literal() {
        let expr = parse_expr("{}");
        assert!(matches!(expr.kind, ExprKind::Hash(_)));
    }

    #[test]
    fn test_brace_block_with_int() {
        // { 1 } is a block, not a hash (no colon after 1)
        match parse_stmt("fn foo() { 1 }") {
            StmtKind::Function(f) => {
                assert_eq!(f.body.len(), 1);
            }
            other => panic!("Expected function, got {:?}", other),
        }
    }

    #[test]
    fn test_brace_block_with_string() {
        // { "hello" } is a block with a string expression, not a hash
        match parse_stmt("fn foo() { \"hello\" }") {
            StmtKind::Function(f) => {
                assert_eq!(f.body.len(), 1);
            }
            other => panic!("Expected function, got {:?}", other),
        }
    }

    // ================================================================
    // Complex real-world patterns
    // ================================================================

    #[test]
    fn test_controller_style_function() {
        // Real-world pattern: function returns a hash after some logic
        let body = parse_fn_body(
            "fn create(req)\n  let data = req\n  if data == null\n    return {\"status\": 422}\n  end\n  {\"status\": 201}\nend",
        );
        assert_eq!(body.len(), 3, "let + if + hash");
    }

    #[test]
    fn test_for_with_hash_accumulation() {
        // Pattern from state_machines_controller: for + hash in body
        let body = parse_fn_body(
            "fn list()\n  let result = []\n  for entry in entries\n    let id = entry\n  end\n  {\"status\": 200, \"data\": result}\nend",
        );
        assert_eq!(body.len(), 3, "let + for + hash");
    }

    #[test]
    fn test_multiple_if_guards_with_return_hash() {
        // Multiple if guards each returning a hash
        let body = parse_fn_body(
            "fn validate(data)\n  if data == null\n    return {\"error\": \"missing\"}\n  end\n  if data == 0\n    return {\"error\": \"zero\"}\n  end\n  {\"ok\": true}\nend",
        );
        assert_eq!(body.len(), 3, "if + if + hash");
    }

    #[test]
    fn test_binary_clock_pattern() {
        // Pattern from live_controller: nested if/else for bit extraction
        let body = parse_fn_body(
            "fn bits(n)\n  let b1 = 0\n  let b0 = 0\n  if (n >= 2)\n    b1 = 1\n  end\n  if (n >= 1)\n    b0 = 1\n  end\n  {\"b1\": b1, \"b0\": b0}\nend",
        );
        assert_eq!(body.len(), 5, "let + let + if + if + hash");
    }

    #[test]
    fn test_section_color_pattern() {
        // Pattern from html.sl: if/elsif/else one-liners
        let body = parse_fn_body(
            "fn color(id)\n  if id == \"a\" return \"red\";\n  elsif id == \"b\" return \"blue\";\n  else return \"gray\";\nend",
        );
        assert_eq!(body.len(), 1, "single if/elsif/else chain");
    }

    #[test]
    fn test_nested_if_else_with_string_formatting() {
        // Nested if/else for string formatting (live_controller pattern)
        let body = parse_fn_body(
            "fn fmt(n)\n  let s = \"\" + n\n  if (n < 100)\n    if (n < 10)\n      s = \"00\" + n\n    else\n      s = \"0\" + n\n    end\n  end\n  s\nend",
        );
        assert_eq!(body.len(), 3, "let + if(nested) + s");
    }

    // ================================================================
    // Multi-line lambdas as method arguments
    // ================================================================

    #[test]
    fn test_pipe_lambda_end_body_in_call() {
        // items.map(|x|\n  x * 2\nend)
        let expr = parse_expr("items.map(|x|\n  let y = x * 2\n  y\nend)");
        match &expr.kind {
            ExprKind::Call { arguments, .. } => match &arguments[0] {
                Argument::Positional(arg) => match &arg.kind {
                    ExprKind::Lambda { params, body, .. } => {
                        assert_eq!(params.len(), 1);
                        assert_eq!(params[0].name, "x");
                        assert_eq!(body.len(), 2, "let + y");
                    }
                    other => panic!("Expected lambda, got {:?}", other),
                },
                other => panic!("Expected positional, got {:?}", other),
            },
            other => panic!("Expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_fn_lambda_end_body_in_call() {
        // items.filter(fn(u)\n  u > 0\nend)
        let expr = parse_expr("items.filter(fn(u)\n  let age = u\n  age >= 18\nend)");
        match &expr.kind {
            ExprKind::Call { arguments, .. } => match &arguments[0] {
                Argument::Positional(arg) => match &arg.kind {
                    ExprKind::Lambda { params, body, .. } => {
                        assert_eq!(params.len(), 1);
                        assert_eq!(params[0].name, "u");
                        assert_eq!(body.len(), 2, "let + comparison");
                    }
                    other => panic!("Expected lambda, got {:?}", other),
                },
                other => panic!("Expected positional, got {:?}", other),
            },
            other => panic!("Expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_pipe_lambda_brace_body_in_call() {
        // items.map(|x| { x * 2 })
        let expr = parse_expr("items.map(|x| { x * 2 })");
        match &expr.kind {
            ExprKind::Call { arguments, .. } => match &arguments[0] {
                Argument::Positional(arg) => match &arg.kind {
                    ExprKind::Lambda { params, body, .. } => {
                        assert_eq!(params.len(), 1);
                        assert_eq!(params[0].name, "x");
                        assert_eq!(body.len(), 1);
                    }
                    other => panic!("Expected lambda, got {:?}", other),
                },
                other => panic!("Expected positional, got {:?}", other),
            },
            other => panic!("Expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_pipe_lambda_end_body_chained() {
        // Chained method calls with end-body lambdas
        let expr = parse_expr("items.filter(|x|\n  x > 0\nend).map(|x|\n  x * 2\nend)");
        match &expr.kind {
            ExprKind::Call {
                callee, arguments, ..
            } => {
                // Outer call is .map(...)
                assert_eq!(arguments.len(), 1);
                match &arguments[0] {
                    Argument::Positional(arg) => {
                        assert!(matches!(&arg.kind, ExprKind::Lambda { .. }));
                    }
                    other => panic!("Expected positional, got {:?}", other),
                }
                // callee should be .map on the result of .filter(...)
                match &callee.kind {
                    ExprKind::Member { name, .. } => assert_eq!(name, "map"),
                    other => panic!("Expected member access, got {:?}", other),
                }
            }
            other => panic!("Expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_empty_pipe_lambda_end_body_in_call() {
        // run(||\n  42\nend)
        let expr = parse_expr("run(||\n  42\nend)");
        match &expr.kind {
            ExprKind::Call { arguments, .. } => match &arguments[0] {
                Argument::Positional(arg) => match &arg.kind {
                    ExprKind::Lambda { params, body, .. } => {
                        assert_eq!(params.len(), 0);
                        assert_eq!(body.len(), 1);
                    }
                    other => panic!("Expected lambda, got {:?}", other),
                },
                other => panic!("Expected positional, got {:?}", other),
            },
            other => panic!("Expected call, got {:?}", other),
        }
    }

    #[test]
    fn test_not_keyword_equals_bang() {
        let bang_expr = parse_expr("!true;");
        let not_expr = parse_expr("not true;");

        match (&bang_expr.kind, &not_expr.kind) {
            (
                ExprKind::Unary {
                    operator: bang_op, ..
                },
                ExprKind::Unary {
                    operator: not_op, ..
                },
            ) => {
                assert_eq!(bang_op, not_op);
            }
            _ => panic!("Expected both to be unary expressions"),
        }
    }

    // ── Safe navigation operator (&.) ──

    #[test]
    fn test_safe_navigation_member() {
        let expr = parse_expr("user&.name;");
        match expr.kind {
            ExprKind::SafeMember { object, name } => {
                assert_eq!(name, "name");
                assert!(matches!(object.kind, ExprKind::Variable(ref v) if v == "user"));
            }
            _ => panic!("Expected SafeMember, got {:?}", expr.kind),
        }
    }

    #[test]
    fn test_safe_navigation_method_call() {
        let expr = parse_expr("user&.greet();");
        match expr.kind {
            ExprKind::Call { callee, arguments } => {
                assert!(arguments.is_empty());
                match callee.kind {
                    ExprKind::SafeMember { object, name } => {
                        assert_eq!(name, "greet");
                        assert!(matches!(object.kind, ExprKind::Variable(ref v) if v == "user"));
                    }
                    _ => panic!("Expected SafeMember callee, got {:?}", callee.kind),
                }
            }
            _ => panic!("Expected Call, got {:?}", expr.kind),
        }
    }

    #[test]
    fn test_safe_navigation_chained() {
        let expr = parse_expr("user&.address&.city;");
        match expr.kind {
            ExprKind::SafeMember { object, name } => {
                assert_eq!(name, "city");
                match object.kind {
                    ExprKind::SafeMember {
                        object: inner,
                        name: inner_name,
                    } => {
                        assert_eq!(inner_name, "address");
                        assert!(matches!(inner.kind, ExprKind::Variable(ref v) if v == "user"));
                    }
                    _ => panic!("Expected inner SafeMember"),
                }
            }
            _ => panic!("Expected SafeMember, got {:?}", expr.kind),
        }
    }

    #[test]
    fn test_safe_navigation_with_nullish_coalescing() {
        let expr = parse_expr("user&.name ?? \"default\";");
        match expr.kind {
            ExprKind::NullishCoalescing { left, right } => {
                assert!(matches!(left.kind, ExprKind::SafeMember { .. }));
                assert!(matches!(right.kind, ExprKind::StringLiteral(ref s) if s == "default"));
            }
            _ => panic!("Expected NullishCoalescing, got {:?}", expr.kind),
        }
    }
}
