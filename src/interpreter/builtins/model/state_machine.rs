//! Declarative, enum-backed state machines for models.
//!
//! Declared in a model class body with the same shape as `scope`/`validates`:
//!
//! ```soli
//! enum OrderState
//!   Pending, Paid, Shipped, Cancelled
//! end
//!
//! class Order < Model
//!   enum_field :status, OrderState
//!
//!   state_machine :status do
//!     initial OrderState.Pending
//!     event :pay do
//!       transition from: OrderState.Pending, to: OrderState.Paid
//!       guard fn() { this.total > 0 }
//!     end
//!     after_transition to: OrderState.Paid do this.send_receipt() end
//!   end
//! end
//! ```
//!
//! The plain transition table lives in the global `MODEL_REGISTRY`
//! (`ModelMetadata.state_machines`). Guard / before / after closures hold
//! `Rc<Function>` (which is `!Send`) so they live in the per-worker thread-locals
//! below — same split as `scopes::SCOPES` and `callbacks::CALLBACK_CLOSURES`.
//!
//! The DSL block invocation (`state_machine … do … end`, `event … do … end`)
//! and the runtime event dispatch both need `&mut Interpreter`, so they live in
//! `executor::calls::function` (a `NativeFunction` can't reach the interpreter).
//! This module exposes the recorder natives, the transient builder, the
//! registries, and the finalize/validation logic.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::value::{Class, Function, HashKey, NativeFunction, Value};

use super::registry::{get_enum_fields, get_state_machines};

/// One state machine declared on a model. Plain data only (Send + Sync) so it
/// can live in the global `MODEL_REGISTRY`.
#[derive(Debug, Clone)]
pub struct StateMachineDef {
    /// The `enum_field` column this machine drives, e.g. `"status"`.
    pub field: String,
    /// Name of the enum class backing the states, e.g. `"OrderState"`.
    pub enum_class_name: String,
    /// Initial variant tag, e.g. `"Pending"`.
    pub initial: Option<String>,
    /// Every distinct variant tag referenced (initial + all from/to), in
    /// declaration order. Drives state predicates and `Model.states`.
    pub states: Vec<String>,
    pub events: Vec<EventDef>,
}

#[derive(Debug, Clone)]
pub struct EventDef {
    pub name: String,
    pub transitions: Vec<TransitionDef>,
    /// Whether a `guard` closure was registered for this event.
    pub has_guard: bool,
}

#[derive(Debug, Clone)]
pub struct TransitionDef {
    /// One or more source variant tags this transition is legal from.
    pub from: Vec<String>,
    /// The destination variant tag.
    pub to: String,
}

impl StateMachineDef {
    pub fn event(&self, name: &str) -> Option<&EventDef> {
        self.events.iter().find(|e| e.name == name)
    }

    /// The destination tag for `event` from `current`, if any transition allows it.
    pub fn target_for(&self, event: &str, current: &str) -> Option<&str> {
        let ev = self.event(event)?;
        ev.transitions
            .iter()
            .find(|t| t.from.iter().any(|f| f == current))
            .map(|t| t.to.as_str())
    }
}

// ---------------------------------------------------------------------------
// Transient builder — only alive while a model class body evaluates.
// ---------------------------------------------------------------------------

struct EventBuilder {
    name: String,
    transitions: Vec<TransitionDef>,
    guard: Option<Rc<Function>>,
}

struct StateMachineBuilder {
    field: String,
    initial: Option<String>,
    events: Vec<EventBuilder>,
    current_event: Option<EventBuilder>,
    before: Vec<(String, Rc<Function>)>,
    after: Vec<(String, Rc<Function>)>,
}

impl StateMachineBuilder {
    fn new(field: String) -> Self {
        Self {
            field,
            initial: None,
            events: Vec::new(),
            current_event: None,
            before: Vec::new(),
            after: Vec::new(),
        }
    }
}

/// Guard closures keyed by `(class, event)`.
type GuardMap = HashMap<(String, String), Rc<Function>>;
/// before/after-transition hooks keyed by `(class, to_tag)`.
type HookMap = HashMap<(String, String), Vec<Rc<Function>>>;

