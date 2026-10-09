//! Test-only helpers that need `&mut Interpreter` to run user blocks.

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{NativeFunction, Value};

pub fn register_test_helpers(env: &mut Environment) {
    // with_transaction(fn() { ... }) — begin → run block → always rollback.
    // Real work happens in the `evaluate_call` interceptor (needs the
    // interpreter to invoke the block); this placeholder catches misuse.
    env.define(
        "with_transaction".to_string(),
        Value::NativeFunction(NativeFunction::new("with_transaction", Some(1), |_args| {
            Err(
                "with_transaction() expects a function block: with_transaction(fn() { ... })"
                    .to_string(),
            )
        })),
    );

    env.define(
        "solidb_available?".to_string(),
        Value::NativeFunction(NativeFunction::new("solidb_available?", Some(0), |_args| {
            Ok(Value::Bool(solidb_reachable().is_ok()))
        })),
    );
    env.define(
        "solikv_available?".to_string(),
        Value::NativeFunction(NativeFunction::new("solikv_available?", Some(0), |_args| {
            Ok(Value::Bool(solikv_reachable().is_ok()))
        })),
    );

    // `before_each do requires_solidb() end` — skip the test when SoliDB does
    // not answer, so a missing database reads as "pending", never as a pass.
    // Under SOLI_REQUIRE_DB=1 (CI) the skip is a failure instead.
    env.define(
        "requires_solidb".to_string(),
        Value::NativeFunction(NativeFunction::new("requires_solidb", Some(0), |_args| {
            require_service("SoliDB", solidb_reachable())
        })),
    );
    env.define(
        "requires_solikv".to_string(),
        Value::NativeFunction(NativeFunction::new("requires_solikv", Some(0), |_args| {
            require_service("SoliKV", solikv_reachable())
        })),
    );
}

thread_local! {
    /// Probe results, once per worker thread: a spec file asks before every
    /// test, and a dead host would cost a connect timeout each time.
    static SOLIDB_PROBE: std::cell::RefCell<Option<Result<(), String>>> =
        const { std::cell::RefCell::new(None) };
    static SOLIKV_PROBE: std::cell::RefCell<Option<Result<(), String>>> =
        const { std::cell::RefCell::new(None) };
}

fn cached_probe(
    cell: &'static std::thread::LocalKey<std::cell::RefCell<Option<Result<(), String>>>>,
    probe: fn() -> Result<(), String>,
) -> Result<(), String> {
    cell.with(|slot| slot.borrow_mut().get_or_insert_with(probe).clone())
}

/// Whether the SoliDB the models talk to answers its health check.
#[cfg(not(target_arch = "wasm32"))]
fn solidb_reachable() -> Result<(), String> {
    cached_probe(&SOLIDB_PROBE, || {
        let url = crate::interpreter::builtins::model::db_config::db_url("/_api/health");
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(2))
            .build();
        agent
            .get(&url)
            .call()
            .map(|_| ())
            .map_err(|e| format!("no SoliDB answers {url} ({e})"))
    })
}
#[cfg(target_arch = "wasm32")]
fn solidb_reachable() -> Result<(), String> {
    Err(crate::platform::unsupported_on_edge("solidb_reachable"))
}

/// Whether the SoliKV that `KV` and `Cache` use answers a PING.
fn solikv_reachable() -> Result<(), String> {
    cached_probe(&SOLIKV_PROBE, || {
        crate::interpreter::builtins::solikv::solikv_cmd(&["PING"])
            .map(|_| ())
            .map_err(|e| format!("no SoliKV ({e})"))
    })
}

fn require_service(service: &str, probe: Result<(), String>) -> Result<Value, String> {
    let Err(why) = probe else {
        return Ok(Value::Null);
    };
    if crate::platform::env::var("SOLI_REQUIRE_DB").is_ok_and(|flag| flag == "1") {
        return Err(format!(
            "SOLI_REQUIRE_DB=1 but this test needs {service}: {why}"
        ));
    }
    crate::interpreter::builtins::test_dsl::skip_test(&format!("needs {service}: {why}"))
}
