//! Stable error codes, and the text `soli explain` prints for each.
//!
//! A code names a *kind* of error for good: `E0301` is an undefined variable
//! today and in every later release, so a link, a doc page or a search for it
//! keeps working. The registry is append-only — a retired code stays, marked
//! `obsolete`, and a new kind gets the next free number in its range. The
//! `codes_never_move` test holds the registry against `codes.snapshot`.
//!
//! Ranges:
//!
//! | range   | what                                   |
//! |---------|----------------------------------------|
//! | `E01xx` | lexer                                  |
//! | `E02xx` | parser                                 |
//! | `E03xx` | names and scopes                       |
//! | `E04xx` | types and calls (static and run time)  |
//! | `E05xx` | other run-time failures                |
//! | `E06xx` | framework (records, authorization)     |
//! | `E07xx` | module resolution                      |
//! | `W0xxx` | warnings                               |
//!
//! An error that is still a free-form message (`RuntimeError::General`,
//! `ParserError::General`, …) has no code until it is given a variant of its
//! own: a catch-all code would be a code that means nothing.
//!
//! Lint rules keep their slash names (`smell/undefined-local`) as their ids;
//! `soli explain` reads them from [`LINT_RULES`].

/// One registered error code.
#[derive(Debug)]
pub struct ErrorCode {
    /// `E0301`.
    pub code: &'static str,
    /// A kebab-case name `soli explain` also accepts: `undefined-variable`.
    pub name: &'static str,
    /// The error class it reports as (`NameError`), or the phase for an error
    /// that is only ever reported statically (`SyntaxError`, `CheckError`).
    pub class: &'static str,
    /// A short title, the first line of the explanation.
    pub title: &'static str,
    /// The Markdown `soli explain` prints.
    pub explanation: &'static str,
    /// Retired: kept so the number is never reused.
    pub obsolete: bool,
    /// A warning rather than an error.
    pub warning: bool,
}

macro_rules! code {
    ($code:literal, $name:literal, $class:literal, $title:literal) => {
        code!($code, $name, $class, $title, false)
    };
    ($code:literal, $name:literal, $class:literal, $title:literal, $warning:expr) => {
        ErrorCode {
            code: $code,
            name: $name,
            class: $class,
            title: $title,
            explanation: include_str!(concat!("explain/", $code, ".md")),
            obsolete: false,
            warning: $warning,
        }
    };
}

/// Every error code, in the order they were assigned within each range.
pub static CODES: &[ErrorCode] = &[
    code!(
        "E0101",
        "unexpected-character",
        "SyntaxError",
        "unexpected character"
    ),
    code!(
        "E0102",
        "unterminated-string",
        "SyntaxError",
        "unterminated string"
    ),
    code!(
        "E0103",
        "invalid-escape",
        "SyntaxError",
        "invalid escape sequence"
    ),
    code!("E0104", "invalid-number", "SyntaxError", "invalid number"),
    code!(
        "E0201",
        "unexpected-token",
        "SyntaxError",
        "unexpected token"
    ),
    code!(
        "E0202",
        "unexpected-end-of-file",
        "SyntaxError",
        "unexpected end of file"
    ),
    code!(
        "E0203",
        "invalid-assignment-target",
        "SyntaxError",
        "invalid assignment target"
    ),
    code!(
        "E0301",
        "undefined-variable",
        "NameError",
        "undefined variable"
    ),
    code!(
        "E0302",
        "undefined-function",
        "CheckError",
        "undefined function"
    ),
    code!("E0303", "undefined-type", "CheckError", "undefined type"),
    code!(
        "E0304",
        "unknown-member",
        "NoMethodError",
        "unknown method or field"
    ),
    code!(
        "E0305",
        "this-outside-class",
        "CheckError",
        "`this` outside a class"
    ),
    code!(
        "E0306",
        "super-outside-class",
        "CheckError",
        "`super` outside a class"
    ),
    code!(
        "E0307",
        "no-superclass",
        "CheckError",
        "class has no superclass"
    ),
    code!("E0401", "type-mismatch", "TypeError", "type mismatch"),
    code!(
        "E0402",
        "not-callable",
        "TypeError",
        "value is not callable"
    ),
    code!(
        "E0403",
        "wrong-arity",
        "ArgumentError",
        "wrong number of arguments"
    ),
    code!(
        "E0501",
        "division-by-zero",
        "ZeroDivisionError",
        "division by zero"
    ),
    code!(
        "E0502",
        "index-out-of-bounds",
        "IndexError",
        "index out of bounds"
    ),
    code!("E0503", "not-a-class", "TypeError", "not a class"),
    code!("E0504", "uncaught-exception", "Error", "uncaught exception"),
    code!(
        "E0601",
        "record-not-found",
        "RuntimeError",
        "record not found"
    ),
    code!("E0602", "forbidden", "RuntimeError", "forbidden"),
    code!(
        "E0701",
        "module-not-found",
        "ImportError",
        "module not found"
    ),
    code!("E0702", "circular-import", "ImportError", "circular import"),
    code!("E0703", "import-error", "ImportError", "import refused"),
    code!(
        "E0704",
        "module-syntax-error",
        "ImportError",
        "syntax error in an imported module"
    ),
    code!(
        "W0401",
        "non-exhaustive-match",
        "CheckWarning",
        "non-exhaustive match",
        true
    ),
];

