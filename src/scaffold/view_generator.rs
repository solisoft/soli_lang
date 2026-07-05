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

    write_file(&view_dir.join("index.html.slv"), &content)?;
    println!("  Created: {}/index.html.slv", view_dir.display());
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

    write_file(&view_dir.join("show.html.slv"), &content)?;
    println!("  Created: {}/show.html.slv", view_dir.display());
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

    // A Soli expression evaluated inside the view's form_with code block.
    let form_action = if action == "new" {
        format!("\"/{}\"", resource_name)
    } else {
        format!("\"/{}/\" + {}[\"_key\"].to_s", resource_name, model_var)
    };
    let method = if action == "new" { "post" } else { "put" };

    let content = view::form_view_template(
        resource_name,
        &to_title_case(resource_name),
        model_var,
        &title,
        &form_action,
        method,
    );

    let filename = format!("{}.html.slv", action);
    write_file(&view_dir.join(&filename), &content)?;
    println!("  Created: {}/{}", view_dir.display(), filename);
    Ok(())
}

/// Create the form partial (_form.html.slv)
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
            let builder_method = match f.field_type.as_str() {
                "email" => "email_field",
                "password" => "password_field",
                "text" | "string" | "url" => "text_field",
                "number" | "integer" | "float" => "number_field",
                "boolean" | "bool" => "check_box",
                "date" => "date_field",
                "datetime" => "datetime_field",
                _ => "text_field",
            };
            let placeholder = format!("Enter {}", label.to_ascii_lowercase());

            if builder_method == "check_box" {
                format!(
                    r#"            <div class="flex items-center">
                <%- f.check_box("{field_name}", {{
                    "class": "h-4 w-4 text-indigo-600 focus:ring-indigo-500 border-slate-600 rounded bg-slate-700"
                }}) %>
                <%- f.label("{field_name}", "{label}", {{"class": "ml-2 block text-sm text-slate-300"}}) %>
            </div>"#
                )
            } else {
                format!(
                    r#"            <div>
                <%- f.label("{field_name}", "{label}", {{"class": "block text-sm font-medium text-slate-300 mb-2"}}) %>
                <%- f.{builder_method}("{field_name}", {{
                    "class": "w-full px-4 py-2 bg-slate-700 border border-slate-600 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:border-transparent",
                    "placeholder": "{placeholder}"
                }}) %>
                <%- f.errors_for("{field_name}") %>
            </div>"#
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let field_inputs = if field_inputs.is_empty() {
        view::DEFAULT_FIELD_INPUT.to_string()
    } else {
        field_inputs
    };

    let content = view::form_partial_template(&resource_name, &model_title, &field_inputs);

    let partial_path = view_dir.join("_form.html.slv");
    write_file(&partial_path, &content)?;
    println!("  Created: {}/_form.html.slv", view_dir.display());

    Ok(())
}
