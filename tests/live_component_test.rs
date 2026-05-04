//! Tests for LiveView component state and app root management.

use serde_json::json;
use std::path::PathBuf;

use solilang::live::component::{get_app_root, set_app_root, ComponentInstance, ComponentState};

#[test]
fn component_state_get_returns_null_for_missing_key() {
    let state = ComponentState::new(json!({"count": 5}));
    assert_eq!(state.get("count"), json!(5));
    assert_eq!(state.get("missing"), json!(null));
}

#[test]
fn component_state_default_is_null_object() {
    let state = ComponentState::default();
    // Default uses serde's default for Value, which is Null. Get on null
    // returns Null.
    assert_eq!(state.get("anything"), json!(null));
}

#[test]
fn component_state_set_modifies_object() {
    let mut state = ComponentState::new(json!({"count": 0}));
    state.set("count", json!(7));
    state.set("name", json!("alice"));
    assert_eq!(state.get("count"), json!(7));
    assert_eq!(state.get("name"), json!("alice"));
}

#[test]
fn component_state_set_on_non_object_is_silent_noop() {
    // Set is documented to only mutate when state is an Object — non-objects
    // are silently skipped (no panic).
    let mut state = ComponentState::new(json!(42));
    state.set("x", json!("value"));
    // The non-object state survives unchanged.
    assert_eq!(state.as_value(), json!(42));
}

#[test]
fn component_state_as_value_clones_inner() {
    let state = ComponentState::new(json!({"a": 1, "b": 2}));
    let v = state.as_value();
    assert_eq!(v, json!({"a": 1, "b": 2}));
}

#[test]
fn app_root_round_trip() {
    let original = get_app_root();
    let new_path = PathBuf::from("/tmp/soli_test_live_root");
    set_app_root(new_path.clone());
    assert_eq!(get_app_root(), new_path);
    // Restore so other tests don't see the change.
    set_app_root(original);
}

#[test]
fn component_instance_mount_with_id_param() {
    let inst = ComponentInstance::mount(json!({}), json!({"id": "my-counter", "initial": 10}))
        .expect("mount ok");
    // The instance is opaque — we just confirm mount succeeds with both
    // explicit id and a starting value.
    let _ = inst;
}

#[test]
fn component_instance_mount_generates_id_when_missing() {
    // Without an id param, mount should still succeed and synthesize one
    // from a UUID.
    let inst = ComponentInstance::mount(json!({}), json!({})).expect("mount ok");
    let _ = inst;
}

#[test]
fn component_instance_mount_with_session() {
    // Session is currently ignored but pass-through should still work.
    let inst =
        ComponentInstance::mount(json!({"user_id": 7}), json!({"id": "x"})).expect("mount ok");
    let _ = inst;
}
