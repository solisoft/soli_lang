use std::collections::HashMap;
use std::sync::RwLock;

use lazy_static::lazy_static;

use super::callbacks::ModelCallbacks;
use super::relations::RelationDef;
use super::validation::ValidationRule;

/// Metadata for a model class (validations, callbacks).
#[derive(Debug, Clone, Default)]
pub struct ModelMetadata {
    pub validations: Vec<ValidationRule>,
    pub callbacks: ModelCallbacks,
    pub relations: Vec<RelationDef>,
    pub soft_delete: bool,
    pub translated_fields: Vec<String>,
}

lazy_static! {
    /// Global registry mapping class names to their metadata.
    pub static ref MODEL_REGISTRY: RwLock<HashMap<String, ModelMetadata>> =
        RwLock::new(HashMap::new());
}

pub fn register_translation(class_name: &str, field_name: &str) {
    let mut registry = MODEL_REGISTRY.write().unwrap();
    let metadata = registry.entry(class_name.to_string()).or_default();
    if !metadata.translated_fields.contains(&field_name.to_string()) {
        metadata.translated_fields.push(field_name.to_string());
    }
}

pub fn get_translated_fields(class_name: &str) -> Vec<String> {
    let registry = MODEL_REGISTRY.read().unwrap();
    registry
        .get(class_name)
        .map(|m| m.translated_fields.clone())
        .unwrap_or_default()
}

pub fn is_translated_field(class_name: &str, field_name: &str) -> bool {
    let registry = MODEL_REGISTRY.read().unwrap();
    registry
        .get(class_name)
        .map(|m| m.translated_fields.contains(&field_name.to_string()))
        .unwrap_or(false)
}

pub fn get_or_create_metadata(class_name: &str) -> ModelMetadata {
    let registry = MODEL_REGISTRY.read().unwrap();
    registry.get(class_name).cloned().unwrap_or_default()
}

pub fn update_metadata(class_name: &str, metadata: ModelMetadata) {
    let mut registry = MODEL_REGISTRY.write().unwrap();
    registry.insert(class_name.to_string(), metadata);
}

pub fn is_soft_delete(class_name: &str) -> bool {
    let registry = MODEL_REGISTRY.read().unwrap();
    registry
        .get(class_name)
        .map(|m| m.soft_delete)
        .unwrap_or(false)
}
