//! Mock — engine-embedded Soli stdlib for test doubles.
//!
//! The class lives in `mock.sl` and is evaluated once per thread, like `Retry`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

pub(crate) const MOCK_SOURCE: &str = include_str!("mock.sl");

thread_local! {
    /// The evaluated Mock class, once per thread, in an environment of its own
    /// for the same reason as `Retry` (see `retry.rs`): evaluating it into the
    /// environment that receives it made the two hold each other forever.
    static MOCK_CLASS: RefCell<Option<Value>> = const { RefCell::new(None) };
}

/// Define the embedded `Mock` test-double class in `env`. Only in the test
/// environment (`APP_ENV=test`, which `soli test` sets): production apps keep
/// the name free for their own classes.
pub fn register_mock_class(env: &Rc<RefCell<Environment>>) -> Result<(), String> {
    if std::env::var("APP_ENV").as_deref() != Ok("test") {
        return Ok(());
    }
    let class = mock_class()?;
    env.borrow_mut().define("Mock".to_string(), class);
    Ok(())
}

fn mock_class() -> Result<Value, String> {
    if let Some(class) = MOCK_CLASS.with(|slot| slot.borrow().clone()) {
        return Ok(class);
    }

    let tokens = crate::lexer::Scanner::new(MOCK_SOURCE)
        .scan_tokens()
        .map_err(|e| format!("mock stdlib lexer error: {}", e))?;
    let program = crate::parser::Parser::new(tokens)
        .parse()
        .map_err(|e| format!("mock stdlib parser error: {}", e))?;
    // The class body only needs builtins: the blocks it runs are the caller's
    // closures, which carry their own environment.
    let home = Rc::new(RefCell::new(Environment::with_builtins_capacity()));
    crate::interpreter::builtins::register_builtins(&mut home.borrow_mut(), false);
    define_stub_natives(&home);
    let mut interpreter = crate::interpreter::Interpreter::with_environment(home.clone());
    for stmt in &program.statements {
        interpreter
            .execute(stmt)
            .map_err(|e| format!("mock stdlib eval error: {}", e))?;
    }
    let class = home
        .borrow()
        .get("Mock")
        .ok_or_else(|| "mock stdlib did not define Mock".to_string())?;
    MOCK_CLASS.with(|slot| *slot.borrow_mut() = Some(class.clone()));
    Ok(class)
}

/// Number of live stubs on any thread. Member access checks this one relaxed
/// load first, so an application that never stubs anything pays nothing else.
static LIVE_STUBS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// `Class.method` (static) / `Class#method` (instance) -> the `Mock` that
    /// answers it. Per thread: test files run on their own worker threads.
    static STUBS: RefCell<HashMap<String, (Value, bool)>> = RefCell::new(HashMap::new());
}

thread_local! {
    /// Set while the interpreter resolves the *real* method behind a spy, so the
    /// stub hook does not intercept its own lookup.
    static BYPASS: Cell<bool> = const { Cell::new(false) };
}

pub fn stubs_active() -> bool {
    LIVE_STUBS.load(Ordering::Relaxed) > 0 && !BYPASS.with(Cell::get)
}

/// Run `f` with the stub hook off (the lookup of a spy's original method).
pub fn with_stubs_bypassed<T>(f: impl FnOnce() -> T) -> T {
    let previous = BYPASS.with(|flag| flag.replace(true));
    let result = f();
    BYPASS.with(|flag| flag.set(previous));
    result
}

/// Append `{name, args}` to a `Mock`'s `calls_made`, the same record its own
/// `method_missing` writes, so `assert_received` and friends work on a spy.
pub fn record_call(mock: &Value, name: &str, args: &[Value]) {
    let Value::Instance(instance) = mock else {
        return;
    };
    let calls = instance.borrow().get("calls_made");
    if let Some(Value::Array(calls)) = calls {
        let args = Value::Array(Rc::new(RefCell::new(args.to_vec())));
        calls
            .borrow_mut()
            .push(crate::interpreter::value::hash_from_pairs([
                ("name", Value::String(name.into())),
                ("args", args),
            ]));
    }
}

