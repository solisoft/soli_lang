//! Central registry of built-in method metadata per type.
//!
//! Single source of truth for method names, zero-arg status, and return types.
//! Used by: tab completion (repl_tui), auto-invoke detection (expressions.rs).

use crate::interpreter::value::Value;

pub struct MethodDef {
    pub name: &'static str,
    pub zero_arg: bool,
    /// Return type name. "" means same type as receiver.
    pub ret: &'static str,
}

/// One table row: `md(name, zero_arg, ret)`.
const fn md(name: &'static str, zero_arg: bool, ret: &'static str) -> MethodDef {
    MethodDef {
        name,
        zero_arg,
        ret,
    }
}

// ---------------------------------------------------------------------------
// Per-type method tables (sorted alphabetically for tab completion)
// ---------------------------------------------------------------------------

pub const INT_METHODS: &[MethodDef] = &[
    md("abs", true, "int"),
    md("between?", false, "bool"),
    md("blank?", true, "bool"),
    md("chr", true, "string"),
    md("divmod", false, "array"),
    md("next", true, "int"),
    md("pred", true, "int"),
    md("succ", true, "int"),
    md("divmod", false, "array"),
    md("clamp", false, "int"),
    md("class", true, "string"),
    md("downto", false, ""),
    md("even?", true, "bool"),
    md("gcd", false, "int"),
    md("inspect", true, "string"),
    md("is_a?", false, "bool"),
    md("lcm", false, "int"),
    md("negative?", true, "bool"),
    md("nil?", true, "bool"),
    md("odd?", true, "bool"),
    md("positive?", true, "bool"),
    md("pow", false, "int"),
    md("present?", true, "bool"),
    md("sleep", true, "null"),
    md("sqrt", true, "float"),
    md("times", false, ""),
    md("to_f", true, "float"),
    md("to_float", true, "float"),
    md("to_i", true, "int"),
    md("to_int", true, "int"),
    md("to_s", true, "string"),
    md("to_string", true, "string"),
    md("upto", false, ""),
    md("zero?", true, "bool"),
];

pub const FLOAT_METHODS: &[MethodDef] = &[
    md("abs", true, "float"),
    md("between?", false, "bool"),
    md("blank?", true, "bool"),
    md("ceil", true, "int"),
    md("clamp", false, "float"),
    md("class", true, "string"),
    md("finite?", true, "bool"),
    md("floor", true, "int"),
    md("infinite?", true, "bool"),
    md("inspect", true, "string"),
    md("is_a?", false, "bool"),
    md("nan?", true, "bool"),
    md("negative?", true, "bool"),
    md("nil?", true, "bool"),
    md("positive?", true, "bool"),
    md("present?", true, "bool"),
    md("round", true, "float"),
    md("sleep", true, "null"),
    md("sqrt", true, "float"),
    md("to_f", true, "float"),
    md("to_float", true, "float"),
    md("to_i", true, "int"),
    md("to_int", true, "int"),
    md("to_s", true, "string"),
    md("to_string", true, "string"),
    md("truncate", true, "int"),
    md("zero?", true, "bool"),
];

pub const DECIMAL_METHODS: &[MethodDef] = &[
    md("abs", true, "decimal"),
    md("between?", false, "bool"),
    md("blank?", true, "bool"),
    md("ceil", true, "int"),
    md("clamp", false, "decimal"),
    md("class", true, "string"),
    md("floor", true, "int"),
    md("inspect", true, "string"),
    md("is_a?", false, "bool"),
    md("negative?", true, "bool"),
    md("nil?", true, "bool"),
    md("positive?", true, "bool"),
    md("present?", true, "bool"),
    md("round", true, "decimal"),
    md("sqrt", true, "float"),
    md("to_f", true, "float"),
    md("to_float", true, "float"),
    md("to_i", true, "int"),
    md("to_int", true, "int"),
    md("to_s", true, "string"),
    md("to_string", true, "string"),
    md("truncate", true, "int"),
    md("zero?", true, "bool"),
];

