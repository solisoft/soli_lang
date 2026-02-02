//! Model scaffolding generator

use std::path::Path;

use crate::scaffold::app_generator::write_file;
use crate::scaffold::templates::model;
use crate::scaffold::utils::{to_pascal_case, to_snake_case, to_snake_case_plural};
use crate::scaffold::FieldDefinition;

/// Create a model for a scaffold
pub fn create_model(app_path: &Path, name: &str, fields: &[FieldDefinition]) -> Result<(), String> {
    let model_name = to_pascal_case(name);
    let collection_name = to_snake_case_plural(name);

    let validations = fields
        .iter()
        .filter(|f| {
            matches!(
                f.field_type.as_str(),
                "string" | "text" | "email" | "password" | "url"
            )
        })
        .map(|f| {
            format!(
                "validates(\"{}\", {{ \"presence\": true }})",
                f.to_snake_case()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let field_comments = fields
        .iter()
        .map(|f| format!("        // {} ({})", f.to_snake_case(), f.field_type))
        .collect::<Vec<_>>()
        .join("\n");

    let field_comments = if field_comments.is_empty() {
        "        // (no additional fields)".to_string()
    } else {
        field_comments
    };

    let validations = if validations.is_empty() {
        "        // (no validations defined)".to_string()
    } else {
        format!("        {}", validations.replace("\n", "\n        "))
    };

    let content =
        model::model_template(&model_name, &collection_name, &field_comments, &validations);

    let model_path = app_path
        .join("app/models")
        .join(format!("{}_model.sl", to_snake_case(name)));
    write_file(&model_path, &content)?;
    Ok(())
}
