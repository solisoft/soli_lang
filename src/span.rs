//! Source location tracking for error reporting.

/// A span represents a range in the source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
}

impl Span {
    pub fn new(start: usize, end: usize, line: usize, column: usize) -> Self {
        Self {
            start,
            end,
            line,
            column,
        }
    }

    /// Create a span that covers from this span to another.
    pub fn merge(&self, other: &Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            line: self.line.min(other.line),
            column: if self.line <= other.line {
                self.column
            } else {
                other.column
            },
        }
    }
}

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// A wrapper that associates a value with a source span.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(node: T, span: Span) -> Self {
        Self { node, span }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_zeroed() {
        let s = Span::default();
        assert_eq!(s.start, 0);
        assert_eq!(s.end, 0);
        assert_eq!(s.line, 0);
        assert_eq!(s.column, 0);
    }

    #[test]
    fn display_is_line_colon_column() {
        assert_eq!(Span::new(0, 5, 3, 7).to_string(), "3:7");
        assert_eq!(Span::new(0, 0, 1, 1).to_string(), "1:1");
    }

    // ---------- merge ----------

    #[test]
    fn merge_takes_min_start_and_max_end() {
        // Self comes first in source: a covers 5..10, b covers 20..30 → 5..30.
        let a = Span::new(5, 10, 1, 3);
        let b = Span::new(20, 30, 1, 18);
        let m = a.merge(&b);
        assert_eq!(m.start, 5);
        assert_eq!(m.end, 30);
    }

    #[test]
    fn merge_is_commutative_for_start_and_end() {
        let a = Span::new(5, 10, 1, 3);
        let b = Span::new(20, 30, 1, 18);
        assert_eq!(a.merge(&b).start, b.merge(&a).start);
        assert_eq!(a.merge(&b).end, b.merge(&a).end);
    }

    #[test]
    fn merge_picks_smaller_line() {
        let a = Span::new(0, 1, 5, 10);
        let b = Span::new(2, 3, 2, 4);
        assert_eq!(a.merge(&b).line, 2);
        assert_eq!(b.merge(&a).line, 2);
    }

    #[test]
    fn merge_column_uses_self_when_self_line_le_other_line() {
        // self.line < other.line — keep self.column.
        let a = Span::new(0, 1, 1, 5);
        let b = Span::new(10, 20, 3, 99);
        assert_eq!(a.merge(&b).column, 5);

        // self.line == other.line — branch is `<=`, so still self.column.
        let c = Span::new(0, 1, 2, 7);
        let d = Span::new(10, 20, 2, 88);
        assert_eq!(c.merge(&d).column, 7);
    }

    #[test]
    fn merge_column_uses_other_when_self_line_gt_other_line() {
        let a = Span::new(0, 1, 5, 50);
        let b = Span::new(10, 20, 2, 4);
        assert_eq!(a.merge(&b).column, 4);
    }

    #[test]
    fn merge_with_self_is_idempotent() {
        let s = Span::new(7, 9, 4, 11);
        assert_eq!(s.merge(&s), s);
    }

    // ---------- Spanned<T> ----------

    #[test]
    fn spanned_holds_node_and_span() {
        let span = Span::new(0, 4, 1, 1);
        let s = Spanned::new(42_i32, span);
        assert_eq!(s.node, 42);
        assert_eq!(s.span, span);
    }
}
