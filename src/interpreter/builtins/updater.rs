//! `Updater` — drive an artifact's auto-update from Soli code.
//!
//! A `soli build --standalone` / `soli desktop build` artifact built with
//! `--update-url` embeds an [`crate::update::UpdateDescriptor`]. The `Updater`
//! builtin is the in-app face of the same check → download → verify → swap flow
//! the `--check-update` / `--update` CLI flags run, so a page can offer an
//! "update available — restart to apply" affordance instead of the terminal.
//!
//! ```soli
//! info = Updater.check()          # { "available": true, "current": "1.0.0",
//!                                 #   "latest": "1.1.0", "notes": "…" }
//! if info["available"] {
//!   Updater.apply()               # downloads + verifies + self-replaces
//!   # tell the user to restart
//! }
//! ```
//!
//! Every method is a no-op-with-explanation when the running process is not an
//! artifact built with `--update-url` (e.g. `soli serve` in development):
//! `Updater.version()` returns `null` and `check` / `apply` return a hash whose
//! `available` / `status` says auto-update is not configured. That keeps app
//! code that calls `Updater` from crashing outside a built artifact.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, HashKey, HashPairs, NativeFunction, Value};

fn record(pairs: &mut HashPairs, key: &str, value: Value) {
    pairs.insert(HashKey::String(key.into()), value);
}

fn string_value(s: impl Into<String>) -> Value {
    Value::String(s.into().into())
}

fn hash(pairs: HashPairs) -> Value {
    Value::Hash(Rc::new(RefCell::new(pairs)))
}

/// `Updater.version()` — the embedded app version, or `null` outside an artifact.
fn version() -> Value {
    match crate::update::active_descriptor() {
        Some(desc) => string_value(desc.app_version.clone()),
        None => Value::Null,
    }
}

/// `Updater.check()` — fetch and verify the manifest, compare versions.
fn check() -> Value {
    let Some(desc) = crate::update::active_descriptor() else {
        let mut out = HashPairs::default();
        record(&mut out, "available", Value::Bool(false));
        record(&mut out, "configured", Value::Bool(false));
        record(
            &mut out,
            "error",
            string_value("this build has no auto-update channel (--update-url)"),
        );
        return hash(out);
    };

    let mut out = HashPairs::default();
    record(&mut out, "configured", Value::Bool(true));
    match crate::update::check(desc) {
        Ok(result) => {
            record(&mut out, "available", Value::Bool(result.available));
            record(&mut out, "current", string_value(result.current));
            record(&mut out, "latest", string_value(result.latest));
            record(&mut out, "notes", string_value(result.notes));
        }
        Err(e) => {
            record(&mut out, "available", Value::Bool(false));
            record(&mut out, "error", string_value(e));
        }
    }
    hash(out)
}

/// `Updater.apply()` — download the newer artifact, verify it, self-replace.
fn apply() -> Value {
    let Some(desc) = crate::update::active_descriptor() else {
        let mut out = HashPairs::default();
        record(&mut out, "status", string_value("not-configured"));
        record(
            &mut out,
            "error",
            string_value("this build has no auto-update channel (--update-url)"),
        );
        return hash(out);
    };

    let mut out = HashPairs::default();
    match crate::update::apply(desc) {
        Ok(msg) => {
            record(&mut out, "status", string_value("updated"));
            record(&mut out, "message", string_value(msg));
            record(&mut out, "restart_required", Value::Bool(true));
        }
        Err(e) => {
            record(&mut out, "status", string_value("error"));
            record(&mut out, "error", string_value(e));
        }
    }
    hash(out)
}

pub fn register_updater_builtins(env: &mut Environment) {
    let mut statics: HashMap<String, Rc<NativeFunction>> = HashMap::new();

    statics.insert(
        "version".to_string(),
        Rc::new(NativeFunction::new("Updater.version", Some(0), |_args| {
            Ok(version())
        })),
    );
    statics.insert(
        "check".to_string(),
        Rc::new(NativeFunction::new("Updater.check", Some(0), |_args| {
            Ok(check())
        })),
    );
    statics.insert(
        "apply".to_string(),
        Rc::new(NativeFunction::new("Updater.apply", Some(0), |_args| {
            Ok(apply())
        })),
    );

    let class = Rc::new(Class {
        name: "Updater".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods: statics,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    });

    env.define("Updater".to_string(), Value::Class(class));
}
