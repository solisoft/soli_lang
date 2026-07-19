//! The data document (`{ template_uuid, data }`) and a dotted-path resolver.

use serde::Deserialize;
use serde_json::Value;

use crate::error::{PdfError, Result};

/// The interpolation data document.
#[derive(Debug, Clone, Deserialize)]
pub struct DataDocument {
    #[serde(default)]
    pub template_uuid: Option<String>,
    /// The object that `${...}` paths resolve against.
    #[serde(default)]
    pub data: Value,
}

impl DataDocument {
    /// Parse a data document from JSON bytes.
    ///
    /// Accepts either `{ "template_uuid": ..., "data": { ... } }` or a bare
    /// data object (in which case `data` is the whole thing).
    pub fn parse(bytes: &[u8]) -> Result<DataDocument> {
        let value: Value = serde_json::from_slice(bytes).map_err(PdfError::from)?;
        // If it looks like the wrapper shape, use it; otherwise treat the whole
        // object as the data payload.
        if value.get("data").is_some() || value.get("template_uuid").is_some() {
            serde_json::from_value(value).map_err(PdfError::from)
        } else {
            Ok(DataDocument {
                template_uuid: None,
                data: value,
            })
        }
    }

    /// An empty data document (used for header/footer interpolation contexts
    /// that don't carry their own data binding).
    pub fn empty() -> DataDocument {
        DataDocument {
            template_uuid: None,
            data: Value::Null,
        }
    }

    /// A resolver rooted at the top-level `data` object.
    pub fn resolver(&self) -> Resolver<'_> {
        Resolver {
            root: &self.data,
            scope: None,
        }
    }

    /// Borrow the array at `key` (used to expand data-bound tables).
    pub fn array(&self, key: &str) -> Option<&[Value]> {
        lookup_value(&self.data, key).and_then(|v| v.as_array().map(|a| a.as_slice()))
    }
}

/// Resolves `${dotted.path}` lookups, with an optional row scope that takes
/// precedence over the root (used inside data-bound table rows).
#[derive(Clone, Copy)]
pub struct Resolver<'a> {
    root: &'a Value,
    scope: Option<&'a Value>,
}

impl<'a> Resolver<'a> {
    /// A new resolver with `item` as the row scope. `${field}` resolves against
    /// `item` first, then falls back to the root.
    pub fn with_scope(&self, item: &'a Value) -> Resolver<'a> {
        Resolver {
            root: self.root,
            scope: Some(item),
        }
    }

    /// Resolve a path to a rendered scalar string. Returns `None` if the path is
    /// missing or points at a non-scalar (object/array).
    pub fn lookup(&self, path: &str) -> Option<String> {
        if let Some(scope) = self.scope {
            if let Some(v) = lookup_value(scope, path) {
                return render_scalar(v);
            }
        }
        lookup_value(self.root, path).and_then(render_scalar)
    }

    /// Borrow the array at `path`, scope first then root — the same precedence
    /// `lookup` uses for scalars. This is what lets a `repeat` (or a data-bound
    /// table) nested inside another `repeat` bind to an array *on the current
    /// item*: `"data": "lines"` inside a repeat over `sections` resolves to that
    /// section's `lines`. Resolving only against the root, as this used to, made
    /// every nested binding silently render nothing.
    pub fn array(&self, path: &str) -> Option<&'a [Value]> {
        if let Some(scope) = self.scope {
            if let Some(a) = lookup_value(scope, path).and_then(|v| v.as_array()) {
                return Some(a.as_slice());
            }
        }
        lookup_value(self.root, path)
            .and_then(|v| v.as_array())
            .map(|a| a.as_slice())
    }
}

/// Walk a dotted path through nested JSON objects.
fn lookup_value<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur)
}

/// Render a JSON scalar as a plain string. Non-scalars yield `None`.
fn render_scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null => Some(String::new()),
        Value::Array(_) | Value::Object(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> DataDocument {
        DataDocument::parse(
            br##"{"data":{"invoice":{"number":"#12345","total":600},
                 "items":[{"name":"A","qty":1},{"name":"B","qty":2}]}}"##,
        )
        .unwrap()
    }

    #[test]
    fn nested_and_numbers() {
        let d = doc();
        let r = d.resolver();
        assert_eq!(r.lookup("invoice.number").as_deref(), Some("#12345"));
        assert_eq!(r.lookup("invoice.total").as_deref(), Some("600"));
        assert_eq!(r.lookup("invoice.missing"), None);
        assert_eq!(r.lookup("items"), None); // array is non-scalar
    }

    #[test]
    fn row_scope_precedence() {
        let d = doc();
        let items = d.array("items").unwrap();
        let r = d.resolver().with_scope(&items[1]);
        assert_eq!(r.lookup("name").as_deref(), Some("B"));
        assert_eq!(r.lookup("qty").as_deref(), Some("2"));
        // falls back to root for non-row paths
        assert_eq!(r.lookup("invoice.number").as_deref(), Some("#12345"));
    }

    #[test]
    fn bare_data_object() {
        let d = DataDocument::parse(br#"{"foo":{"bar":1}}"#).unwrap();
        assert_eq!(d.resolver().lookup("foo.bar").as_deref(), Some("1"));
    }
}
