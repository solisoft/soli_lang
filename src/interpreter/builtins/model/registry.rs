use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::RwLock;

use lazy_static::lazy_static;

use super::callbacks::ModelCallbacks;
use super::relations::RelationDef;
use super::state_machine::StateMachineDef;
use super::uploaders::UploaderConfig;
use super::validation::ValidationRule;
use crate::interpreter::value::Class;

/// A model field bound to an enum type via `enum_field`: (field name, enum class).
type EnumFieldBinding = (String, Rc<Class>);

/// Metadata for a model class (validations, callbacks).
#[derive(Debug, Clone, Default)]
pub struct ModelMetadata {
    pub validations: Vec<ValidationRule>,
    pub callbacks: ModelCallbacks,
    pub relations: Vec<RelationDef>,
    pub soft_delete: bool,
    pub translated_fields: Vec<String>,
    /// Fields encrypted at rest via the `encrypts` DSL (AES-256-GCM).
    pub encrypted_fields: Vec<String>,
    pub uploaders: Vec<UploaderConfig>,
    /// `attr_accessible` whitelist. `None` = not declared (legacy/unsafe
    /// passthrough); `Some([])` = nothing is mass-assignable; `Some(list)` =
    /// only listed keys are accepted in mass-assign paths.
    pub accessible_attributes: Option<Vec<String>>,
    /// Declarative state machines (`state_machine :field do … end`). Plain data
    /// only — guard/before/after closures live in the `state_machine`
    /// thread-locals. Keyed by field, so a model may declare more than one.
    pub state_machines: Vec<StateMachineDef>,
}

lazy_static! {
    /// Global registry mapping class names to their metadata.
    pub static ref MODEL_REGISTRY: RwLock<HashMap<String, ModelMetadata>> =
        RwLock::new(HashMap::new());

    /// Collection name -> encrypted field names. Keyed by collection so the DB
    /// write layer (exec_insert/exec_update) can encrypt without a class handle.
    static ref ENCRYPTED_COLLECTIONS: RwLock<HashMap<String, Vec<String>>> =
        RwLock::new(HashMap::new());
}

/// Register an encrypted field for a model (by class) and its collection.
pub fn register_encryption(class_name: &str, collection: &str, field: &str) {
    {
        let mut registry = MODEL_REGISTRY.write().unwrap();
        let metadata = registry.entry(class_name.to_string()).or_default();
        if !metadata.encrypted_fields.iter().any(|s| s == field) {
            metadata.encrypted_fields.push(field.to_string());
        }
    }
    let mut cols = ENCRYPTED_COLLECTIONS.write().unwrap();
    let fields = cols.entry(collection.to_string()).or_default();
    if !fields.iter().any(|s| s == field) {
        fields.push(field.to_string());
    }
}

/// Encrypted field names for a model class (used to decrypt on load).
pub fn get_encrypted_fields(class_name: &str) -> Vec<String> {
    MODEL_REGISTRY
        .read()
        .unwrap()
        .get(class_name)
        .map(|m| m.encrypted_fields.clone())
        .unwrap_or_default()
}

/// Encrypt the declared `encrypts` fields of `document` in place (string values
/// only). Called from the DB write layer so every standard Model write path
/// (create/save/update) is covered, including inside a transaction.
pub fn encrypt_document_fields(
    collection: &str,
    document: &mut serde_json::Value,
) -> Result<(), String> {
    let fields = {
        let cols = ENCRYPTED_COLLECTIONS.read().unwrap();
        match cols.get(collection) {
            Some(f) if !f.is_empty() => f.clone(),
            _ => return Ok(()),
        }
    };
    if let Some(obj) = document.as_object_mut() {
        for field in &fields {
            if let Some(serde_json::Value::String(plaintext)) = obj.get(field) {
                let ciphertext = crate::interpreter::builtins::crypto::encrypt_field(plaintext)?;
                obj.insert(field.clone(), serde_json::Value::String(ciphertext));
            }
        }
    }
    Ok(())
}

// Thread-local registry for model classes (used for lazy relation conversion).
thread_local! {
    pub static MODEL_CLASSES: RefCell<HashMap<String, Rc<Class>>> = RefCell::new(HashMap::new());
    /// `enum_field` declarations: model class name → [(field, enum class)].
    /// Thread-local because it holds `Rc<Class>` (registered per worker when the
    /// model's class body runs). Drives enum reconstruction on DB reads.
    pub static ENUM_FIELDS: RefCell<HashMap<String, Vec<EnumFieldBinding>>> =
        RefCell::new(HashMap::new());
}