thread_local! {
    /// Stack of state machines currently being built (one frame per active
    /// `state_machine … do … end`). A stack, not a single slot, so a nested or
    /// re-entrant declaration can't clobber an outer one.
    static SM_BUILDER_STACK: RefCell<Vec<StateMachineBuilder>> = const { RefCell::new(Vec::new()) };

    /// Guard closures keyed by `(class, event)`. `this` is bound to the
    /// instance; returns a truthy/falsy value.
    static SM_GUARDS: RefCell<GuardMap> = RefCell::new(HashMap::new());
    /// before-transition hooks keyed by `(class, to_tag)`.
    static SM_BEFORE: RefCell<HookMap> = RefCell::new(HashMap::new());
    /// after-transition hooks keyed by `(class, to_tag)`.
    static SM_AFTER: RefCell<HookMap> = RefCell::new(HashMap::new());
}

/// True while a `state_machine` block is being evaluated. Used to scope the
/// `event(...)` interceptor tightly so it only fires inside an SM block.
pub fn builder_active() -> bool {
    SM_BUILDER_STACK.with(|s| !s.borrow().is_empty())
}

pub fn push_builder(field: String) {
    SM_BUILDER_STACK.with(|s| s.borrow_mut().push(StateMachineBuilder::new(field)));
}

/// Begin an `event` frame. Errors if called outside a machine or while another
/// event is already open (no nested events).
pub fn begin_event(name: String) -> Result<(), String> {
    SM_BUILDER_STACK.with(|s| {
        let mut stack = s.borrow_mut();
        let Some(top) = stack.last_mut() else {
            return Err("event() can only be used inside a state_machine block".to_string());
        };
        if top.current_event.is_some() {
            return Err("event() cannot be nested inside another event".to_string());
        }
        top.current_event = Some(EventBuilder {
            name,
            transitions: Vec::new(),
            guard: None,
        });
        Ok(())
    })
}

