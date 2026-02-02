//! Scaffold module for generating new Soli MVC applications.
//!
//! Provides functionality for `soli new app_name` command and resource scaffolding.

pub mod app_generator;
pub mod controller_generator;
pub mod migration_generator;
pub mod model_generator;
pub mod templates;
pub mod ui;
pub mod utils;
pub mod view_generator;

use std::path::Path;

use crate::scaffold::app_generator::write_file;
use crate::scaffold::controller_generator::{create_controller, create_tests};
use crate::scaffold::migration_generator::create_migration;
use crate::scaffold::model_generator::create_model;
use crate::scaffold::utils::{to_snake_case, to_snake_case_plural};
use crate::scaffold::view_generator::{create_form_partial, create_views};

/// A field definition parsed from scaffold arguments
#[derive(Debug, Clone)]
pub struct FieldDefinition {
    name: String,
    field_type: String,
}

impl FieldDefinition {
    pub fn parse(field_str: &str) -> Option<Self> {
        let parts: Vec<&str> = field_str.split(':').collect();
        match parts.as_slice() {
            [name, field_type] => Some(Self {
                name: name.to_string(),
                field_type: field_type.to_string(),
            }),
            _ => None,
        }
    }

    pub fn to_snake_case(&self) -> String {
        let mut result = String::new();
        for (i, c) in self.name.chars().enumerate() {
            if c.is_uppercase() {
                if i > 0 {
                    result.push('_');
                }
                result.push(c.to_ascii_lowercase());
            } else {
                result.push(c);
            }
        }
        result
    }

    pub fn to_title_case(&self) -> String {
        let snake = self.to_snake_case();
        snake
            .split('_')
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Create scaffold for a resource (model, controller, views)
pub fn create_scaffold(folder: &str, name: &str) -> Result<(), String> {
    create_scaffold_with_fields(folder, name, &[])
}

/// Create scaffold for a resource with field definitions
pub fn create_scaffold_with_fields(
    folder: &str,
    name: &str,
    fields: &[String],
) -> Result<(), String> {
    let app_path = Path::new(folder);

    if !app_path.exists() {
        return Err(format!("Directory '{}' does not exist", folder));
    }

    if !app_path.is_dir() {
        return Err(format!("'{}' is not a directory", folder));
    }

    let parsed_fields: Vec<FieldDefinition> = fields
        .iter()
        .filter_map(|f| FieldDefinition::parse(f))
        .collect();

    // Ensure directory structure exists
    ensure_directory_structure(app_path)?;

    // Create model
    create_model(app_path, name, &parsed_fields)?;

    // Create controller
    create_controller(app_path, name, &parsed_fields)?;

    // Create views (index, show, new, edit)
    create_views(app_path, name, &parsed_fields)?;

    // Create form partial (shared by new/edit)
    create_form_partial(app_path, name, &parsed_fields)?;

    // Create migration
    create_migration(app_path, name, &parsed_fields)?;

    // Create tests
    create_tests(app_path, name)?;

    // Add routes
    add_routes(app_path, name)?;

    Ok(())
}

fn ensure_directory_structure(app_path: &Path) -> Result<(), String> {
    use std::fs;

    let dirs = [
        "app/models",
        "app/controllers",
        "app/views",
        "tests",
        "tests/models",
        "tests/controllers",
        "config",
        "db/migrations",
    ];

    for dir in dirs {
        let path = app_path.join(dir);
        if !path.exists() {
            fs::create_dir_all(&path)
                .map_err(|e| format!("Failed to create directory '{}': {}", path.display(), e))?;
        }
    }

    Ok(())
}

fn add_routes(app_path: &Path, name: &str) -> Result<(), String> {
    use std::fs;

    let resource_name = to_snake_case_plural(name);
    let routes_file = app_path.join("config/routes.sl");

    let new_routes = format!(
        r#"

// {name} resource routes
get("/{resource}", "{resource}#index")
get("/{resource}/new", "{resource}#new")
post("/{resource}", "{resource}#create")
get("/{resource}/:id", "{resource}#show")
get("/{resource}/:id/edit", "{resource}#edit")
put("/{resource}/:id", "{resource}#update")
delete("/{resource}/:id", "{resource}#delete")
"#,
        name = name,
        resource = resource_name
    );

    if routes_file.exists() {
        let mut content = fs::read_to_string(&routes_file)
            .map_err(|e| format!("Failed to read routes file: {}", e))?;
        content.push_str(&new_routes);
        fs::write(&routes_file, content)
            .map_err(|e| format!("Failed to write routes file: {}", e))?;
        println!("  Updated: {}/config/routes.sl", app_path.display());
    } else {
        write_file(&routes_file, &new_routes)?;
    }

    Ok(())
}

/// Print success message after creating a scaffold
pub fn print_scaffold_success_message(name: &str) {
    println!();
    println!(
        "  \x1b[32m\x1b[1mSuccess!\x1b[0m Created scaffold for \x1b[1m{}\x1b[0m",
        name
    );
    println!();
    println!("  \x1b[2mGenerated files:\x1b[0m");
    println!();
    println!(
        "    \x1b[36mapp/models/{}_model.sl\x1b[0m",
        to_snake_case(name)
    );
    println!(
        "    \x1b[36mapp/controllers/{}_controller.sl\x1b[0m",
        to_snake_case(name)
    );
    println!(
        "    \x1b[36mapp/views/{}/index.html.erb\x1b[0m",
        to_snake_case_plural(name)
    );
    println!(
        "    \x1b[36mapp/views/{}/show.html.erb\x1b[0m",
        to_snake_case_plural(name)
    );
    println!(
        "    \x1b[36mapp/views/{}/new.html.erb\x1b[0m",
        to_snake_case_plural(name)
    );
    println!(
        "    \x1b[36mapp/views/{}/edit.html.erb\x1b[0m",
        to_snake_case_plural(name)
    );
    println!(
        "    \x1b[36mapp/views/{}/_form.html.erb\x1b[0m",
        to_snake_case_plural(name)
    );
    println!();
    println!("  \x1b[2mTest files:\x1b[0m");
    println!();
    println!(
        "    \x1b[36mtests/models/{}_test.sl\x1b[0m",
        to_snake_case(name)
    );
    println!(
        "    \x1b[36mtests/controllers/{}_controller_test.sl\x1b[0m",
        to_snake_case(name)
    );
    println!();
    println!("  \x1b[2mRoutes added to:\x1b[0m \x1b[36mconfig/routes.sl\x1b[0m");
    println!();
}

// Re-export public functions for backward compatibility
pub use app_generator::create_app;
