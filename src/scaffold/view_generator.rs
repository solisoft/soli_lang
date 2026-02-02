//! View scaffolding generator

use std::fs;
use std::path::Path;

use crate::scaffold::app_generator::write_file;
use crate::scaffold::templates::view;
use crate::scaffold::utils::{to_snake_case, to_snake_case_plural, to_title_case};
use crate::scaffold::FieldDefinition;

/// Create all views for a scaffold
pub fn create_views(app_path: &Path, name: &str, fields: &[FieldDefinition]) -> Result<(), String> {
    let resource_name = to_snake_case_plural(name);
    let model_var = to_snake_case(name);

    // Create view directory
    let view_dir = app_path.join("app/views").join(&resource_name);
    fs::create_dir_all(&view_dir)
        .map_err(|e| format!("Failed to create directory '{}': {}", view_dir.display(), e))?;

    // Create index view
    create_resource_index_view(&view_dir, &resource_name, &model_var, fields)?;

    // Create show view
    create_show_view(&view_dir, &resource_name, &model_var, fields)?;

    // Create new view
    create_form_view(&view_dir, &resource_name, &model_var, "new")?;

    // Create edit view
    create_form_view(&view_dir, &resource_name, &model_var, "edit")?;

    Ok(())
}

/// Create the index view for a resource
fn create_resource_index_view(
    view_dir: &Path,
    resource_name: &str,
    model_var: &str,
    fields: &[FieldDefinition],
) -> Result<(), String> {
    let title = to_title_case(resource_name);
    let model_title = to_title_case(model_var);

    let table_headers = fields
        .iter()
        .map(|f| {
            format!(
                r#"                    <th class="px-6 py-3 text-left text-xs font-medium text-slate-300 uppercase tracking-wider">{}</th>"#,
                f.to_title_case()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let table_cells = fields
        .iter()
        .map(|f| {
            format!(
                r#"                    <td class="px-6 py-4 whitespace-nowrap text-white"><%= {model_var}["{field_name}"] %></td>"#,
                model_var = model_var,
                field_name = f.to_snake_case()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let content = view::index_view_template(
        &title,
        resource_name,
        &model_title,
        model_var,
        &table_headers,
        &table_cells,
        2 + fields.len(),
    );

    write_file(&view_dir.join("index.html.erb"), &content)?;
    println!("  Created: {}/index.html.erb", view_dir.display());
    Ok(())
}

/// Create the show view for a resource
fn create_show_view(
    view_dir: &Path,
    resource_name: &str,
    model_var: &str,
    fields: &[FieldDefinition],
) -> Result<(), String> {
    let resource_title = to_title_case(resource_name);
    let model_title = to_title_case(model_var);

    let detail_rows = fields
        .iter()
        .map(|f| {
            format!(
                r#"                <div>
                    <dt class="text-sm font-medium text-slate-400">{field_title}</dt>
                    <dd class="mt-1 text-sm text-white"><%= {model_var}["{field_name}"] %></dd>
                </div>"#,
                model_var = model_var,
                field_title = f.to_title_case(),
                field_name = f.to_snake_case()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let content = view::show_view_template(
        resource_name,
        &resource_title,
        model_var,
        &model_title,
        &detail_rows,
    );

    write_file(&view_dir.join("show.html.erb"), &content)?;
    println!("  Created: {}/show.html.erb", view_dir.display());
    Ok(())
}

/// Create the new/edit form view
fn create_form_view(
    view_dir: &Path,
    resource_name: &str,
    model_var: &str,
    action: &str,
) -> Result<(), String> {
    let title = if action == "new" {
        format!("New {}", to_title_case(model_var))
    } else {
        format!("Edit {}", to_title_case(model_var))
    };

    let form_action = if action == "new" {
        format!("/ {}", resource_name)
    } else {
        format!("/{}/<%= {}[\"id\"] %>", resource_name, model_var)
    };
    let method = if action == "new" { "POST" } else { "PUT" };

    let content = view::form_view_template(
        resource_name,
        &to_title_case(resource_name),
        model_var,
        &title,
        &form_action,
        method,
    );

    let filename = format!("{}.html.erb", action);
    write_file(&view_dir.join(&filename), &content)?;
    println!("  Created: {}/{}", view_dir.display(), filename);
    Ok(())
}

/// Create the form partial (_form.html.erb)
pub fn create_form_partial(
    app_path: &Path,
    name: &str,
    fields: &[FieldDefinition],
) -> Result<(), String> {
    let resource_name = to_snake_case_plural(name);
    let model_var = to_snake_case(name);
    let model_title = to_title_case(&model_var);

    let view_dir = app_path.join("app/views").join(&resource_name);

    let field_inputs = fields
        .iter()
        .map(|f| {
            let label = f.to_title_case();
            let field_name = f.to_snake_case();
            let input_type = match f.field_type.as_str() {
                "email" => "email",
                "password" => "password",
                "text" | "string" | "url" => "text",
                "number" | "integer" | "float" => "number",
                "boolean" | "bool" => "checkbox",
                "date" => "date",
                "datetime" => "datetime-local",
                _ => "text",
            };
            let placeholder = format!("Enter {}", label.to_ascii_lowercase());

            if input_type == "checkbox" {
                format!(
                    r#"            <div class="flex items-center">
                <input type="checkbox" id="{field_name}" name="{field_name}" value="true"
                    class="h-4 w-4 text-indigo-600 focus:ring-indigo-500 border-slate-600 rounded bg-slate-700"
                    <% if {model_var}["{field_name}"] == true %>checked<% end %>>
                <label for="{field_name}" class="ml-2 block text-sm text-slate-300">{label}</label>
            </div>"#
                )
            } else {
                format!(
                    r#"            <div>
                <label for="{field_name}" class="block text-sm font-medium text-slate-300 mb-2">{label}</label>
                <input type="{input_type}" id="{field_name}" name="{field_name}" value="<%= {model_var}["{field_name}"] %>"
                    class="w-full px-4 py-2 bg-slate-700 border border-slate-600 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:border-transparent"
                    placeholder="{placeholder}">
            </div>"#,
                    field_name = field_name,
                    input_type = input_type,
                    label = label,
                    placeholder = placeholder,
                    model_var = model_var
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let field_inputs = if field_inputs.is_empty() {
        view::DEFAULT_FIELD_INPUT.replace("{model_var}", &model_var)
    } else {
        field_inputs
    };

    let content =
        view::form_partial_template(&model_var, &resource_name, &model_title, &field_inputs);

    let partial_path = view_dir.join("_form.html.erb");
    write_file(&partial_path, &content)?;
    println!("  Created: {}/_form.html.erb", view_dir.display());

    Ok(())
}