pub const BOOL_METHODS: &[MethodDef] = &[
    md("blank?", true, "bool"),
    md("class", true, "string"),
    md("inspect", true, "string"),
    md("is_a?", false, "bool"),
    md("nil?", true, "bool"),
    md("present?", true, "bool"),
    md("to_i", true, "int"),
    md("to_int", true, "int"),
    md("to_s", true, "string"),
    md("to_string", true, "string"),
];

pub const NULL_METHODS: &[MethodDef] = &[
    md("blank?", true, "bool"),
    md("class", true, "string"),
    md("inspect", true, "string"),
    md("is_a?", false, "bool"),
    md("nil?", true, "bool"),
    md("present?", true, "bool"),
    md("to_a", true, "array"),
    md("to_array", true, "array"),
    md("to_f", true, "float"),
    md("to_float", true, "float"),
    md("to_i", true, "int"),
    md("to_int", true, "int"),
    md("to_s", true, "string"),
    md("to_string", true, "string"),
];

pub const SYMBOL_METHODS: &[MethodDef] = &[
    md("blank?", true, "bool"),
    md("class", true, "string"),
    md("inspect", true, "string"),
    md("is_a?", false, "bool"),
    md("nil?", true, "bool"),
    md("present?", true, "bool"),
    md("to_s", true, "string"),
    md("to_string", true, "string"),
];

pub const STRING_METHODS: &[MethodDef] = &[
    md("blank?", true, "bool"),
    md("bytes", true, "array"),
    md("bytesize", true, "int"),
    md("camelize", true, "string"),
    md("capitalize", true, "string"),
    md("casecmp", false, "int"),
    md("casecmp?", false, "bool"),
    md("chop", true, "string"),
    md("center", false, "string"),
    md("chars", true, "array"),
    md("chomp", true, "string"),
    md("chr", true, "string"),
    md("class", true, "string"),
    md("contains", false, "bool"),
    md("includes?", false, "bool"),
    md("count", false, "int"),
    md("delete", false, "string"),
    md("delete_prefix", false, "string"),
    md("delete_suffix", false, "string"),
    md("downcase", true, "string"),
    md("html_entities", true, "string"),
    md("ascii_only?", true, "bool"),
    md("empty?", true, "bool"),
    md("ends_with", false, "bool"),
    md("ends_with?", false, "bool"),
    md("gsub", false, "string"),
    md("hex", true, "string"),
    md("includes?", false, "bool"),
    md("index_of", false, "int"),
    md("insert", false, "string"),
    md("inspect", true, "string"),
    md("is_a?", false, "bool"),
    md("join", false, "string"),
    md("len", true, "int"),
    md("length", true, "int"),
    md("size", true, "int"),
    md("lines", true, "array"),
    md("ljust", false, "string"),
    md("lowercase", true, "string"),
    md("lpad", false, "string"),
    md("lstrip", true, "string"),
    md("match", false, ""),
    md("nil?", true, "bool"),
    md("oct", true, "string"),
    md("ord", true, "int"),
    md("partition", false, "array"),
    md("present?", true, "bool"),
    md("prepend", false, "string"),
    md("replace", false, "string"),
    md("replace_all", false, "string"),
    md("reverse", true, "string"),
    md("rjust", false, "string"),
    md("rpad", false, "string"),
    md("rpartition", false, "array"),
    md("rstrip", true, "string"),
    md("scan", false, "array"),
    md("slugify", true, "string"),
    md("split", true, "array"),
    md("squeeze", true, "string"),
    md("starts_with", false, "bool"),
    md("starts_with?", false, "bool"),
    md("sub", false, "string"),
    md("substring", false, "string"),
    md("swapcase", true, "string"),
    md("to_f", true, "float"),
    md("to_float", true, "float"),
    md("to_i", true, "int"),
    md("to_int", true, "int"),
    md("parse_json", true, "hash"),
    md("to_h", true, "hash"),
    md("to_s", true, "string"),
    md("to_string", true, "string"),
    md("to_sym", true, "symbol"),
    md("tr", false, "string"),
    md("trim", true, "string"),
    md("strip", true, "string"),
    md("succ", true, "string"),
    md("next", true, "string"),
    md("truncate", false, "string"),
    md("upcase", true, "string"),
    md("uppercase", true, "string"),
];