/// Close the current `event` frame, attaching it to the machine being built.
pub fn end_event() -> Result<(), String> {
    SM_BUILDER_STACK.with(|s| {
        let mut stack = s.borrow_mut();
        let Some(top) = stack.last_mut() else {
            return Err("event() can only be used inside a state_machine block".to_string());
        };
        let Some(ev) = top.current_event.take() else {
            return Err("internal: no open event to close".to_string());
        };
        if top.events.iter().any(|e| e.name == ev.name) {
            return Err(format!("duplicate event '{}' in state machine", ev.name));
        }
        top.events.push(ev);
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Recorder natives — registered as globals, mutate the top builder frame.
// They only record data + stash closures, so a plain NativeFunction suffices.
// ---------------------------------------------------------------------------

/// Pull a variant tag out of an enum value (or a bare tag string/symbol).
fn extract_tag(v: &Value) -> Result<String, String> {
    match v {
        Value::Instance(inst) => {
            let b = inst.borrow();
            match b.fields.get("__variant") {
                Some(Value::String(tag)) => Ok(tag.to_string()),
                _ => Err(format!(
                    "expected an enum value, got a '{}' instance (no variant tag)",
                    b.class.name
                )),
            }
        }
        Value::String(s) => Ok(s.to_string()),
        Value::Symbol(s) => Ok(s.to_string()),
        other => Err(format!("expected an enum value, got {}", other.type_name())),
    }
}

/// A `from` may be a single state or an array of states.
fn extract_tags(v: &Value) -> Result<Vec<String>, String> {
    if let Value::Array(arr) = v {
        arr.borrow().iter().map(extract_tag).collect()
    } else {
        Ok(vec![extract_tag(v)?])
    }
}

fn as_function(v: &Value) -> Result<Rc<Function>, String> {
    match v {
        Value::Function(f) => Ok(f.clone()),
        other => Err(format!("expected a function, got {}", other.type_name())),
    }
}

fn hash_get<'a>(hash: &'a crate::interpreter::value::HashPairs, key: &str) -> Option<&'a Value> {
    hash.get(&HashKey::String(key.into()))
}

fn with_current_event<F>(f: F) -> Result<(), String>
where
    F: FnOnce(&mut EventBuilder) -> Result<(), String>,
{
    SM_BUILDER_STACK.with(|s| {
        let mut stack = s.borrow_mut();
        let Some(top) = stack.last_mut() else {
            return Err("this DSL call can only be used inside a state_machine block".to_string());
        };
        let Some(ev) = top.current_event.as_mut() else {
            return Err("transition()/guard() can only be used inside an event block".to_string());
        };
        f(ev)
    })
}

/// `initial(StateEnum.Variant)` — sets the machine's initial state.
fn record_initial(args: Vec<Value>) -> Result<Value, String> {
    let arg = args
        .first()
        .ok_or_else(|| "initial(state) expects one argument".to_string())?;
    let tag = extract_tag(arg)?;
    SM_BUILDER_STACK.with(|s| -> Result<(), String> {
        let mut stack = s.borrow_mut();
        let top = stack
            .last_mut()
            .ok_or_else(|| "initial() can only be used inside a state_machine block".to_string())?;
        top.initial = Some(tag);
        Ok(())
    })?;
    Ok(Value::Null)
}

/// `transition(from, to)` or `transition({"from": ..., "to": ...})`.
fn record_transition(args: Vec<Value>) -> Result<Value, String> {
    let (from, to) = match args.as_slice() {
        [Value::Hash(h)] => {
            let h = h.borrow();
            let from = hash_get(&h, "from")
                .ok_or_else(|| "transition(...) is missing 'from'".to_string())?;
            let to =
                hash_get(&h, "to").ok_or_else(|| "transition(...) is missing 'to'".to_string())?;
            (extract_tags(from)?, extract_tag(to)?)
        }
        [from, to] => (extract_tags(from)?, extract_tag(to)?),
        _ => return Err("transition expects (from, to) or a {from:, to:} hash".to_string()),
    };
    with_current_event(|ev| {
        ev.transitions.push(TransitionDef { from, to });
        Ok(())
    })?;
    Ok(Value::Null)
}

/// `guard fn() { ... }` — a predicate run with `this` bound to the instance.
fn record_guard(args: Vec<Value>) -> Result<Value, String> {
    let func = as_function(
        args.first()
            .ok_or_else(|| "guard(fn) expects a function".to_string())?,
    )?;
    with_current_event(|ev| {
        ev.guard = Some(func);
        Ok(())
    })?;
    Ok(Value::Null)
}

/// `before_transition(to_state, fn)` / `before_transition(to: X) do ... end`.
fn record_hook(args: Vec<Value>, after: bool) -> Result<Value, String> {
    let label = if after {
        "after_transition"
    } else {
        "before_transition"
    };
    if args.len() < 2 {
        return Err(format!("{}(to_state, fn) expects two arguments", label));
    }
    let func = as_function(args.last().unwrap())?;
    let to_tag = match &args[0] {
        Value::Hash(h) => {
            let h = h.borrow();
            let to = hash_get(&h, "to").ok_or_else(|| format!("{}(...) is missing 'to'", label))?;
            extract_tag(to)?
        }
        other => extract_tag(other)?,
    };
    SM_BUILDER_STACK.with(|s| -> Result<(), String> {
        let mut stack = s.borrow_mut();
        let top = stack
            .last_mut()
            .ok_or_else(|| format!("{}() can only be used inside a state_machine block", label))?;
        if after {
            top.after.push((to_tag, func));
        } else {
            top.before.push((to_tag, func));
        }
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Build the recorder natives that the DSL block calls. Registered as globals.
pub fn recorder_natives() -> Vec<(&'static str, NativeFunction)> {
    vec![
        (
            "initial",
            NativeFunction::new("initial", Some(1), record_initial),
        ),
        (
            "transition",
            NativeFunction::new("transition", None, record_transition),
        ),
        ("guard", NativeFunction::new("guard", Some(1), record_guard)),
        (
            "before_transition",
            NativeFunction::new("before_transition", None, |args| record_hook(args, false)),
        ),
        (
            "after_transition",
            NativeFunction::new("after_transition", None, |args| record_hook(args, true)),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Finalize: turn the popped builder into a registered definition + closures,
// validating against the backing enum.
// ---------------------------------------------------------------------------

/// Read the variant names declared on an enum class (`static const
/// __enum_variants = { "Active": [...], ... }`).
fn enum_variant_names(enum_class: &Rc<Class>) -> Vec<String> {
    if let Some(Value::Hash(h)) = enum_class.static_fields.borrow().get("__enum_variants") {
        return h
            .borrow()
            .iter()
            .filter_map(|(k, _)| match k {
                HashKey::String(s) => Some(s.to_string()),
                _ => None,
            })
            .collect();
    }
    Vec::new()
}

/// Build the enum `Value` for `tag` on a model's `enum_field` column, used when
/// a transition sets the new state. Returns `None` if the field isn't declared.
pub fn build_state_value(class_name: &str, field: &str, tag: &str) -> Option<Value> {
    let enum_class = get_enum_fields(class_name)
        .into_iter()
        .find(|(f, _)| f == field)
        .map(|(_, c)| c)?;
    Some(crate::interpreter::value::build_enum_value(
        &enum_class,
        &Value::String(tag.into()),
    ))
}

/// Pop the active builder, validate it against the backing `enum_field` enum,
/// and register it in the global registry plus the closure thread-locals.
/// Returns a clear error (raised by the caller) on any structural problem.
/// Always pops, so a failed validation can't leave a builder frame behind.
pub fn finalize(class_name: &str) -> Result<(), String> {
    let builder = SM_BUILDER_STACK
        .with(|s| s.borrow_mut().pop())
        .ok_or_else(|| "internal: no state machine builder to finalize".to_string())?;

    // The field must be backed by an `enum_field` declaration (declared earlier
    // in the same class body).
    let enum_class = get_enum_fields(class_name)
        .into_iter()
        .find(|(f, _)| f == &builder.field)
        .map(|(_, c)| c)
        .ok_or_else(|| {
            format!(
                "state_machine(:{field}) requires `enum_field :{field}, <Enum>` declared first",
                field = builder.field
            )
        })?;
    let enum_class_name = enum_class.name.clone();
    let valid: Vec<String> = enum_variant_names(&enum_class);
    let is_valid = |tag: &str| valid.iter().any(|v| v == tag);

    // Validate + collect declaration-order distinct states.
    let mut states: Vec<String> = Vec::new();
    let push_state = |tag: &str, states: &mut Vec<String>| -> Result<(), String> {
        if !is_valid(tag) {
            return Err(format!(
                "state machine for {}: '{}' is not a variant of enum {}",
                class_name, tag, enum_class_name
            ));
        }
        if !states.iter().any(|s| s == tag) {
            states.push(tag.to_string());
        }
        Ok(())
    };

    let initial = match &builder.initial {
        Some(tag) => {
            push_state(tag, &mut states)?;
            Some(tag.clone())
        }
        None => {
            return Err(format!(
                "state machine for {} has no `initial` state",
                class_name
            ))
        }
    };

    let mut events: Vec<EventDef> = Vec::new();
    for ev in &builder.events {
        for t in &ev.transitions {
            for f in &t.from {
                push_state(f, &mut states)?;
            }
            push_state(&t.to, &mut states)?;
        }
        events.push(EventDef {
            name: ev.name.clone(),
            transitions: ev.transitions.clone(),
            has_guard: ev.guard.is_some(),
        });
    }

    // Validate hook target states before committing closures.
    for (tag, _) in builder.before.iter().chain(builder.after.iter()) {
        push_state(tag, &mut states)?;
    }

    // Commit closures into the thread-local registries.
    for ev in &builder.events {
        if let Some(g) = &ev.guard {
            SM_GUARDS.with(|m| {
                m.borrow_mut()
                    .insert((class_name.to_string(), ev.name.clone()), g.clone());
            });
        }
    }
    for (tag, func) in builder.before {
        SM_BEFORE.with(|m| {
            m.borrow_mut()
                .entry((class_name.to_string(), tag))
                .or_default()
                .push(func);
        });
    }
    for (tag, func) in builder.after {
        SM_AFTER.with(|m| {
            m.borrow_mut()
                .entry((class_name.to_string(), tag))
                .or_default()
                .push(func);
        });
    }

    super::registry::set_state_machine(
        class_name,
        StateMachineDef {
            field: builder.field,
            enum_class_name,
            initial,
            states,
            events,
        },
    );
    Ok(())
}

/// Discard the active builder frame without registering it. Used to clean up
/// when the DSL block itself throws.
pub fn abort_builder() {
    SM_BUILDER_STACK.with(|s| {
        s.borrow_mut().pop();
    });
}

// ---------------------------------------------------------------------------
// Runtime lookups (used by the dispatch helpers in `executor`).
// ---------------------------------------------------------------------------

pub fn lookup_guard(class_name: &str, event: &str) -> Option<Rc<Function>> {
    SM_GUARDS.with(|m| {
        m.borrow()
            .get(&(class_name.to_string(), event.to_string()))
            .cloned()
    })
}

pub fn lookup_before(class_name: &str, to_tag: &str) -> Vec<Rc<Function>> {
    SM_BEFORE.with(|m| {
        m.borrow()
            .get(&(class_name.to_string(), to_tag.to_string()))
            .cloned()
            .unwrap_or_default()
    })
}

pub fn lookup_after(class_name: &str, to_tag: &str) -> Vec<Rc<Function>> {
    SM_AFTER.with(|m| {
        m.borrow()
            .get(&(class_name.to_string(), to_tag.to_string()))
            .cloned()
            .unwrap_or_default()
    })
}

/// All state machines registered on a class (from the global registry).
pub fn machines_for(class_name: &str) -> Vec<StateMachineDef> {
    get_state_machines(class_name)
}

/// Whether `name` resolves to a state machine member (event `pay`, bang `pay!`,
/// query `can_pay?`, or state predicate `paid?`) for `class_name`. Used by the
/// VM to decide when to fall back to the interpreter (which owns the full
/// guard/callback machinery).
pub fn is_sm_member(class_name: &str, name: &str) -> bool {
    let machines = machines_for(class_name);
    for machine in &machines {
        if let Some(stem) = name.strip_suffix('?') {
            if let Some(event) = stem.strip_prefix("can_") {
                if machine.event(event).is_some() {
                    return true;
                }
            }
            if machine.states.iter().any(|t| snake_case(t) == stem) {
                return true;
            }
        }
        let event = name.strip_suffix('!').unwrap_or(name);
        if machine.event(event).is_some() {
            return true;
        }
    }
    false
}

/// Snake-case an enum variant tag for predicate dispatch: `InTransit` →
/// `in_transit`, `Paid` → `paid`. ASCII-only, matching Soli's snake_case lint.
pub fn snake_case(tag: &str) -> String {
    let mut out = String::with_capacity(tag.len() + 4);
    for (i, ch) in tag.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Clear per-worker state machine closures (hot reload).
pub fn clear() {
    SM_BUILDER_STACK.with(|s| s.borrow_mut().clear());
    SM_GUARDS.with(|m| m.borrow_mut().clear());
    SM_BEFORE.with(|m| m.borrow_mut().clear());
    SM_AFTER.with(|m| m.borrow_mut().clear());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_cases_variant_tags() {
        assert_eq!(snake_case("Paid"), "paid");
        assert_eq!(snake_case("InTransit"), "in_transit");
        assert_eq!(snake_case("Pending"), "pending");
    }

    #[test]
    fn target_for_resolves_legal_transition() {
        let def = StateMachineDef {
            field: "status".into(),
            enum_class_name: "OrderState".into(),
            initial: Some("Pending".into()),
            states: vec!["Pending".into(), "Paid".into()],
            events: vec![EventDef {
                name: "pay".into(),
                transitions: vec![TransitionDef {
                    from: vec!["Pending".into()],
                    to: "Paid".into(),
                }],
                has_guard: false,
            }],
        };
        assert_eq!(def.target_for("pay", "Pending"), Some("Paid"));
        assert_eq!(def.target_for("pay", "Paid"), None);
        assert_eq!(def.target_for("ship", "Pending"), None);
    }
}
