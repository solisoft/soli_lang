//! Served-action regressions for the VM's dispatch: a worker loads the app on
//! the tree-walker, copies its globals into the VM, and runs every action on
//! the VM (`call_class_method`). These run that arrangement in-process and
//! assert the VM answered without demoting the action, so a VM gap cannot hide
//! behind the tree-walker re-run.

use std::cell::RefCell;
use std::rc::Rc;

use super::call_class_method;
use crate::interpreter::value::{HashPairs, Instance, Value};
use crate::interpreter::Interpreter;
use crate::lexer::Scanner;
use crate::parser::Parser;

/// An app loaded the way a production worker loads it.
pub(super) struct ServedApp {
    pub interpreter: Interpreter,
    pub vm: crate::vm::Vm,
}

impl ServedApp {
    pub fn load(source: &str) -> Self {
        let tokens = Scanner::new(source).scan_tokens().expect("lexer error");
        let program = Parser::new(tokens).parse().expect("parser error");
        let mut interpreter = Interpreter::new();
        interpreter.interpret(&program).expect("app should load");
        let mut vm = crate::vm::Vm::new();
        let all_globals = interpreter.environment.borrow().get_all_bindings();
        for (name, value) in all_globals {
            vm.globals.insert(name, value);
        }
        ServedApp { interpreter, vm }
    }

    /// Run `Class#action` on the VM, as a request would, and insist the VM
    /// answered: a demotion re-runs the action on the tree-walker, which
    /// would hide what the VM did.
    pub fn call(&mut self, class_name: &str, action: &str) -> Value {
        let class = match self.interpreter.environment.borrow().get(class_name) {
            Some(Value::Class(class)) => class,
            other => panic!("{class_name} is not a class: {other:?}"),
        };
        let instance = Value::Instance(Rc::new(RefCell::new(Instance::new(class.clone()))));
        let request = Value::Hash(Rc::new(RefCell::new(HashPairs::default())));
        let result = call_class_method(
            &mut self.interpreter,
            Some(&mut self.vm),
            &class,
            &instance,
            action,
            &request,
        )
        .unwrap_or_else(|e| panic!("{class_name}#{action} failed: {e}"));
        assert!(
            self.vm.failed_handlers.is_empty(),
            "{class_name}#{action} was demoted to the tree-walker: {:?}",
            self.vm.failed_handlers
        );
        result
    }
}

const POLICIES: &str = r#"
class ArticlePolicy
  def show?
    true
  end
end

class AdminReportPolicy < ArticlePolicy
  def show?
    false
  end
end

class Base
  def m
    1
  end
end

class Mid < Base
  def m
    super.m() + 10
  end
end

class Leaf < Mid
  def m
    super.m() + 100
  end
end

class PagesController
  def articles
    ArticlePolicy.new().show?()
  end

  def reports
    AdminReportPolicy.new().show?()
  end

  def reports_by_send
    AdminReportPolicy.new().send("show?")
  end

  def reports_bare
    policy = AdminReportPolicy.new()
    policy.show?
  end

  def base_m
    Base.new().m()
  end

  def leaf_m
    Leaf.new().m()
  end
end
"#;

/// A subclass override must win even after the parent's method was compiled.
///
/// The VM caches a compiled tree-walker method on the class that declares it,
/// and dispatch took the first compiled method anywhere up the chain before
/// compiling the receiver's own. So once `ArticlePolicy#show?` had run on a
/// worker, `AdminReportPolicy#show?` (admin-only) was skipped for it and a
/// guest was let through.
#[test]
fn a_subclass_override_wins_after_the_parent_method_was_compiled() {
    let mut app = ServedApp::load(POLICIES);
    assert_eq!(app.call("PagesController", "articles"), Value::Bool(true));
    assert_eq!(
        app.call("PagesController", "reports"),
        Value::Bool(false),
        "a direct call must reach the subclass override"
    );
    assert_eq!(
        app.call("PagesController", "reports_by_send"),
        Value::Bool(false),
        "send must reach the subclass override"
    );
    assert_eq!(
        app.call("PagesController", "reports_bare"),
        Value::Bool(false),
        "a paren-less call must reach the subclass override"
    );
}

/// The same through `send` first, on a fresh worker: the parent compiled by
/// `send`, then the child reached by `send`.
#[test]
fn send_reaches_the_override_after_the_parent_was_sent_to() {
    let mut app = ServedApp::load(&format!(
        "{POLICIES}\nclass SendController\n  def parent\n    ArticlePolicy.new().send(\"show?\")\n  end\nend\n"
    ));
    assert_eq!(app.call("SendController", "parent"), Value::Bool(true));
    assert_eq!(
        app.call("PagesController", "reports_by_send"),
        Value::Bool(false)
    );
}

/// `super.m()` resolves level by level too: with `Base#m` compiled first,
/// `Leaf#m`'s `super` must reach `Mid#m`, not jump to `Base#m`.
#[test]
fn super_reaches_the_nearest_override_after_an_ancestor_was_compiled() {
    let mut app = ServedApp::load(POLICIES);
    assert_eq!(app.call("PagesController", "base_m"), Value::Int(1));
    assert_eq!(app.call("PagesController", "leaf_m"), Value::Int(111));
    // And again, now that every level is compiled.
    assert_eq!(app.call("PagesController", "leaf_m"), Value::Int(111));
}

const GUARDED: &str = r#"
def signed_in?
  false
end

def current_name
  "guest"
end

class AccountController
  def show
    return "redirect:/login" unless signed_in?

    "secret page"
  end

  def greet
    name = current_name
    "hello #{name} #{current_name.upcase}"
  end

  def keeps_lambdas
    callback = fn() { "called" }
    holder = callback
    holder()
  end
end
"#;

/// A zero-argument global def read bare is called on the VM, as on the
/// tree-walker. The VM read `signed_in?` as the function itself — always
/// truthy — so `return redirect("/login") unless signed_in?` served the
/// protected page in production while --dev redirected.
#[test]
fn a_bare_zero_argument_global_is_called_in_a_vm_action() {
    let mut app = ServedApp::load(GUARDED);
    assert_eq!(
        app.call("AccountController", "show"),
        Value::String("redirect:/login".into())
    );
    assert_eq!(
        app.call("AccountController", "greet"),
        Value::String("hello guest GUEST".into())
    );
    assert_eq!(
        app.call("AccountController", "keeps_lambdas"),
        Value::String("called".into()),
        "a lambda kept in a variable is read as the function"
    );
}

/// A zero-parameter static that reads `this`, read bare (`Post.recent`, the
/// house style), runs once and returns to the action. It ran through
/// `call_method_bound_args`, whose `run()` did not stop at its own frame: the
/// rest of the action ran inside it (its side effects twice), and the outer
/// dispatch loop then panicked on an empty frame stack — a 500.
#[test]
fn a_bare_static_reading_this_returns_to_the_action() {
    let mut app = ServedApp::load(
        r##"
hits = []

class Probe
  static def me
    this
  end
end

class BareStaticController
  def index
    klass = Probe.me
    hits.push("after")
    "#{klass} #{hits.length}"
  end
end
"##,
    );
    assert_eq!(
        app.call("BareStaticController", "index"),
        Value::String("<class Probe> 1".into())
    );
    let hits = app.interpreter.environment.borrow().get("hits").unwrap();
    let Value::Array(hits) = hits else {
        panic!("hits is not an array: {hits:?}")
    };
    assert_eq!(hits.borrow().len(), 1, "the action's side effect ran twice");
}
