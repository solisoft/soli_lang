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
//!
//! # soli-lint-disable smell/dangerous-server-builtin         # block: from here
//! Trusted.read(p)
//! Trusted.write(q)
//! # soli-lint-enable smell/dangerous-server-builtin          # block: back on
//! ```
//!
//! `disable` / `enable` are block directives that take effect from the
//! comment line onward (inclusive). With no rule list they affect every
//! rule. An `enable` for a specific rule undoes a prior `disable all` for
//! that rule, matching rubocop semantics.
//!
//! Directives are matched by scanning the raw source for the `#` marker and
//! then for the literal directive keyword. This is a textual scan, so a
//! directive string embedded inside a string literal would match too — but
//! that combination is vanishingly rare in practice.

use std::collections::HashMap;

/// Per-line suppression set populated by `disable-line` and
/// `disable-next-line`. `all = true` = suppress every rule on this line.
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

#[derive(Debug, Clone)]
enum Target {
    All,
    Specific(Vec<String>),
}

impl Target {
    fn matches(&self, rule: &str) -> bool {
        match self {
            Target::All => true,
            Target::Specific(rules) => rules.iter().any(|r| r == rule),
        }
    }
}

#[derive(Debug, Clone)]
struct BlockDirective {
    line: u32,
    enable: bool,
    target: Target,
}

/// Aggregate suppression state derived from a source file.
#[derive(Debug, Default)]
pub struct Suppressions {
    line_scoped: HashMap<u32, LineSuppression>,
    /// Sorted by line ascending — order matters because later directives
    /// override earlier ones for the rules they touch.
    block_directives: Vec<BlockDirective>,
}

impl Suppressions {
    pub fn suppresses(&self, line: u32, rule: &str) -> bool {
        if let Some(s) = self.line_scoped.get(&line) {
            if s.suppresses(rule) {
                return true;
            }
        }
        let mut disabled = false;
        for d in &self.block_directives {
            if d.line > line {
                break;
            }
            if d.target.matches(rule) {
                disabled = !d.enable;
            }
        }
        disabled
    }
}

const NEXT_LINE_DIRECTIVE: &str = "soli-lint-disable-next-line";
const SAME_LINE_DIRECTIVE: &str = "soli-lint-disable-line";
const DISABLE_DIRECTIVE: &str = "soli-lint-disable";
const ENABLE_DIRECTIVE: &str = "soli-lint-enable";

/// Scan the source and build a suppression aggregate. Line numbers are
/// 1-based, matching `Span::line`.
pub fn collect_suppressions(source: &str) -> Suppressions {
    let mut suppressions = Suppressions::default();
    for (idx, line) in source.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let Some(hash_pos) = line.find('#') else {
            continue;
        };
        let after_hash = line[hash_pos + 1..].trim_start();

        // Order matters: longer prefixes must be checked first so
        // `disable-next-line` isn't mistaken for `disable`.
        if let Some(rest) = strip_directive(after_hash, NEXT_LINE_DIRECTIVE) {
            let (all, rules) = parse_rule_list(rest);
            suppressions
                .line_scoped
                .entry(line_no + 1)
                .or_default()
                .merge(LineSuppression { all, rules });
        } else if let Some(rest) = strip_directive(after_hash, SAME_LINE_DIRECTIVE) {
            let (all, rules) = parse_rule_list(rest);
            suppressions
                .line_scoped
                .entry(line_no)
                .or_default()
                .merge(LineSuppression { all, rules });
        } else if let Some(rest) = strip_directive(after_hash, ENABLE_DIRECTIVE) {
            suppressions.block_directives.push(BlockDirective {
                line: line_no,
                enable: true,
                target: parse_target(rest),
            });
        } else if let Some(rest) = strip_directive(after_hash, DISABLE_DIRECTIVE) {
            suppressions.block_directives.push(BlockDirective {
                line: line_no,
                enable: false,
                target: parse_target(rest),
            });
        }
    }
    suppressions
        .block_directives
        .sort_by_key(|d| (d.line, d.enable));
    suppressions
}

/// Strip `keyword` from the start of `after_hash` only if the keyword is
/// followed by whitespace or end-of-input — prevents matching a longer
/// directive (e.g. `disable-next-line`) against the shorter prefix
/// (`disable`).
fn strip_directive<'a>(after_hash: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = after_hash.strip_prefix(keyword)?;
    if rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace()) {
        Some(rest)
    } else {
        None
    }
}

