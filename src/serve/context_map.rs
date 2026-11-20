//! The context map: which bounded context may use which.
//!
//! A bounded context is a top-level `module` of the application (`Billing`,
//! `Shipping`). `private class` already keeps a context's internals to
//! itself; this keeps the *dependencies* between contexts to the ones the
//! team decided on, written in `soli.toml`:
//!
//! ```toml
//! [contexts]
//! Shipping = ["Billing:events"]     # only Billing's domain events
//! Reporting = ["Billing", "Shipping"]
//! Billing = []                      # nothing from another context
//! ```
//!
//! A context listed there may use, from another context, only what its entry
//! names: `"Billing"` for Billing's classes, `"Billing:events"` for its
//! `DomainEvent` subclasses only. A context not listed is not restricted, and
//! without a `[contexts]` table nothing is checked. Code outside every module
//! (controllers, top-level services) is not a context and is not restricted.
//!
//! What counts as a use: a qualified name (`Billing::Invoice`, in any
//! expression of the module's code) and a string naming a class of another
//! context (`belongs_to("invoice", {"class_name": "Billing::Invoice"})`).
//! `soli lint` reports each one as `smell/context-boundary`; a name in
//! `[contexts]` that is not a module of the application is reported too.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::expr::{Expr, ExprKind};
use crate::ast::stmt::{ClassDecl, Program, StmtKind};

/// One use the context map does not allow (or a map naming no module).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crossing {
    pub file: PathBuf,
    pub line: u32,
    pub message: String,
}

/// What a context may use from one other context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Allowed {
    Everything,
    EventsOnly,
}

/// Context → other context → what of it.
type ContextMap = HashMap<String, HashMap<String, Allowed>>;

/// `[contexts]` of `root/soli.toml`: context → other context → what of it.
/// `None` when the file or the table is absent.
fn read_map(root: &Path) -> Option<Result<ContextMap, String>> {
    let text = crate::platform::fs::read(root.join("soli.toml")).ok()?;
    let text = String::from_utf8_lossy(&text);
    let parsed: toml::Value = match text.parse() {
        Ok(value) => value,
        Err(e) => return Some(Err(format!("soli.toml does not parse: {e}"))),
    };
    let table = parsed.get("contexts")?.as_table()?;
    let mut map = HashMap::new();
    for (context, uses) in table {
        let Some(list) = uses.as_array() else {
            return Some(Err(format!(
                "soli.toml [contexts] {context} must be a list, like [\"Billing\", \"Shipping:events\"]"
            )));
        };
        let mut allowed = HashMap::new();
        for entry in list {
            let Some(entry) = entry.as_str() else {
                return Some(Err(format!(
                    "soli.toml [contexts] {context} lists something that is not a string"
                )));
            };
            let (other, what) = match entry.split_once(':') {
                Some((other, "events")) => (other, Allowed::EventsOnly),
                Some((_, other_kind)) => {
                    return Some(Err(format!(
                        "soli.toml [contexts] {context}: `{entry}` — after `:` only `events` is understood, not `{other_kind}`"
                    )))
                }
                None => (entry, Allowed::Everything),
            };
            allowed.insert(other.to_string(), what);
        }
        map.insert(context.clone(), allowed);
    }
    Some(Ok(map))
}

/// Every class and module of `files`, by qualified name, with the context it
/// belongs to and its superclass as written.
struct Classes {
    /// qualified name → (context, superclass as written, enclosing module)
    by_name: HashMap<String, (String, Option<String>, String)>,
    contexts: HashSet<String>,
}

fn collect_classes(files: &[(PathBuf, Program, bool)]) -> Classes {
    fn add(decl: &ClassDecl, qualified: String, context: &str, enclosing: &str, out: &mut Classes) {
        for nested in &decl.nested_classes {
            add(
                nested,
                format!("{qualified}::{}", nested.name),
                context,
                &qualified,
                out,
            );
        }
        out.by_name.entry(qualified).or_insert((
            context.to_string(),
            decl.superclass.clone(),
            enclosing.to_string(),
        ));
    }
    let mut out = Classes {
        by_name: HashMap::new(),
        contexts: HashSet::new(),
    };
    for (_, program, _) in files {
        for stmt in &program.statements {
            let kind = match &stmt.kind {
                StmtKind::Export(inner) => &inner.kind,
                other => other,
            };
            if let StmtKind::Class(decl) = kind {
                if decl.is_module {
                    out.contexts.insert(decl.name.clone());
                    add(decl, decl.name.clone(), &decl.name, "", &mut out);
                } else {
                    add(decl, decl.name.clone(), "", "", &mut out);
                }
            }
        }
    }
    out
}

