//! Type environment for the type checker.

use std::collections::HashMap;

use crate::types::type_repr::{ClassType, EnumType, InterfaceType, MethodInfo, Type};

/// A type environment tracking types of variables and declarations.
#[derive(Debug, Clone)]
pub struct TypeEnvironment {
    scopes: Vec<HashMap<String, Type>>,
    classes: HashMap<String, ClassType>,
    enums: HashMap<String, EnumType>,
    interfaces: HashMap<String, InterfaceType>,
    functions: HashMap<String, Type>,
    current_class: Option<String>,
    current_function_return: Option<Type>,
}

impl TypeEnvironment {
    pub fn new() -> Self {
        let mut env = Self {
            scopes: vec![HashMap::new()],
            classes: HashMap::new(),
            enums: HashMap::new(),
            interfaces: HashMap::new(),
            functions: HashMap::new(),
            current_class: None,
            current_function_return: None,
        };

        // Register built-in functions
        env.register_builtins();

        // Built-in globals injected at request time by the server (see call_handler).
        env.define("params".to_string(), Type::Any);

        // Class-object globals for primitive types — used for metaprogramming
        // (e.g. `Int.class_eval do define_method(:double) { ... } end`). Typed
        // as Any here so that `.method_name` access doesn't fail type-checking;
        // the runtime resolves the actual Class object via `register_builtins`
        // / `register_primitive_classes`.
        for name in [
            "Int", "Float", "Bool", "Decimal", "String", "Array", "Hash", "Null", "Symbol",
        ] {
            env.define(name.to_string(), Type::Any);
        }

        env
    }

