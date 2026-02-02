//! Controller scaffolding generator

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::write_file;
use crate::scaffold::templates::controller;
use crate::scaffold::utils::{to_pascal_case, to_snake_case, to_snake_case_plural};
use crate::scaffold::FieldDefinition;

/// Create a controller for a scaffold
pub fn create_controller(
    app_path: &Path,
    name: &str,
    fields: &[FieldDefinition],
) -> Result<(), String> {
    let controller_name = to_pascal_case(name) + "Controller";
    let resource_name = to_snake_case_plural(name);
    let model_name = to_pascal_case(name);
    let model_var = to_snake_case(name);

    // Generate the list of permitted parameters for mass assignment protection
    let permitted_params = fields
        .iter()
        .map(|f| {
            format!(
                r#"            "{}": params["{}"]"#,
                f.to_snake_case(),
                f.to_snake_case()
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");

    let permitted_params = if permitted_params.is_empty() {
        "            // (no fields defined)".to_string()
    } else {
        permitted_params
    };

    let content = controller::controller_template(
        &controller_name,
        &resource_name,
        &model_name,
        &model_var,
        &permitted_params,
    );

    let controller_path = app_path
        .join("app/controllers")
        .join(format!("{}_controller.sl", to_snake_case(name)));
    write_file(&controller_path, &content)?;
    Ok(())
}

/// Create controller tests for a scaffold
pub fn create_tests(app_path: &Path, name: &str) -> Result<(), String> {
    let snake_name = to_snake_case(name);
    let resource_path = to_snake_case_plural(name);

    let tests_dir = app_path.join("tests");
    let controllers_dir = tests_dir.join("controllers");

    if !controllers_dir.exists() {
        fs::create_dir_all(&controllers_dir).map_err(|e| {
            format!(
                "Failed to create directory '{}': {}",
                controllers_dir.display(),
                e
            )
        })?;
    }

    let controller_name = to_pascal_case(name) + "Controller";
    let controller_test_content =
        controller::controller_test_template(&controller_name, &resource_path);

    let controller_test_path = controllers_dir.join(format!("{}_controller_spec.sl", snake_name));
    write_file(&controller_test_path, &controller_test_content)?;
    println!("  Created: {}", controller_test_path.display());

    Ok(())
}