impl Classes {
    /// Whether `name` is a `DomainEvent` subclass, as far as the
    /// application's own classes tell.
    fn is_event(&self, name: &str) -> bool {
        let mut current = name.to_string();
        for _ in 0..32 {
            let Some((_, superclass, enclosing)) = self.by_name.get(&current) else {
                return false;
            };
            let Some(parent) = superclass else {
                return false;
            };
            if parent == "DomainEvent" {
                return true;
            }
            // An unqualified parent is first looked for beside the class.
            let sibling = if enclosing.is_empty() {
                parent.clone()
            } else {
                format!("{enclosing}::{parent}")
            };
            current = if self.by_name.contains_key(&sibling) {
                sibling
            } else {
                parent.clone()
            };
        }
        false
    }
}

/// `A::B::C` as its parts, when `expr` is a qualified name rooted at a
/// plain name.
fn qualified_parts(expr: &Expr) -> Option<Vec<String>> {
    match &expr.kind {
        ExprKind::Variable(name) => Some(vec![name.clone()]),
        ExprKind::QualifiedName { qualifier, name } => {
            let mut parts = qualified_parts(qualifier)?;
            parts.push(name.clone());
            Some(parts)
        }
        _ => None,
    }
}

/// Every use the `[contexts]` map of the application at `root` does not
/// allow, in file order; empty without a map.
pub fn find_in_app(root: &Path) -> Vec<Crossing> {
    let map = match read_map(root) {
        None => return Vec::new(),
        Some(Err(message)) => {
            return vec![Crossing {
                file: root.join("soli.toml"),
                line: 1,
                message,
            }]
        }
        Some(Ok(map)) => map,
    };
    let files = crate::serve::global_names::parsed_under(root, crate::app_layout::APP_GLOBAL_DIRS);
    find_in_files(&map, &files, &root.join("soli.toml"))
}

