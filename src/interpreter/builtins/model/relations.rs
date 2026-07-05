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
use crate::interpreter::value::Value;

/// The type of relationship between two models.
#[derive(Debug, Clone, PartialEq)]
pub enum RelationType {
    HasMany,
    HasOne,
    BelongsTo,
    /// Polymorphic relation - stores type in a separate field (e.g., commentable_type)
    Polymorphic,
    /// Many-to-many through a join table (e.g., posts_tags).
    HasAndBelongsToMany,
}

/// What happens to associated records when the owner is hard-deleted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DependentStrategy {
    /// Per-row instance deletes: child callbacks, nested cascades, and the
    /// child's own soft-delete semantics all apply.
    Delete,
    /// One bulk REMOVE: no callbacks, hard delete, no nesting.
    DeleteAll,
    /// One bulk UPDATE setting the foreign key to null.
    Nullify,
}

/// `counter_cache:` option value as written in the DSL.
#[derive(Debug, Clone, PartialEq)]
pub enum CounterCacheOption {
    /// `counter_cache: true` — column name derived from the child collection.
    Enabled,
    /// `counter_cache: "custom_count"` — explicit column name.
    Column(String),
}

/// Parsed relation options hash (shared by all four DSL entry points).
#[derive(Debug, Clone, Default)]
pub struct RelationOptions {
    pub class_name: Option<String>,
    pub foreign_key: Option<String>,
    /// HABTM only.
    pub join_table: Option<String>,
    /// HABTM only.
    pub association_foreign_key: Option<String>,
    pub dependent: Option<DependentStrategy>,
    pub through: Option<String>,
    pub source: Option<String>,
    pub counter_cache: Option<CounterCacheOption>,
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
    /// For polymorphic relations: the field storing the type (e.g., "commentable_type")
    pub polymorphic_type_field: Option<String>,
    /// For polymorphic relations: the expected type value (e.g., "Post")
    pub polymorphic_type_value: Option<String>,
    /// HABTM join table name, e.g. "posts_tags"
    pub join_table: Option<String>,
    /// HABTM foreign key on the join table pointing at the related class, e.g. "tag_id"
    pub association_foreign_key: Option<String>,
    /// Cascade strategy applied when the owner is hard-deleted.
    pub dependent: Option<DependentStrategy>,
    /// `has_many through:` — the name of the intermediate relation on the owner.
    pub through: Option<String>,
    /// Optional `source:` on a through relation (relation name on the through model).
    pub source: Option<String>,
    /// belongs_to `counter_cache:` — the resolved parent column name.
    pub counter_cache: Option<String>,
}

fn option_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.to_string()),
        Value::Symbol(s) => Some(s.to_string()),
        _ => None,
    }
}