/// A lint rule, explained by `soli explain <rule>`.
#[derive(Debug)]
pub struct LintRule {
    pub id: &'static str,
    pub summary: &'static str,
}

/// Every rule `soli lint` reports (the `lint_rules_are_all_explained` test
/// reads the rule ids out of `src/lint` and checks each is here).
pub static LINT_RULES: &[LintRule] = &[
    LintRule { id: "naming/snake-case", summary: "Variables, functions, methods and parameters use snake_case: `my_var`, `process_data`." },
    LintRule { id: "naming/pascal-case", summary: "Classes and interfaces use PascalCase: `MyClass`." },
    LintRule { id: "style/empty-block", summary: "A block should do something, or not exist." },
    LintRule { id: "style/line-length", summary: "Lines are at most 120 characters." },
    LintRule { id: "style/redundant-model-import", summary: "No `import \"../models/*.sl\"` inside `app/controllers/`: models are auto-loaded." },
    LintRule { id: "smell/unreachable-code", summary: "Code after `return` never runs. Delete it, or move the return." },
    LintRule { id: "smell/empty-catch", summary: "A `catch` that does nothing swallows the error silently. Log it, or handle it." },
    LintRule { id: "smell/duplicate-methods", summary: "Two methods of one class share a name; the later one replaces the earlier. An instance `def x` and a `static def x` are distinct." },
    LintRule { id: "smell/duplicate-global", summary: "Two files of an app declare the same top-level class, module, `def` or `const`: the one loaded last replaces the other, so `soli serve` refuses to boot. Controllers' top-level `def`s are exempt." },
    LintRule { id: "smell/deep-nesting", summary: "More than four levels of nesting. Extract a method, or return early with a guard clause." },
    LintRule { id: "smell/undefined-local", summary: "A bare name is read but never assigned in this scope. `let` is optional, so a typo fails only at run time; this rule finds it statically and suggests the closest name. At run time the same mistake is E0301." },
    LintRule { id: "smell/dangerous-server-builtin", summary: "An injection sink (`db_query_raw`, `Trusted.*`, `System.shell`, backticks) called from a controller, middleware or view. Use the parameterised or jailed form the message names." },
    LintRule { id: "smell/closure-cycle", summary: "A closure stored onto `this` captures the environment holding `this`, so the instance and the closure never free. Store a method name, or pass the closure per call." },
    LintRule { id: "smell/shadowed-builtin", summary: "A local named after a builtin, in a scope that also calls that builtin: the call reaches the local. Rename the local." },
    LintRule { id: "idiom/nil-comparison", summary: "Write `x.nil?` / `!x.nil?` rather than `x == nil` / `x != nil` (or `null`)." },
    LintRule { id: "idiom/prefer-blank", summary: "Write `name.blank?` / `name.present?` rather than comparing to `\"\"` — it covers nil too." },
    LintRule { id: "idiom/prefer-includes", summary: "Replace three or more `==` / `!=` on one value with `[\"a\", \"b\"].includes?(x)`." },
    LintRule { id: "idiom/prefer-to-s", summary: "Write `x.to_s` rather than `x ?? \"\"`. A real fallback (`x ?? \"Guest\"`) is left alone." },
    LintRule { id: "idiom/redundant-template-escape", summary: "In a view, `<%= h(x) %>` (or another escape helper) escapes twice: `<%= %>` already HTML-escapes. Write `<%= x %>`, or `<%- %>` for HTML you built yourself." },
    LintRule { id: "idiom/manual-find-guard", summary: "A nil-check after `Model.find` is dead code: `find` raises RecordNotFound (E0601). Use `find_by` / `first_by` for \"or nil\"." },
    LintRule { id: "security/unfiltered-mass-assignment", summary: "`Model.create(params)`, `update` or `create_many` with raw request params in a controller or service persists every key an attacker sends. Whitelist with `permit` first." },
    LintRule { id: "component/props", summary: "A component's `props(...)` uses string-literal names, with no duplicates." },
    LintRule { id: "docs/openapi", summary: "A mistake in an action's OpenAPI doc comments: an unknown tag, a `@response` with no status, a `@param` with no name, or JSON that does not parse." },
];

