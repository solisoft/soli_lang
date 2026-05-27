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

// ---------------------------------------------------------------------------
// Per-type method tables (sorted alphabetically for tab completion)
// ---------------------------------------------------------------------------

pub const INT_METHODS: &[MethodDef] = &[
    MethodDef {
        name: "abs",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "between?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "blank?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "chr",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "divmod",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "next",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "pred",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "succ",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "divmod",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "clamp",
        zero_arg: false,
        ret: "int",
    },
    MethodDef {
        name: "class",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "downto",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "even?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "gcd",
        zero_arg: false,
        ret: "int",
    },
    MethodDef {
        name: "inspect",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "is_a?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "lcm",
        zero_arg: false,
        ret: "int",
    },
    MethodDef {
        name: "negative?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "nil?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "none?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "one?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "odd?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "positive?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "pow",
        zero_arg: false,
        ret: "int",
    },
    MethodDef {
        name: "present?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "sleep",
        zero_arg: true,
        ret: "null",
    },
    MethodDef {
        name: "sqrt",
        zero_arg: true,
        ret: "float",
    },
    MethodDef {
        name: "times",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "to_f",
        zero_arg: true,
        ret: "float",
    },
    MethodDef {
        name: "to_float",
        zero_arg: true,
        ret: "float",
    },
    MethodDef {
        name: "to_s",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "to_string",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "upto",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "zero?",
        zero_arg: true,
        ret: "bool",
    },
];

pub const FLOAT_METHODS: &[MethodDef] = &[
    MethodDef {
        name: "abs",
        zero_arg: true,
        ret: "float",
    },
    MethodDef {
        name: "between?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "blank?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "ceil",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "clamp",
        zero_arg: false,
        ret: "float",
    },
    MethodDef {
        name: "class",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "finite?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "floor",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "infinite?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "inspect",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "is_a?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "nan?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "negative?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "nil?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "positive?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "present?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "round",
        zero_arg: true,
        ret: "float",
    },
    MethodDef {
        name: "sleep",
        zero_arg: true,
        ret: "null",
    },
    MethodDef {
        name: "sqrt",
        zero_arg: true,
        ret: "float",
    },
    MethodDef {
        name: "to_i",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "to_int",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "to_s",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "to_string",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "truncate",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "zero?",
        zero_arg: true,
        ret: "bool",
    },
];

pub const DECIMAL_METHODS: &[MethodDef] = &[
    MethodDef {
        name: "abs",
        zero_arg: true,
        ret: "decimal",
    },
    MethodDef {
        name: "between?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "blank?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "ceil",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "clamp",
        zero_arg: false,
        ret: "decimal",
    },
    MethodDef {
        name: "class",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "floor",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "inspect",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "is_a?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "negative?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "nil?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "positive?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "present?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "round",
        zero_arg: true,
        ret: "decimal",
    },
    MethodDef {
        name: "sqrt",
        zero_arg: true,
        ret: "float",
    },
    MethodDef {
        name: "to_f",
        zero_arg: true,
        ret: "float",
    },
    MethodDef {
        name: "to_float",
        zero_arg: true,
        ret: "float",
    },
    MethodDef {
        name: "to_i",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "to_int",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "to_s",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "to_string",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "truncate",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "zero?",
        zero_arg: true,
        ret: "bool",
    },
];

pub const BOOL_METHODS: &[MethodDef] = &[
    MethodDef {
        name: "blank?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "class",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "inspect",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "is_a?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "nil?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "present?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "to_i",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "to_int",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "to_s",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "to_string",
        zero_arg: true,
        ret: "string",
    },
];

pub const NULL_METHODS: &[MethodDef] = &[
    MethodDef {
        name: "blank?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "class",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "inspect",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "is_a?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "nil?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "present?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "to_a",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "to_array",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "to_f",
        zero_arg: true,
        ret: "float",
    },
    MethodDef {
        name: "to_float",
        zero_arg: true,
        ret: "float",
    },
    MethodDef {
        name: "to_i",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "to_int",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "to_s",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "to_string",
        zero_arg: true,
        ret: "string",
    },
];

