//! `Path`: lexical slash-separated paths.
//!
//! `File` reads and writes. `Path` only joins and splits the string, so it
//! never touches the disk and never escapes the file jail. Separators are
//! `/`, the form app paths and URLs already use, on every platform.

use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

pub fn register_path_class(env: &mut Environment) {
    let mut methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    methods.insert(
        "join".to_string(),
        Rc::new(NativeFunction::new("Path.join", None, |args| {
            if args.is_empty() {
                return Err("Path.join() requires a path".to_string());
            }
            let mut parts = Vec::with_capacity(args.len());
            for arg in args {
                match arg {
                    Value::String(text) => parts.push(text.to_string()),
                    other => {
                        return Err(format!(
                            "Path.join() expects string paths, got {}",
                            other.type_name()
                        ))
                    }
                }
            }
            Ok(Value::String(join(&parts).into()))
        })),
    );
    for (name, split) in [
        ("dirname", dirname as fn(&str) -> String),
        ("basename", basename),
        ("extname", extname),
    ] {
        let call = format!("Path.{name}");
        methods.insert(
            name.to_string(),
            Rc::new(NativeFunction::new(call.clone(), Some(1), move |args| {
                let path = match args.first() {
                    Some(Value::String(text)) => text.to_string(),
                    Some(other) => {
                        return Err(format!(
                            "{call}() expects a string path, got {}",
                            other.type_name()
                        ))
                    }
                    None => return Err(format!("{call}() requires a path")),
                };
                Ok(Value::String(split(&path).into()))
            })),
        );
    }
    env.define(
        "Path".to_string(),
        Value::Class(Rc::new(Class {
            name: "Path".to_string(),
            native_static_methods: methods,
            ..Default::default()
        })),
    );
}

/// Join with `/`, dropping empty segments from doubled slashes. A leading
/// slash on the first part stays. A later absolute part does not reset the
/// prefix. `.` and `..` are ordinary segments.
fn join(parts: &[String]) -> String {
    let absolute = parts.first().is_some_and(|part| part.starts_with('/'));
    let mut segments = Vec::new();
    for part in parts {
        for segment in part.split('/') {
            if !segment.is_empty() {
                segments.push(segment);
            }
        }
    }
    let body = segments.join("/");
    if absolute {
        format!("/{body}")
    } else {
        body
    }
}

fn basename(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.contains('/') {
            "/".to_string()
        } else {
            String::new()
        };
    }
    trimmed.rsplit('/').next().unwrap_or("").to_string()
}

fn dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.contains('/') {
            "/".to_string()
        } else {
            ".".to_string()
        };
    }
    match trimmed.rfind('/') {
        None => ".".to_string(),
        Some(index) => {
            // `a//b` is `a`, as `Path.join` reads it: the slashes before the
            // last name go with the separator.
            let parent = trimmed[..index].trim_end_matches('/');
            if parent.is_empty() {
                "/".to_string()
            } else {
                parent.to_string()
            }
        }
    }
}

fn extname(path: &str) -> String {
    let base = basename(path);
    if base == ".." {
        return String::new();
    }
    match base.rfind('.') {
        Some(0) | None => String::new(),
        Some(index) => base[index..].to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(env: &Environment, method: &str, args: &[Value]) -> Result<Value, String> {
        let Value::Class(class) = env.get("Path").expect("Path") else {
            panic!("Path is not a class");
        };
        let native = class
            .native_static_methods
            .get(method)
            .unwrap_or_else(|| panic!("Path.{method}"));
        (native.func)(args)
    }

    fn text(value: Value) -> String {
        match value {
            Value::String(text) => text.to_string(),
            other => panic!("expected string, got {other:?}"),
        }
    }

    #[test]
    fn framework_class_path_is_lexical() {
        let mut env = Environment::new();
        register_path_class(&mut env);
        let part = |text: &str| Value::String(text.into());
        assert_eq!(
            text(
                call(
                    &env,
                    "join",
                    &[part("app"), part("models"), part("post.sl")]
                )
                .unwrap()
            ),
            "app/models/post.sl"
        );
        assert_eq!(
            text(call(&env, "join", &[part("/var/"), part("/app")]).unwrap()),
            "/var/app"
        );
        assert_eq!(
            text(call(&env, "dirname", &[part("app/models/post.sl")]).unwrap()),
            "app/models"
        );
        assert_eq!(
            text(call(&env, "basename", &[part("app/models/post.sl")]).unwrap()),
            "post.sl"
        );
        assert_eq!(
            text(call(&env, "extname", &[part("app/models/post.sl")]).unwrap()),
            ".sl"
        );
        assert_eq!(
            text(call(&env, "extname", &[part(".gitignore")]).unwrap()),
            ""
        );
        assert_eq!(
            text(call(&env, "dirname", &[part("post.sl")]).unwrap()),
            "."
        );
        for (path, parent) in [
            ("a//b", "a"),
            ("/a//b", "/a"),
            ("//b", "/"),
            ("/b", "/"),
            ("a/b//", "a"),
        ] {
            assert_eq!(
                text(call(&env, "dirname", &[part(path)]).unwrap()),
                parent,
                "dirname {path}"
            );
        }
        for path in ["..", ".", "a/..", "a/."] {
            assert_eq!(
                text(call(&env, "extname", &[part(path)]).unwrap()),
                "",
                "extname {path}"
            );
        }
        let err = call(&env, "join", &[]).unwrap_err();
        assert!(err.contains("Path.join()"), "{err}");
    }
}
