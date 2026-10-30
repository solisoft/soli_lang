//! Flash messages: a notice that survives exactly one redirect.
//!
//! ```soli
//! flash("success", "Post saved.")      # shown by the next request that reads it
//! redirect(post_path(@post))
//!
//! flash_now("error", "Fix the errors below.")   # shown by this render only
//! render("new", {}, {"status": 422})
//! ```
//!
//! ```erb
//! <% flashes().each do |f| %>
//!   <div class="flash flash-<%= f.type %>"><%= f.message %></div>
//! <% end %>
//! ```
//!
//! `flash` queues `{"type", "message"}` under the session key `_flashes` — the
//! key and shape the docs' hand-written helper used, so an app that switches
//! from its own helper to the builtin keeps the messages already queued.
//! `flashes()` takes that queue out of the session on its first call in a
//! request and answers the same list to every later call, so a layout and a
//! partial can both read it; `flash_now` messages follow it, and never touch
//! the session.
//!
//! The session is read only when `flashes()` runs, and written back only when
//! there was something to take: a layout that calls it on every page costs one
//! session read and no write.

use std::cell::RefCell;

use serde_json::{json, Value as JsonValue};

use crate::interpreter::builtins::session::{
    get_current_session_id, get_current_store, write_session_value,
};
use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};
use crate::interpreter::value_json::{json_ref_to_value_or_null as json_to_value, value_to_json};

/// The session key the queue lives under.
const SESSION_KEY: &str = "_flashes";

crate::request_local! {
    /// The messages this request took from the session — `None` until
    /// `flashes()` first runs, so a request that never reads leaves the queue
    /// for the next one.
    static TAKEN: RefCell<Option<Vec<JsonValue>>> = const { RefCell::new(None) };
    /// `flash_now` messages: this request only.
    static NOW: RefCell<Vec<JsonValue>> = const { RefCell::new(Vec::new()) };
}

/// Forget the previous request's flashes. Worker threads are reused; without
/// this the next visitor on the thread would see them.
pub fn clear_request_flashes() {
    TAKEN.with(|t| *t.borrow_mut() = None);
    NOW.with(|n| n.borrow_mut().clear());
}

fn message(name: &str, args: &[Value]) -> Result<JsonValue, String> {
    let kind = match &args[0] {
        Value::String(s) => s.to_string(),
        other => {
            return Err(format!(
                "{name}() expects a string type, got {}",
                other.type_name()
            ))
        }
    };
    Ok(json!({ "type": kind, "message": value_to_json(&args[1])? }))
}

fn queued() -> Vec<JsonValue> {
    let Some(id) = get_current_session_id() else {
        return Vec::new();
    };
    match get_current_store().get(&id, SESSION_KEY) {
        Some(JsonValue::Array(list)) => list,
        _ => Vec::new(),
    }
}

/// Take the session's queue on the first call, then answer from the request.
fn take_queue() -> Vec<JsonValue> {
    if let Some(taken) = TAKEN.with(|t| t.borrow().clone()) {
        return taken;
    }
    let taken = queued();
    if let Some(id) = get_current_session_id() {
        if !taken.is_empty() {
            get_current_store().delete(&id, SESSION_KEY);
        }
    }
    TAKEN.with(|t| *t.borrow_mut() = Some(taken.clone()));
    taken
}

