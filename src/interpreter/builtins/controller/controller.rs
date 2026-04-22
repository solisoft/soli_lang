//! Controller implementation for OOP-style controllers.
//!
//! This module provides OOP-style controllers with:
//! - Class-based controller architecture
//! - Before/after action hooks
//! - Request context injection
//! - Shared helper methods
//!
//! # Example Usage
//!
//! ```soli
//! class ApplicationController extends Controller {
//!     static {
//!         this.layout = "application";
//!         this.before_action = fn(req) {
//!             let user_id = req.session["user_id"];
//!             if user_id != null {
//!                 req["current_user"] = User.find(user_id);
//!             }
//!             return req;
//!         };
//!     }
//! }
//!
//! class PostsController extends ApplicationController {
//!     static {
//!         this.layout = "posts";
//!         this.before_action(:show, :edit, :update, :delete) = fn(req) {
//!             let post = Post.find(req.params["id"]);
//!             if post == null {
//!                 return error(404, "Post not found");
//!             }
//!             req["post"] = post;
//!             return req;
//!         };
//!     }
//!
//!     fn index(req: Any) -> Any {
//!         let posts = Post.all();
//!         return render("posts/index", { "posts": posts });
//!     }
//!
//!     fn show(req: Any) -> Any {
//!         return render("posts/show", { "post": req["post"] });
//!     }
//!
//!     # Private helper (not exposed as action)
//!     fn _validate_post(req: Any) -> Bool {
//!         return req.params["title"] != null;
//!     }
//! }
//! ```

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::interpreter::environment::Environment;
use crate::interpreter::value::{Class, NativeFunction, Value};

#[cfg(test)]
mod tests {
    use super::*;

    fn controller_class() -> Rc<Class> {
        let mut env = Environment::new();
        register_controller_class(&mut env);
        match env.get("Controller").unwrap() {
            Value::Class(c) => c,
            _ => panic!("Controller should be registered as a Class"),
        }
    }

    // `this.before_action(:show) = fn(...)` is parsed as a call to `before_action`.
    // At class-definition time we need the static method to exist on Controller so
    // the call doesn't blow up with "Cannot access property 'before_action'".
    // Actual hook registration happens via the registry's textual scan.
    #[test]
    fn controller_exposes_noop_before_action_and_after_action() {
        let class = controller_class();
        assert!(
            class.native_static_methods.contains_key("before_action"),
            "Controller must expose a static before_action so the filtered-hook DSL call resolves"
        );
        assert!(
            class.native_static_methods.contains_key("after_action"),
            "Controller must expose a static after_action for symmetry with before_action"
        );
        let f = class.native_static_methods.get("before_action").unwrap();
        // No-op: returns Null regardless of arity so `before_action(:a, :b, handler)` is safe.
        let result = (f.func)(vec![Value::Null, Value::Int(1)]).unwrap();
        assert!(matches!(result, Value::Null));
    }
}

/// Register all controller built-ins in the environment.
pub fn register_controller_builtins(env: &mut Environment) {
    register_controller_class(env);
}

/// Define the Controller base class.
fn register_controller_class(env: &mut Environment) {
    // `this.before_action(:show, :edit) = fn(req) {...}` parses as the call
    // `this.before_action(:show, :edit, fn(req) {...})` (see parser desugar in
    // `parse_infix`). Actual hook registration is done by the controller
    // registry's textual scan of `app/controllers/*.sl` at `soli serve` startup,
    // so the runtime call just needs to be a silent no-op. Same for after_action.
    let mut native_static_methods: HashMap<String, Rc<NativeFunction>> = HashMap::new();
    native_static_methods.insert(
        "before_action".to_string(),
        Rc::new(NativeFunction::new(
            "Controller.before_action",
            None,
            |_| Ok(Value::Null),
        )),
    );
    native_static_methods.insert(
        "after_action".to_string(),
        Rc::new(NativeFunction::new("Controller.after_action", None, |_| {
            Ok(Value::Null)
        })),
    );

    let controller_class = Class {
        name: "Controller".to_string(),
        superclass: None,
        methods: Rc::new(RefCell::new(HashMap::new())),
        static_methods: HashMap::new(),
        native_static_methods,
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        nested_classes: Rc::new(RefCell::new(HashMap::new())),
        ..Default::default()
    };
    env.define(
        "Controller".to_string(),
        Value::Class(Rc::new(controller_class)),
    );
}

/// Controller action information for routing.
#[derive(Debug, Clone)]
pub struct ControllerAction {
    pub controller_name: String, // "posts"
    pub class_name: String,      // "PostsController"
    pub action_name: String,     // "index"
    pub is_public: bool,         // true if not starting with _
}

/// Before action hook.
#[derive(Debug, Clone)]
pub struct BeforeAction {
    pub actions: Vec<String>,   // Empty = all actions
    pub handler_source: String, // Soli function source code
}

/// After action hook.
#[derive(Debug, Clone)]
pub struct AfterAction {
    pub actions: Vec<String>,   // Empty = all actions
    pub handler_source: String, // Soli function source code
}

/// Controller metadata for routing.
#[derive(Debug, Clone)]
pub struct ControllerInfo {
    pub name: String,       // "PostsController"
    pub class_name: String, // "posts"
    pub actions: Vec<ControllerAction>,
    pub before_actions: Vec<BeforeAction>,
    pub after_actions: Vec<AfterAction>,
    pub layout: Option<String>,
}

impl ControllerInfo {
    /// Create a new ControllerInfo.
    pub fn new(name: &str, class_name: &str) -> Self {
        Self {
            name: name.to_string(),
            class_name: class_name.to_string(),
            actions: Vec::new(),
            before_actions: Vec::new(),
            after_actions: Vec::new(),
            layout: None,
        }
    }

    /// Check if an action should run before-actions.
    pub fn should_run_before(&self, action: &str) -> bool {
        // If no before_actions defined, run for all
        if self.before_actions.is_empty() {
            return true;
        }
        // Check if action is in any before_action's list
        for ba in &self.before_actions {
            if ba.actions.is_empty() || ba.actions.contains(&action.to_string()) {
                return true;
            }
        }
        false
    }

    /// Check if an action should run after-actions.
    pub fn should_run_after(&self, action: &str) -> bool {
        // If no after_actions defined, skip
        if self.after_actions.is_empty() {
            return false;
        }
        // Check if action is in any after_action's list
        for aa in &self.after_actions {
            if aa.actions.is_empty() || aa.actions.contains(&action.to_string()) {
                return true;
            }
        }
        false
    }
}
