//! Model/ORM built-in functions for SoliLang.
//!
//! This module provides a complete Model/ORM system for working with SoliDB.
//! It includes:
//! - Field definitions with type safety and validation
//! - Model class with CRUD operations
//! - Chainable query builder (ModelResult)
//! - Relationship management (has_many, has_one, embedded)
//! - Lifecycle hooks
//! - Migration system
//!
//! # Example Usage
//!
//! ```soli
//! class User extends Model {
//!     static {
//!         this.collection = "users";
//!         this.fields = {
//!             "name": Field.string({ required: true }),
//!             "email": Field.string({ unique: true, index: true }),
//!             "age": Field.int({ min: 0 })
//!         };
//!     }
//! }
//!
//! let user = User.create({ "name": "Alice", "email": "alice@example.com", "age": 30 });
//! let adults = User.where("age", ">=", 18).order_by("name", "asc").find();
//! ```

pub mod field;
pub mod schema;
pub mod hooks;
pub mod relationship;
pub mod migration;
pub mod model_result;
pub mod model;

pub use model::register_model_builtins;
