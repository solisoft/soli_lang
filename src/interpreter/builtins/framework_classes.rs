//! `Session`, `Cookie`, `Channel`, `Path`, `Bytes`, `Component` and `Zip`.
//!
//! The first three are the class form of function piles. Each free function
//! stays, warns once per process, and keeps its old error text. `Cookie.signed`
//! and `Cookie.encrypted` are not aliases: one argument reads in that mode,
//! two or three write in it. `Cookie.set` / `Cookie.read` with a
//! `"signed"` / `"encrypted"` flag still run and warn once. `csrf_token`, the
//! `cookies` hash, `sse` and `stream` stay as they are.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFn, NativeFunction, Value};

const SESSION: &[(&str, &str)] = &[
    ("session_get", "get"),
    ("session_set", "set"),
    ("session_delete", "delete"),
    ("session_has", "has"),
    ("session_destroy", "destroy"),
    ("session_regenerate", "regenerate"),
    ("session_id", "id"),
    ("session_driver", "driver"),
    ("session_configure", "configure"),
    ("session_config", "config"),
    ("flash", "flash"),
    ("flash_now", "flash_now"),
    ("flashes", "flashes"),
];

const CHANNEL: &[(&str, &str)] = &[
    ("ws_join", "join"),
    ("ws_leave", "leave"),
    ("ws_send", "send"),
    ("ws_close", "close"),
    ("ws_broadcast", "broadcast_all"),
    ("ws_broadcast_room", "broadcast_room"),
    ("broadcast", "broadcast"),
    ("ws_clients", "clients"),
    ("ws_clients_in", "clients_in"),
    ("ws_count", "count"),
    ("ws_list_presence", "list_presence"),
    ("ws_presence_count", "presence_count"),
    ("ws_get_presence", "get_presence"),
    ("sse_subscribe", "subscribe"),
    ("sse_broadcast", "sse_broadcast"),
    ("sse_subscribers", "subscribers"),
];

/// Framework code that reads the session (the upload prelude) calls these,
/// not `Session.get`: an app class named `Session` replaces the builtin
/// global, and the free functions warn.
const INTERNAL_SESSION: &[(&str, &str)] = &[
    ("session_get", "__soli_session_get"),
    ("session_set", "__soli_session_set"),
];

pub fn register(env: &mut Environment) {
    for (public, internal) in INTERNAL_SESSION {
        if let Some(Value::NativeFunction(native)) = env.get(public) {
            let func = native.func.clone();
            env.define(
                (*internal).to_string(),
                Value::NativeFunction(NativeFunction::new_with_auto(
                    *internal,
                    native.arity,
                    native.is_auto_invocable,
                    move |args| func(args),
                )),
            );
        }
    }
    super::bind_deprecated_class(env, "Session", SESSION);
    super::bind_deprecated_class(env, "Channel", CHANNEL);

    let set_cookie = native_func(env, "set_cookie");
    let read_cookie = native_func(env, "read_cookie");
    super::bind_deprecated_class(
        env,
        "Cookie",
        &[("set_cookie", "set"), ("read_cookie", "read")],
    );
    if let (Some(set_cookie), Some(read_cookie)) = (set_cookie, read_cookie) {
        let mut cookie = HashMap::new();
        cookie.insert(
            "signed".to_string(),
            seal_method(set_cookie.clone(), read_cookie.clone(), "signed"),
        );
        cookie.insert(
            "encrypted".to_string(),
            seal_method(set_cookie, read_cookie, "encrypted"),
        );
        if let Some(Value::Class(class)) = env.get("Cookie") {
            for (method, options_at) in [("set", 2), ("read", 1)] {
                if let Some(native) = class.native_static_methods.get(method) {
                    cookie.insert(
                        method.to_string(),
                        warn_on_seal_flag(native, method, options_at),
                    );
                }
            }
        }
        extend_static(env, "Cookie", cookie);
    }

    super::path_class::register_path_class(env);
    super::bytes::register_bytes_class(env);
    env.define(
        "Component".to_string(),
        Value::Class(Rc::new(Class {
            name: "Component".to_string(),
            ..Default::default()
        })),
    );
    #[cfg(feature = "office")]
    super::zip_archive::register_zip_class(env);
}

fn native_func(env: &Environment, name: &str) -> Option<NativeFn> {
    match env.get(name) {
        Some(Value::NativeFunction(native)) => Some(native.func.clone()),
        _ => None,
    }
}

fn extend_static(
    env: &mut Environment,
    class_name: &str,
    extra: HashMap<String, Rc<NativeFunction>>,
) {
    let Some(Value::Class(class)) = env.get(class_name) else {
        return;
    };
    let mut updated = (*class).clone();
    updated.native_static_methods.extend(extra);
    env.define(class_name.to_string(), Value::Class(Rc::new(updated)));
}