pub const SYMBOL_METHODS: &[MethodDef] = &[
    MethodDef {
        name: "blank?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "class",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "inspect",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "is_a?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "nil?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "present?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "to_s",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "to_string",
        zero_arg: true,
        ret: "string",
    },
];

pub const STRING_METHODS: &[MethodDef] = &[
    MethodDef {
        name: "blank?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "bytes",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "bytesize",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "camelize",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "capitalize",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "casecmp",
        zero_arg: false,
        ret: "int",
    },
    MethodDef {
        name: "casecmp?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "chop",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "center",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "chars",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "chomp",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "chr",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "class",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "contains",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "includes?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "count",
        zero_arg: false,
        ret: "int",
    },
    MethodDef {
        name: "delete",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "delete_prefix",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "delete_suffix",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "downcase",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "ascii_only?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "empty?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "ends_with",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "ends_with?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "gsub",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "hex",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "includes?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "index_of",
        zero_arg: false,
        ret: "int",
    },
    MethodDef {
        name: "insert",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "inspect",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "is_a?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "join",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "len",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "length",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "size",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "lines",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "ljust",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "lowercase",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "lpad",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "lstrip",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "match",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "nil?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "oct",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "ord",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "partition",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "present?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "prepend",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "replace",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "replace_all",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "reverse",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "rjust",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "rpad",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "rpartition",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "rstrip",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "scan",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "slugify",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "split",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "squeeze",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "starts_with",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "starts_with?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "sub",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "substring",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "swapcase",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "to_f",
        zero_arg: true,
        ret: "float",
    },
    MethodDef {
        name: "to_float",
        zero_arg: true,
        ret: "float",
    },
    MethodDef {
        name: "to_i",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "to_int",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "parse_json",
        zero_arg: true,
        ret: "hash",
    },
    MethodDef {
        name: "to_h",
        zero_arg: true,
        ret: "hash",
    },
    MethodDef {
        name: "to_s",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "to_string",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "to_sym",
        zero_arg: true,
        ret: "symbol",
    },
    MethodDef {
        name: "tr",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "trim",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "strip",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "succ",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "next",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "truncate",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "upcase",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "uppercase",
        zero_arg: true,
        ret: "string",
    },
];

pub const ARRAY_METHODS: &[MethodDef] = &[
    // `arr.all` (no parens) returns the array itself — convenience for
    // controllers that treat a preloaded has_many accessor like a Rails
    // Relation and call `.all` at the end of a chain.
    MethodDef {
        name: "all",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "all?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "any?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "includes",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "insert",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "order",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "blank?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "class",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "clear",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "compact",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "compact_blank",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "concat",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "count",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "delete",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "delete_at",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "difference",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "drop",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "each",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "each_with_index",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "empty?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "filter",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "find",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "first",
        zero_arg: true,
        ret: "",
    },
    MethodDef {
        name: "flatten",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "get",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "include?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "includes?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "index_of",
        zero_arg: false,
        ret: "int",
    },
    MethodDef {
        name: "inspect",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "intersection",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "is_a?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "join",
        zero_arg: false,
        ret: "string",
    },
    MethodDef {
        name: "last",
        zero_arg: true,
        ret: "",
    },
    MethodDef {
        name: "len",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "length",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "size",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "map",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "max",
        zero_arg: true,
        ret: "",
    },
    MethodDef {
        name: "min",
        zero_arg: true,
        ret: "",
    },
    MethodDef {
        name: "nil?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "pop",
        zero_arg: true,
        ret: "",
    },
    MethodDef {
        name: "present?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "push",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "reduce",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "reject",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "reverse",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "rotate",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "sample",
        zero_arg: true,
        ret: "",
    },
    MethodDef {
        name: "shift",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "shuffle",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "sort",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "sort_by",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "sum",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "unshift",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "take",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "to_json",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "to_string",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "uniq",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "union",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "values_at",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "pluck",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "pick",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "zip",
        zero_arg: false,
        ret: "array",
    },
];

