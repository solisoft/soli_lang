//! Test server infrastructure for Rails-like E2E controller testing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::net::SocketAddr;

use tokio::net::TcpListener;
use tokio::runtime::Runtime;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

static TEST_SERVER_RUNNING: AtomicBool = AtomicBool::new(false);
static TEST_SERVER_PORT: AtomicU16 = AtomicU16::new(0);

thread_local! {
    static LAST_RESPONSE: RefCell<Option<Value>> = const { RefCell::new(None) };
    static LAST_REQUEST: RefCell<Option<HashMap<String, Value>>> = const { RefCell::new(None) };
    static LAST_ASSIGNS: RefCell<Option<HashMap<String, Value>>> = const { RefCell::new(None) };
    static LAST_VIEW_PATH: RefCell<Option<String>> = const { RefCell::new(None) };
    static CURRENT_USER: RefCell<Option<Value>> = const { RefCell::new(None) };
}

/// Register test server built-in functions.
pub fn register_test_server_builtins(env: &mut Environment) {
    env.define(
        "test_server_start".to_string(),
        Value::NativeFunction(NativeFunction::new("test_server_start", Some(0), |_args| {
            let port = start_test_server();
            Ok(Value::Int(port as i64))
        })),
    );

    env.define(
        "test_server_stop".to_string(),
        Value::NativeFunction(NativeFunction::new("test_server_stop", Some(0), |_args| {
            stop_test_server();
            Ok(Value::Null)
        })),
    );

    env.define(
        "test_server_url".to_string(),
        Value::NativeFunction(NativeFunction::new("test_server_url", Some(0), |_args| {
            let url = get_test_server_url();
            Ok(Value::String(url))
        })),
    );

    env.define(
        "test_server_running".to_string(),
        Value::NativeFunction(NativeFunction::new(
            "test_server_running",
            Some(0),
            |_args| {
                let running = is_test_server_running();
                Ok(Value::Bool(running))
            },
        )),
    );
}

/// Start the test server on a random available port.
pub fn start_test_server() -> u16 {
    if TEST_SERVER_RUNNING.load(Ordering::SeqCst) {
        return TEST_SERVER_PORT.load(Ordering::SeqCst);
    }

    let rt = Runtime::new().expect("Failed to create Tokio runtime");
    let port = rt.block_on(async {
        let addr = SocketAddr::from(([127, 0, 0, 1], 0));
        let listener = TcpListener::bind(addr).await.unwrap();
        listener.local_addr().unwrap().port()
    });

    TEST_SERVER_PORT.store(port, Ordering::SeqCst);
    TEST_SERVER_RUNNING.store(true, Ordering::SeqCst);

    port
}

/// Stop the test server.
pub fn stop_test_server() {
    TEST_SERVER_RUNNING.store(false, Ordering::SeqCst);
    TEST_SERVER_PORT.store(0, Ordering::SeqCst);
}

/// Get the test server base URL.
fn get_test_server_url() -> String {
    if !TEST_SERVER_RUNNING.load(Ordering::SeqCst) {
        return String::new();
    }
    let port = TEST_SERVER_PORT.load(Ordering::SeqCst);
    format!("http://127.0.0.1:{}", port)
}

/// Check if test server is running.
fn is_test_server_running() -> bool {
    TEST_SERVER_RUNNING.load(Ordering::SeqCst)
}

/// Get the test server port.
pub fn get_test_server_port() -> Option<u16> {
    if TEST_SERVER_RUNNING.load(Ordering::SeqCst) {
        Some(TEST_SERVER_PORT.load(Ordering::SeqCst))
    } else {
        None
    }
}

/// Store the last response for inspection.
pub fn set_last_response(response: Value) {
    LAST_RESPONSE.with(|cell| {
        *cell.borrow_mut() = Some(response);
    });
}

/// Get the last response.
pub fn get_last_response() -> Option<Value> {
    LAST_RESPONSE.with(|cell| cell.borrow().clone())
}

/// Store the last request for inspection.
pub fn set_last_request(request: HashMap<String, Value>) {
    LAST_REQUEST.with(|cell| {
        *cell.borrow_mut() = Some(request);
    });
}

/// Get the last request.
pub fn get_last_request() -> Option<HashMap<String, Value>> {
    LAST_REQUEST.with(|cell| cell.borrow().clone())
}

/// Store the last view assigns.
pub fn set_last_assigns(assigns: HashMap<String, Value>) {
    LAST_ASSIGNS.with(|cell| {
        *cell.borrow_mut() = Some(assigns);
    });
}

/// Get the last view assigns.
pub fn get_last_assigns() -> Option<HashMap<String, Value>> {
    LAST_ASSIGNS.with(|cell| cell.borrow().clone())
}

/// Store the last view path.
pub fn set_last_view_path(path: String) {
    LAST_VIEW_PATH.with(|cell| {
        *cell.borrow_mut() = Some(path);
    });
}

/// Get the last view path.
pub fn get_last_view_path() -> Option<String> {
    LAST_VIEW_PATH.with(|cell| cell.borrow().clone())
}

/// Set the current user from session.
pub fn set_current_user(user: Value) {
    CURRENT_USER.with(|cell| {
        *cell.borrow_mut() = Some(user);
    });
}

/// Get the current user.
pub fn get_current_user() -> Option<Value> {
    CURRENT_USER.with(|cell| cell.borrow().clone())
}

/// Clear the current user.
pub fn clear_current_user() {
    CURRENT_USER.with(|cell| {
        *cell.borrow_mut() = None;
    });
}