/// The keys `set_cookie` reads in its options hash.
const COOKIE_OPTIONS: &[&str] = &[
    "path",
    "max_age",
    "expires",
    "http_only",
    "secure",
    "same_site",
    "domain",
    "encrypted",
    "signed",
];

/// `Cookie.set` / `Cookie.read` warn once when their options carry the
/// `"signed"` or `"encrypted"` flag: `Cookie.signed` / `Cookie.encrypted`
/// say it in the name. The call still runs.
fn warn_on_seal_flag(
    native: &Rc<NativeFunction>,
    method: &'static str,
    options_at: usize,
) -> Rc<NativeFunction> {
    let func = native.func.clone();
    Rc::new(NativeFunction::new_with_auto(
        native.name.clone(),
        native.arity,
        native.is_auto_invocable,
        move |args| {
            if let Some(Value::Hash(options)) = args.get(options_at) {
                let flag = ["signed", "encrypted"].into_iter().find(|flag| {
                    options.borrow().iter().any(|(key, value)| {
                        matches!(key, HashKey::String(name) if name.as_ref() == *flag)
                            && matches!(value, Value::Bool(true))
                    })
                });
                if let Some(flag) = flag {
                    let (old, new) = if method == "set" {
                        ("name, value, options", "name, value, options?")
                    } else {
                        ("name, options", "name")
                    };
                    super::warn_once(
                        &format!("Cookie.{method}:{flag}"),
                        &format!(
                            "warning: Cookie.{method}({old}) with {{\"{flag}\": true}} is deprecated; \
                             use Cookie.{flag}({new}) instead"
                        ),
                    );
                }
            }
            func(args)
        },
    ))
}

/// `Cookie.signed(name)` reads. `Cookie.signed(name, value, options?)` writes.
/// The flag is forced on, then the existing `set_cookie` / `read_cookie`
/// closure runs, so sealing stays in one place.
///
/// Two arguments whose second is a hash of cookie options only
/// (`Cookie.signed("uid", {"max_age": 60})`) raise: that is a read written
/// the `read_cookie(name, options)` way, and running it would overwrite the
/// cookie with the options. A hash meant as the value goes through with a
/// third argument (`Cookie.signed("prefs", {"path": "/x"}, {})`).
fn seal_method(set_cookie: NativeFn, read_cookie: NativeFn, flag: &str) -> Rc<NativeFunction> {
    let flag = flag.to_string();
    let call = format!("Cookie.{flag}");
    Rc::new(NativeFunction::new(call.clone(), None, move |args| {
        if args.is_empty() || args.len() > 3 {
            return Err(format!(
                "{call}() takes a name, or a name, value and optional options, got {} argument(s)",
                args.len()
            ));
        }
        if args.len() == 1 {
            let options = hash_with_flag(None, &flag, &call)?;
            return read_cookie(&[args[0].clone(), options])
                .map_err(|error| rewrite_seal_error(error, &call));
        }
        if args.len() == 2 && looks_like_options(&args[1]) {
            return Err(format!(
                "{call}() with two arguments writes, and {} looks like cookie options: \
                 read with {call}(name), write with {call}(name, value, options); \
                 to store this hash as the value, pass options too: {call}(name, value, {{}})",
                args[1]
            ));
        }
        let options = hash_with_flag(args.get(2), &flag, &call)?;
        set_cookie(&[args[0].clone(), args[1].clone(), options])
            .map_err(|error| rewrite_seal_error(error, &call))
    }))
}

/// A non-empty hash whose every key is a `set_cookie` option.
fn looks_like_options(value: &Value) -> bool {
    let Value::Hash(hash) = value else {
        return false;
    };
    let hash = hash.borrow();
    !hash.is_empty()
        && hash.iter().all(|(key, _)| {
            matches!(key, HashKey::String(name) if COOKIE_OPTIONS.contains(&name.as_ref()))
        })
}

/// The sealed call names itself. The jar's "set a secret" error still says
/// `session_configure`, which is the same deprecated pile.
fn rewrite_seal_error(error: String, call: &str) -> String {
    let error = super::rename_call_in_error(&error, "read_cookie", call);
    let error = super::rename_call_in_error(&error, "set_cookie", call);
    super::rename_call_in_error(&error, "session_configure", "Session.configure")
}