pub const ARRAY_METHODS: &[MethodDef] = &[
    // `arr.all` (no parens) returns the array itself — convenience for
    // controllers that treat a preloaded has_many accessor like a Rails
    // Relation and call `.all` at the end of a chain.
    md("all", true, "array"),
    md("all?", false, "bool"),
    md("any?", false, "bool"),
    md("includes", false, "array"),
    md("insert", false, "array"),
    md("order", false, "array"),
    md("blank?", true, "bool"),
    md("class", true, "string"),
    md("clear", true, "array"),
    md("compact", true, "array"),
    md("compact_blank", true, "array"),
    md("concat", false, "array"),
    md("count", true, "int"),
    md("delete", false, "array"),
    md("delete_at", false, "array"),
    md("difference", false, "array"),
    md("drop", false, "array"),
    md("each", false, ""),
    // Bare, it answers the `[item, index]` pairs: `xs.each_with_index.map`.
    md("each_with_index", true, "array"),
    md("empty?", true, "bool"),
    md("filter", false, "array"),
    md("find", false, ""),
    md("first", true, ""),
    md("flatten", true, "array"),
    md("get", false, ""),
    md("include?", false, "bool"),
    md("includes?", false, "bool"),
    md("index_of", false, "int"),
    md("inspect", true, "string"),
    md("intersection", false, "array"),
    md("is_a?", false, "bool"),
    md("join", false, "string"),
    md("last", true, ""),
    md("len", true, "int"),
    md("length", true, "int"),
    md("size", true, "int"),
    md("map", false, "array"),
    md("max", true, ""),
    md("min", true, ""),
    md("nil?", true, "bool"),
    md("pop", true, ""),
    md("present?", true, "bool"),
    md("push", false, "array"),
    md("reduce", false, ""),
    md("reject", false, "array"),
    md("reverse", true, "array"),
    md("rotate", true, "array"),
    md("sample", true, ""),
    md("shift", true, "array"),
    md("shuffle", true, "array"),
    md("sort", true, "array"),
    md("sort_by", false, "array"),
    md("sum", true, "int"),
    md("unshift", false, "array"),
    md("take", false, "array"),
    md("to_json", true, "string"),
    md("to_string", true, "string"),
    md("uniq", true, "array"),
    md("union", false, "array"),
    md("values_at", false, "array"),
    md("pluck", false, "array"),
    md("pick", false, ""),
    md("zip", false, "array"),
];

pub const HASH_METHODS: &[MethodDef] = &[
    md("all?", false, "bool"),
    md("any?", false, "bool"),
    md("assoc", false, "array"),
    md("blank?", true, "bool"),
    md("class", true, "string"),
    md("clear", true, "hash"),
    md("compact", true, "hash"),
    md("delete", false, ""),
    md("delete_if", false, "hash"),
    md("dig", false, ""),
    md("each", false, ""),
    md("each_key", false, "hash"),
    md("each_value", false, "hash"),
    md("empty?", true, "bool"),
    md("entries", true, "array"),
    md("except", false, "hash"),
    md("fetch", false, ""),
    md("fetch_values", false, "array"),
    md("filter", false, "hash"),
    md("flatten", true, "array"),
    md("get", false, ""),
    md("has_key", false, "bool"),
    md("has_value?", false, "bool"),
    md("inspect", true, "string"),
    md("invert", true, "hash"),
    md("is_a?", false, "bool"),
    md("keep_if", false, "hash"),
    md("key", false, ""),
    md("keys", true, "array"),
    md("length", true, "int"),
    md("size", true, "int"),
    md("map", false, "array"),
    md("merge", false, "hash"),
    md("nil?", true, "bool"),
    md("present?", true, "bool"),
    md("rassoc", false, "array"),
    md("reject", false, "hash"),
    md("select", false, "hash"),
    md("set", false, ""),
    md("shift", true, "array"),
    md("slice", false, "hash"),
    md("to_h", true, "hash"),
    md("to_json", true, "string"),
    md("to_string", true, "string"),
    md("transform_keys", false, "hash"),
    md("transform_values", false, "hash"),
    md("update", false, "hash"),
    md("value?", false, "bool"),
    md("values", true, "array"),
    md("values_at", false, "array"),
];

