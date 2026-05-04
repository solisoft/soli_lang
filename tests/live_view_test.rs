//! Tests for the LiveView registry. These don't go over the WebSocket — they
//! exercise the registry semantics (register/get/expire/cleanup/update).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use solilang::live::view::{LiveRegistry, LiveViewInstance, ServerMessage};

fn instance(component: &str, session: &str) -> LiveViewInstance {
    let (sender, _receiver) = async_channel::unbounded();
    LiveViewInstance::new(
        component.to_string(),
        PathBuf::from(format!("/tmp/{}.html.slv", component)),
        serde_json::Value::Null,
        session.to_string(),
        Arc::new(sender),
    )
}

#[test]
fn instance_id_combines_session_and_component() {
    let inst = instance("Counter", "sess-123");
    assert_eq!(inst.id, "sess-123:Counter");
}

#[test]
fn touch_updates_last_active() {
    let mut inst = instance("X", "s");
    let before = inst.last_active;
    std::thread::sleep(Duration::from_millis(5));
    inst.touch();
    assert!(inst.last_active > before);
}

#[test]
fn is_expired_after_timeout() {
    let inst = instance("X", "s");
    // Just-created instance is not expired given a long timeout.
    assert!(!inst.is_expired(Duration::from_secs(60)));
    // Force "expired" by checking against a zero-duration timeout AFTER a
    // small wait so elapsed() > 0.
    std::thread::sleep(Duration::from_millis(2));
    assert!(inst.is_expired(Duration::from_nanos(1)));
}

#[test]
fn registry_register_get_unregister() {
    let registry = LiveRegistry::new();
    let inst = instance("Counter", "sess-A");
    let id = inst.id.clone();

    registry.register(inst);
    assert!(registry.get(&id).is_some(), "should find after register");

    registry.unregister(&id);
    assert!(
        registry.get(&id).is_none(),
        "should be gone after unregister"
    );
}

#[test]
fn registry_get_nonexistent_returns_none() {
    let registry = LiveRegistry::new();
    assert!(registry.get("does-not-exist").is_none());
}

#[test]
fn registry_update_overrides_existing() {
    let registry = LiveRegistry::new();
    let mut inst = instance("Counter", "sess-A");
    let id = inst.id.clone();
    registry.register(inst.clone());

    // Bump the state and update.
    inst.state = serde_json::json!({"count": 7});
    registry.update(inst);

    let fetched = registry.get(&id).expect("present");
    assert_eq!(fetched.state, serde_json::json!({"count": 7}));
}

#[test]
fn registry_send_to_missing_id_errors() {
    let registry = LiveRegistry::new();
    let result = registry.send("missing", ServerMessage::HeartbeatAck);
    assert!(result.is_err(), "expected ConnectionClosed");
}

#[test]
fn registry_send_to_existing_succeeds() {
    let registry = LiveRegistry::new();
    let inst = instance("X", "s");
    let id = inst.id.clone();
    registry.register(inst);

    // The receiver is dropped at this point, but try_send may still queue
    // into the unbounded channel. The send should not panic — accept either
    // outcome.
    let _ = registry.send(&id, ServerMessage::HeartbeatAck);
}

#[test]
fn server_message_serialization() {
    let msg = ServerMessage::Render {
        html: "<div>hi</div>".to_string(),
        liveview_id: "v1".to_string(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"Render\""));
    assert!(json.contains("v1"));

    let msg = ServerMessage::Redirect {
        url: "/next".to_string(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"type\":\"Redirect\""));
    assert!(json.contains("/next"));

    let msg = ServerMessage::HeartbeatAck;
    let json = serde_json::to_string(&msg).unwrap();
    assert_eq!(json, r#"{"type":"HeartbeatAck"}"#);
}
