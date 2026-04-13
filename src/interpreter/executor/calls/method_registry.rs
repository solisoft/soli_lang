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
        name: "capitalize",
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
        name: "replace",
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
        name: "zip",
        zero_arg: false,
        ret: "array",
    },
];

pub const HASH_METHODS: &[MethodDef] = &[
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
        name: "filter",
        zero_arg: false,
        ret: "hash",
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
        name: "slice",
        zero_arg: false,
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
        name: "values",
        zero_arg: true,
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
        name: "count",
        zero_arg: true,
        ret: "int",
    },
    MethodDef {
        name: "fields",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "first",
        zero_arg: true,
        ret: "",
    },
    MethodDef {
        name: "includes",
        zero_arg: false,
        ret: "",
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
        name: "limit",
        zero_arg: false,
        ret: "",
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
        name: "select",
        zero_arg: false,
        ret: "",
    },
    MethodDef {
        name: "to_query",
        zero_arg: true,
        ret: "string",
    },
    MethodDef {
        name: "where",
        zero_arg: false,
        ret: "",
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