/// The registered code `query` names: its code (`E0301`, any case) or its
/// name (`undefined-variable`).
pub fn lookup(query: &str) -> Option<&'static ErrorCode> {
    let query = query.trim();
    CODES
        .iter()
        .find(|c| c.code.eq_ignore_ascii_case(query) || c.name == query)
}

/// The registry entry for a code an error reported.
pub fn by_code(code: &str) -> Option<&'static ErrorCode> {
    CODES.iter().find(|c| c.code == code)
}

/// The lint rule `query` names.
pub fn lint_rule(query: &str) -> Option<&'static LintRule> {
    LINT_RULES.iter().find(|r| r.id == query.trim())
}

/// What `soli explain <query>` prints, or the closest known id when nothing
/// matches (`Err(Some("E0301"))`).
pub fn explain(query: &str) -> Result<String, Option<String>> {
    if let Some(code) = lookup(query) {
        let mut text = format!(
            "{}\n\n{} `{}` · reported as `{}`\n",
            code.explanation.trim_end(),
            if code.warning { "Warning" } else { "Error" },
            code.name,
            code.class
        );
        if code.obsolete {
            text.push_str("\nThis code is no longer reported.\n");
        }
        return Ok(text);
    }
    if let Some(rule) = lint_rule(query) {
        return Ok(format!(
            "# {} (lint rule)\n\n{}\n\nReported by `soli lint`. Silence one line with\n`# soli-lint-disable-next-line {}`.\n",
            rule.id, rule.summary, rule.id
        ));
    }
    let candidates = CODES
        .iter()
        .flat_map(|c| [c.code, c.name])
        .chain(LINT_RULES.iter().map(|r| r.id));
    Err(super::suggest::best_match(query.trim(), candidates))
}

