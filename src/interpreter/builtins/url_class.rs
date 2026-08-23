//! Url built-in class for Soli.
//!
//! Parse, build, join, and manipulate URLs without string surgery.
//! All methods are static: `Url.parse(u)`, `Url.join(base, rel)`,
//! `Url.set_param(u, k, v)`, … Parsing delegates to the `url` crate so
//! malformed input produces a clean error rather than a wrong result;
//! percent-encoding uses the `urlencoding` crate already in the tree.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFunction, Value};

fn parse_url(input: &str, ctx: &str) -> Result<url::Url, String> {
    url::Url::parse(input.trim()).map_err(|e| format!("{ctx}: invalid URL \"{input}\": {e}"))
}

/// Percent-decode, leaving the raw text alone when the escapes are not valid
/// UTF-8.
///
/// `unwrap_or_default()` turned an undecodable string into an *empty* one, so
/// `Url.decode_component("%FF")` returned `""` — contradicting the documented
/// "invalid escapes are left as-is" — and, worse, `Url.set_param` round-trips
/// the whole query through `decode_query`/`encode_query`, so it silently blanked
/// any sibling param it could not decode. `server::parse_query_pairs` has always
/// fallen back to the raw text; match it.
fn percent_decode(raw: &str) -> String {
    urlencoding::decode(raw)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| raw.to_string())
}

/// Query-string flavour of [`percent_decode`]: `+` is an encoded space.
///
/// Without this, `Url.params("?q=red+shoe")["q"]` was `"red+shoe"` while
/// `req["params"]["q"]` for the same query was `"red shoe"`, and `set_param`
/// re-encoded the `+` as `%2B` — changing the value the next server sees.
fn decode_query_component(raw: &str) -> String {
    percent_decode(&raw.replace('+', " "))
}

/// Query string (no leading `?`) → decoded param pairs. Pairs with no `=`
/// map to null, matching form semantics; a repeated key keeps its last
/// value (the common convention).
pub fn decode_query(query: &str) -> HashPairs {
    let mut pairs = HashPairs::default();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (raw_k, raw_v) = match pair.split_once('=') {
            Some((k, v)) => (k, Some(v)),
            None => (pair, None),
        };
        let key = decode_query_component(raw_k);
        let value = match raw_v {
            Some(v) => Value::String(decode_query_component(v).into()),
            None => Value::Null,
        };
        pairs.insert(HashKey::String(key.into()), value);
    }
    pairs
}

/// Nesting cap for query encoding.
///
/// `encode_pair` recurses through arrays and nested hashes, and a Soli hash can
/// contain itself (`h["self"] = h`), so the recursion is not bounded by the
/// input's size. A native stack overflow **aborts** without unwinding, so this
/// has to be a cap rather than something a caller can catch after the fact —
/// the same reason `parse_json` and `value_to_json` carry one.
const MAX_QUERY_DEPTH: usize = 32;

/// Decoded param hash → encoded query string (no leading `?`). Null values
/// render as bare keys (`flag`), matching `decode_query`.
///
/// Errors when the structure nests past [`MAX_QUERY_DEPTH`]: bracket names that
/// deep are not a real query string, and silently truncating would be the
/// data-losing behaviour this function was fixed to stop doing.
pub fn encode_query(pairs: &HashPairs) -> Result<String, String> {
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in pairs.iter() {
        let key = match k {
            HashKey::String(s) | HashKey::Symbol(s) => s.to_string(),
            _ => continue,
        };
        encode_pair(&mut parts, &key, v, 0)?;
    }
    Ok(parts.join("&"))
}

