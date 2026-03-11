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
//! let result = User.create({ "name": "Bob" });  // result["valid"], result["record"]
//!
//! // Batch create
//! let batch = User.create_many([{ "name": "A" }, { "name": "B" }]);
//! // batch["created"], batch["errors"]
//!
//! // Read
//! let found = User.find("user_id");
//! let by_field = User.find_by("email", "alice@example.com");
//! let first = User.first_by("name", "Alice");  // with ordering
//! let all = User.all();
//! let adults = User.where("doc.age >= @age", { "age": 18 }).all();
//!
//! // Find or create
//! let user = User.find_or_create_by("email", "new@example.com");
//! let user = User.find_or_create_by("email", "new@example.com", { "name": "New" });
//!
//! // Update
//! User.update("user_id", { "name": "Alice Smith" });
//! user.name = "Alice";
//! user.update();  // instance method
//! user.save();  // insert or update based on _key
//!
//! // Upsert (insert or update by id)
//! User.upsert("user_id", { "name": "Alice" });  // update if exists, insert if not
//!
//! // Delete
//! User.delete("user_id");
//! user.delete();  // instance method
//!
//! // Count
//! let total = User.count();
//! let exists = User.where(...).exists();  // boolean
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
//!
//! // Pluck - get single field values
//! let names = User.where(...).pluck("name");  // ["Alice", "Bob", ...]
//! let name = User.find_by("id", "123").pluck("name");  // single value
//!
//! // Aggregations
//! let total = User.where(...).sum("balance");
//! let average = User.where(...).avg("score");
//! let min_score = User.where(...).min("score");
//! let max_score = User.where(...).max("score");
//!
//! // Group by aggregation
//! let by_country = User.group_by("country", "sum", "balance");
//! // Returns: [{group: "US", result: 1000}, {group: "FR", result: 500}, ...]
//! ```
//!
//! # Instance Methods
//!
//! ```soli
//! let user = User.find("id");
//!
//! // Field operations
//! user.name = "New Name";
//! user.update();  // persist changes
//! user.save();   // insert (if new) or update (if exists)
//! user.reload(); // refresh from database
//!
//! // Atomic operations
//! user.increment("view_count");     // +1
//! user.increment("view_count", 5);  // +5
//! user.decrement("stock");           // -1
//! user.touch();                      // update _updated_at
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
//!
//! # Relationships
//!
//! Declare associations using `has_many`, `has_one`, and `belongs_to`:
//!
//! ```soli
//! class User extends Model {
//!     has_many("posts")
//!     has_one("profile")
//! }
//!
//! class Post extends Model {
//!     belongs_to("user")
//!     has_many("comments")
//! }
//! ```
//!
//! Naming conventions (overridable via options hash):
//! - `has_many("posts")` → class `Post`, collection `posts`, FK `user_id`
//! - `has_one("profile")` → class `Profile`, collection `profiles`, FK `user_id`
//! - `belongs_to("user")` → class `User`, collection `users`, FK `user_id`
//!
//! # Eager Loading (includes)
//!
//! Preload related records to avoid N+1 queries:
//!
//! ```soli
//! User.includes("posts", "profile").all()
//! User.where("active = @a", { "a": true }).includes("posts").first()
//! ```
//!
//! Generated SDBQL uses LET subqueries + MERGE:
//! - `User.includes("posts")` →
//!   `FOR doc IN users LET _rel_posts = (...) RETURN MERGE(doc, {posts: _rel_posts})`
//!
//! # Join Filtering
//!
//! Filter by existence of related records (no preloading):
//!
//! ```soli
//! User.join("posts").all()                                 // users who have posts
//! User.join("posts", "published = @p", { "p": true }).count()  // users with published posts
//! ```
//!
//! # Relationship Accessors (Instance Methods)
//!
//! Access related records directly from model instances:
//!
//! ```soli
//! let user = User.find("user_id");
//! let posts = user.posts;        // has_many relation
//! let profile = user.profile;    // has_one relation
//! let author = post.user;        // belongs_to relation
//!
//! // Chain query builder methods on relations
//! let published_posts = user.posts.where("published = @p", { "p": true }).all();
//! ```
//!
//! # Scopes
//!
//! Define reusable query scopes:
//!
//! ```soli
//! class User extends Model
//!     scope("active", "active = @a", { "a": true })
//!     scope("recent", "1 = 1", {})  // no filter
//! end
//!
//! // Use scopes
//! let active_users = User.scope("active").all();
//! User.scope("recent").limit(10).all();
//! ```
//!
//! # Soft Delete
//!
//! Enable soft delete to mark records as deleted instead of removing them:
//!
//! ```soli
//! class Post extends Model
//!     soft_delete
//! end
//!
//! let post = Post.find("id");
//! post.delete();           // Sets deleted_at timestamp
//! post.restore();          // Clears deleted_at (restores)
//! Post.with_deleted.all()  // Include soft-deleted records
//! Post.only_deleted.all()  // Query only deleted records
//! ```
//!
//! # Polymorphic Relations
//!
//! Define polymorphic associations where a model can belong to multiple types:
//!
//! ```soli
//! class Comment extends Model
//!     belongs_to("commentable", { "polymorphic": true })
//! end
//!
//! // Access polymorphically
//! let comment = Comment.find("id");
//! let item = comment.commentable;  // Returns Post, Photo, etc.
//! ```
//!
//! # Transactions
//!
//! (Placeholder - requires SoliDB transaction API)
//!
//! ```soli
//! // Not yet implemented
//! // Model.transaction(fn() { ... })
//! ```
//!
//! For now, use individual Model operations within your application logic.

pub mod callbacks;
pub mod core;
pub mod crud;
pub mod query;
pub mod relations;
pub mod validation;

pub use callbacks::{register_callback, ModelCallbacks};
pub use core::{
    class_name_to_collection, get_or_create_metadata, get_translated_fields, init_db_config,
    is_translated_field, register_model_builtins, register_translation, update_metadata, Model,
    ModelMetadata, DB_CONFIG, MODEL_REGISTRY,
};
pub use crud::{
    exec_async_query, exec_async_query_raw, exec_async_query_with_binds, exec_auto_collection,
    exec_auto_collection_with_binds, exec_db_json, json_to_value,
};
pub use query::{
    build_aggregation_query, execute_query_builder, execute_query_builder_aggregate,
    execute_query_builder_count, execute_query_builder_exists, execute_query_builder_first,
    execute_query_builder_group_by, AggregationFunc, IncludeClause, JoinClause, QueryBuilder,
};
pub use relations::{
    build_relation, classify, get_relation, get_relations, register_relation, singularize,
    RelationDef, RelationType,
};
pub use validation::{
    build_validation_result, register_validation, run_validations, ValidationError, ValidationRule,
};

// Re-export value_to_json from value module for backward compatibility
pub use crate::interpreter::value::value_to_json;