/// One line per code and rule, for `soli explain` with no argument.
pub fn index() -> String {
    let mut out = String::from("Error codes:\n");
    for code in CODES.iter().filter(|c| !c.obsolete) {
        out.push_str(&format!(
            "  {}  {:<28} {}\n",
            code.code, code.name, code.title
        ));
    }
    out.push_str("\nLint rules:\n");
    for rule in LINT_RULES {
        out.push_str(&format!("  {}\n", rule.id));
    }
    out.push_str("\nRun `soli explain <code or rule>` for the details.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn codes_and_names_are_unique_and_well_formed() {
        let mut codes = HashSet::new();
        let mut names = HashSet::new();
        for c in CODES {
            assert!(codes.insert(c.code), "duplicate code {}", c.code);
            assert!(names.insert(c.name), "duplicate name {}", c.name);
            let (prefix, digits) = c.code.split_at(1);
            assert!(prefix == "E" || prefix == "W", "{}", c.code);
            assert_eq!(c.warning, prefix == "W", "{}", c.code);
            assert!(digits.len() == 4 && digits.chars().all(|d| d.is_ascii_digit()));
            assert!(
                c.explanation.starts_with(&format!("# {}: ", c.code)),
                "{} explanation must start with its heading",
                c.code
            );
            assert!(c.explanation.lines().count() > 3, "{} is too thin", c.code);
        }
    }

    /// Codes are a public promise: a code once given keeps its number and
    /// its name. New codes are appended to the snapshot, never inserted.
    #[test]
    fn codes_never_move() {
        let snapshot = include_str!("codes.snapshot");
        let current: Vec<String> = CODES
            .iter()
            .map(|c| format!("{} {}", c.code, c.name))
            .collect();
        for line in snapshot.lines().filter(|l| !l.trim().is_empty()) {
            assert!(
                current.iter().any(|c| c == line),
                "code `{line}` was renumbered, renamed or removed — retire it with `obsolete` instead"
            );
        }
        let recorded = snapshot.lines().filter(|l| !l.trim().is_empty()).count();
        assert_eq!(
            recorded,
            current.len(),
            "a code was added: append `{}` to src/error/codes.snapshot",
            current.last().unwrap()
        );
    }

    #[test]
    fn lookup_takes_the_code_in_any_case_or_the_name() {
        assert_eq!(lookup("E0301").unwrap().name, "undefined-variable");
        assert_eq!(lookup("e0301").unwrap().code, "E0301");
        assert_eq!(lookup("undefined-variable").unwrap().code, "E0301");
        assert!(lookup("E9999").is_none());
    }

    #[test]
    fn explain_covers_lint_rules_and_suggests_on_a_miss() {
        assert!(explain("E0301").unwrap().contains("Undefined variable"));
        assert!(explain("smell/undefined-local")
            .unwrap()
            .contains("smell/undefined-local"));
        assert_eq!(explain("E0310"), Err(Some("E0301".to_string())));
        assert_eq!(
            explain("smell/undefined-locl"),
            Err(Some("smell/undefined-local".to_string()))
        );
        assert_eq!(explain("zzzzzzzzzz"), Err(None));
    }

    /// The documentation lists every code, on both surfaces.
    #[test]
    fn the_docs_list_every_code() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for doc in [
            "www/docs/error-codes.md",
            "www/app/views/docs/development-tools/error-codes.html.slv",
        ] {
            let text = std::fs::read_to_string(root.join(doc)).unwrap();
            for code in CODES.iter().filter(|c| !c.obsolete) {
                assert!(
                    text.contains(code.code) && text.contains(code.name),
                    "{doc} does not list {} ({})",
                    code.code,
                    code.name
                );
            }
        }
    }

    /// Every rule id the linter can emit has an entry: read them out of the
    /// lint sources, so a new rule without one fails here.
    #[test]
    fn lint_rules_are_all_explained() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut sources = Vec::new();
        let mut dirs = vec![root.join("src")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    sources.push(path);
                }
            }
        }
        let mut found = HashSet::new();
        for path in sources {
            let text = std::fs::read_to_string(&path).unwrap();
            for marker in ["rule: \"", "const RULE: &str = \""] {
                for (at, _) in text.match_indices(marker) {
                    let rest = &text[at + marker.len()..];
                    let end = rest.find('"').unwrap_or(0);
                    let id = rest[..end].to_string();
                    if id.contains('/') && !id.contains(' ') {
                        found.insert(id);
                    }
                }
            }
        }
        assert!(found.len() >= 20, "found only {found:?}");
        for id in &found {
            assert!(
                lint_rule(id).is_some(),
                "lint rule `{id}` has no entry in LINT_RULES"
            );
        }
    }
}