/// Encode one `key = value` into `parts`, expanding arrays and nested hashes
/// with the bracket names the framework's own param decoding understands
/// (`tags[]=a&tags[]=b`, `author[name]=x`).
///
/// The old code matched only scalars and `continue`d on anything else, so
/// `Url.build({"query": {"tags": ["a", "b"], "page": 2}})` silently returned
/// `?page=2` — a pagination or filter URL quietly losing its filters.
fn encode_pair(parts: &mut Vec<String>, key: &str, v: &Value, depth: usize) -> Result<(), String> {
    if depth > MAX_QUERY_DEPTH {
        return Err(format!(
            "query parameters nested too deeply (over {MAX_QUERY_DEPTH} levels)"
        ));
    }
    match v {
        Value::Null => parts.push(urlencoding::encode(key).to_string()),
        Value::Array(items) => {
            let nested = format!("{key}[]");
            // Clone the handles so the borrow is released before recursing: a
            // hash that contains itself would otherwise hit a RefCell
            // double-borrow panic instead of the depth error.
            let items: Vec<Value> = items.borrow().iter().cloned().collect();
            for item in &items {
                encode_pair(parts, &nested, item, depth + 1)?;
            }
        }
        Value::Hash(h) => {
            let entries: Vec<(HashKey, Value)> = h
                .borrow()
                .iter()
                .map(|(k, val)| (k.clone(), val.clone()))
                .collect();
            for (k, val) in &entries {
                let sub = match k {
                    HashKey::String(s) | HashKey::Symbol(s) => s.to_string(),
                    _ => continue,
                };
                encode_pair(parts, &format!("{key}[{sub}]"), val, depth + 1)?;
            }
        }
        other => {
            let rendered = match other {
                Value::String(s) => s.to_string(),
                Value::Int(n) => n.to_string(),
                Value::Float(f) => f.to_string(),
                Value::Bool(b) => b.to_string(),
                // Decimals and the like still have a faithful text form; using
                // it beats dropping the parameter.
                _ => crate::interpreter::value_stringify::stringify_to_string(other)
                    .unwrap_or_else(|_| other.type_name().to_string()),
            };
            parts.push(format!(
                "{}={}",
                urlencoding::encode(key),
                urlencoding::encode(&rendered)
            ));
        }
    }
    Ok(())
}

/// Serialize a parsed URL back to a string, applying overrides from an
/// options hash: any of scheme/host/port/path/query/fragment may be given;
/// `query` accepts either a string or a params hash; `null` clears it.
pub fn build_from_hash(base: &url::Url, opts: &HashPairs) -> Result<String, String> {
    let mut u = base.clone();
    for (k, v) in opts.iter() {
        let key = match k {
            HashKey::String(s) | HashKey::Symbol(s) => s.as_str(),
            _ => continue,
        };
        match key {
            "scheme" => match v {
                Value::String(s) => {
                    u.set_scheme(s)
                        .map_err(|e| format!("Url.build: bad scheme: {e:?}"))?;
                }
                _ => return Err("Url.build: \"scheme\" expects a string".to_string()),
            },
            "host" => match v {
                Value::String(s) => {
                    u.set_host(Some(s))
                        .map_err(|e| format!("Url.build: bad host: {e:?}"))?;
                }
                Value::Null => {
                    u.set_host(None)
                        .map_err(|e| format!("Url.build: bad host: {e:?}"))?;
                }
                _ => return Err("Url.build: \"host\" expects a string".to_string()),
            },
            "port" => match v {
                Value::Int(n) if *n > 0 && *n <= 65535 => {
                    u.set_port(Some(*n as u16))
                        .map_err(|e| format!("Url.build: bad port: {e:?}"))?;
                }
                Value::Null => {
                    u.set_port(None)
                        .map_err(|e| format!("Url.build: bad port: {e:?}"))?;
                }
                _ => {
                    return Err(
                        "Url.build: \"port\" expects an Int in 1..=65535 or null".to_string()
                    )
                }
            },
            "path" => match v {
                Value::String(s) => u.set_path(s),
                _ => return Err("Url.build: \"path\" expects a string".to_string()),
            },
            "query" => match v {
                Value::String(s) => {
                    u.set_query(Some(s));
                }
                Value::Hash(h) => {
                    let s = encode_query(&h.borrow())
                        .map_err(|e| format!("Url.build: \"query\": {e}"))?;
                    u.set_query(if s.is_empty() { None } else { Some(&s) });
                }
                Value::Null => u.set_query(None),
                _ => return Err("Url.build: \"query\" expects a string, hash, or null".to_string()),
            },
            "fragment" => match v {
                Value::String(s) => u.set_fragment(Some(s)),
                Value::Null => u.set_fragment(None),
                _ => return Err("Url.build: \"fragment\" expects a string or null".to_string()),
            },
            // `Url.parse` emits these, so ignoring them made the natural
            // round-trip `Url.build(Url.parse(u))` silently strip credentials
            // from an authenticated upstream URL.
            "username" => match v {
                Value::String(s) => u
                    .set_username(s)
                    .map_err(|e| format!("Url.build: bad username: {e:?}"))?,
                Value::Null => u
                    .set_username("")
                    .map_err(|e| format!("Url.build: bad username: {e:?}"))?,
                _ => return Err("Url.build: \"username\" expects a string or null".to_string()),
            },
            "password" => match v {
                Value::String(s) => u
                    .set_password(Some(s))
                    .map_err(|e| format!("Url.build: bad password: {e:?}"))?,
                Value::Null => u
                    .set_password(None)
                    .map_err(|e| format!("Url.build: bad password: {e:?}"))?,
                _ => return Err("Url.build: \"password\" expects a string or null".to_string()),
            },
            // Name every key or say so. Silently ignoring the rest turned a
            // typo (`"pathh"`) into a no-op that looked like it worked.
            other => {
                return Err(format!(
                    "Url.build: unknown key \"{other}\" (expected scheme, username, password, \
                     host, port, path, query, or fragment)"
                ))
            }
        }
    }
    Ok(u.to_string())
}