/// The `Mock` standing in for `name` on `class` (or on one of its ancestors),
/// if `Mock.stub_class` / `Mock.stub_instance` put one there.
/// The bool is true for a spy: record the call, then run the real method.
pub fn lookup_stub(class: &Class, name: &str, instance: bool) -> Option<(Value, bool)> {
    let separator = if instance { '#' } else { '.' };
    STUBS.with(|stubs| {
        let stubs = stubs.borrow();
        if stubs.is_empty() {
            return None;
        }
        let mut cursor: Option<&Class> = Some(class);
        while let Some(current) = cursor {
            if let Some(entry) = stubs.get(&format!("{}{}{}", current.name, separator, name)) {
                return Some(entry.clone());
            }
            cursor = current.superclass.as_deref();
        }
        None
    })
}

/// Drop every stub of this thread (`Mock.unstub_all()`).
pub fn clear_stubs() {
    restore_stubs(StubSnapshot::default());
}

/// This thread's stubs at one moment, to go back to later.
#[derive(Clone, Default)]
pub struct StubSnapshot(HashMap<String, (Value, bool)>);

pub fn snapshot_stubs() -> StubSnapshot {
    StubSnapshot(STUBS.with(|stubs| stubs.borrow().clone()))
}

/// Put this thread's stubs back to `snapshot`. The test runner takes one after a
/// suite's `before_all` and restores it after each test, so a stub set in a
/// test or `before_each` ends with that test while one from `before_all` lasts
/// the whole suite; the snapshot from before `before_all` is restored once the
/// suite is done.
pub fn restore_stubs(snapshot: StubSnapshot) {
    let (before, after) = STUBS.with(|stubs| {
        let mut stubs = stubs.borrow_mut();
        let before = stubs.len();
        *stubs = snapshot.0;
        (before, stubs.len())
    });
    if after > before {
        LIVE_STUBS.fetch_add(after - before, Ordering::Relaxed);
    } else {
        LIVE_STUBS.fetch_sub(before - after, Ordering::Relaxed);
    }
}

/// The two natives `mock.sl` needs; defined in the class's home environment
/// only, so they are not part of the language's global surface.
fn define_stub_natives(home: &Rc<RefCell<Environment>>) {
    let register = NativeFunction::new("__mock_register_stub", Some(5), |args| {
        let (
            Value::Class(target),
            Value::String(name),
            mock,
            Value::Bool(instance),
            Value::Bool(spy),
        ) = (&args[0], &args[1], &args[2], &args[3], &args[4])
        else {
            return Err("Mock.stub_*: expected (Class, String, Mock, Bool, Bool)".to_string());
        };
        let separator = if *instance { '#' } else { '.' };
        let key = format!("{}{}{}", target.name, separator, name);
        let replaced = STUBS.with(|stubs| stubs.borrow_mut().insert(key, (mock.clone(), *spy)));
        if replaced.is_none() {
            LIVE_STUBS.fetch_add(1, Ordering::Relaxed);
        }
        Ok(Value::Null)
    });
    let clear = NativeFunction::new("__mock_clear_stubs", Some(0), |_args| {
        clear_stubs();
        Ok(Value::Null)
    });
    let class_name = NativeFunction::new("__mock_class_name", Some(1), |args| match &args[0] {
        Value::Class(target) => Ok(Value::String(target.name.clone().into())),
        _ => Err("Mock.stub_*: the target must be a class".to_string()),
    });
    let mut env = home.borrow_mut();
    env.define(
        "__mock_class_name".to_string(),
        Value::NativeFunction(class_name),
    );
    env.define(
        "__mock_register_stub".to_string(),
        Value::NativeFunction(register),
    );
    env.define(
        "__mock_clear_stubs".to_string(),
        Value::NativeFunction(clear),
    );
}