/// Parse the optional trailing options hash of a relation DSL call.
/// Unknown keys stay silently ignored (back-compat); the new option keys
/// validate their host relation kind and value shape, raising at class-load
/// time with an actionable message.
pub fn parse_relation_options(
    arg: Option<&Value>,
    kind: &RelationType,
) -> Result<RelationOptions, String> {
    let mut options = RelationOptions::default();
    let Some(Value::Hash(hash)) = arg else {
        return Ok(options);
    };

    use crate::interpreter::value::HashKey;
    for (k, v) in hash.borrow().iter() {
        let HashKey::String(key) = k else { continue };
        match key.as_ref() {
            "class_name" => options.class_name = option_string(v),
            "foreign_key" => options.foreign_key = option_string(v),
            "join_table" if matches!(kind, RelationType::HasAndBelongsToMany) => {
                options.join_table = option_string(v)
            }
            "association_foreign_key" if matches!(kind, RelationType::HasAndBelongsToMany) => {
                options.association_foreign_key = option_string(v)
            }
            "dependent" => {
                if !matches!(kind, RelationType::HasMany | RelationType::HasOne) {
                    return Err(
                        "`dependent:` is only supported on has_many/has_one relations".to_string(),
                    );
                }
                let strategy = option_string(v).ok_or_else(|| {
                    "`dependent:` expects \"delete\", \"delete_all\" or \"nullify\"".to_string()
                })?;
                options.dependent = Some(match strategy.as_str() {
                    "delete" | "destroy" => DependentStrategy::Delete,
                    "delete_all" => DependentStrategy::DeleteAll,
                    "nullify" => DependentStrategy::Nullify,
                    other => {
                        return Err(format!(
                            "`dependent:` expects \"delete\", \"delete_all\" or \"nullify\", got \"{}\"",
                            other
                        ))
                    }
                });
            }
            "through" => {
                if !matches!(kind, RelationType::HasMany) {
                    return Err("`through:` is only supported on has_many relations".to_string());
                }
                options.through = option_string(v);
                if options.through.is_none() {
                    return Err("`through:` expects a relation name".to_string());
                }
            }
            "source" => {
                options.source = option_string(v);
                if options.source.is_none() {
                    return Err("`source:` expects a relation name".to_string());
                }
            }
            "counter_cache" => {
                if !matches!(kind, RelationType::BelongsTo) {
                    return Err(
                        "`counter_cache:` is only supported on belongs_to relations".to_string()
                    );
                }
                options.counter_cache = match v {
                    Value::Bool(true) => Some(CounterCacheOption::Enabled),
                    Value::Bool(false) => None,
                    other => match option_string(other) {
                        Some(column) => Some(CounterCacheOption::Column(column)),
                        None => {
                            return Err("`counter_cache:` expects true or a column name".to_string())
                        }
                    },
                };
            }
            _ => {}
        }
    }

    if options.dependent.is_some() && options.through.is_some() {
        return Err(
            "`dependent:` cannot be combined with `through:` (through relations are read-only)"
                .to_string(),
        );
    }
    if options.source.is_some() && options.through.is_none() {
        return Err("`source:` requires `through:`".to_string());
    }

    Ok(options)
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
    options: &RelationOptions,
) -> RelationDef {
    let class_name = options.class_name.clone().unwrap_or_else(|| classify(name));

    let collection = if let Some(cn) = &options.class_name {
        super::core::class_name_to_collection(cn)
    } else {
        // For has_many, name is already plural (e.g. "posts")
        // For belongs_to/has_one, name is singular → pluralize
        match relation_type {
            RelationType::HasMany
            | RelationType::Polymorphic
            | RelationType::HasAndBelongsToMany => name.to_string(),
            RelationType::HasOne | RelationType::BelongsTo => pluralize(name),
        }
    };

    let foreign_key = options.foreign_key.clone().unwrap_or_else(|| {
        match relation_type {
            // has_many/has_one: FK is on the related model, named after the owner
            RelationType::HasMany | RelationType::HasOne | RelationType::Polymorphic => {
                format!("{}_id", to_snake_case(owner_class))
            }
            // belongs_to: FK is on the owner model, named after the relation
            RelationType::BelongsTo => format!("{}_id", name),
            // habtm uses build_habtm_relation; not built here
            RelationType::HasAndBelongsToMany => {
                format!("{}_id", to_snake_case(owner_class))
            }
        }
    });

    // belongs_to counter_cache: true derives the parent column from the
    // owner's (child's) collection: Comment.belongs_to("post") → comments_count.
    let counter_cache = match &options.counter_cache {
        Some(CounterCacheOption::Enabled) => Some(format!(
            "{}_count",
            super::core::class_name_to_collection(owner_class)
        )),
        Some(CounterCacheOption::Column(column)) => Some(column.clone()),
        None => None,
    };

    RelationDef {
        name: name.to_string(),
        relation_type,
        class_name,
        collection,
        foreign_key,
        polymorphic_type_field: None,
        polymorphic_type_value: None,
        join_table: None,
        association_foreign_key: None,
        dependent: options.dependent,
        through: options.through.clone(),
        source: options.source.clone(),
        counter_cache,
    }
}