pub const HASH_METHODS: &[MethodDef] = &[
    MethodDef {
        name: "all?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "any?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "assoc",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "blank?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "class",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "clear",
        zero_arg: true,
        ret: "hash",
    },
    MethodDef {
        name: "compact",
        zero_arg: true,
        ret: "hash",
    },
    MethodDef {
        name: "delete",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "delete_if",
        zero_arg: false,
        ret: "hash",
    },
    MethodDef {
        name: "dig",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "each",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "each_key",
        zero_arg: false,
        ret: "hash",
    },
    MethodDef {
        name: "each_value",
        zero_arg: false,
        ret: "hash",
    },
    MethodDef {
        name: "empty?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "entries",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "except",
        zero_arg: false,
        ret: "hash",
    },
    MethodDef {
        name: "fetch",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "fetch_values",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "filter",
        zero_arg: false,
        ret: "hash",
    },
    MethodDef {
        name: "flatten",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "get",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "has_key",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "has_value?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "inspect",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "invert",
        zero_arg: true,
        ret: "hash",
    },
    MethodDef {
        name: "is_a?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "keep_if",
        zero_arg: false,
        ret: "hash",
    },
    MethodDef {
        name: "key",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "keys",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "length",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "size",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "map",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "merge",
        zero_arg: false,
        ret: "hash",
    },
    MethodDef {
        name: "nil?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "present?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "rassoc",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "reject",
        zero_arg: false,
        ret: "hash",
    },
    MethodDef {
        name: "select",
        zero_arg: false,
        ret: "hash",
    },
    MethodDef {
        name: "set",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "shift",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "slice",
        zero_arg: false,
        ret: "hash",
    },
    MethodDef {
        name: "to_h",
        zero_arg: true,
        ret: "hash",
    },
    MethodDef {
        name: "to_json",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "to_string",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "transform_keys",
        zero_arg: false,
        ret: "hash",
    },
    MethodDef {
        name: "transform_values",
        zero_arg: false,
        ret: "hash",
    },
    MethodDef {
        name: "update",
        zero_arg: false,
        ret: "hash",
    },
    MethodDef {
        name: "value?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "values",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "values_at",
        zero_arg: false,
        ret: "array",
    },
];

pub const QUERY_BUILDER_METHODS: &[MethodDef] = &[
    MethodDef {
        name: "all",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "all?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "any?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "blank?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "class",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "compact",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "compact_blank",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "contains",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "count",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "delete_all",
        zero_arg: true,
        ret: "",
    },
    MethodDef {
        name: "drop",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "each",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "empty?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "fields",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "filter",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "find",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "first",
        zero_arg: true,
        ret: "",
    },
    MethodDef {
        name: "flatten",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "includes",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "includes?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "inspect",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "is_a?",
        zero_arg: false,
        ret: "bool",
    },
    MethodDef {
        name: "join",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "last",
        zero_arg: true,
        ret: "",
    },
    MethodDef {
        name: "len",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "length",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "limit",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "map",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "nil?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "offset",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "order",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "present?",
        zero_arg: true,
        ret: "bool",
    },
    MethodDef {
        name: "reduce",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "reverse",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "sample",
        zero_arg: true,
        ret: "",
    },
    MethodDef {
        name: "select",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "shuffle",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "size",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "sort",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "sort_by",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "take",
        zero_arg: false,
        ret: "array",
    },
    MethodDef {
        name: "to_a",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "to_array",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "to_json",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "to_query",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "to_string",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "uniq",
        zero_arg: true,
        ret: "array",
    },
    MethodDef {
        name: "where",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "zip",
        zero_arg: false,
        ret: "array",
    },
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
