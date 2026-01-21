//! Type environment for the type checker.

use std::collections::HashMap;

use crate::types::type_repr::{ClassType, InterfaceType, Type};

/// A type environment tracking types of variables and declarations.
#[derive(Debug, Clone)]
pub struct TypeEnvironment {
    scopes: Vec<HashMap<String, Type>>,
    classes: HashMap<String, ClassType>,
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
            interfaces: HashMap::new(),
            functions: HashMap::new(),
            current_class: None,
            current_function_return: None,
        };

        // Register built-in functions
        env.register_builtins();
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

        // HTTP client functions
        // http_get(String) -> String
        self.functions.insert(
            "http_get".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::String),
            },
        );

        // http_post(String, String|Hash) -> String
        self.functions.insert(
            "http_post".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Any],
                return_type: Box::new(Type::String),
            },
        );

        // http_get_json(String) -> Any
        self.functions.insert(
            "http_get_json".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::Any),
            },
        );

        // http_post_json(String, Any) -> Any
        self.functions.insert(
            "http_post_json".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Any],
                return_type: Box::new(Type::Any),
            },
        );

        // http_request(String, String, Hash?, String|Hash?) -> Hash
        self.functions.insert(
            "http_request".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::Any),
            },
        );

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

        // HTTP server functions
        // http_server_get(String, Function) -> Void
        self.functions.insert(
            "http_server_get".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Any],
                return_type: Box::new(Type::Void),
            },
        );

        // http_server_post(String, Function) -> Void
        self.functions.insert(
            "http_server_post".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Any],
                return_type: Box::new(Type::Void),
            },
        );

        // http_server_put(String, Function) -> Void
        self.functions.insert(
            "http_server_put".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Any],
                return_type: Box::new(Type::Void),
            },
        );

        // http_server_delete(String, Function) -> Void
        self.functions.insert(
            "http_server_delete".to_string(),
            Type::Function {
                params: vec![Type::String, Type::Any],
                return_type: Box::new(Type::Void),
            },
        );

        // http_server_route(String, String, Function) -> Void
        self.functions.insert(
            "http_server_route".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String, Type::Any],
                return_type: Box::new(Type::Void),
            },
        );

        // http_server_listen(Int) -> Void (blocking)
        self.functions.insert(
            "http_server_listen".to_string(),
            Type::Function {
                params: vec![Type::Int],
                return_type: Box::new(Type::Void),
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

        // regex_match(String, String) -> Bool
        self.functions.insert(
            "regex_match".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::Bool),
            },
        );

        // regex_find(String, String) -> Hash|null
        self.functions.insert(
            "regex_find".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::Any),
            },
        );

        // regex_find_all(String, String) -> Array
        self.functions.insert(
            "regex_find_all".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::Array(Box::new(Type::Any))),
            },
        );

        // regex_replace(String, String, String) -> String
        self.functions.insert(
            "regex_replace".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String, Type::String],
                return_type: Box::new(Type::String),
            },
        );

        // regex_replace_all(String, String, String) -> String
        self.functions.insert(
            "regex_replace_all".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String, Type::String],
                return_type: Box::new(Type::String),
            },
        );

        // regex_split(String, String) -> Array
        self.functions.insert(
            "regex_split".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::Array(Box::new(Type::String))),
            },
        );

        // regex_capture(String, String) -> Hash|null
        self.functions.insert(
            "regex_capture".to_string(),
            Type::Function {
                params: vec![Type::String, Type::String],
                return_type: Box::new(Type::Any),
            },
        );

        // regex_escape(String) -> String
        self.functions.insert(
            "regex_escape".to_string(),
            Type::Function {
                params: vec![Type::String],
                return_type: Box::new(Type::String),
            },
        );
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
