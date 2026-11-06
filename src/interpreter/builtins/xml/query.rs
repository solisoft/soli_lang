//! `find`, `find_all`, `xpath` and `at_xpath` on `XmlNode` / `XmlDocument`:
//! the XPath subset of `crate::xml::xpath`, in its two flavours.
//!
//! | method | flavour | returns |
//! |---|---|---|
//! | `find(expr, ns = nil)` | lenient | first result or `nil` |
//! | `find_all(expr, ns = nil)` | lenient | Array |
//! | `at_xpath(expr, ns = nil)` | strict XPath 1.0 | first result or `nil` |
//! | `xpath(expr, ns = nil)` | strict XPath 1.0 | Array |
//!
//! A result is an `XmlNode` (an `XmlDocument` for `/`), or a String for an
//! `@attr` or `text()` step. `ns` is a Hash of prefix => URI (named
//! arguments work: `doc.xpath("//r:x", r: uri)`); its `""` key is the
//! namespace of unprefixed element names. Errors start with `Xml.find:` or
//! `Xml.xpath:`.
//!
//! Compiled expressions are kept in a per-thread cache of
//! [`CACHE_CAPACITY`] entries (least recently used goes first): a compiled
//! expression depends on neither the flavour nor the table.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::value::{add_method, wrap, Arity, MethodTable, XmlRef};
use crate::interpreter::value::{HashKey, Value};
use crate::xml::{select, Flavour, Item, Query, XPath};

/// Compiled expressions kept per thread.
pub const CACHE_CAPACITY: usize = 128;

struct Cache {
    entries: HashMap<String, (Rc<XPath>, u64)>,
    tick: u64,
}

thread_local! {
    static XPATH_CACHE: RefCell<Cache> = RefCell::new(Cache {
        entries: HashMap::new(),
        tick: 0,
    });
}

/// `expr` compiled, from the cache when it is there.
fn compiled(expr: &str) -> Result<Rc<XPath>, crate::xml::XPathError> {
    if let Some(hit) = XPATH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.tick += 1;
        let tick = cache.tick;
        cache.entries.get_mut(expr).map(|(xpath, used)| {
            *used = tick;
            xpath.clone()
        })
    }) {
        return Ok(hit);
    }
    let xpath = Rc::new(XPath::compile(expr)?);
    XPATH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.entries.len() >= CACHE_CAPACITY {
            let oldest = cache
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                cache.entries.remove(&oldest);
            }
        }
        let tick = cache.tick;
        cache
            .entries
            .insert(expr.to_string(), (xpath.clone(), tick));
    });
    Ok(xpath)
}

#[derive(Clone, Copy)]
enum Want {
    First,
    All,
}

/// `ns`: `nil`, or a Hash of String prefix => String URI.
fn namespaces(value: Option<&Value>, who: &str) -> Result<Vec<(String, String)>, String> {
    let hash = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Hash(hash)) => hash,
        Some(other) => {
            return Err(format!(
                "{who}: the namespaces must be a Hash of prefix => URI, got {}",
                other.type_name()
            ))
        }
    };
    let hash = hash.borrow();
    let mut out = Vec::with_capacity(hash.len());
    for (key, value) in hash.iter() {
        let prefix = match key {
            HashKey::String(s) | HashKey::Symbol(s) => s.to_string(),
            _ => return Err(format!("{who}: namespace prefixes must be Strings")),
        };
        if !prefix.is_empty() && !crate::xml::escape::is_ncname(&prefix) {
            return Err(format!("{who}: '{prefix}' is not a valid namespace prefix"));
        }
        let Value::String(uri) = value else {
            return Err(format!(
                "{who}: the URI for prefix '{prefix}' must be a String, got {}",
                value.type_name()
            ));
        };
        out.push((prefix, uri.to_string()));
    }
    Ok(out)
}

fn run(node: &XmlRef, args: &[Value], flavour: Flavour, want: Want) -> Result<Value, String> {
    let who = match flavour {
        Flavour::Lenient => "Xml.find",
        Flavour::Strict => "Xml.xpath",
    };
    if args.len() > 2 {
        return Err(format!(
            "{who} takes (expression, namespaces = nil), got {} arguments",
            args.len()
        ));
    }
    let expr = match args.first() {
        Some(Value::String(s)) => s.to_string(),
        Some(other) => {
            return Err(format!(
                "{who}: the expression must be a String, got {}",
                other.type_name()
            ))
        }
        None => return Err(format!("{who}: missing the expression")),
    };
    let table = namespaces(args.get(1), who)?;
    let xpath = compiled(&expr).map_err(|e| format!("{who}: {e}"))?;
    let max_visits = crate::interpreter::limits::max_xpath_visits();
    let (items, strings) = node
        .write(|tree, id| {
            let items = select(
                tree,
                id,
                &xpath,
                &Query {
                    flavour,
                    namespaces: &table,
                    max_visits,
                },
            )?;
            let take = match want {
                Want::First => items.len().min(1),
                Want::All => items.len(),
            };
            let strings: Vec<Option<String>> =
                items[..take].iter().map(|item| item.string(tree)).collect();
            Ok::<_, crate::xml::XPathError>((items, strings))
        })?
        .map_err(|e| format!("{who}: {e}"))?;
    let values = items
        .iter()
        .zip(strings)
        .map(|(item, string)| match (item, string) {
            (_, Some(s)) => Value::String(s.into()),
            (Item::Node(id), None) => wrap(node.at(*id)),
            _ => Value::Null,
        });
    Ok(match want {
        Want::First => values.into_iter().next().unwrap_or(Value::Null),
        Want::All => Value::Array(Rc::new(RefCell::new(values.collect()))),
    })
}

/// Register the four query methods on the node method table.
pub(crate) fn register(table: &mut MethodTable) {
    add_method(table, "find", Arity::Optional, |n, args| {
        run(n, args, Flavour::Lenient, Want::First)
    });
    add_method(table, "find_all", Arity::Optional, |n, args| {
        run(n, args, Flavour::Lenient, Want::All)
    });
    add_method(table, "at_xpath", Arity::Optional, |n, args| {
        run(n, args, Flavour::Strict, Want::First)
    });
    add_method(table, "xpath", Arity::Optional, |n, args| {
        run(n, args, Flavour::Strict, Want::All)
    });
}
