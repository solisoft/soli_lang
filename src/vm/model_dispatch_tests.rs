//! Integration tests for VM dispatch of model native methods and lifecycle
//! callbacks. These mirror the production setup: a tree-walking `Interpreter`
//! defines the model classes and handlers, its globals are copied into a `Vm`
//! (exactly as `serve` does at startup), and handlers are invoked on the VM.
//!
//! They guard two bugs that were invisible because every existing Soli test
//! runs on the tree-walking executor, never the VM. Bug A: the VM dropped the
//! receiver for native instance methods, so `record.delete()` reached the
//! native with no `args[0]`. Bug B: the VM ran no lifecycle callbacks, so
//! `before_delete` never fired. Neither test touches a database — Bug A is
//! detected by the native's own "missing _key" guard, and Bug B vetoes
//! (`before_delete` returns `false`) before any DB call.

#[cfg(test)]
mod tests {
    use crate::interpreter::value::{Instance, Value};
    use crate::interpreter::Interpreter;
    use crate::span::Span;
    use crate::vm::Vm;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Build a full `Interpreter`, run `program` so model classes lazy-load and
    /// handlers are defined, then copy its globals into a fresh `Vm` (the same
    /// hand-off `serve` performs for production mode).
    fn interp_and_vm(program: &str) -> (Interpreter, Vm) {
        let mut interp = Interpreter::new();
        let parsed = crate::parse(program).expect("program should parse");
        interp.interpret(&parsed).expect("program should run");

        let mut vm = Vm::new();
        for (name, value) in interp.environment.borrow().get_all_bindings() {
            vm.globals.insert(name, value);
        }
        (interp, vm)
    }

    fn instance_of(interp: &Interpreter, class_name: &str) -> Rc<RefCell<Instance>> {
        let class_val = interp
            .environment
            .borrow()
            .get(class_name)
            .unwrap_or_else(|| panic!("class {class_name} should be defined"));
        let class_rc = match class_val {
            Value::Class(c) => c,
            other => panic!("{class_name} is not a class: {other:?}"),
        };
        Rc::new(RefCell::new(Instance::new(class_rc)))
    }

    // Bug A: the VM must prepend the receiver to native instance methods.
    // With the fix, `req.delete()` reaches `Model#delete`, which finds no
    // `_key` and returns that specific error. Without the fix the native saw
    // empty args and failed with "Expected instance".
    #[test]
    fn vm_passes_receiver_to_native_instance_method() {
        let (interp, mut vm) =
            interp_and_vm("class Widget extends Model\nend\nfn probe(req) { return req.delete() }");
        let inst = instance_of(&interp, "Widget"); // no _key set

        let probe = vm.globals.get("probe").cloned().expect("probe defined");
        let result = vm.call_value_direct_one(probe, Value::Instance(inst), Span::default());

        let err = result.expect_err("delete on a keyless instance should error");
        let msg = format!("{err}");
        assert!(
            msg.contains("_key"),
            "expected the native's missing-_key guard (receiver reached it), got: {msg}"
        );
        assert!(
            !msg.contains("Expected instance"),
            "receiver was dropped before reaching the native: {msg}"
        );
    }

    // Bug B: the VM must run lifecycle callbacks. A `before_delete` that returns
    // `false` vetoes the delete before any DB call, so this test never needs a
    // database. We assert both the side effect (callback ran) and the veto
    // result (`false`).
    #[test]
    fn vm_runs_before_delete_callback_and_honors_veto() {
        let (interp, mut vm) = interp_and_vm(
            "class Vetoer extends Model\n  before_delete(:block_it)\n  fn block_it {\n    this.veto_ran = true\n    return false\n  }\nend\nfn probe(req) { return req.delete() }",
        );
        let inst = instance_of(&interp, "Vetoer");
        // Give it a _key so that, absent the veto, the native would attempt a
        // real DB delete — proving the veto (not a missing key) short-circuits.
        inst.borrow_mut()
            .set("_key".to_string(), Value::String("k1".to_string()));

        let probe = vm.globals.get("probe").cloned().expect("probe defined");
        let result = vm
            .call_value_direct_one(probe, Value::Instance(inst.clone()), Span::default())
            .expect("vetoed delete should return cleanly, not error");

        assert_eq!(
            inst.borrow().get("veto_ran"),
            Some(Value::Bool(true)),
            "before_delete callback did not run in the VM"
        );
        assert_eq!(
            result,
            Value::Bool(false),
            "a before_delete returning false must veto the delete"
        );
    }

    // Model.create(...) is a class static — exercises the VM's class-receiver
    // prepend (Bug A for statics) plus before_save/before_create. The veto
    // short-circuits before any DB call and returns the instance with _errors.
    #[test]
    fn vm_runs_before_create_callback_on_static_create_and_vetoes() {
        let (interp, mut vm) = interp_and_vm(
            "class Gated extends Model\n  before_create(:deny)\n  fn deny {\n    this.denied = true\n    return false\n  }\nend\nfn probe(req) { return Gated.create({\"name\": \"x\"}) }",
        );
        let _ = interp; // class lives in vm.globals after the copy

        let probe = vm.globals.get("probe").cloned().expect("probe defined");
        let result = vm
            .call_value_direct_one(probe, Value::Null, Span::default())
            .expect("vetoed create should return an instance, not error");

        let inst = match result {
            Value::Instance(inst) => inst,
            other => panic!("Model.create should return an instance, got {other:?}"),
        };
        assert_eq!(
            inst.borrow().get("denied"),
            Some(Value::Bool(true)),
            "before_create callback did not run on the static Model.create in the VM"
        );
        assert!(
            inst.borrow().get("_errors").is_some(),
            "a vetoed create must surface _errors"
        );
    }

    // instance.save() on a brand-new (keyless) instance runs the create chain;
    // a before_save veto returns false before any DB call.
    #[test]
    fn vm_runs_before_save_callback_on_instance_save_and_vetoes() {
        let (interp, mut vm) = interp_and_vm(
            "class SaveGated extends Model\n  before_save(:block)\n  fn block {\n    this.blocked = true\n    return false\n  }\nend\nfn probe(req) { return req.save() }",
        );
        let inst = instance_of(&interp, "SaveGated");

        let probe = vm.globals.get("probe").cloned().expect("probe defined");
        let result = vm
            .call_value_direct_one(probe, Value::Instance(inst.clone()), Span::default())
            .expect("vetoed save should return cleanly");

        assert_eq!(
            inst.borrow().get("blocked"),
            Some(Value::Bool(true)),
            "before_save callback did not run on instance.save() in the VM"
        );
        assert_eq!(
            result,
            Value::Bool(false),
            "before_save veto must return false"
        );
    }
}
