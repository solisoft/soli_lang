//! Simplified OOP Model system for SoliLang.
//!
//! Provides a minimal Model class where collection name is auto-derived from class name:
//! - `User` → `"users"`
//! - `BlogPost` → `"blog_posts"`
//!
//! # Example Usage
//!
//! ```soli
//! class User extends Model { }
//!
//! let user = User.create({ "name": "Alice" });
//! let found = User.find(user.id);
//! let adults = User.where("age", ">=", 18);
//! let all = User.all();
//! User.update(user.id, { "name": "Bob" });
//! User.delete(user.id);
//! ```

pub mod model;

pub use model::register_model_builtins;