/// Build a `has_and_belongs_to_many` relation applying naming conventions.
///
/// - `has_and_belongs_to_many("tags")` on `Post`
///   → class `Tag`, collection `tags`, join table `posts_tags`,
///   owner FK `post_id`, association FK `tag_id`
/// - Join table is the alphabetical concatenation of the two pluralized
///   collections (Rails convention): `posts_tags`, not `tags_posts`.
pub fn build_habtm_relation(
    owner_class: &str,
    name: &str,
    options: &RelationOptions,
) -> RelationDef {
    let class_name = options.class_name.clone().unwrap_or_else(|| classify(name));

    let collection = if let Some(cn) = &options.class_name {
        super::core::class_name_to_collection(cn)
    } else {
        // habtm name is plural (e.g. "tags")
        name.to_string()
    };

    let owner_collection = super::core::class_name_to_collection(owner_class);

    let foreign_key = options
        .foreign_key
        .clone()
        .unwrap_or_else(|| format!("{}_id", to_snake_case(owner_class)));

    let association_foreign_key = options
        .association_foreign_key
        .clone()
        .unwrap_or_else(|| format!("{}_id", to_snake_case(&class_name)));

    let join_table = options.join_table.clone().unwrap_or_else(|| {
        let mut pair = [owner_collection.as_str(), collection.as_str()];
        pair.sort();
        format!("{}_{}", pair[0], pair[1])
    });

    RelationDef {
        name: name.to_string(),
        relation_type: RelationType::HasAndBelongsToMany,
        class_name,
        collection,
        foreign_key,
        polymorphic_type_field: None,
        polymorphic_type_value: None,
        join_table: Some(join_table),
        association_foreign_key: Some(association_foreign_key),
        dependent: None,
        through: None,
        source: None,
        counter_cache: None,
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

fn pluralize(s: &str) -> String {
    crate::inflect::pluralize(s)
}

pub fn singularize(s: &str) -> String {
    crate::inflect::singularize(s)
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
pub fn to_snake_case(s: &str) -> String {
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
        let rel = build_relation(
            "User",
            "posts",
            RelationType::HasMany,
            &RelationOptions::default(),
        );
        assert_eq!(rel.name, "posts");
        assert_eq!(rel.relation_type, RelationType::HasMany);
        assert_eq!(rel.class_name, "Post");
        assert_eq!(rel.collection, "posts");
        assert_eq!(rel.foreign_key, "user_id");
    }

    #[test]
    fn test_build_has_one() {
        let rel = build_relation(
            "User",
            "profile",
            RelationType::HasOne,
            &RelationOptions::default(),
        );
        assert_eq!(rel.name, "profile");
        assert_eq!(rel.relation_type, RelationType::HasOne);
        assert_eq!(rel.class_name, "Profile");
        assert_eq!(rel.collection, "profiles");
        assert_eq!(rel.foreign_key, "user_id");
    }

    #[test]
    fn test_build_belongs_to() {
        let rel = build_relation(
            "Post",
            "user",
            RelationType::BelongsTo,
            &RelationOptions::default(),
        );
        assert_eq!(rel.name, "user");
        assert_eq!(rel.relation_type, RelationType::BelongsTo);
        assert_eq!(rel.class_name, "User");
        assert_eq!(rel.collection, "users");
        assert_eq!(rel.foreign_key, "user_id");
    }

    #[test]
    fn test_build_has_many_with_overrides() {
        let options = RelationOptions {
            class_name: Some("Article".to_string()),
            foreign_key: Some("author_id".to_string()),
            ..Default::default()
        };
        let rel = build_relation("User", "posts", RelationType::HasMany, &options);
        assert_eq!(rel.class_name, "Article");
        assert_eq!(rel.collection, "articles");
        assert_eq!(rel.foreign_key, "author_id");
    }

    #[test]
    fn test_build_belongs_to_compound_name() {
        let rel = build_relation(
            "Comment",
            "blog_post",
            RelationType::BelongsTo,
            &RelationOptions::default(),
        );
        assert_eq!(rel.class_name, "BlogPost");
        assert_eq!(rel.collection, "blog_posts");
        assert_eq!(rel.foreign_key, "blog_post_id");
    }

    #[test]
    fn test_build_habtm_alphabetical_join_table() {
        let rel = build_habtm_relation("Post", "tags", &RelationOptions::default());
        assert_eq!(rel.relation_type, RelationType::HasAndBelongsToMany);
        assert_eq!(rel.class_name, "Tag");
        assert_eq!(rel.collection, "tags");
        assert_eq!(rel.foreign_key, "post_id");
        assert_eq!(rel.association_foreign_key.as_deref(), Some("tag_id"));
        assert_eq!(rel.join_table.as_deref(), Some("posts_tags"));
    }

    #[test]
    fn test_build_habtm_reverse_alphabetical() {
        // Tag's side of the relation: "posts" — alphabetical sort still gives posts_tags
        let rel = build_habtm_relation("Tag", "posts", &RelationOptions::default());
        assert_eq!(rel.foreign_key, "tag_id");
        assert_eq!(rel.association_foreign_key.as_deref(), Some("post_id"));
        assert_eq!(rel.join_table.as_deref(), Some("posts_tags"));
    }

    #[test]
    fn test_build_habtm_with_overrides() {
        let options = RelationOptions {
            class_name: Some("Tag".to_string()),
            foreign_key: Some("article_id".to_string()),
            association_foreign_key: Some("tag_id".to_string()),
            join_table: Some("article_labels".to_string()),
            ..Default::default()
        };
        let rel = build_habtm_relation("Article", "labels", &options);
        assert_eq!(rel.class_name, "Tag");
        assert_eq!(rel.collection, "tags");
        assert_eq!(rel.foreign_key, "article_id");
        assert_eq!(rel.association_foreign_key.as_deref(), Some("tag_id"));
        assert_eq!(rel.join_table.as_deref(), Some("article_labels"));
    }

    fn options_hash(pairs: &[(&str, Value)]) -> Value {
        use crate::interpreter::value::{HashKey, HashPairs};
        use std::cell::RefCell;
        use std::rc::Rc;
        let mut map = HashPairs::default();
        for (k, v) in pairs {
            map.insert(HashKey::String((*k).into()), v.clone());
        }
        Value::Hash(Rc::new(RefCell::new(map)))
    }

    #[test]
    fn parse_dependent_strategies_and_destroy_alias() {
        for (raw, expected) in [
            ("delete", DependentStrategy::Delete),
            ("destroy", DependentStrategy::Delete),
            ("delete_all", DependentStrategy::DeleteAll),
            ("nullify", DependentStrategy::Nullify),
        ] {
            let hash = options_hash(&[("dependent", Value::String(raw.into()))]);
            let options = parse_relation_options(Some(&hash), &RelationType::HasMany).unwrap();
            assert_eq!(options.dependent, Some(expected), "for {raw}");
        }
        // Symbol values (named-arg style) parse identically.
        let hash = options_hash(&[("dependent", Value::Symbol("delete_all".into()))]);
        let options = parse_relation_options(Some(&hash), &RelationType::HasOne).unwrap();
        assert_eq!(options.dependent, Some(DependentStrategy::DeleteAll));
    }

    #[test]
    fn parse_rejects_bad_dependent() {
        let hash = options_hash(&[("dependent", Value::String("purge".into()))]);
        let err = parse_relation_options(Some(&hash), &RelationType::HasMany).unwrap_err();
        assert!(err.contains("purge"), "got: {err}");

        let hash = options_hash(&[("dependent", Value::String("delete".into()))]);
        let err = parse_relation_options(Some(&hash), &RelationType::BelongsTo).unwrap_err();
        assert!(err.contains("has_many/has_one"), "got: {err}");
    }

    #[test]
    fn parse_rejects_dependent_with_through() {
        let hash = options_hash(&[
            ("dependent", Value::String("delete".into())),
            ("through", Value::String("memberships".into())),
        ]);
        let err = parse_relation_options(Some(&hash), &RelationType::HasMany).unwrap_err();
        assert!(err.contains("through"), "got: {err}");
    }

    #[test]
    fn parse_through_and_source() {
        let hash = options_hash(&[
            ("through", Value::String("memberships".into())),
            ("source", Value::String("company".into())),
        ]);
        let options = parse_relation_options(Some(&hash), &RelationType::HasMany).unwrap();
        assert_eq!(options.through.as_deref(), Some("memberships"));
        assert_eq!(options.source.as_deref(), Some("company"));

        let hash = options_hash(&[("through", Value::String("x".into()))]);
        let err = parse_relation_options(Some(&hash), &RelationType::BelongsTo).unwrap_err();
        assert!(err.contains("has_many"), "got: {err}");

        let hash = options_hash(&[("source", Value::String("company".into()))]);
        let err = parse_relation_options(Some(&hash), &RelationType::HasMany).unwrap_err();
        assert!(err.contains("requires `through:`"), "got: {err}");
    }

    #[test]
    fn parse_counter_cache_forms() {
        let hash = options_hash(&[("counter_cache", Value::Bool(true))]);
        let options = parse_relation_options(Some(&hash), &RelationType::BelongsTo).unwrap();
        assert_eq!(options.counter_cache, Some(CounterCacheOption::Enabled));

        let hash = options_hash(&[("counter_cache", Value::String("my_count".into()))]);
        let options = parse_relation_options(Some(&hash), &RelationType::BelongsTo).unwrap();
        assert_eq!(
            options.counter_cache,
            Some(CounterCacheOption::Column("my_count".to_string()))
        );

        let hash = options_hash(&[("counter_cache", Value::Bool(false))]);
        let options = parse_relation_options(Some(&hash), &RelationType::BelongsTo).unwrap();
        assert_eq!(options.counter_cache, None);

        let hash = options_hash(&[("counter_cache", Value::Bool(true))]);
        let err = parse_relation_options(Some(&hash), &RelationType::HasMany).unwrap_err();
        assert!(err.contains("belongs_to"), "got: {err}");

        let hash = options_hash(&[("counter_cache", Value::Int(3))]);
        let err = parse_relation_options(Some(&hash), &RelationType::BelongsTo).unwrap_err();
        assert!(err.contains("true or a column name"), "got: {err}");
    }

    #[test]
    fn counter_cache_true_derives_column_from_owner_collection() {
        let hash = options_hash(&[("counter_cache", Value::Bool(true))]);
        let options = parse_relation_options(Some(&hash), &RelationType::BelongsTo).unwrap();
        let rel = build_relation("Comment", "post", RelationType::BelongsTo, &options);
        assert_eq!(rel.counter_cache.as_deref(), Some("comments_count"));

        let hash = options_hash(&[("counter_cache", Value::String("cc".into()))]);
        let options = parse_relation_options(Some(&hash), &RelationType::BelongsTo).unwrap();
        let rel = build_relation("Comment", "post", RelationType::BelongsTo, &options);
        assert_eq!(rel.counter_cache.as_deref(), Some("cc"));
    }

    #[test]
    fn parse_unknown_keys_stay_ignored() {
        let hash = options_hash(&[("polymorphic", Value::Bool(true))]);
        let options = parse_relation_options(Some(&hash), &RelationType::BelongsTo).unwrap();
        assert!(options.class_name.is_none());
    }

    #[test]
    fn test_has_many_on_compound_owner() {
        let rel = build_relation(
            "BlogPost",
            "comments",
            RelationType::HasMany,
            &RelationOptions::default(),
        );
        assert_eq!(rel.foreign_key, "blog_post_id");
        assert_eq!(rel.collection, "comments");
        assert_eq!(rel.class_name, "Comment");
    }
}
