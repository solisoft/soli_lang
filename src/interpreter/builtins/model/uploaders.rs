//! Uploader DSL: declarative blob attachments on models.
//!
//! ```soli
//! class Contact extends Model
//!     uploader("photo", {
//!         "multiple":      false,
//!         "content_types": ["image/jpeg", "image/png"],
//!         "max_size":      2_000_000,
//!         "collection":    "contact_photos"
//!     })
//! end
//! ```
//!
//! Configs are stored alongside validations/callbacks/relations in
//! `MODEL_REGISTRY`. Soli code reads them through the global
//! `model_uploader_config(class_name, field)` helper to drive validation,
//! storage, and route resolution.

use super::core::MODEL_REGISTRY;

/// Single uploader declaration on a model class.
#[derive(Debug, Clone)]
pub struct UploaderConfig {
    /// Field name (e.g. "photo"). The model column is `<name>_blob_id` for
    /// single uploaders or `<name>_blob_ids` for multiple.
    pub name: String,
    /// `true` keeps an array of blob ids on the document; `false` keeps one.
    pub multiple: bool,
    /// MIME types accepted; everything else is rejected before upload.
    pub content_types: Vec<String>,
    /// Hard cap (bytes). Uploads above this are rejected.
    pub max_size: u64,
    /// SolidB collection name. Defaults to `<class_snake>_<name>s`.
    pub collection: String,
}

/// Register an uploader on a class. Called from the `uploader(...)` native
/// when the class body is executed.
pub fn register_uploader(class_name: &str, config: UploaderConfig) {
    let mut registry = MODEL_REGISTRY.write().unwrap();
    let metadata = registry.entry(class_name.to_string()).or_default();
    if let Some(slot) = metadata
        .uploaders
        .iter_mut()
        .find(|u| u.name == config.name)
    {
        *slot = config;
    } else {
        metadata.uploaders.push(config);
    }
}

/// Get one uploader config by field name.
pub fn get_uploader(class_name: &str, field: &str) -> Option<UploaderConfig> {
    let registry = MODEL_REGISTRY.read().unwrap();
    registry
        .get(class_name)
        .and_then(|m| m.uploaders.iter().find(|u| u.name == field).cloned())
}

/// Get every uploader config on a class. Used by the `before_delete` cleanup
/// path and by the `AttachmentsController` to enumerate files.
pub fn get_uploaders(class_name: &str) -> Vec<UploaderConfig> {
    let registry = MODEL_REGISTRY.read().unwrap();
    registry
        .get(class_name)
        .map(|m| m.uploaders.clone())
        .unwrap_or_default()
}

/// Read a string-valued field from an Instance — convenience for native
/// helpers that need to inspect an instance's blob-id columns. Returns
/// `None` for missing fields, null values, or non-string values.
pub fn get_uploader_field_value_as_string(
    inst: &crate::interpreter::value::Instance,
    field: &str,
) -> Option<String> {
    match inst.get(field)? {
        crate::interpreter::value::Value::String(s) => Some(s),
        _ => None,
    }
}

/// Default SolidB collection name from a class + field. Mirrors
/// `class_name_to_collection` (snake-case + plural) and appends `_<field>s`.
/// e.g. `("Contact", "photo")` → `"contact_photos"`.
pub fn default_collection(class_name: &str, field: &str) -> String {
    let snake = camel_to_snake(class_name);
    format!("{}_{}s", snake, field)
}

fn camel_to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i != 0 {
                out.push('_');
            }
            for lower in ch.to_lowercase() {
                out.push(lower);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_collection_simple() {
        assert_eq!(default_collection("Contact", "photo"), "contact_photos");
    }

    #[test]
    fn default_collection_camel_case_class() {
        assert_eq!(default_collection("BlogPost", "cover"), "blog_post_covers");
    }
}
