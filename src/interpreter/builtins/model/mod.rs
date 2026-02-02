//! Simplified OOP Model system for SoliLang.
//!
//! Collection name is auto-derived from the class name:
//! - `User` → `"users"`
//! - `BlogPost` → `"blog_posts"`
//!
//! # Query Generation
//!
//! The Model system generates SDBQL (SoliDB Query Language) queries:
//! - `User.all()` → `FOR doc IN users RETURN doc`
//! - `User.where("doc.age >= @age", { "age": 18 })` → `FOR doc IN users FILTER doc.age >= @age RETURN doc`
//! - `User.count()` → `FOR doc IN users COLLECT WITH COUNT INTO count RETURN count`
//!
//! # CRUD Operations
//!
//! ```soli
//! // Create
//! let user = User.create({ "name": "Alice", "email": "alice@example.com" });
//!
//! // Read
//! let found = User.find("user_id");
//! let all = User.all();
//! let adults = User.where("doc.age >= @age", { "age": 18 }).all();
//!
//! // Update
//! User.update("user_id", { "name": "Alice Smith" });
//!
//! // Delete
//! User.delete("user_id");
//!
//! // Count
//! let total = User.count();
//! ```
//!
//! # Query Builder Chaining
//!
//! ```soli
//! User.where("doc.age >= @age", { "age": 18 })
//!     .where("doc.active == @active", { "active": true })
//!     .order("created_at", "desc")
//!     .limit(10)
//!     .offset(20)
//!     .all();
//! ```
//!
//! # Validations
//!
//! ```soli
//! class User extends Model {
//!     validates("email", { "presence": true, "uniqueness": true })
//!     validates("name", { "presence": true, "min_length": 2, "max_length": 100 })
//!     validates("age", { "numericality": true, "min": 0, "max": 150 })
//!     validates("website", { "format": "^https?://" })
//! }
//! ```
//!
//! Validation options:
//! - `presence: true` - Field must be present and not empty
//! - `uniqueness: true` - Field value must be unique in collection
//! - `min_length: n` - String must be at least n characters
//! - `max_length: n` - String must be at most n characters
//! - `format: "regex"` - String must match regex pattern
//! - `numericality: true` - Value must be a number
//! - `min: n` - Number must be >= n
//! - `max: n` - Number must be <= n
//! - `custom: "method_name"` - Call custom validation method
//!
//! # Callbacks
//!
//! ```soli
//! class User extends Model {
//!     before_save("normalize_email")
//!     after_create("send_welcome_email")
//!     before_update("log_changes")
//!     after_delete("cleanup_related")
//! }
//! ```
//!
//! Available callbacks:
//! - `before_save`, `after_save`
//! - `before_create`, `after_create`
//! - `before_update`, `after_update`
//! - `before_delete`, `after_delete`

pub mod callbacks;
pub mod core;
pub mod crud;
pub mod query;
pub mod relations;
pub mod validation;

pub use callbacks::{register_callback, ModelCallbacks};
pub use core::{
    class_name_to_collection, get_or_create_metadata, init_db_config, register_model_builtins,
    update_metadata, Model, ModelMetadata, DB_CONFIG, MODEL_REGISTRY,
};
pub use crud::{
    exec_async_query, exec_async_query_raw, exec_async_query_with_binds, exec_db_json,
    json_to_value,
};
pub use query::{
    execute_query_builder, execute_query_builder_count, execute_query_builder_first, QueryBuilder,
};
pub use validation::{
    build_validation_result, register_validation, run_validations, ValidationError, ValidationRule,
};

// Re-export value_to_json from value module for backward compatibility
pub use crate::interpreter::value::value_to_json;