fn hash_with_flag(existing: Option<&Value>, flag: &str, method: &str) -> Result<Value, String> {
    let mut pairs = HashPairs::default();
    match existing {
        None | Some(Value::Null) => {}
        Some(Value::Hash(hash)) => {
            for (key, value) in hash.borrow().iter() {
                pairs.insert(key.clone(), value.clone());
            }
        }
        Some(other) => {
            return Err(format!(
                "{method}() options must be a hash, got {}",
                other.type_name()
            ));
        }
    }
    pairs.insert(HashKey::String(flag.into()), Value::Bool(true));
    Ok(Value::Hash(Rc::new(RefCell::new(pairs))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class_method(env: &Environment, class_name: &str, method: &str) -> NativeFunction {
        match env.get(class_name) {
            Some(Value::Class(class)) => class
                .native_static_methods
                .get(method)
                .cloned()
                .unwrap_or_else(|| panic!("{class_name}.{method} is not registered"))
                .as_ref()
                .clone(),
            other => panic!("expected {class_name}, got {other:?}"),
        }
    }

    fn global(env: &Environment, name: &str) -> NativeFunction {
        match env.get(name) {
            Some(Value::NativeFunction(native)) => native.clone(),
            other => panic!("expected {name}, got {other:?}"),
        }
    }

    #[test]
    fn framework_class_aliases_name_themselves() {
        let mut env = Environment::new();
        crate::interpreter::builtins::register_builtins(&mut env, false);

        let bad = [Value::Int(1)];
        let session = (class_method(&env, "Session", "get").func)(&bad).unwrap_err();
        assert!(session.contains("Session.get()"), "{session}");
        assert!(!session.contains("session_get"), "{session}");
        let alias = (global(&env, "session_get").func)(&bad).unwrap_err();
        assert!(alias.contains("session_get()"), "{alias}");
        assert!(!alias.contains("Session.get"), "{alias}");

        let channel = (class_method(&env, "Channel", "join").func)(&[]).unwrap_err();
        assert!(channel.contains("Channel.join()"), "{channel}");
        let ws = (global(&env, "ws_join").func)(&[]).unwrap_err();
        assert!(ws.contains("ws_join()"), "{ws}");
        assert!(!ws.contains("Channel.join"), "{ws}");

        let cookie = (class_method(&env, "Cookie", "set").func)(&[]).unwrap_err();
        assert!(cookie.contains("Cookie.set()"), "{cookie}");
        assert!(!cookie.contains("set_cookie"), "{cookie}");
        let set_cookie = (global(&env, "set_cookie").func)(&[]).unwrap_err();
        assert!(set_cookie.contains("set_cookie()"), "{set_cookie}");

        let missing =
            (class_method(&env, "Cookie", "signed").func)(&[Value::String("nope".into())]).unwrap();
        assert_eq!(missing, Value::Null);
        assert!(env.get("Component").is_some());
        assert!(env.get("Path").is_some());
        assert!(env.get("Bytes").is_some());
    }

    #[test]
    fn framework_class_methods_keep_bare_call() {
        let mut env = Environment::new();
        crate::interpreter::builtins::register_builtins(&mut env, false);
        assert!(global(&env, "ws_count").is_auto_invocable);
        assert!(class_method(&env, "Channel", "count").is_auto_invocable);
        assert_eq!(
            class_method(&env, "Session", "destroy").arity,
            global(&env, "session_destroy").arity
        );
    }

    #[test]
    fn framework_class_errors_rename_only_the_call() {
        let mut env = Environment::new();
        crate::interpreter::builtins::register_builtins(&mut env, false);
        let options = crate::interpreter::value::hash_from_pairs([(
            "set_cookie_path",
            Value::String("/".into()),
        )]);
        let err = (class_method(&env, "Cookie", "set").func)(&[
            Value::String("a".into()),
            Value::String("b".into()),
            options,
        ])
        .unwrap_err();
        assert!(err.starts_with("Cookie.set()"), "{err}");
        assert!(err.contains("\"set_cookie_path\""), "{err}");

        assert_eq!(
            crate::interpreter::builtins::rename_call_in_error(
                "broadcast() unknown option \"rebroadcast(\"",
                "broadcast",
                "Channel.broadcast"
            ),
            "Channel.broadcast() unknown option \"rebroadcast(\""
        );
    }

    #[test]
    fn framework_class_cookie_seal_refuses_options_as_value() {
        let mut env = Environment::new();
        crate::interpreter::builtins::register_builtins(&mut env, false);
        let signed = class_method(&env, "Cookie", "signed");
        let options = crate::interpreter::value::hash_from_pairs([("max_age", Value::Int(60))]);
        let err = (signed.func)(&[Value::String("uid".into()), options]).unwrap_err();
        assert!(err.contains("looks like cookie options"), "{err}");
        assert!(err.starts_with("Cookie.signed()"), "{err}");
    }

    #[test]
    fn framework_internal_session_natives_exist() {
        let mut env = Environment::new();
        crate::interpreter::builtins::register_builtins(&mut env, false);
        for (_, internal) in INTERNAL_SESSION {
            assert!(
                matches!(env.get(internal), Some(Value::NativeFunction(_))),
                "{internal}"
            );
        }
    }
}
