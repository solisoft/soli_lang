use std::cell::RefCell;

thread_local! {
    static ENGINE_CONTEXT: RefCell<Option<String>> = const { RefCell::new(None) };
}

pub fn set_model_engine_context(engine: Option<&str>) {
    ENGINE_CONTEXT.with(|ctx| {
        *ctx.borrow_mut() = engine.map(String::from);
    });
}

pub fn get_model_engine_context() -> Option<String> {
    ENGINE_CONTEXT.with(|ctx| ctx.borrow().clone())
}

/// RAII guard that sets the engine context on creation and clears it on drop.
pub struct EngineContextGuard;

impl EngineContextGuard {
    pub fn enter(name: &str) -> Self {
        set_model_engine_context(Some(name));
        Self
    }
}

impl Drop for EngineContextGuard {
    fn drop(&mut self) {
        set_model_engine_context(None);
    }
}
