//! A namespace scope with constant-time lookups.
//!
//! The parser's scope ([`super::NsScope`]), the serialiser's output scope and
//! [`super::XmlTree::in_scope_namespaces`] all keep one binding stack **per
//! prefix** plus an undo log of the prefixes pushed, instead of one `Vec` of
//! every binding scanned backwards. A hostile document can hold tens of
//! thousands of live bindings (256 declarations on each of 256 levels); with
//! a scan, every prefixed name cost a walk over all of them.
//!
//! - `get` is one hash lookup;
//! - `truncate(mark)` pops exactly the bindings pushed since `mark`;
//! - `in_scope` is linear in the live bindings (one pass over the log).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::tree::NsDecl;

/// Live namespace bindings. A URI is stored as declared: an empty one is an
/// undeclaration (`xmlns=""`), which callers treat as "unbound".
#[derive(Debug, Clone, Default)]
pub(crate) struct Bindings {
    default: Vec<Arc<str>>,
    prefixed: HashMap<Box<str>, Vec<Arc<str>>>,
    /// The prefix of every live binding, in the order they were pushed.
    log: Vec<Option<Box<str>>>,
}

impl Bindings {
    /// Live bindings: the mark to pass to [`Bindings::truncate`].
    pub(crate) fn len(&self) -> usize {
        self.log.len()
    }

    /// Bind `prefix` (`None` = the default namespace) to `uri`, shadowing any
    /// outer binding until it is truncated away.
    pub(crate) fn push(&mut self, prefix: Option<&str>, uri: Arc<str>) {
        match prefix {
            None => self.default.push(uri),
            Some(p) => match self.prefixed.get_mut(p) {
                Some(stack) => stack.push(uri),
                None => {
                    self.prefixed.insert(p.into(), vec![uri]);
                }
            },
        }
        self.log.push(prefix.map(Box::from));
    }

    /// Drop every binding pushed after `mark` was taken.
    pub(crate) fn truncate(&mut self, mark: usize) {
        while self.log.len() > mark {
            let Some(prefix) = self.log.pop() else {
                break;
            };
            match prefix {
                None => {
                    self.default.pop();
                }
                Some(p) => {
                    if let Some(stack) = self.prefixed.get_mut(&*p) {
                        stack.pop();
                        if stack.is_empty() {
                            self.prefixed.remove(&*p);
                        }
                    }
                }
            }
        }
    }

    /// The innermost binding of `prefix`, an empty undeclaration included.
    pub(crate) fn get(&self, prefix: Option<&str>) -> Option<&Arc<str>> {
        match prefix {
            None => self.default.last(),
            Some(p) => self.prefixed.get(p).and_then(|stack| stack.last()),
        }
    }

    /// `true` when `prefix` currently means `uri` (`""`: unbound).
    pub(crate) fn means(&self, prefix: Option<&str>, uri: &str) -> bool {
        self.get(prefix).map_or("", |bound| &**bound) == uri
    }

    /// Every prefix in scope with its innermost URI, in the order the
    /// prefixes were first bound; undeclared defaults are left out.
    pub(crate) fn in_scope(&self) -> Vec<NsDecl> {
        let mut seen: HashSet<Option<&str>> = HashSet::new();
        let mut out = Vec::new();
        for prefix in &self.log {
            let key = prefix.as_deref();
            if !seen.insert(key) {
                continue;
            }
            if let Some(uri) = self.get(key).filter(|uri| !uri.is_empty()) {
                out.push(NsDecl {
                    prefix: prefix.clone(),
                    uri: uri.clone(),
                });
            }
        }
        out
    }
}