fn url_to_hash(u: &url::Url) -> Value {
    fn opt_s(v: Option<&str>) -> Value {
        match v {
            Some(s) => Value::String(s.to_string().into()),
            None => Value::Null,
        }
    }
    let mut h = HashPairs::default();
    h.insert(
        HashKey::String("scheme".into()),
        Value::String(u.scheme().to_string().into()),
    );
    h.insert(
        HashKey::String("username".into()),
        opt_s(if u.username().is_empty() {
            None
        } else {
            Some(u.username())
        }),
    );
    h.insert(HashKey::String("password".into()), opt_s(u.password()));
    h.insert(HashKey::String("host".into()), opt_s(u.host_str()));
    h.insert(
        HashKey::String("port".into()),
        match u.port() {
            Some(p) => Value::Int(p as i64),
            None => Value::Null,
        },
    );
    h.insert(
        HashKey::String("path".into()),
        Value::String(u.path().to_string().into()),
    );
    h.insert(HashKey::String("query".into()), opt_s(u.query()));
    h.insert(HashKey::String("fragment".into()), opt_s(u.fragment()));
    Value::Hash(Rc::new(RefCell::new(h)))
}

pub fn register_url_class(env: &mut Environment) {
    let mut m: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    // Url.parse(str) -> Hash of components (null where absent).
    m.insert(
        "parse".to_string(),
        Rc::new(NativeFunction::new(
            "Url.parse",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(url_to_hash(&parse_url(s, "Url.parse")?)),
                other => Err(format!(
                    "Url.parse() expects a string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // Url.build(hash) -> String. The hash must include enough to resolve:
    // typically scheme + host (+ path). Other keys override those fields.
    m.insert(
        "build".to_string(),
        Rc::new(NativeFunction::new("Url.build", Some(1), |args| {
            match &args[0] {
                Value::Hash(h) => {
                    // A minimal valid anchor URL to mutate; every field the
                    // caller supplies overrides it.
                    let base = url::Url::parse("http://localhost/").expect("anchor url");
                    Ok(Value::String(build_from_hash(&base, &h.borrow())?.into()))
                }
                other => Err(format!(
                    "Url.build() expects a hash, got {}",
                    other.type_name()
                )),
            }
        })),
    );

    // Url.join(base, relative) -> resolved absolute URL string.
    m.insert(
        "join".to_string(),
        Rc::new(NativeFunction::new("Url.join", Some(2), |args| {
            match (&args[0], &args[1]) {
                (Value::String(base), Value::String(rel)) => {
                    let b = parse_url(base, "Url.join")?;
                    let joined = b.join(rel.trim()).map_err(|e| format!("Url.join: {e}"))?;
                    Ok(Value::String(joined.to_string().into()))
                }
                _ => Err("Url.join() expects (string, string)".to_string()),
            }
        })),
    );

    // Url.params(url) -> Hash of decoded query parameters.
    m.insert(
        "params".to_string(),
        Rc::new(NativeFunction::new("Url.params", Some(1), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Url.params() expects a string, got {}",
                        other.type_name()
                    ))
                }
            };
            let u = parse_url(&s, "Url.params")?;
            let pairs = match u.query() {
                Some(q) => decode_query(q),
                None => HashPairs::default(),
            };
            Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
        })),
    );

    // Url.param(url, name) -> decoded value string, or null when absent.
    m.insert(
        "param".to_string(),
        Rc::new(NativeFunction::new("Url.param", Some(2), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Url.param() expects a URL string, got {}",
                        other.type_name()
                    ))
                }
            };
            let name = match &args[1] {
                Value::String(n) => n.clone(),
                other => {
                    return Err(format!(
                        "Url.param() expects a param name string, got {}",
                        other.type_name()
                    ))
                }
            };
            let u = parse_url(&s, "Url.param")?;
            let pairs = match u.query() {
                Some(q) => decode_query(q),
                None => return Ok(Value::Null),
            };
            Ok(pairs
                .get(&HashKey::String(name.as_str().into()))
                .cloned()
                .unwrap_or(Value::Null))
        })),
    );

    // Url.set_param(url, name, value_or_null) -> new URL string. Setting a
    // param that exists replaces it; setting null removes it.
    m.insert(
        "set_param".to_string(),
        Rc::new(NativeFunction::new("Url.set_param", Some(3), |args| {
            let s = match &args[0] {
                Value::String(s) => s.clone(),
                other => {
                    return Err(format!(
                        "Url.set_param() expects a URL string, got {}",
                        other.type_name()
                    ))
                }
            };
            let name = match &args[1] {
                Value::String(n) => n.clone(),
                other => {
                    return Err(format!(
                        "Url.set_param() expects a param name string, got {}",
                        other.type_name()
                    ))
                }
            };
            let mut u = parse_url(&s, "Url.set_param")?;
            // Rewrite only the pair being addressed and leave every other pair's
            // raw text byte-identical.
            //
            // Decoding the whole query and re-encoding it rewrote params the
            // caller never mentioned: `?name=%FF&page=2` came back as
            // `name=%25FF` (and, before `percent_decode`, as a blank `name=`),
            // and a `+` in an untouched value became `%2B`. Only the target key
            // should change.
            let new_value: Option<String> = match &args[2] {
                Value::Null => None,
                Value::String(v) => Some(urlencoding::encode(v).to_string()),
                Value::Int(n) => Some(urlencoding::encode(&n.to_string()).to_string()),
                other => {
                    return Err(format!(
                        "Url.set_param() value expects string/int/null, got {}",
                        other.type_name()
                    ))
                }
            };
            let encoded_name = urlencoding::encode(name.as_str()).to_string();
            let existing: Vec<String> = u
                .query()
                .unwrap_or("")
                .split('&')
                .filter(|p| !p.is_empty())
                .map(|p| p.to_string())
                .collect();
            let mut out: Vec<String> = Vec::with_capacity(existing.len() + 1);
            let mut replaced = false;
            for pair in existing {
                let raw_key = pair.split_once('=').map(|(k, _)| k).unwrap_or(&pair);
                if decode_query_component(raw_key) == name.as_str() {
                    // First hit takes the new value; later duplicates of the
                    // same key drop, so the result has one entry per key.
                    if !replaced {
                        if let Some(ref v) = new_value {
                            out.push(format!("{encoded_name}={v}"));
                        }
                        replaced = true;
                    }
                    continue;
                }
                out.push(pair);
            }
            if !replaced {
                if let Some(ref v) = new_value {
                    out.push(format!("{encoded_name}={v}"));
                }
            }
            let q = out.join("&");
            u.set_query(if q.is_empty() { None } else { Some(&q) });
            Ok(Value::String(u.to_string().into()))
        })),
    );

    // Url.encode_component(str) -> percent-encoded (spaces become %20).
    m.insert(
        "encode_component".to_string(),
        Rc::new(NativeFunction::new(
            "Url.encode_component",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(urlencoding::encode(s).to_string().into())),
                other => Err(format!(
                    "Url.encode_component() expects a string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    // Url.decode_component(str) -> decoded; invalid escapes are left as-is.
    m.insert(
        "decode_component".to_string(),
        Rc::new(NativeFunction::new(
            "Url.decode_component",
            Some(1),
            |args| match &args[0] {
                Value::String(s) => Ok(Value::String(percent_decode(s).into())),
                other => Err(format!(
                    "Url.decode_component() expects a string, got {}",
                    other.type_name()
                )),
            },
        )),
    );

    let class = Class {
        name: "Url".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: m,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define("Url".to_string(), Value::Class(Rc::new(class)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_components() {
        let u = parse_url("https://u:p@ex.com:8443/a/b?q=1&x#frag", "t").unwrap();
        let Value::Hash(h) = url_to_hash(&u) else {
            panic!()
        };
        let hh = h.borrow();
        assert_eq!(
            hh.get(&HashKey::String("scheme".into())),
            Some(&Value::String("https".into()))
        );
        assert_eq!(
            hh.get(&HashKey::String("username".into())),
            Some(&Value::String("u".into()))
        );
        assert_eq!(
            hh.get(&HashKey::String("password".into())),
            Some(&Value::String("p".into()))
        );
        assert_eq!(
            hh.get(&HashKey::String("host".into())),
            Some(&Value::String("ex.com".into()))
        );
        assert_eq!(
            hh.get(&HashKey::String("port".into())),
            Some(&Value::Int(8443))
        );
        assert_eq!(
            hh.get(&HashKey::String("path".into())),
            Some(&Value::String("/a/b".into()))
        );
        assert_eq!(
            hh.get(&HashKey::String("query".into())),
            Some(&Value::String("q=1&x".into()))
        );
        assert_eq!(
            hh.get(&HashKey::String("fragment".into())),
            Some(&Value::String("frag".into()))
        );
    }

    #[test]
    fn defaults_are_null_not_missing() {
        let u = parse_url("https://ex.com", "t").unwrap();
        let Value::Hash(h) = url_to_hash(&u) else {
            panic!()
        };
        let hh = h.borrow();
        assert_eq!(hh.get(&HashKey::String("port".into())), Some(&Value::Null));
        assert_eq!(hh.get(&HashKey::String("query".into())), Some(&Value::Null));
        assert_eq!(
            hh.get(&HashKey::String("fragment".into())),
            Some(&Value::Null)
        );
        assert_eq!(
            hh.get(&HashKey::String("username".into())),
            Some(&Value::Null)
        );
    }

    #[test]
    fn malformed_url_is_a_clean_error() {
        assert!(parse_url("not a url", "t").is_err());
        assert!(parse_url("http://", "t").is_err());
    }

    #[test]
    fn round_trips_params_through_encoding() {
        let pairs = decode_query("name=Ann%20Lee&flag&n=42");
        assert_eq!(
            pairs.get(&HashKey::String("name".into())),
            Some(&Value::String("Ann Lee".into()))
        );
        assert_eq!(
            pairs.get(&HashKey::String("flag".into())),
            Some(&Value::Null)
        );
        assert_eq!(
            pairs.get(&HashKey::String("n".into())),
            Some(&Value::String("42".into()))
        );
        assert_eq!(
            encode_query(&pairs).expect("flat pairs encode"),
            "name=Ann%20Lee&flag&n=42"
        );
    }

    /// A hash can contain itself, so the bracket expansion must be bounded.
    ///
    /// Recursing on arrays and nested hashes to stop `Url.build` dropping them
    /// introduced an unbounded recursion: `h["self"] = h` aborted the process
    /// with a stack overflow, which does not unwind and so cannot be caught.
    #[test]
    fn encode_query_rejects_a_self_referential_hash() {
        let inner = Rc::new(RefCell::new(HashPairs::default()));
        inner
            .borrow_mut()
            .insert(HashKey::String("a".into()), Value::Int(1));
        // Close the cycle: the hash holds itself under "self".
        let cyclic = Value::Hash(inner.clone());
        inner
            .borrow_mut()
            .insert(HashKey::String("self".into()), cyclic);

        let err = encode_query(&inner.borrow())
            .expect_err("a cyclic hash must be refused, not overflow the stack");
        assert!(
            err.contains("nested too deeply"),
            "expected a depth error, got: {err}"
        );
    }

    #[test]
    fn joins_relative_paths() {
        let base = parse_url("https://ex.com/a/b", "t").unwrap();
        assert_eq!(base.join("c").unwrap().to_string(), "https://ex.com/a/c");
        assert_eq!(
            base.join("/root").unwrap().to_string(),
            "https://ex.com/root"
        );
        assert_eq!(
            base.join("?page=2").unwrap().to_string(),
            "https://ex.com/a/b?page=2"
        );
    }

    #[test]
    fn build_overrides_fields() {
        let base = url::Url::parse("http://localhost/").unwrap();
        let mut opts = HashPairs::default();
        opts.insert(
            HashKey::String("scheme".into()),
            Value::String("https".into()),
        );
        opts.insert(
            HashKey::String("host".into()),
            Value::String("api.ex.com".into()),
        );
        opts.insert(
            HashKey::String("path".into()),
            Value::String("/v1/x".into()),
        );
        let mut q = HashPairs::default();
        q.insert(HashKey::String("page".into()), Value::Int(2));
        q.insert(HashKey::String("q".into()), Value::String("a b".into()));
        opts.insert(
            HashKey::String("query".into()),
            Value::Hash(Rc::new(RefCell::new(q))),
        );
        assert_eq!(
            build_from_hash(&base, &opts).unwrap(),
            "https://api.ex.com/v1/x?page=2&q=a%20b"
        );
    }

    #[test]
    fn build_rejects_bad_values() {
        let base = url::Url::parse("http://localhost/").unwrap();
        let mut opts = HashPairs::default();
        opts.insert(HashKey::String("port".into()), Value::Int(99_999));
        assert!(build_from_hash(&base, &opts).is_err());
        let mut opts = HashPairs::default();
        opts.insert(HashKey::String("scheme".into()), Value::Int(1));
        assert!(build_from_hash(&base, &opts).is_err());
    }
}
