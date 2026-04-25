use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::RwLock;

use lazy_static::lazy_static;

use super::callbacks::ModelCallbacks;
use super::relations::RelationDef;
use super::uploaders::UploaderConfig;
use super::validation::ValidationRule;
use crate::interpreter::value::Class;

/// Metadata for a model class (validations, callbacks).
#[derive(Debug, Clone, Default)]
pub struct ModelMetadata {
    pub validations: Vec<ValidationRule>,
    pub callbacks: ModelCallbacks,
    pub relations: Vec<RelationDef>,
    pub soft_delete: bool,
    pub translated_fields: Vec<String>,
    pub uploaders: Vec<UploaderConfig>,
}

lazy_static! {
    /// Global registry mapping class names to their metadata.
    pub static ref MODEL_REGISTRY: RwLock<HashMap<String, ModelMetadata>> =
        RwLock::new(HashMap::new());
}

// Thread-local registry for model classes (used for lazy relation conversion).
thread_local! {
    pub static MODEL_CLASSES: RefCell<HashMap<String, Rc<Class>>> = RefCell::new(HashMap::new());
}

pub fn register_translation(class_name: &str, field_name: &str) {
    let mut registry = MODEL_REGISTRY.write().unwrap();
    let metadata = registry.entry(class_name.to_string()).or_default();
    if !metadata.translated_fields.iter().any(|s| s == field_name) {
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
        .map(|m| m.translated_fields.iter().any(|s| s == field_name))
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

/// Register a model class for lazy relation conversion.
/// Called when model classes are set up.
pub fn register_model_class(class_name: &str, class: Rc<Class>) {
    MODEL_CLASSES.with(|classes: &RefCell<HashMap<String, Rc<Class>>>| {
        classes.borrow_mut().insert(class_name.to_string(), class);
    });
}

/// Get a model class by name. Returns None if not registered.
pub fn get_model_class(class_name: &str) -> Option<Rc<Class>> {
    MODEL_CLASSES.with(|classes: &RefCell<HashMap<String, Rc<Class>>>| {
        classes.borrow().get(class_name).cloned()
    })
}

/// Clear all registered model classes. Used during hot reload.
pub fn clear_model_classes() {
    MODEL_CLASSES.with(|classes: &RefCell<HashMap<String, Rc<Class>>>| {
        classes.borrow_mut().clear();
    });
}

/// Clear all model registries (MODEL_REGISTRY and MODEL_CLASSES). Used during hot reload.
pub fn clear_all_model_registries() {
    MODEL_REGISTRY.write().unwrap().clear();
    MODEL_CLASSES.with(|classes: &RefCell<HashMap<String, Rc<Class>>>| {
        classes.borrow_mut().clear();
    });
}
