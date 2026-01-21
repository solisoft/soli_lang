//! Controller built-in functions for SoliLang.
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

pub mod controller;
pub mod registry;

pub use controller::register_controller_builtins;
pub use registry::CONTROLLER_REGISTRY;