pub const QUERY_BUILDER_METHODS: &[MethodDef] = &[
    md("all", true, "array"),
    md("all?", false, "bool"),
    md("any?", false, "bool"),
    md("blank?", true, "bool"),
    md("class", true, "string"),
    md("compact", true, "array"),
    md("compact_blank", true, "array"),
    md("contains", false, "bool"),
    md("count", true, "int"),
    md("delete_all", true, ""),
    md("drop", false, "array"),
    md("each", false, ""),
    md("empty?", true, "bool"),
    md("fields", false, ""),
    md("filter", false, "array"),
    md("find", false, ""),
    md("find_each", false, ""),
    md("find_in_batches", false, ""),
    md("first", true, ""),
    md("flatten", true, "array"),
    md("includes", false, ""),
    md("includes?", false, "bool"),
    md("in_batches", false, ""),
    md("inspect", true, "string"),
    md("is_a?", false, "bool"),
    md("join", false, ""),
    md("last", true, ""),
    md("len", true, "int"),
    md("length", true, "int"),
    md("limit", false, ""),
    md("map", false, "array"),
    md("nil?", true, "bool"),
    md("offset", false, ""),
    md("order", false, ""),
    md("present?", true, "bool"),
    md("reduce", false, ""),
    md("reverse", true, "array"),
    md("sample", true, ""),
    md("select", false, ""),
    md("shuffle", true, "array"),
    md("size", true, "int"),
    md("sort", true, "array"),
    md("sort_by", false, "array"),
    md("take", false, "array"),
    md("timeout", false, ""),
    md("to_a", true, "array"),
    md("to_array", true, "array"),
    md("to_json", true, "string"),
    md("to_query", true, "string"),
    md("to_string", true, "string"),
    md("uniq", true, "array"),
    md("update_all", false, ""),
    md("where", false, ""),
    md("zip", false, "array"),
];

// ---------------------------------------------------------------------------
// Lookup functions
// ---------------------------------------------------------------------------

/// All method definitions for a type (already sorted alphabetically).
pub fn known_methods(type_name: &str) -> &'static [MethodDef] {
    match type_name {
        "int" => INT_METHODS,
        "float" => FLOAT_METHODS,
        "decimal" => DECIMAL_METHODS,
        "bool" => BOOL_METHODS,
        "null" => NULL_METHODS,
        "string" => STRING_METHODS,
        "symbol" => SYMBOL_METHODS,
        "array" => ARRAY_METHODS,
        "hash" => HASH_METHODS,
        "query_builder" => QUERY_BUILDER_METHODS,
        _ => &[],
    }
}

/// Is this method a zero-arg built-in for the given receiver value?
pub fn is_zero_arg_method(method_name: &str, receiver: &Value) -> bool {
    use super::user_methods::{has_user_methods, lookup_user_method, PrimType};

    // User-defined methods on primitives may also be zero-arg. Gated by the
    // same atomic flag so this is a no-op when no user methods exist.
    let user_prim = match receiver {
        Value::Int(_) => Some(PrimType::Int),
        Value::Float(_) => Some(PrimType::Float),
        Value::Bool(_) => Some(PrimType::Bool),
        Value::Null => Some(PrimType::Null),
        Value::Decimal(_) => Some(PrimType::Decimal),
        Value::String(_) => Some(PrimType::String),
        Value::Symbol(_) => Some(PrimType::Symbol),
        Value::Array(_) => Some(PrimType::Array),
        Value::Hash(_) => Some(PrimType::Hash),
        _ => None,
    };
    if let Some(t) = user_prim {
        if has_user_methods(t) {
            if let Some(f) = lookup_user_method(t, method_name) {
                return f.params.is_empty();
            }
        }
    }

    let type_name = match receiver {
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Decimal(_) => "decimal",
        Value::Bool(_) => "bool",
        Value::Null => "null",
        Value::String(_) => "string",
        Value::Symbol(_) => "symbol",
        Value::Array(_) => "array",
        Value::Hash(_) => "hash",
        Value::QueryBuilder(_) => "query_builder",
        _ => return false,
    };
    known_methods(type_name)
        .iter()
        .any(|m| m.name == method_name && m.zero_arg)
}

