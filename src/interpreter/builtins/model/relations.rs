//! Model relationships: has_many, has_one, belongs_to.
//!
//! Provides Rails-style association DSL for declaring relationships between models.
//! Relations are stored in the MODEL_REGISTRY alongside validations and callbacks.
//!
//! ```soli
//! class User extends Model
//!     has_many("posts")
//!     has_one("profile")
//! end
//!
//! class Post extends Model
//!     belongs_to("user")
//! end
//! ```

use super::core::MODEL_REGISTRY;

/// The type of relationship between two models.
#[derive(Debug, Clone, PartialEq)]
pub enum RelationType {
    HasMany,
    HasOne,
    BelongsTo,
}

/// Definition of a single relationship.
#[derive(Debug, Clone)]
pub struct RelationDef {
    /// The name used in DSL calls, e.g. "posts", "profile", "user"
    pub name: String,
    /// The type of relationship
    pub relation_type: RelationType,
    /// The related model class name, e.g. "Post", "Profile", "User"
    pub class_name: String,
    /// The collection name for the related model, e.g. "posts", "profiles", "users"
    pub collection: String,
    /// The foreign key field, e.g. "user_id"
    pub foreign_key: String,
}

/// Build a RelationDef applying naming conventions.
///
/// - `has_many("posts")` → class `Post`, collection `posts`, fk `user_id`
/// - `belongs_to("user")` → class `User`, collection `users`, fk `user_id`
/// - `has_one("profile")` → class `Profile`, collection `profiles`, fk `user_id`
pub fn build_relation(
    owner_class: &str,
    name: &str,
    relation_type: RelationType,
    class_name_override: Option<&str>,
    foreign_key_override: Option<&str>,
) -> RelationDef {
    let class_name = class_name_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| classify(name));

    let collection = if let Some(cn) = class_name_override {
        super::core::class_name_to_collection(cn)
    } else {
        // For has_many, name is already plural (e.g. "posts")
        // For belongs_to/has_one, name is singular → pluralize
        match relation_type {
            RelationType::HasMany => name.to_string(),
            RelationType::HasOne | RelationType::BelongsTo => pluralize(name),
        }
    };

    let foreign_key = foreign_key_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            match relation_type {
                // has_many/has_one: FK is on the related model, named after the owner
                RelationType::HasMany | RelationType::HasOne => {
                    format!("{}_id", to_snake_case(owner_class))
                }
                // belongs_to: FK is on the owner model, named after the relation
                RelationType::BelongsTo => format!("{}_id", name),
            }
        });

    RelationDef {
        name: name.to_string(),
        relation_type,
        class_name,
        collection,
        foreign_key,
    }
}

/// Register a relation for a model class in the MODEL_REGISTRY.
pub fn register_relation(class_name: &str, relation: RelationDef) {
    let mut registry = MODEL_REGISTRY.write().unwrap();
    let metadata = registry.entry(class_name.to_string()).or_default();
    metadata.relations.push(relation);
}

/// Get all relations for a model class.
pub fn get_relations(class_name: &str) -> Vec<RelationDef> {
    let registry = MODEL_REGISTRY.read().unwrap();
    registry
        .get(class_name)
        .map(|m| m.relations.clone())
        .unwrap_or_default()
}

/// Get a specific relation by name for a model class.
pub fn get_relation(class_name: &str, relation_name: &str) -> Option<RelationDef> {
    let registry = MODEL_REGISTRY.read().unwrap();
    registry.get(class_name).and_then(|m| {
        m.relations
            .iter()
            .find(|r| r.name == relation_name)
            .cloned()
    })
}

// ---------------------------------------------------------------------------
// Naming helpers
// ---------------------------------------------------------------------------

/// Simple pluralize: add "s" if not already ending in "s".
fn pluralize(s: &str) -> String {
    if s.ends_with('s') {
        s.to_string()
    } else {
        format!("{}s", s)
    }
}

/// Simple singularize: strip trailing "s".
pub fn singularize(s: &str) -> String {
    if s.ends_with('s') && s.len() > 1 {
        s[..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Convert a relation name to PascalCase class name.
/// "posts" → "Post", "blog_posts" → "BlogPost", "profile" → "Profile"
pub fn classify(name: &str) -> String {
    let singular = singularize(name);
    to_pascal_case(&singular)
}

/// Convert snake_case to PascalCase.
fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) => {
                    let mut result = c.to_uppercase().to_string();
                    result.extend(chars);
                    result
                }
                None => String::new(),
            }
        })
        .collect()
}

/// Convert PascalCase to snake_case.
fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            result.push('_');
        }
        result.push(c.to_lowercase().next().unwrap());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_singularize() {
        assert_eq!(singularize("posts"), "post");
        assert_eq!(singularize("users"), "user");
        assert_eq!(singularize("profile"), "profile");
        assert_eq!(singularize("s"), "s"); // edge case: single char
    }

    #[test]
    fn test_classify() {
        assert_eq!(classify("posts"), "Post");
        assert_eq!(classify("blog_posts"), "BlogPost");
        assert_eq!(classify("profile"), "Profile");
        assert_eq!(classify("user"), "User");
    }

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("User"), "user");
        assert_eq!(to_snake_case("BlogPost"), "blog_post");
        assert_eq!(to_snake_case("UserProfile"), "user_profile");
    }

    #[test]
    fn test_build_has_many() {
        let rel = build_relation("User", "posts", RelationType::HasMany, None, None);
        assert_eq!(rel.name, "posts");
        assert_eq!(rel.relation_type, RelationType::HasMany);
        assert_eq!(rel.class_name, "Post");
        assert_eq!(rel.collection, "posts");
        assert_eq!(rel.foreign_key, "user_id");
    }

    #[test]
    fn test_build_has_one() {
        let rel = build_relation("User", "profile", RelationType::HasOne, None, None);
        assert_eq!(rel.name, "profile");
        assert_eq!(rel.relation_type, RelationType::HasOne);
        assert_eq!(rel.class_name, "Profile");
        assert_eq!(rel.collection, "profiles");
        assert_eq!(rel.foreign_key, "user_id");
    }

    #[test]
    fn test_build_belongs_to() {
        let rel = build_relation("Post", "user", RelationType::BelongsTo, None, None);
        assert_eq!(rel.name, "user");
        assert_eq!(rel.relation_type, RelationType::BelongsTo);
        assert_eq!(rel.class_name, "User");
        assert_eq!(rel.collection, "users");
        assert_eq!(rel.foreign_key, "user_id");
    }

    #[test]
    fn test_build_has_many_with_overrides() {
        let rel = build_relation(
            "User",
            "posts",
            RelationType::HasMany,
            Some("Article"),
            Some("author_id"),
        );
        assert_eq!(rel.class_name, "Article");
        assert_eq!(rel.collection, "articles");
        assert_eq!(rel.foreign_key, "author_id");
    }

    #[test]
    fn test_build_belongs_to_compound_name() {
        let rel = build_relation("Comment", "blog_post", RelationType::BelongsTo, None, None);
        assert_eq!(rel.class_name, "BlogPost");
        assert_eq!(rel.collection, "blog_posts");
        assert_eq!(rel.foreign_key, "blog_post_id");
    }

    #[test]
    fn test_has_many_on_compound_owner() {
        let rel = build_relation("BlogPost", "comments", RelationType::HasMany, None, None);
        assert_eq!(rel.foreign_key, "blog_post_id");
        assert_eq!(rel.collection, "comments");
        assert_eq!(rel.class_name, "Comment");
    }
}