    fn register_builtins(&mut self) {
        // print(...) -> Void
        self.functions.insert(
            "print".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Void),
            },
        );

        // println(...) -> Void
        self.functions.insert(
            "println".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Void),
            },
        );

        // puts(...) -> Void (alias for println)
        self.functions.insert(
            "puts".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Void),
            },
        );

        // grouped(fn() { ... }) -> Any — request-coalescing batch; runs the
        // block and returns its value (the DB reads inside are combined into a
        // single round-trip).
        self.functions.insert(
            "grouped".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            },
        );

        // input(String?) -> String
        self.functions.insert(
            "input".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::String),
            },
        );

        // len(Array|String) -> Int
        self.functions.insert(
            "len".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Int),
            },
        );

        // str(Any) -> String
        self.functions.insert(
            "str".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::String),
            },
        );

        // int(Any) -> Int
        self.functions.insert(
            "int".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Int),
            },
        );

        // float(Any) -> Float
        self.functions.insert(
            "float".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Float),
            },
        );

        // type(Any) -> String
        self.functions.insert(
            "type".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::String),
            },
        );

        // clock() -> Float
        self.functions.insert(
            "clock".to_string(),
            Type::Function {
                params: vec![],
                return_type: Box::new(Type::Float),
            },
        );

        // const_get(String) -> Any — resolve a name (class/function/var) at runtime.
        // Used by the auth Policy layer (`const_get(record.class + "Policy")`).
        self.functions.insert(
            "const_get".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Any),
            },
        );

        // current_action() -> String — the controller action name for the
        // in-flight request (e.g. "update"). Empty string outside a request.
        // Lets `authorize(record)` infer the policy method without an argument.
        self.functions.insert(
            "current_action".to_string(),
            Type::Function {
                params: vec![],
                return_type: Box::new(Type::String),
            },
        );

        // forbidden(message?) -> Void — raise a 403 authorization error.
        self.functions.insert(
            "forbidden".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Void),
            },
        );

        // range(Int, Int) -> Int[]
        self.functions.insert(
            "range".to_string(),
            Type::Function {
                params: vec![Type::Int, Type::Int],
                return_type: Box::new(Type::Array(Box::new(Type::Int))),
            },
        );

        // abs(Int|Float) -> Int|Float
        self.functions.insert(
            "abs".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            },
        );

        // min, max, sqrt, pow
        for name in ["min", "max", "pow"] {
            self.functions.insert(
                name.to_string(),
                Type::Function {
                    params: vec![Type::Any, Type::Any],
                    return_type: Box::new(Type::Any),
                },
            );
        }

        self.functions.insert(
            "sqrt".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Float),
            },
        );

        // push, pop
        self.functions.insert(
            "push".to_string(),
            Type::Function {
                params: vec![Type::Any, Type::Any],
                return_type: Box::new(Type::Void),
            },
        );

        self.functions.insert(
            "pop".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            },
        );

        // Hash functions
        // keys(Hash) -> Array
        self.functions.insert(
            "keys".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(Type::Any))),
            },
        );

        // values(Hash) -> Array
        self.functions.insert(
            "values".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(Type::Any))),
            },
        );

        // has_key(Hash, Any) -> Bool
        self.functions.insert(
            "has_key".to_string(),
            Type::Function {
                params: vec![Type::Any, Type::Any],
                return_type: Box::new(Type::Bool),
            },
        );

        // delete(Hash, Any) -> Any
        self.functions.insert(
            "delete".to_string(),
            Type::Function {
                params: vec![Type::Any, Type::Any],
                return_type: Box::new(Type::Any),
            },
        );

        // merge(Hash, Hash) -> Hash
        self.functions.insert(
            "merge".to_string(),
            Type::Function {
                params: vec![Type::Any, Type::Any],
                return_type: Box::new(Type::Any),
            },
        );

        // entries(Hash) -> Array
        self.functions.insert(
            "entries".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Array(Box::new(Type::Any))),
            },
        );

        // clear(Hash|Array) -> Void
        self.functions.insert(
            "clear".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Void),
            },
        );

        // await(Any) -> Any - Explicitly resolve a Future
        self.functions.insert(
            "await".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Any),
            },
        );

        // defined(String) -> Bool - Check if a variable is defined
        self.functions.insert(
            "defined".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Bool),
            },
        );

        // HTTP client functions are exposed via the `HTTP` class
        // (HTTP.get, HTTP.post, HTTP.request, etc.) — see http_class.rs.

        // JSON functions
        // json_parse(String) -> Any
        self.functions.insert(
            "json_parse".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Any),
            },
        );

        // json_stringify(Any) -> String
        self.functions.insert(
            "json_stringify".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::String),
            },
        );

        // url_encode(String) -> String
        self.functions.insert(
            "url_encode".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::String),
            },
        );

        // url_decode(String) -> String
        self.functions.insert(
            "url_decode".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::String),
            },
        );

        // HTTP status check functions
        // http_ok(Hash|Int) -> Bool
        self.functions.insert(
            "http_ok".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Bool),
            },
        );

        // http_success(Hash|Int) -> Bool
        self.functions.insert(
            "http_success".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Bool),
            },
        );

        // http_redirect(Hash|Int) -> Bool
        self.functions.insert(
            "http_redirect".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Bool),
            },
        );

        // http_client_error(Hash|Int) -> Bool
        self.functions.insert(
            "http_client_error".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Bool),
            },
        );

        // http_server_error(Hash|Int) -> Bool
        self.functions.insert(
            "http_server_error".to_string(),
            Type::Function {
                params: vec![Type::Any],
                return_type: Box::new(Type::Bool),
            },
        );

        // Cryptographic functions
        // argon2_hash(String) -> String
        self.functions.insert(
            "argon2_hash".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::String),
            },
        );

        // argon2_verify(String, String) -> Bool
        self.functions.insert(
            "argon2_verify".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::Bool),
            },
        );

        // uuid_v4() / uuid_v7() -> String
        self.functions.insert(
            "uuid_v4".to_string(),
            Type::Function {
                params: vec![],
                return_type: Box::new(Type::String),
            },
        );
        self.functions.insert(
            "uuid_v7".to_string(),
            Type::Function {
                params: vec![],
                return_type: Box::new(Type::String),
            },
        );

        // ulid() -> String
        self.functions.insert(
            "ulid".to_string(),
            Type::Function {
                params: vec![],
                return_type: Box::new(Type::String),
            },
        );

        // nanoid(size?, alphabet?) -> String
        // Variadic 0-2 args; params use Any so the type checker accepts all forms.
        self.functions.insert(
            "nanoid".to_string(),
            Type::Function {
                params: vec![Type::Any, Type::Any],
                return_type: Box::new(Type::String),
            },
        );

        // Environment access
        // getenv(String) -> String|Null  (Any so callers can compare against null)
        self.functions.insert(
            "getenv".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Any),
            },
        );
        // hasenv(String) -> Bool
        self.functions.insert(
            "hasenv".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Bool),
            },
        );

        // File I/O functions
        // barf(String, String|Array<Int>) -> Void
        self.functions.insert(
            "barf".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Any],
                return_type: Box::new(Type::Void),
            },
        );

        // slurp(String, String?) -> String|Array<Int>
        // Note: Function type doesn't easily support overloads, so we use Any for params
        self.functions.insert(
            "slurp".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Any],
                return_type: Box::new(Type::Any),
            },
        );

        // file_write_base64(path, base64_data) -> Bool (decodes + writes bytes)
        self.functions.insert(
            "file_write_base64".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::Bool),
            },
        );

        // pdf_render(template_json, data_json, options?) -> String (base64 PDF)
        self.functions.insert(
            "pdf_render".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String, Type::Any],
                return_type: Box::new(Type::String),
            },
        );

        // pdf_facturx(template_json, data_json, xml, options?) -> String (base64 PDF/A-3b)
        self.functions.insert(
            "pdf_facturx".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String, Type::String, Type::Any],
                return_type: Box::new(Type::String),
            },
        );

        // pdf_facturx_from_invoice(template_json, invoice_json, options?) -> String (base64 PDF/A-3b)
        self.functions.insert(
            "pdf_facturx_from_invoice".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String, Type::Any],
                return_type: Box::new(Type::String),
            },
        );

        // pdf_from_markdown(markdown, options?) -> String (base64 PDF)
        self.functions.insert(
            "pdf_from_markdown".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Any],
                return_type: Box::new(Type::String),
            },
        );

        // pdf_fill(pdf, data, options?) -> String (base64 filled PDF)
        self.functions.insert(
            "pdf_fill".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Any, Type::Any],
                return_type: Box::new(Type::String),
            },
        );

        // pdf_merge(pdfs) -> String (base64 merged PDF)
        self.functions.insert(
            "pdf_merge".to_string(),
            Type::Function {
                params: vec![Type::Array(Box::new(Type::String))],
                return_type: Box::new(Type::String),
            },
        );

        // pdf_pages(pdf, selection) -> String (base64 page subset)
        self.functions.insert(
            "pdf_pages".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Any],
                return_type: Box::new(Type::String),
            },
        );

        // pdf_stamp(pdf, text, options?) -> String (base64 stamped PDF)
        self.functions.insert(
            "pdf_stamp".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String, Type::Any],
                return_type: Box::new(Type::String),
            },
        );

        // String functions
        // split(String, String) -> Array
        self.functions.insert(
            "split".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::Array(Box::new(Type::String))),
            },
        );

        // join(Array, String) -> String
        self.functions.insert(
            "join".to_string(),
            Type::Function {
                params: vec![Type::Any, Type::String],
                return_type: Box::new(Type::String),
            },
        );

        // contains(String, String) -> Bool
        self.functions.insert(
            "contains".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::Bool),
            },
        );

        // index_of(String, String) -> Int
        self.functions.insert(
            "index_of".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::Int),
            },
        );

        // substring(String, Int, Int) -> String
        self.functions.insert(
            "substring".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Int, Type::Int],
                return_type: Box::new(Type::String),
            },
        );

        // upcase(String) -> String
        self.functions.insert(
            "upcase".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::String),
            },
        );

        // downcase(String) -> String
        self.functions.insert(
            "downcase".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::String),
            },
        );

        // trim(String) -> String
        self.functions.insert(
            "trim".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::String),
            },
        );

        // lpad(String, Int, String?) -> String
        self.functions.insert(
            "lpad".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Int, Type::String],
                return_type: Box::new(Type::String),
            },
        );

        // rpad(String, Int, String?) -> String
        self.functions.insert(
            "rpad".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Int, Type::String],
                return_type: Box::new(Type::String),
            },
        );

        // Cache global functions
        for name in &[
            "cache_set",
            "cache_get",
            "cache_delete",
            "cache_has",
            "cache_clear",
            "cache_clear_expired",
            "cache_keys",
            "cache_size",
            "cache_ttl",
            "cache_touch",
            "cache_config",
            "cache",
        ] {
            self.functions.insert(
                name.to_string(),
                Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Any),
                },
            );
        }

        // SoliDB: `Solidb(host, database)` returns a Solidb instance, plus
        // the standalone `solidb_*` helpers used by migrations and scripts.
        self.functions.insert(
            "Solidb".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::Any),
            },
        );
        for name in &[
            "solidb_connect",
            "solidb_ping",
            "solidb_auth",
            "solidb_query",
        ] {
            self.functions.insert(
                name.to_string(),
                Type::Function {
                    params: vec![Type::Any],
                    return_type: Box::new(Type::Any),
                },
            );
        }

        // Register built-in classes
        self.register_builtin_classes();
    }

    fn register_builtin_classes(&mut self) {
        // DateTime class
        let mut datetime_class = ClassType::new("DateTime".to_string());
        datetime_class.methods.insert(
            "now".to_string(),
            MethodInfo {
                name: "now".to_string(),
                params: vec![],
                return_type: Type::Class(ClassType::new("DateTime".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        datetime_class.methods.insert(
            "utc".to_string(),
            MethodInfo {
                name: "utc".to_string(),
                params: vec![],
                return_type: Type::Class(ClassType::new("DateTime".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        datetime_class.methods.insert(
            "parse".to_string(),
            MethodInfo {
                name: "parse".to_string(),
                params: vec![("s".to_string(), Type::String)],
                return_type: Type::Class(ClassType::new("DateTime".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        datetime_class.methods.insert(
            "epoch".to_string(),
            MethodInfo {
                name: "epoch".to_string(),
                params: vec![],
                return_type: Type::Class(ClassType::new("DateTime".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        datetime_class.methods.insert(
            "from_unix".to_string(),
            MethodInfo {
                name: "from_unix".to_string(),
                params: vec![("ts".to_string(), Type::Int)],
                return_type: Type::Class(ClassType::new("DateTime".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        // Instance methods
        datetime_class.methods.insert(
            "year".to_string(),
            MethodInfo {
                name: "year".to_string(),
                params: vec![],
                return_type: Type::Int,
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "month".to_string(),
            MethodInfo {
                name: "month".to_string(),
                params: vec![],
                return_type: Type::Int,
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "day".to_string(),
            MethodInfo {
                name: "day".to_string(),
                params: vec![],
                return_type: Type::Int,
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "hour".to_string(),
            MethodInfo {
                name: "hour".to_string(),
                params: vec![],
                return_type: Type::Int,
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "minute".to_string(),
            MethodInfo {
                name: "minute".to_string(),
                params: vec![],
                return_type: Type::Int,
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "second".to_string(),
            MethodInfo {
                name: "second".to_string(),
                params: vec![],
                return_type: Type::Int,
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "millisecond".to_string(),
            MethodInfo {
                name: "millisecond".to_string(),
                params: vec![],
                return_type: Type::Int,
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "weekday".to_string(),
            MethodInfo {
                name: "weekday".to_string(),
                params: vec![],
                return_type: Type::String,
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "to_unix".to_string(),
            MethodInfo {
                name: "to_unix".to_string(),
                params: vec![],
                return_type: Type::Int,
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "to_iso".to_string(),
            MethodInfo {
                name: "to_iso".to_string(),
                params: vec![],
                return_type: Type::String,
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "to_string".to_string(),
            MethodInfo {
                name: "to_string".to_string(),
                params: vec![],
                return_type: Type::String,
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "add_days".to_string(),
            MethodInfo {
                name: "add_days".to_string(),
                params: vec![("days".to_string(), Type::Int)],
                return_type: Type::Class(ClassType::new("DateTime".to_string())),
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "add_hours".to_string(),
            MethodInfo {
                name: "add_hours".to_string(),
                params: vec![("hours".to_string(), Type::Int)],
                return_type: Type::Class(ClassType::new("DateTime".to_string())),
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "add_minutes".to_string(),
            MethodInfo {
                name: "add_minutes".to_string(),
                params: vec![("minutes".to_string(), Type::Int)],
                return_type: Type::Class(ClassType::new("DateTime".to_string())),
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "subtract_days".to_string(),
            MethodInfo {
                name: "subtract_days".to_string(),
                params: vec![("days".to_string(), Type::Int)],
                return_type: Type::Class(ClassType::new("DateTime".to_string())),
                is_private: false,
                is_static: false,
            },
        );
        datetime_class.methods.insert(
            "format".to_string(),
            MethodInfo {
                name: "format".to_string(),
                params: vec![("fmt".to_string(), Type::String)],
                return_type: Type::String,
                is_private: false,
                is_static: false,
            },
        );
        // Period-boundary helpers — zero-arg, return a new DateTime.
        // Keep in sync with `datetime_class.rs` registrations.
        for name in [
            "beginning_of_day",
            "beginning_of_hour",
            "beginning_of_minute",
            "beginning_of_month",
            "beginning_of_year",
            "end_of_day",
            "end_of_hour",
            "end_of_minute",
            "end_of_month",
            "end_of_year",
        ] {
            datetime_class.methods.insert(
                name.to_string(),
                MethodInfo {
                    name: name.to_string(),
                    params: vec![],
                    return_type: Type::Class(ClassType::new("DateTime".to_string())),
                    is_private: false,
                    is_static: false,
                },
            );
        }
        self.classes.insert("DateTime".to_string(), datetime_class);

        // Duration class
        let mut duration_class = ClassType::new("Duration".to_string());
        duration_class.methods.insert(
            "between".to_string(),
            MethodInfo {
                name: "between".to_string(),
                params: vec![
                    (
                        "start".to_string(),
                        Type::Class(ClassType::new("DateTime".to_string())),
                    ),
                    (
                        "end".to_string(),
                        Type::Class(ClassType::new("DateTime".to_string())),
                    ),
                ],
                return_type: Type::Class(ClassType::new("Duration".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        duration_class.methods.insert(
            "of_seconds".to_string(),
            MethodInfo {
                name: "of_seconds".to_string(),
                params: vec![("seconds".to_string(), Type::Float)],
                return_type: Type::Class(ClassType::new("Duration".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        duration_class.methods.insert(
            "of_minutes".to_string(),
            MethodInfo {
                name: "of_minutes".to_string(),
                params: vec![("minutes".to_string(), Type::Float)],
                return_type: Type::Class(ClassType::new("Duration".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        duration_class.methods.insert(
            "of_hours".to_string(),
            MethodInfo {
                name: "of_hours".to_string(),
                params: vec![("hours".to_string(), Type::Float)],
                return_type: Type::Class(ClassType::new("Duration".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        duration_class.methods.insert(
            "of_days".to_string(),
            MethodInfo {
                name: "of_days".to_string(),
                params: vec![("days".to_string(), Type::Float)],
                return_type: Type::Class(ClassType::new("Duration".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        duration_class.methods.insert(
            "of_weeks".to_string(),
            MethodInfo {
                name: "of_weeks".to_string(),
                params: vec![("weeks".to_string(), Type::Float)],
                return_type: Type::Class(ClassType::new("Duration".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        // Aliases: seconds, minutes, hours, days, weeks
        duration_class.methods.insert(
            "seconds".to_string(),
            MethodInfo {
                name: "seconds".to_string(),
                params: vec![("seconds".to_string(), Type::Float)],
                return_type: Type::Class(ClassType::new("Duration".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        duration_class.methods.insert(
            "minutes".to_string(),
            MethodInfo {
                name: "minutes".to_string(),
                params: vec![("minutes".to_string(), Type::Float)],
                return_type: Type::Class(ClassType::new("Duration".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        duration_class.methods.insert(
            "hours".to_string(),
            MethodInfo {
                name: "hours".to_string(),
                params: vec![("hours".to_string(), Type::Float)],
                return_type: Type::Class(ClassType::new("Duration".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        duration_class.methods.insert(
            "days".to_string(),
            MethodInfo {
                name: "days".to_string(),
                params: vec![("days".to_string(), Type::Float)],
                return_type: Type::Class(ClassType::new("Duration".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        duration_class.methods.insert(
            "weeks".to_string(),
            MethodInfo {
                name: "weeks".to_string(),
                params: vec![("weeks".to_string(), Type::Float)],
                return_type: Type::Class(ClassType::new("Duration".to_string())),
                is_private: false,
                is_static: true,
            },
        );
        // Duration instance methods
        duration_class.methods.insert(
            "total_seconds".to_string(),
            MethodInfo {
                name: "total_seconds".to_string(),
                params: vec![],
                return_type: Type::Float,
                is_private: false,
                is_static: false,
            },
        );
        duration_class.methods.insert(
            "total_minutes".to_string(),
            MethodInfo {
                name: "total_minutes".to_string(),
                params: vec![],
                return_type: Type::Float,
                is_private: false,
                is_static: false,
            },
        );
        duration_class.methods.insert(
            "total_hours".to_string(),
            MethodInfo {
                name: "total_hours".to_string(),
                params: vec![],
                return_type: Type::Float,
                is_private: false,
                is_static: false,
            },
        );
        duration_class.methods.insert(
            "total_days".to_string(),
            MethodInfo {
                name: "total_days".to_string(),
                params: vec![],
                return_type: Type::Float,
                is_private: false,
                is_static: false,
            },
        );
        duration_class.methods.insert(
            "to_string".to_string(),
            MethodInfo {
                name: "to_string".to_string(),
                params: vec![],
                return_type: Type::String,
                is_private: false,
                is_static: false,
            },
        );
        duration_class.methods.insert(
            "humanize".to_string(),
            MethodInfo {
                name: "humanize".to_string(),
                params: vec![],
                return_type: Type::String,
                is_private: false,
                is_static: false,
            },
        );
        self.classes.insert("Duration".to_string(), duration_class);

        // Regex class
        let mut regex_class = ClassType::new("Regex".to_string());
        regex_class.methods.insert(
            "matches".to_string(),
            MethodInfo {
                name: "matches".to_string(),
                params: vec![
                    ("pattern".to_string(), Type::String),
                    ("string".to_string(), Type::String),
                ],
                return_type: Type::Bool,
                is_private: false,
                is_static: true,
            },
        );
        regex_class.methods.insert(
            "find".to_string(),
            MethodInfo {
                name: "find".to_string(),
                params: vec![
                    ("pattern".to_string(), Type::String),
                    ("string".to_string(), Type::String),
                ],
                return_type: Type::Any,
                is_private: false,
                is_static: true,
            },
        );
        regex_class.methods.insert(
            "find_all".to_string(),
            MethodInfo {
                name: "find_all".to_string(),
                params: vec![
                    ("pattern".to_string(), Type::String),
                    ("string".to_string(), Type::String),
                ],
                return_type: Type::Array(Box::new(Type::Any)),
                is_private: false,
                is_static: true,
            },
        );
        regex_class.methods.insert(
            "replace".to_string(),
            MethodInfo {
                name: "replace".to_string(),
                params: vec![
                    ("pattern".to_string(), Type::String),
                    ("string".to_string(), Type::String),
                    ("replacement".to_string(), Type::String),
                ],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        regex_class.methods.insert(
            "replace_all".to_string(),
            MethodInfo {
                name: "replace_all".to_string(),
                params: vec![
                    ("pattern".to_string(), Type::String),
                    ("string".to_string(), Type::String),
                    ("replacement".to_string(), Type::String),
                ],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        regex_class.methods.insert(
            "split".to_string(),
            MethodInfo {
                name: "split".to_string(),
                params: vec![
                    ("pattern".to_string(), Type::String),
                    ("string".to_string(), Type::String),
                ],
                return_type: Type::Array(Box::new(Type::String)),
                is_private: false,
                is_static: true,
            },
        );
        regex_class.methods.insert(
            "capture".to_string(),
            MethodInfo {
                name: "capture".to_string(),
                params: vec![
                    ("pattern".to_string(), Type::String),
                    ("string".to_string(), Type::String),
                ],
                return_type: Type::Any,
                is_private: false,
                is_static: true,
            },
        );
        regex_class.methods.insert(
            "escape".to_string(),
            MethodInfo {
                name: "escape".to_string(),
                params: vec![("string".to_string(), Type::String)],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        self.classes.insert("Regex".to_string(), regex_class);

        // I18n class
        let mut i18n_class = ClassType::new("I18n".to_string());
        i18n_class.methods.insert(
            "locale".to_string(),
            MethodInfo {
                name: "locale".to_string(),
                params: vec![],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        i18n_class.methods.insert(
            "set_locale".to_string(),
            MethodInfo {
                name: "set_locale".to_string(),
                params: vec![("locale".to_string(), Type::String)],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        i18n_class.methods.insert(
            "translate".to_string(),
            MethodInfo {
                name: "translate".to_string(),
                params: vec![
                    ("key".to_string(), Type::String),
                    ("locale".to_string(), Type::Any),
                    ("translations".to_string(), Type::Any),
                ],
                return_type: Type::Any,
                is_private: false,
                is_static: true,
            },
        );
        i18n_class.methods.insert(
            "plural".to_string(),
            MethodInfo {
                name: "plural".to_string(),
                params: vec![
                    ("key".to_string(), Type::String),
                    ("n".to_string(), Type::Int),
                    ("locale".to_string(), Type::Any),
                    ("translations".to_string(), Type::Any),
                ],
                return_type: Type::Any,
                is_private: false,
                is_static: true,
            },
        );
        i18n_class.methods.insert(
            "format_number".to_string(),
            MethodInfo {
                name: "format_number".to_string(),
                params: vec![
                    ("n".to_string(), Type::Any),
                    ("locale".to_string(), Type::Any),
                ],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        i18n_class.methods.insert(
            "format_currency".to_string(),
            MethodInfo {
                name: "format_currency".to_string(),
                params: vec![
                    ("amount".to_string(), Type::Any),
                    ("currency".to_string(), Type::String),
                    ("locale".to_string(), Type::Any),
                ],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        i18n_class.methods.insert(
            "format_date".to_string(),
            MethodInfo {
                name: "format_date".to_string(),
                params: vec![
                    ("ts".to_string(), Type::Int),
                    ("locale".to_string(), Type::Any),
                ],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        self.classes.insert("I18n".to_string(), i18n_class);

        // SOAP class
        let mut soap_class = ClassType::new("SOAP".to_string());
        soap_class.methods.insert(
            "call".to_string(),
            MethodInfo {
                name: "call".to_string(),
                params: vec![
                    ("url".to_string(), Type::String),
                    ("action".to_string(), Type::String),
                    ("envelope".to_string(), Type::String),
                ],
                return_type: Type::Any,
                is_private: false,
                is_static: true,
            },
        );
        soap_class.methods.insert(
            "wrap".to_string(),
            MethodInfo {
                name: "wrap".to_string(),
                params: vec![("body".to_string(), Type::String)],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        soap_class.methods.insert(
            "parse".to_string(),
            MethodInfo {
                name: "parse".to_string(),
                params: vec![("xml".to_string(), Type::String)],
                return_type: Type::Any,
                is_private: false,
                is_static: true,
            },
        );
        soap_class.methods.insert(
            "xml_escape".to_string(),
            MethodInfo {
                name: "xml_escape".to_string(),
                params: vec![("text".to_string(), Type::String)],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        soap_class.methods.insert(
            "to_xml".to_string(),
            MethodInfo {
                name: "to_xml".to_string(),
                params: vec![("hash".to_string(), Type::Any)],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        self.classes.insert("SOAP".to_string(), soap_class);

        // JSON class
        let mut json_class = ClassType::new("JSON".to_string());
        json_class.methods.insert(
            "parse".to_string(),
            MethodInfo {
                name: "parse".to_string(),
                params: vec![("json".to_string(), Type::String)],
                return_type: Type::Any,
                is_private: false,
                is_static: true,
            },
        );
        json_class.methods.insert(
            "stringify".to_string(),
            MethodInfo {
                name: "stringify".to_string(),
                params: vec![("value".to_string(), Type::Any)],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        self.classes.insert("JSON".to_string(), json_class);

        // HTTP class
        let mut http_class = ClassType::new("HTTP".to_string());
        http_class.methods.insert(
            "get".to_string(),
            MethodInfo {
                name: "get".to_string(),
                params: vec![("url".to_string(), Type::String)],
                return_type: Type::Future(Box::new(Type::String)),
                is_private: false,
                is_static: true,
            },
        );
        http_class.methods.insert(
            "post".to_string(),
            MethodInfo {
                name: "post".to_string(),
                params: vec![
                    ("url".to_string(), Type::String),
                    ("body".to_string(), Type::Any),
                ],
                return_type: Type::Future(Box::new(Type::String)),
                is_private: false,
                is_static: true,
            },
        );
        http_class.methods.insert(
            "put".to_string(),
            MethodInfo {
                name: "put".to_string(),
                params: vec![
                    ("url".to_string(), Type::String),
                    ("body".to_string(), Type::Any),
                ],
                return_type: Type::Future(Box::new(Type::String)),
                is_private: false,
                is_static: true,
            },
        );
        http_class.methods.insert(
            "delete".to_string(),
            MethodInfo {
                name: "delete".to_string(),
                params: vec![("url".to_string(), Type::String)],
                return_type: Type::Future(Box::new(Type::String)),
                is_private: false,
                is_static: true,
            },
        );
        http_class.methods.insert(
            "request".to_string(),
            MethodInfo {
                name: "request".to_string(),
                params: vec![
                    ("method".to_string(), Type::String),
                    ("url".to_string(), Type::String),
                    ("options".to_string(), Type::Any),
                ],
                return_type: Type::Future(Box::new(Type::Any)),
                is_private: false,
                is_static: true,
            },
        );
        http_class.methods.insert(
            "get_all".to_string(),
            MethodInfo {
                name: "get_all".to_string(),
                params: vec![("urls".to_string(), Type::Array(Box::new(Type::String)))],
                return_type: Type::Array(Box::new(Type::Any)),
                is_private: false,
                is_static: true,
            },
        );
        http_class.methods.insert(
            "get_all_json".to_string(),
            MethodInfo {
                name: "get_all_json".to_string(),
                params: vec![("urls".to_string(), Type::Array(Box::new(Type::String)))],
                return_type: Type::Array(Box::new(Type::Any)),
                is_private: false,
                is_static: true,
            },
        );
        self.classes.insert("HTTP".to_string(), http_class);

        // Pop3 email-reading class: `Pop3.new(host, user, pass, opts?)` returns
        // a client instance (typed Any, so its `.stat()/.list()/.fetch()/…`
        // calls are permissive, matching the Solidb instance pattern).
        let mut pop3_class = ClassType::new("Pop3".to_string());
        pop3_class.methods.insert(
            "new".to_string(),
            MethodInfo {
                name: "new".to_string(),
                params: vec![
                    ("host".to_string(), Type::String),
                    ("user".to_string(), Type::String),
                    ("password".to_string(), Type::String),
                    // Optional opts hash; `Any` also makes the arg count lenient.
                    ("opts".to_string(), Type::Any),
                ],
                return_type: Type::Any,
                is_private: false,
                is_static: true,
            },
        );
        self.classes.insert("Pop3".to_string(), pop3_class);

        // System class
        let mut system_class = ClassType::new("System".to_string());
        system_class.methods.insert(
            "run".to_string(),
            MethodInfo {
                name: "run".to_string(),
                params: vec![("command".to_string(), Type::String)],
                return_type: Type::Future(Box::new(Type::Any)),
                is_private: false,
                is_static: true,
            },
        );
        system_class.methods.insert(
            "run_sync".to_string(),
            MethodInfo {
                name: "run_sync".to_string(),
                params: vec![("command".to_string(), Type::String)],
                return_type: Type::Any,
                is_private: false,
                is_static: true,
            },
        );
        self.classes.insert("System".to_string(), system_class);

        // Markdown class
        let mut markdown_class = ClassType::new("Markdown".to_string());
        markdown_class.methods.insert(
            "to_html".to_string(),
            MethodInfo {
                name: "to_html".to_string(),
                params: vec![("markdown".to_string(), Type::String)],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        markdown_class.methods.insert(
            "to_safe_html".to_string(),
            MethodInfo {
                name: "to_safe_html".to_string(),
                params: vec![("markdown".to_string(), Type::String)],
                return_type: Type::String,
                is_private: false,
                is_static: true,
            },
        );
        // Markdown.to_spans(markdown) -> Array of span hashes for PDF inline rich text.
        markdown_class.methods.insert(
            "to_spans".to_string(),
            MethodInfo {
                name: "to_spans".to_string(),
                params: vec![("markdown".to_string(), Type::String)],
                return_type: Type::Array(Box::new(Type::Any)),
                is_private: false,
                is_static: true,
            },
        );
        self.classes.insert("Markdown".to_string(), markdown_class);

        // Cache class
        let mut cache_class = ClassType::new("Cache".to_string());
        for name in &[
            "set",
            "get",
            "delete",
            "has",
            "clear",
            "clear_expired",
            "keys",
            "size",
            "ttl",
            "touch",
            "configure",
        ] {
            cache_class.methods.insert(
                name.to_string(),
                MethodInfo {
                    name: name.to_string(),
                    params: vec![("args".to_string(), Type::Any)],
                    return_type: Type::Any,
                    is_private: false,
                    is_static: true,
                },
            );
        }
        self.classes.insert("Cache".to_string(), cache_class);

        // KV class
        let mut kv_class = ClassType::new("KV".to_string());
        for name in &[
            "set",
            "get",
            "delete",
            "exists",
            "keys",
            "ttl",
            "expire",
            "persist",
            "rename",
            "type",
            "incr",
            "decr",
            "incrby",
            "decrby",
            "incrbyfloat",
            "setnx",
            "getset",
            "getdel",
            "append",
            "strlen",
            "mget",
            "mset",
            "pexpire",
            "pttl",
            "expireat",
            "touch",
            "unlink",
            "lpush",
            "rpush",
            "lpop",
            "rpop",
            "lrange",
            "llen",
            "lindex",
            "lset",
            "lrem",
            "ltrim",
            "rpoplpush",
            "sadd",
            "srem",
            "smembers",
            "sismember",
            "scard",
            "spop",
            "srandmember",
            "sinter",
            "sunion",
            "sdiff",
            "smismember",
            "smove",
            "hset",
            "hget",
            "hdel",
            "hgetall",
            "hexists",
            "hkeys",
            "hlen",
            "hsetnx",
            "hincrby",
            "hincrbyfloat",
            "hmget",
            "hvals",
            "zadd",
            "zrem",
            "zscore",
            "zincrby",
            "zrank",
            "zrevrank",
            "zcard",
            "zcount",
            "zrange",
            "zrevrange",
            "zrangebyscore",
            "setbit",
            "getbit",
            "bitcount",
            "pfadd",
            "pfcount",
            "pfmerge",
            "ping",
            "dbsize",
            "flushdb",
            "cmd",
            "configure",
        ] {
            kv_class.methods.insert(
                name.to_string(),
                MethodInfo {
                    name: name.to_string(),
                    params: vec![("args".to_string(), Type::Any)],
                    return_type: Type::Any,
                    is_private: false,
                    is_static: true,
                },
            );
        }
        self.classes.insert("KV".to_string(), kv_class);

        // UUID class — UUID.v4() / UUID.v7() -> String
        let mut uuid_class = ClassType::new("UUID".to_string());
        for name in &["v4", "v7"] {
            uuid_class.methods.insert(
                name.to_string(),
                MethodInfo {
                    name: name.to_string(),
                    params: vec![],
                    return_type: Type::String,
                    is_private: false,
                    is_static: true,
                },
            );
        }
        self.classes.insert("UUID".to_string(), uuid_class);

        // ULID class — ULID.generate() / ULID.new() -> String
        let mut ulid_class = ClassType::new("ULID".to_string());
        for name in &["generate", "new"] {
            ulid_class.methods.insert(
                name.to_string(),
                MethodInfo {
                    name: name.to_string(),
                    params: vec![],
                    return_type: Type::String,
                    is_private: false,
                    is_static: true,
                },
            );
        }
        self.classes.insert("ULID".to_string(), ulid_class);

        // NanoID class — NanoID.generate(size?, alphabet?) / NanoID.new(...) -> String
        let mut nanoid_class = ClassType::new("NanoID".to_string());
        for name in &["generate", "new"] {
            nanoid_class.methods.insert(
                name.to_string(),
                MethodInfo {
                    name: name.to_string(),
                    params: vec![
                        ("size".to_string(), Type::Any),
                        ("alphabet".to_string(), Type::Any),
                    ],
                    return_type: Type::String,
                    is_private: false,
                    is_static: true,
                },
            );
        }
        self.classes.insert("NanoID".to_string(), nanoid_class);
    }

    /// Enter a new scope.
    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Exit the current scope.
    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Define a variable in the current scope.
    pub fn define(&mut self, name: String, ty: Type) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, ty);
        }
    }

    /// Look up a variable's type.
    pub fn get(&self, name: &str) -> Option<Type> {
        // Search scopes from innermost to outermost
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty.clone());
            }
        }

        // Check functions
        if let Some(ty) = self.functions.get(name) {
            return Some(ty.clone());
        }

        // Check classes (as types)
        if let Some(class) = self.classes.get(name) {
            return Some(Type::Class(class.clone()));
        }

        None
    }

    /// Define a class type.
    pub fn define_class(&mut self, class: ClassType) {
        self.classes.insert(class.name.clone(), class);
    }

    /// Get a class type.
    pub fn get_class(&self, name: &str) -> Option<&ClassType> {
        self.classes.get(name)
    }

    /// Define an enum type (its variant set, for exhaustiveness checking).
    pub fn define_enum(&mut self, enum_type: EnumType) {
        self.enums.insert(enum_type.name.clone(), enum_type);
    }

    /// Get an enum type.
    pub fn get_enum(&self, name: &str) -> Option<&EnumType> {
        self.enums.get(name)
    }

    /// Define an interface type.
    pub fn define_interface(&mut self, iface: InterfaceType) {
        self.interfaces.insert(iface.name.clone(), iface);
    }

    /// Get an interface type.
    pub fn get_interface(&self, name: &str) -> Option<&InterfaceType> {
        self.interfaces.get(name)
    }

    /// Define a function type.
    pub fn define_function(&mut self, name: String, ty: Type) {
        self.functions.insert(name, ty);
    }

    /// Set the current class context.
    pub fn set_current_class(&mut self, name: Option<String>) {
        self.current_class = name;
    }

    /// Get the current class context.
    pub fn current_class(&self) -> Option<&str> {
        self.current_class.as_deref()
    }

    /// Get the current class type.
    pub fn current_class_type(&self) -> Option<&ClassType> {
        self.current_class
            .as_ref()
            .and_then(|n| self.classes.get(n))
    }

    /// Set the expected return type for the current function.
    pub fn set_return_type(&mut self, ty: Option<Type>) {
        self.current_function_return = ty;
    }

    /// Get the expected return type for the current function.
    pub fn return_type(&self) -> Option<&Type> {
        self.current_function_return.as_ref()
    }
}

impl Default for TypeEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::type_repr::{ClassType, InterfaceType, MethodSignature};

    fn fresh() -> TypeEnvironment {
        TypeEnvironment::new()
    }

    // ---------- builtin pre-registration ----------

    #[test]
    fn new_registers_request_global_params() {
        let env = fresh();
        assert!(matches!(env.get("params"), Some(Type::Any)));
    }

    #[test]
    fn new_registers_print_as_function_to_void() {
        let env = fresh();
        let ty = env.get("print").expect("print should be registered");
        match ty {
            Type::Function { return_type, .. } => assert_eq!(*return_type, Type::Void),
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn new_registers_str_int_float_conversions() {
        let env = fresh();
        for (name, expected_ret) in &[
            ("str", Type::String),
            ("int", Type::Int),
            ("float", Type::Float),
        ] {
            let ty = env
                .get(name)
                .unwrap_or_else(|| panic!("{name} should be registered"));
            match ty {
                Type::Function { return_type, .. } => assert_eq!(&*return_type, expected_ret),
                other => panic!("{name}: expected Function, got {other:?}"),
            }
        }
    }

    #[test]
    fn default_equals_new_for_builtin_visibility() {
        // Default::default() must surface the same builtins as ::new().
        let env = TypeEnvironment::default();
        assert!(env.get("print").is_some());
        assert!(env.get("params").is_some());
    }

    // ---------- scopes / define / get ----------

    #[test]
    fn define_in_top_scope_is_visible_to_get() {
        let mut env = fresh();
        env.define("x".to_string(), Type::Int);
        assert_eq!(env.get("x"), Some(Type::Int));
    }

    #[test]
    fn get_returns_none_for_unknown_name() {
        let env = fresh();
        assert!(env.get("definitely_does_not_exist_xyz").is_none());
    }

    #[test]
    fn inner_scope_shadows_outer() {
        let mut env = fresh();
        env.define("x".to_string(), Type::Int);
        env.push_scope();
        env.define("x".to_string(), Type::String);
        assert_eq!(env.get("x"), Some(Type::String));
    }

    #[test]
    fn pop_scope_restores_outer_binding() {
        let mut env = fresh();
        env.define("x".to_string(), Type::Int);
        env.push_scope();
        env.define("x".to_string(), Type::String);
        env.pop_scope();
        assert_eq!(env.get("x"), Some(Type::Int));
    }

    #[test]
    fn lookup_walks_outward_for_inner_scope_misses() {
        let mut env = fresh();
        env.define("outer".to_string(), Type::Bool);
        env.push_scope();
        // No "outer" defined here — the walk falls back to the outer scope.
        assert_eq!(env.get("outer"), Some(Type::Bool));
        env.pop_scope();
    }

    #[test]
    fn pop_scope_is_silent_when_only_root_scope_left() {
        // Defensive: pop_scope shouldn't panic if called more times than
        // push_scope. It empties the scope stack — and a subsequent
        // define() then silently no-ops (per the docs on define).
        let mut env = fresh();
        env.pop_scope();
        // No panic. After this, define silently no-ops; get still finds
        // builtins and classes via the function/class fallbacks.
        env.define("x".to_string(), Type::Int);
        assert!(env.get("x").is_none());
        // builtins still resolve via the functions map.
        assert!(env.get("print").is_some());
    }

    #[test]
    fn function_lookup_falls_through_when_no_local_binding() {
        let mut env = fresh();
        env.define_function(
            "my_fn".to_string(),
            Type::Function {
                params: vec![Type::Int],
                return_type: Box::new(Type::Bool),
            },
        );
        match env.get("my_fn") {
            Some(Type::Function { return_type, .. }) => assert_eq!(*return_type, Type::Bool),
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn local_variable_shadows_builtin_function() {
        // If a user defines `let print = 1`, the local should shadow the
        // builtin `print` function on lookup.
        let mut env = fresh();
        env.define("print".to_string(), Type::Int);
        assert_eq!(env.get("print"), Some(Type::Int));
    }

    // ---------- classes ----------

    #[test]
    fn define_and_get_class_round_trip() {
        let mut env = fresh();
        env.define_class(ClassType::new("Foo".to_string()));
        let c = env.get_class("Foo").expect("class should be registered");
        assert_eq!(c.name, "Foo");
    }

    #[test]
    fn get_class_returns_none_for_unknown_name() {
        let env = fresh();
        assert!(env.get_class("Nope").is_none());
    }

    #[test]
    fn class_is_also_resolvable_via_get_as_type() {
        let mut env = fresh();
        env.define_class(ClassType::new("User".to_string()));
        match env.get("User") {
            Some(Type::Class(c)) => assert_eq!(c.name, "User"),
            other => panic!("expected Class type, got {other:?}"),
        }
    }

    #[test]
    fn redefining_class_overwrites_previous_entry() {
        let mut env = fresh();
        let mut a = ClassType::new("X".to_string());
        a.interfaces.push("OldIface".to_string());
        env.define_class(a);

        let mut b = ClassType::new("X".to_string());
        b.interfaces.push("NewIface".to_string());
        env.define_class(b);

        let c = env.get_class("X").unwrap();
        assert_eq!(c.interfaces, vec!["NewIface".to_string()]);
    }

    // ---------- interfaces ----------

    #[test]
    fn define_and_get_interface_round_trip() {
        let mut env = fresh();
        let mut iface = InterfaceType::new("Greeter".to_string());
        iface.methods.insert(
            "greet".to_string(),
            MethodSignature {
                name: "greet".to_string(),
                params: vec![],
                return_type: Type::String,
            },
        );
        env.define_interface(iface);

        let i = env
            .get_interface("Greeter")
            .expect("interface should be registered");
        assert_eq!(i.name, "Greeter");
        assert!(i.methods.contains_key("greet"));
    }

    #[test]
    fn get_interface_returns_none_for_unknown_name() {
        assert!(fresh().get_interface("Nope").is_none());
    }

    #[test]
    fn interface_does_not_resolve_via_get_as_type() {
        // get() falls back to functions and classes — but NOT interfaces.
        // (The TypeChecker resolves interfaces directly via get_interface.)
        let mut env = fresh();
        env.define_interface(InterfaceType::new("OnlyIface".to_string()));
        assert!(env.get("OnlyIface").is_none());
    }

    // ---------- current class context ----------

    #[test]
    fn current_class_starts_none() {
        let env = fresh();
        assert!(env.current_class().is_none());
        assert!(env.current_class_type().is_none());
    }

    #[test]
    fn set_and_read_current_class() {
        let mut env = fresh();
        env.define_class(ClassType::new("Cat".to_string()));
        env.set_current_class(Some("Cat".to_string()));
        assert_eq!(env.current_class(), Some("Cat"));
        assert_eq!(
            env.current_class_type().map(|c| c.name.clone()),
            Some("Cat".to_string())
        );
    }

    #[test]
    fn current_class_type_is_none_when_class_not_registered() {
        // current_class points at a name; current_class_type only resolves
        // it if the class was actually defined. Pin that this is silent.
        let mut env = fresh();
        env.set_current_class(Some("MissingClass".to_string()));
        assert_eq!(env.current_class(), Some("MissingClass"));
        assert!(env.current_class_type().is_none());
    }

    #[test]
    fn clearing_current_class_returns_none() {
        let mut env = fresh();
        env.define_class(ClassType::new("X".to_string()));
        env.set_current_class(Some("X".to_string()));
        env.set_current_class(None);
        assert!(env.current_class().is_none());
        assert!(env.current_class_type().is_none());
    }

    // ---------- return type tracking ----------

    #[test]
    fn return_type_round_trip() {
        let mut env = fresh();
        assert!(env.return_type().is_none());
        env.set_return_type(Some(Type::Int));
        assert_eq!(env.return_type(), Some(&Type::Int));
        env.set_return_type(None);
        assert!(env.return_type().is_none());
    }
}