/// Return type of a method on a given type. Returns `None` if unknown.
pub fn method_return_type(type_name: &str, method_name: &str) -> Option<&'static str> {
    // Resolve the static type string so the return is always 'static.
    let static_type: &'static str = match type_name {
        "int" => "int",
        "float" => "float",
        "decimal" => "decimal",
        "bool" => "bool",
        "null" => "null",
        "string" => "string",
        "symbol" => "symbol",
        "array" => "array",
        "hash" => "hash",
        "query_builder" => "query_builder",
        _ => return None,
    };
    known_methods(static_type)
        .iter()
        .find(|m| m.name == method_name)
        .map(|m| if m.ret.is_empty() { static_type } else { m.ret })
}

#[cfg(test)]
mod registry_reality_tests {
    use super::*;
    use crate::interpreter::Interpreter;
    use crate::lexer::Scanner;
    use crate::parser::Parser;

    /// Run a snippet, returning the error text if it failed.
    fn run(src: &str) -> Result<(), String> {
        let tokens = Scanner::new(src).scan_tokens().map_err(|e| e.to_string())?;
        let program = Parser::new(tokens).parse().map_err(|e| e.to_string())?;
        Interpreter::new()
            .interpret(&program)
            .map_err(|e| e.to_string())
    }

    /// Every method this registry advertises must actually resolve.
    ///
    /// The registry calls itself the single source of truth, but nothing
    /// dispatches through it — it drives tab completion and auto-invoke, so a
    /// wrong entry is invisible until a user tab-completes a method that does
    /// not exist. Three had drifted: `chr` on string (the registry, the type
    /// checker and the member whitelist all claimed it while both engines
    /// lacked the dispatch arm, so `soli check` accepted a call the runtime
    /// refused), and `none?`/`one?` on int, which are array predicates that
    /// were never int methods.
    ///
    /// This walks every entry and asserts the runtime knows the name. It does
    /// NOT assert behaviour — only that dispatch finds it — so it stays cheap
    /// and does not duplicate the per-method suites.
    ///
    /// `query_builder` is skipped: its methods need a live database.
    #[test]
    fn every_registered_method_resolves_at_runtime() {
        // Receiver literal per registered type.
        let receivers: &[(&str, &str)] = &[
            ("int", "7"),
            ("float", "2.5"),
            ("decimal", "1.5d"),
            ("bool", "true"),
            ("null", "null"),
            ("symbol", ":sym"),
            ("string", "\"abc\""),
            ("array", "[3, 1, 2]"),
            ("hash", "{\"a\": 1}"),
        ];
        // Argument shapes to try; a method "exists" if any shape gets past
        // name resolution. Wrong arity or a type error both mean it was found.
        let arg_shapes = ["", "1", "\"a\"", "\"a\", \"b\"", "fn(x) x", "1, 2"];

        let mut absent: Vec<String> = Vec::new();
        for (type_name, receiver) in receivers {
            for def in known_methods(type_name) {
                let shapes: &[&str] = if def.zero_arg { &[""] } else { &arg_shapes };
                let found = shapes.iter().any(|args| {
                    let src = format!("let _ = {receiver}.{}({args})\n", def.name);
                    match run(&src) {
                        Ok(()) => true,
                        Err(msg) => {
                            !msg.contains("Cannot access property")
                                && !msg.contains("Unknown method")
                        }
                    }
                });
                if !found {
                    absent.push(format!("{type_name}.{}", def.name));
                }
            }
        }
        assert!(
            absent.is_empty(),
            "the registry advertises methods that do not resolve at runtime \
             (either implement them or drop the entry): {absent:?}"
        );
    }
}