pub fn register_flash_builtins(env: &mut Environment) {
    // flash(type, message) -> null: queue a message for the next request.
    env.define(
        "flash".to_string(),
        Value::NativeFunction(NativeFunction::new("flash", Some(2), |args| {
            let entry = message("flash", args)?;
            let mut list = queued();
            list.push(entry);
            write_session_value(SESSION_KEY, JsonValue::Array(list));
            crate::template::response_cache::mark_response_dirty();
            Ok(Value::Null)
        })),
    );

    // flash_now(type, message) -> null: a message for this request's render.
    env.define(
        "flash_now".to_string(),
        Value::NativeFunction(NativeFunction::new("flash_now", Some(2), |args| {
            let entry = message("flash_now", args)?;
            NOW.with(|n| n.borrow_mut().push(entry));
            // The message is not in the data `render` hashes: a cached body
            // would show it to the next request, or hide it from this one.
            crate::template::response_cache::mark_response_dirty();
            Ok(Value::Null)
        })),
    );

    // flashes() -> Array of {"type", "message"}: queued messages, then
    // flash_now ones. Takes the queue out of the session.
    env.define(
        "flashes".to_string(),
        Value::NativeFunction(NativeFunction::new("flashes", Some(0), |_args| {
            let mut list = take_queue();
            NOW.with(|n| list.extend(n.borrow().iter().cloned()));
            Ok(json_to_value(&JsonValue::Array(list)))
        })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::builtins::session::{set_current_session_id, SESSION_STORE_TEST_LOCK};
    use std::sync::MutexGuard;

    fn call(env: &Environment, name: &str, args: &[Value]) -> Value {
        match env.get(name) {
            Some(Value::NativeFunction(f)) => (f.func)(args).unwrap(),
            other => panic!("expected NativeFunction for {name}, got {other:?}"),
        }
    }

    /// The env, and the lock that keeps the session tests from swapping the
    /// store while this test runs.
    fn fresh_env() -> (Environment, MutexGuard<'static, ()>) {
        let guard = SESSION_STORE_TEST_LOCK
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        set_current_session_id(None);
        clear_request_flashes();
        let mut env = Environment::new();
        register_flash_builtins(&mut env);
        (env, guard)
    }

    fn s(text: &str) -> Value {
        Value::String(text.into())
    }

    fn as_json(value: &Value) -> JsonValue {
        value_to_json(value).unwrap()
    }

    #[test]
    fn a_flash_survives_one_redirect_and_no_more() {
        let (env, _guard) = fresh_env();
        call(&env, "flash", &[s("success"), s("Saved.")]);
        call(&env, "flash", &[s("info"), s("Twice.")]);
        let id = get_current_session_id().expect("flash creates the session");

        // Next request: the queue is there, and read twice in the request.
        clear_request_flashes();
        set_current_session_id(Some(id.clone()));
        let expected = json!([
            {"type": "success", "message": "Saved."},
            {"type": "info", "message": "Twice."},
        ]);
        assert_eq!(as_json(&call(&env, "flashes", &[])), expected);
        assert_eq!(as_json(&call(&env, "flashes", &[])), expected);

        // The request after that: gone.
        clear_request_flashes();
        set_current_session_id(Some(id));
        assert_eq!(as_json(&call(&env, "flashes", &[])), json!([]));
    }

    #[test]
    fn flash_now_shows_in_this_request_and_never_reaches_the_session() {
        let (env, _guard) = fresh_env();
        call(&env, "flash_now", &[s("error"), s("Fix it.")]);
        assert_eq!(
            as_json(&call(&env, "flashes", &[])),
            json!([{"type": "error", "message": "Fix it."}])
        );
        assert!(get_current_session_id().is_none());

        clear_request_flashes();
        assert_eq!(as_json(&call(&env, "flashes", &[])), json!([]));
    }

    #[test]
    fn a_flash_set_after_the_read_waits_for_the_next_request() {
        let (env, _guard) = fresh_env();
        assert_eq!(as_json(&call(&env, "flashes", &[])), json!([]));
        call(&env, "flash", &[s("success"), s("Later.")]);
        assert_eq!(as_json(&call(&env, "flashes", &[])), json!([]));

        let id = get_current_session_id().unwrap();
        clear_request_flashes();
        set_current_session_id(Some(id));
        assert_eq!(
            as_json(&call(&env, "flashes", &[])),
            json!([{"type": "success", "message": "Later."}])
        );
    }

    #[test]
    fn the_type_must_be_a_string() {
        let (env, _guard) = fresh_env();
        let Some(Value::NativeFunction(f)) = env.get("flash") else {
            panic!("flash is registered");
        };
        let err = (f.func)(&[Value::Int(1), s("x")]).unwrap_err();
        assert!(err.contains("expects a string type"), "{err}");
    }
}