fn find_in_files(
    map: &ContextMap,
    files: &[(PathBuf, Program, bool)],
    config: &Path,
) -> Vec<Crossing> {
    let classes = collect_classes(files);
    let mut found = Vec::new();

    let mut names: Vec<&String> = map.keys().collect();
    names.sort();
    for context in names {
        let mut listed: Vec<&String> = map[context].keys().collect();
        listed.sort();
        for name in std::iter::once(context).chain(listed) {
            if !classes.contexts.contains(name) {
                found.push(Crossing {
                    file: config.to_path_buf(),
                    line: 1,
                    message: format!(
                        "soli.toml [contexts] names `{name}`, which no top-level module of the application declares"
                    ),
                });
            }
        }
    }

    for (path, program, _) in files {
        for stmt in &program.statements {
            let StmtKind::Class(decl) = &stmt.kind else {
                continue;
            };
            if !decl.is_module {
                continue;
            }
            let context = decl.name.as_str();
            let Some(allowed) = map.get(context) else {
                continue;
            };
            let mut uses: Vec<(u32, String)> = Vec::new();
            crate::ast::walk::walk_stmt(stmt, &mut |_| {}, &mut |expr| {
                match &expr.kind {
                    ExprKind::QualifiedName { .. } => {
                        if let Some(parts) = qualified_parts(expr) {
                            uses.push((expr.span.line, parts.join("::")));
                        }
                    }
                    // `{"class_name": "Billing::Invoice"}`: a string naming
                    // a class of the application.
                    ExprKind::StringLiteral(text)
                        if text.contains("::") && classes.by_name.contains_key(text.as_str()) =>
                    {
                        uses.push((expr.span.line, text.clone()));
                    }
                    _ => {}
                }
            });
            // `A::B::C` is also walked as its qualifier `A::B`: keep the
            // longest name per line and root.
            uses.sort();
            uses.dedup();
            let mut reported: HashSet<(u32, String)> = HashSet::new();
            for (line, used) in &uses {
                let Some((other, _)) = used.split_once("::") else {
                    continue;
                };
                if other == context || !classes.contexts.contains(other) {
                    continue;
                }
                let longer_on_line = uses.iter().any(|(l, u)| {
                    l == line && u.len() > used.len() && u.starts_with(&format!("{used}::"))
                });
                if longer_on_line {
                    continue;
                }
                let verdict = match allowed.get(other) {
                    Some(Allowed::Everything) => None,
                    Some(Allowed::EventsOnly) if classes.is_event(used) => None,
                    Some(Allowed::EventsOnly) => Some(format!(
                        "`{context}` uses `{used}`, but soli.toml [contexts] lets it use only `{other}`'s domain events"
                    )),
                    None => Some(format!(
                        "`{context}` uses `{used}`, but soli.toml [contexts] does not let it use `{other}` — \
                         add \"{other}\" (or \"{other}:events\") to its entry, or go through an event"
                    )),
                };
                if let Some(message) = verdict {
                    if reported.insert((*line, used.clone())) {
                        found.push(Crossing {
                            file: path.clone(),
                            line: *line,
                            message,
                        });
                    }
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(sources: &[(&str, &str)]) -> Vec<(PathBuf, Program, bool)> {
        sources
            .iter()
            .map(|(path, source)| {
                let tokens = crate::lexer::Scanner::new(source).scan_tokens().unwrap();
                let program = crate::parser::Parser::new(tokens).parse().unwrap();
                (PathBuf::from(path), program, false)
            })
            .collect()
    }

    fn map(entries: &[(&str, &[(&str, Allowed)])]) -> HashMap<String, HashMap<String, Allowed>> {
        entries
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    v.iter().map(|(o, a)| (o.to_string(), *a)).collect(),
                )
            })
            .collect()
    }

    const BILLING: (&str, &str) = (
        "app/models/billing.sl",
        "module Billing\n  class Invoice < Model\n  end\n  class InvoicePaid < DomainEvent\n    number: String\n  end\nend\n",
    );

    fn messages(found: &[Crossing]) -> Vec<String> {
        found
            .iter()
            .map(|c| format!("{}:{} {}", c.file.display(), c.line, c.message))
            .collect()
    }

    #[test]
    fn a_context_limited_to_events_may_use_them_and_nothing_else() {
        let all = files(&[
            BILLING,
            (
                "app/services/shipping.sl",
                "module Shipping\n  class Notifier\n    def ok(event)\n      event.is_a?(\"Billing::InvoicePaid\")\n      Billing::InvoicePaid\n    end\n\n    def bad\n      Billing::Invoice.all\n    end\n  end\nend\n",
            ),
        ]);
        let found = find_in_files(
            &map(&[
                ("Shipping", &[("Billing", Allowed::EventsOnly)]),
                ("Billing", &[]),
            ]),
            &all,
            Path::new("soli.toml"),
        );
        assert_eq!(
            messages(&found),
            vec!["app/services/shipping.sl:9 `Shipping` uses `Billing::Invoice`, but soli.toml [contexts] lets it use only `Billing`'s domain events"]
        );
    }

    #[test]
    fn an_unlisted_dependency_is_reported_with_the_way_out() {
        let all = files(&[
            BILLING,
            (
                "app/models/shipping.sl",
                "module Shipping\n  class Parcel < Model\n    belongs_to(\"invoice\", {\"class_name\": \"Billing::Invoice\"})\n  end\nend\n",
            ),
        ]);
        let found = find_in_files(&map(&[("Shipping", &[])]), &all, Path::new("soli.toml"));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].message.contains("does not let it use `Billing`"),
            "{found:?}"
        );
        assert_eq!(found[0].line, 3);
    }

    #[test]
    fn listed_contexts_unlisted_contexts_and_outside_code_are_free() {
        let all = files(&[
            BILLING,
            ("app/services/reporting.sl", "module Reporting\n  class Totals\n    def run\n      Billing::Invoice.count\n    end\n  end\nend\n"),
            ("app/services/audit.sl", "module Audit\n  class Log\n    def run\n      Billing::Invoice.count\n    end\n  end\nend\n"),
            ("app/controllers/x_controller.sl", "class XController < Controller\n  def index(req)\n    Billing::Invoice.all\n  end\nend\n"),
        ]);
        let found = find_in_files(
            &map(&[("Reporting", &[("Billing", Allowed::Everything)])]),
            &all,
            Path::new("soli.toml"),
        );
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn a_map_naming_no_module_is_reported() {
        let all = files(&[BILLING]);
        let found = find_in_files(
            &map(&[("Biling", &[("Shiping", Allowed::Everything)])]),
            &all,
            Path::new("soli.toml"),
        );
        let text = messages(&found).join("\n");
        assert!(text.contains("names `Biling`"), "{text}");
        assert!(text.contains("names `Shiping`"), "{text}");
    }

    #[test]
    fn the_map_is_read_from_soli_toml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("soli.toml"),
            "[contexts]\nShipping = [\"Billing:events\"]\nBilling = []\n",
        )
        .unwrap();
        let Some(Ok(read)) = read_map(dir.path()) else {
            panic!("no map read");
        };
        assert_eq!(read["Shipping"]["Billing"], Allowed::EventsOnly);
        assert!(read["Billing"].is_empty());
        std::fs::write(
            dir.path().join("soli.toml"),
            "[contexts]\nShipping = [\"Billing:classes\"]\n",
        )
        .unwrap();
        assert!(matches!(read_map(dir.path()), Some(Err(m)) if m.contains("only `events`")));
        std::fs::write(dir.path().join("soli.toml"), "[package]\nname = \"x\"\n").unwrap();
        assert!(read_map(dir.path()).is_none());
    }
}