fn parse_rule_list(rest: &str) -> (bool, Vec<String>) {
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        return (true, Vec::new());
    }
    let rules: Vec<String> = trimmed
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if rules.is_empty() {
        (true, Vec::new())
    } else {
        (false, rules)
    }
}

fn parse_target(rest: &str) -> Target {
    let (all, rules) = parse_rule_list(rest);
    if all {
        Target::All
    } else {
        Target::Specific(rules)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disable_next_line_all_rules() {
        let s = collect_suppressions("# soli-lint-disable-next-line\nfoo()\n");
        assert!(s.suppresses(2, "any/rule"));
        assert!(!s.suppresses(1, "any/rule"));
    }

    #[test]
    fn disable_next_line_specific_rule() {
        let s = collect_suppressions(
            "# soli-lint-disable-next-line smell/dangerous-server-builtin\nfoo()\n",
        );
        assert!(s.suppresses(2, "smell/dangerous-server-builtin"));
        assert!(!s.suppresses(2, "other/rule"));
    }

    #[test]
    fn disable_next_line_multiple_rules() {
        let s = collect_suppressions("# soli-lint-disable-next-line rule1, rule2\nfoo()\n");
        assert!(s.suppresses(2, "rule1"));
        assert!(s.suppresses(2, "rule2"));
        assert!(!s.suppresses(2, "rule3"));
    }

    #[test]
    fn disable_same_line() {
        let s = collect_suppressions(
            "foo()  # soli-lint-disable-line smell/dangerous-server-builtin\n",
        );
        assert!(s.suppresses(1, "smell/dangerous-server-builtin"));
    }

    #[test]
    fn unrelated_comment_is_ignored() {
        let s = collect_suppressions("# just a normal comment\nfoo()\n");
        assert!(!s.suppresses(2, "any/rule"));
    }

    #[test]
    fn directive_with_leading_indent() {
        let s = collect_suppressions("    # soli-lint-disable-next-line\n    foo()\n");
        assert!(s.suppresses(2, "any/rule"));
    }

    #[test]
    fn block_disable_then_enable() {
        let src = "\
ok_line()
# soli-lint-disable smell/dangerous-server-builtin
bad_line_1()
bad_line_2()
# soli-lint-enable smell/dangerous-server-builtin
ok_line_again()
";
        let s = collect_suppressions(src);
        assert!(!s.suppresses(1, "smell/dangerous-server-builtin"));
        assert!(s.suppresses(2, "smell/dangerous-server-builtin"));
        assert!(s.suppresses(3, "smell/dangerous-server-builtin"));
        assert!(s.suppresses(4, "smell/dangerous-server-builtin"));
        assert!(!s.suppresses(5, "smell/dangerous-server-builtin"));
        assert!(!s.suppresses(6, "smell/dangerous-server-builtin"));
    }

    #[test]
    fn block_disable_without_enable_runs_to_end() {
        let src = "\
foo()
# soli-lint-disable rule_a
bar()
baz()
";
        let s = collect_suppressions(src);
        assert!(!s.suppresses(1, "rule_a"));
        assert!(s.suppresses(2, "rule_a"));
        assert!(s.suppresses(3, "rule_a"));
        assert!(s.suppresses(4, "rule_a"));
    }

    #[test]
    fn block_disable_all_rules() {
        let src = "\
# soli-lint-disable
anything()
";
        let s = collect_suppressions(src);
        assert!(s.suppresses(2, "rule_a"));
        assert!(s.suppresses(2, "rule_b"));
    }

    #[test]
    fn enable_specific_after_disable_all() {
        let src = "\
# soli-lint-disable
# soli-lint-enable rule_b
both_disabled()
";
        let s = collect_suppressions(src);
        // rule_a stays disabled by the "all" directive, rule_b was re-enabled.
        assert!(s.suppresses(3, "rule_a"));
        assert!(!s.suppresses(3, "rule_b"));
    }

    #[test]
    fn block_disable_targets_only_listed_rules() {
        let src = "\
# soli-lint-disable rule_a
both_visible_for_b()
";
        let s = collect_suppressions(src);
        assert!(s.suppresses(2, "rule_a"));
        assert!(!s.suppresses(2, "rule_b"));
    }

    #[test]
    fn disable_prefix_not_confused_with_disable_next_line() {
        // The `disable-next-line` directive must not be parsed as a `disable`
        // block — the keyword match has to require a whitespace/EOL boundary.
        let src = "# soli-lint-disable-next-line rule_a\nfoo()\nbar()\n";
        let s = collect_suppressions(src);
        assert!(s.suppresses(2, "rule_a"));
        assert!(!s.suppresses(3, "rule_a"));
    }
}
