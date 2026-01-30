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
use crate::interpreter::value::{Class, Value};

/// Register all controller built-ins in the environment.
pub fn register_controller_builtins(env: &mut Environment) {
    register_controller_class(env);
}

/// Define the Controller base class.
fn register_controller_class(env: &mut Environment) {
    let controller_class = Class {
        name: "Controller".to_string(),
        superclass: None,
        methods: HashMap::new(),
        static_methods: HashMap::new(),
        native_static_methods: HashMap::new(),
        native_methods: HashMap::new(),
        static_fields: Rc::new(RefCell::new(HashMap::new())),
        fields: HashMap::new(),
        constructor: None,
        all_methods_cache: RefCell::new(None),
        all_native_methods_cache: RefCell::new(None),
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