/// Register a model field as holding values of the given enum class (the
/// `enum_field(:status, Status)` DSL). On read, `json_doc_to_instance` rebuilds
/// the enum from its stored string/object via `build_enum_value`.
pub fn register_enum_field(class_name: &str, field: &str, enum_class: Rc<Class>) {
    ENUM_FIELDS.with(|fields| {
        let mut map = fields.borrow_mut();
        let entry = map.entry(class_name.to_string()).or_default();
        // Re-declaring a field replaces the prior mapping.
        entry.retain(|(name, _)| name != field);
        entry.push((field.to_string(), enum_class));
    });
}

/// The enum fields declared on a model class: [(field, enum class)].
pub fn get_enum_fields(class_name: &str) -> Vec<EnumFieldBinding> {
    ENUM_FIELDS.with(|fields| fields.borrow().get(class_name).cloned().unwrap_or_default())
}

/// Register (or replace, by field) a state machine declared on a model class.
/// Re-declaring the same field overwrites the prior machine.
pub fn set_state_machine(class_name: &str, def: StateMachineDef) {
    let mut registry = MODEL_REGISTRY.write().unwrap();
    let metadata = registry.entry(class_name.to_string()).or_default();
    metadata.state_machines.retain(|m| m.field != def.field);
    metadata.state_machines.push(def);
}

/// All state machines declared on a model class.
pub fn get_state_machines(class_name: &str) -> Vec<StateMachineDef> {
    MODEL_REGISTRY
        .read()
        .unwrap()
        .get(class_name)
        .map(|m| m.state_machines.clone())
        .unwrap_or_default()
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

/// Declare the whitelist of attributes accepted by mass-assign on this
/// model. Each call replaces the previous list (re-declaring is OK; the
/// last declaration wins). Pass an empty list to lock down the model
/// completely (`Model.create({...})` will then drop every key).
pub fn register_accessible_attributes(class_name: &str, fields: Vec<String>) {
    let mut registry = MODEL_REGISTRY.write().unwrap();
    let metadata = registry.entry(class_name.to_string()).or_default();
    metadata.accessible_attributes = Some(fields);
}

/// `None` means the model never called `attr_accessible(...)` and falls
/// back to the legacy "all fields writable" behaviour. Returning the cloned
/// list is fine — it's typically a handful of strings, so cloning beats
/// keeping a long-lived read lock across the filter loop.
pub fn get_accessible_attributes(class_name: &str) -> Option<Vec<String>> {
    let registry = MODEL_REGISTRY.read().unwrap();
    registry
        .get(class_name)
        .and_then(|m| m.accessible_attributes.clone())
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
    ENUM_FIELDS.with(|fields| fields.borrow_mut().clear());
    super::state_machine::clear();
}

/// Clear all model registries (MODEL_REGISTRY and MODEL_CLASSES). Used during hot reload.
pub fn clear_all_model_registries() {
    MODEL_REGISTRY.write().unwrap().clear();
    ENUM_FIELDS.with(|fields| fields.borrow_mut().clear());
    MODEL_CLASSES.with(|classes: &RefCell<HashMap<String, Rc<Class>>>| {
        classes.borrow_mut().clear();
    });
    super::state_machine::clear();
}

#[cfg(test)]
mod encryption_tests {
    use super::*;

    #[test]
    fn registers_encrypts_and_round_trips_a_document() {
        std::env::set_var("SOLI_ENCRYPTION_KEY", "unit-test-key-please-rotate");
        register_encryption("EncTestUser", "enc_test_users", "ssn");
        assert_eq!(get_encrypted_fields("EncTestUser"), vec!["ssn".to_string()]);

        let mut doc = serde_json::json!({ "ssn": "123-45-6789", "name": "Bob" });
        encrypt_document_fields("enc_test_users", &mut doc).unwrap();

        let stored = doc["ssn"].as_str().unwrap();
        assert_ne!(stored, "123-45-6789", "ssn should be encrypted at rest");
        assert_eq!(doc["name"], "Bob", "non-encrypted field is untouched");

        let decrypted = crate::interpreter::builtins::crypto::decrypt_field(stored).unwrap();
        assert_eq!(decrypted, "123-45-6789", "decrypts back to plaintext");
    }

    #[test]
    fn unencrypted_collection_is_left_alone() {
        let mut doc = serde_json::json!({ "x": "plain" });
        encrypt_document_fields("collection_with_no_encrypts", &mut doc).unwrap();
        assert_eq!(doc["x"], "plain");
    }
}
