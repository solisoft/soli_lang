//! Parser tests.

#[cfg(test)]
mod tests {
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
}
