//! Inline lint-suppression directives.
//!
//! Recognised forms (Soli uses `#` for line comments):
//!
//! ```text
//! # soli-lint-disable-next-line                              # all rules on next line
//! # soli-lint-disable-next-line smell/dangerous-server-builtin
//! # soli-lint-disable-next-line rule1, rule2
//! foo()  # soli-lint-disable-line                            # all rules on this line
//! foo()  # soli-lint-disable-line smell/dangerous-server-builtin
//! ```
//!
//! Directives are matched by scanning the raw source for the `#` marker and
//! then for the literal directive keyword. This is a textual scan, so a
//! directive string embedded inside a string literal would match too — but
//! that combination is vanishingly rare in practice.

use std::collections::HashMap;

/// Per-line suppression set: `None` = suppress every rule on this line;
/// `Some(rules)` = suppress only the listed rules.
#[derive(Debug, Default, Clone)]
pub struct LineSuppression {
    pub all: bool,
    pub rules: Vec<String>,
}

impl LineSuppression {
    fn merge(&mut self, other: LineSuppression) {
        if other.all {
            self.all = true;
        }
        self.rules.extend(other.rules);
    }

    pub fn suppresses(&self, rule: &str) -> bool {
        self.all || self.rules.iter().any(|r| r == rule)
    }
}

const NEXT_LINE_DIRECTIVE: &str = "soli-lint-disable-next-line";
const SAME_LINE_DIRECTIVE: &str = "soli-lint-disable-line";

/// Scan the source and build a line-indexed map of suppressions. Line
/// numbers are 1-based, matching `Span::line`.
pub fn collect_suppressions(source: &str) -> HashMap<u32, LineSuppression> {
    let mut map: HashMap<u32, LineSuppression> = HashMap::new();
    for (idx, line) in source.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let Some(hash_pos) = line.find('#') else {
            continue;
        };
        let after_hash = line[hash_pos + 1..].trim_start();
        if let Some(rest) = after_hash.strip_prefix(NEXT_LINE_DIRECTIVE) {
            let suppression = parse_rule_list(rest);
            map.entry(line_no + 1).or_default().merge(suppression);
        } else if let Some(rest) = after_hash.strip_prefix(SAME_LINE_DIRECTIVE) {
            let suppression = parse_rule_list(rest);
            map.entry(line_no).or_default().merge(suppression);
        }
    }
    map
}

fn parse_rule_list(rest: &str) -> LineSuppression {
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        return LineSuppression {
            all: true,
            rules: Vec::new(),
        };
    }
    let rules: Vec<String> = trimmed
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if rules.is_empty() {
        LineSuppression {
            all: true,
            rules: Vec::new(),
        }
    } else {
        LineSuppression { all: false, rules }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disable_next_line_all_rules() {
        let src = "# soli-lint-disable-next-line\nfoo()\n";
        let map = collect_suppressions(src);
        assert!(map.get(&2).unwrap().suppresses("any/rule"));
    }

    #[test]
    fn disable_next_line_specific_rule() {
        let src = "# soli-lint-disable-next-line smell/dangerous-server-builtin\nfoo()\n";
        let map = collect_suppressions(src);
        let entry = map.get(&2).unwrap();
        assert!(entry.suppresses("smell/dangerous-server-builtin"));
        assert!(!entry.suppresses("other/rule"));
    }

    #[test]
    fn disable_next_line_multiple_rules() {
        let src = "# soli-lint-disable-next-line rule1, rule2\nfoo()\n";
        let map = collect_suppressions(src);
        let entry = map.get(&2).unwrap();
        assert!(entry.suppresses("rule1"));
        assert!(entry.suppresses("rule2"));
        assert!(!entry.suppresses("rule3"));
    }

    #[test]
    fn disable_same_line() {
        let src = "foo()  # soli-lint-disable-line smell/dangerous-server-builtin\n";
        let map = collect_suppressions(src);
        assert!(map
            .get(&1)
            .unwrap()
            .suppresses("smell/dangerous-server-builtin"));
    }

    #[test]
    fn unrelated_comment_is_ignored() {
        let src = "# just a normal comment\nfoo()\n";
        let map = collect_suppressions(src);
        assert!(map.is_empty());
    }

    #[test]
    fn directive_with_leading_indent() {
        let src = "    # soli-lint-disable-next-line\n    foo()\n";
        let map = collect_suppressions(src);
        assert!(map.get(&2).unwrap().all);
    }
}
