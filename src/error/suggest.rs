//! "Did you mean …?" suggestions for errors about unknown names.
//!
//! An error about an undefined variable, an unknown method or an unknown
//! field names what was typed; when a known name is one or two keystrokes
//! away, the message names that too. The distance is the optimal string
//! alignment distance (Levenshtein plus adjacent transpositions, so
//! `lenght` → `length` is one edit), over chars, and a candidate is accepted
//! only within `max(1, len / 3)` edits — the threshold rustc uses — so short
//! names never pick up a far-fetched match.
//!
//! Some of these errors are control flow rather than messages: a template
//! reads an undefined local as `nil`, and `@items ||= []` reads an unset
//! field through the error path on every call. Collecting candidates there
//! would cost on a path that never shows the text, so [`Hint::lazy`] defers
//! the work to the moment the error is formatted.

use std::fmt;
use std::rc::Rc;

/// Optimal string alignment distance between `a` and `b`, over chars.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    // Three rows: two back (for transpositions), one back, current.
    let mut prev2: Vec<usize> = vec![0; m + 1];
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(prev2[j - 2] + 1);
            }
            cur[j] = best;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

/// The candidate closest to `target`, if one is close enough to be a
/// plausible typo. A case-only difference wins outright. Ties go to the
/// alphabetically first name, so the suggestion does not depend on hash
/// iteration order. Names starting with `__` are internal and never offered.
pub fn best_match<I, S>(target: &str, candidates: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let target_len = target.chars().count();
    let max = (target_len / 3).max(1);
    let lower = target.to_lowercase();
    let mut best: Option<(usize, String)> = None;
    for candidate in candidates {
        let candidate = candidate.as_ref();
        if candidate == target || candidate.is_empty() || candidate.starts_with("__") {
            continue;
        }
        let len = candidate.chars().count();
        if len.abs_diff(target_len) > max {
            continue;
        }
        let distance = if candidate.to_lowercase() == lower {
            0
        } else {
            edit_distance(target, candidate)
        };
        if distance > max {
            continue;
        }
        let better = match &best {
            None => true,
            Some((d, name)) => distance < *d || (distance == *d && candidate < name.as_str()),
        };
        if better {
            best = Some((distance, candidate.to_string()));
        }
    }
    best.map(|(_, name)| name)
}

/// The text appended to an error message for a suggestion: empty when there
/// is none, ` (did you mean 'x'?)` otherwise. It sits before the ` at L:C`
/// location so tools that read the location off the end keep working.
pub fn suffix(suggestion: Option<&str>) -> String {
    match suggestion {
        Some(name) => format!(" (did you mean '{name}'?)"),
        None => String::new(),
    }
}

type LazyFn = dyn Fn() -> Option<String>;

/// A suggestion attached to a runtime error: none, already known, or
/// computed when the error is formatted (see the module docs for why).
///
/// One pointer wide: `RuntimeError` is kept at 48 bytes (see the assertion
/// in `error/mod.rs`), and `UndefinedVariable` carries one of these next to
/// its name and span.
#[derive(Clone, Default)]
pub struct Hint(Option<Rc<HintKind>>);

enum HintKind {
    Ready(String),
    Lazy(Box<LazyFn>),
}

impl Hint {
    pub fn none() -> Self {
        Self(None)
    }

    pub fn ready(suggestion: Option<String>) -> Self {
        Self(suggestion.map(|s| Rc::new(HintKind::Ready(s))))
    }

    /// Defer the search until the message is shown.
    pub fn lazy(f: impl Fn() -> Option<String> + 'static) -> Self {
        Self(Some(Rc::new(HintKind::Lazy(Box::new(f)))))
    }

    pub fn is_none(&self) -> bool {
        self.0.is_none()
    }

    /// The suggested name, computing it now for a lazy hint.
    pub fn suggestion(&self) -> Option<String> {
        match self.0.as_deref() {
            None => None,
            Some(HintKind::Ready(s)) => Some(s.clone()),
            Some(HintKind::Lazy(f)) => f(),
        }
    }

    /// The message suffix (see [`suffix`]).
    pub fn suffix(&self) -> String {
        suffix(self.suggestion().as_deref())
    }
}

impl fmt::Debug for Hint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.as_deref() {
            None => f.write_str("Hint(None)"),
            Some(HintKind::Ready(s)) => write!(f, "Hint({s:?})"),
            Some(HintKind::Lazy(_)) => f.write_str("Hint(<lazy>)"),
        }
    }
}

/// The built-in methods of a primitive value type, under the name a
/// `Cannot access property` error gives that type (`String`, `int`, …).
pub fn primitive_methods(
    value_type: &str,
) -> &'static [crate::interpreter::executor::calls::method_registry::MethodDef] {
    use crate::interpreter::executor::calls::method_registry::known_methods;
    let key = match value_type {
        "String" | "string" => "string",
        "Array" | "array" => "array",
        "Hash" | "hash" => "hash",
        "int" | "Int" => "int",
        "float" | "Float" => "float",
        "decimal" | "Decimal" => "decimal",
        "bool" | "Bool" => "bool",
        "null" | "Null" => "null",
        "symbol" | "Symbol" => "symbol",
        "QueryBuilder" => "query_builder",
        _ => return &[],
    };
    known_methods(key)
}

/// A suggestion for an unknown method on a primitive value type.
pub fn primitive_method_suggestion(value_type: &str, name: &str) -> Option<String> {
    let methods = primitive_methods(value_type);
    if methods.is_empty() {
        return None;
    }
    best_match(name, methods.iter().map(|m| m.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_counts_a_transposition_as_one_edit() {
        assert_eq!(edit_distance("lenght", "length"), 1);
        assert_eq!(edit_distance("countr", "counter"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("same", "same"), 0);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn best_match_picks_the_closest_plausible_name() {
        let names = ["counter", "count_items", "total", "print"];
        assert_eq!(best_match("countr", names), Some("counter".into()));
        assert_eq!(best_match("prnt", names), Some("print".into()));
        assert_eq!(best_match("zebra", names), None);
    }

    #[test]
    fn short_names_need_a_close_match() {
        // len 2 → at most 1 edit
        assert_eq!(best_match("xy", ["ab", "xz"]), Some("xz".into()));
        assert_eq!(best_match("xy", ["ab"]), None);
    }

    #[test]
    fn case_difference_wins_and_ties_are_alphabetical() {
        assert_eq!(best_match("user", ["User", "uses"]), Some("User".into()));
        assert_eq!(best_match("cat", ["bat", "hat"]), Some("bat".into()));
    }

    #[test]
    fn internal_and_identical_names_are_never_offered() {
        assert_eq!(best_match("foo", ["foo"]), None);
        assert_eq!(best_match("__x", ["__y"]), None);
    }

    #[test]
    fn lazy_hint_runs_only_when_formatted() {
        use std::cell::Cell;
        let calls = Rc::new(Cell::new(0));
        let seen = calls.clone();
        let hint = Hint::lazy(move || {
            seen.set(seen.get() + 1);
            Some("length".into())
        });
        assert_eq!(calls.get(), 0);
        assert_eq!(hint.suffix(), " (did you mean 'length'?)");
        assert_eq!(calls.get(), 1);
        assert_eq!(Hint::none().suffix(), "");
    }

    #[test]
    fn primitive_suggestions_use_the_method_registry() {
        assert_eq!(
            primitive_method_suggestion("String", "upcse"),
            Some("upcase".into())
        );
        assert_eq!(
            primitive_method_suggestion("Array", "lenght"),
            Some("length".into())
        );
        assert_eq!(primitive_method_suggestion("Widget", "lenght"), None);
    }
}
