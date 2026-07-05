//! View template strings

/// Resource index view template
pub fn index_view_template(
    title: &str,
    resource_name: &str,
    model_title: &str,
    model_var: &str,
    table_headers: &str,
    table_cells: &str,
    colspan: usize,
) -> String {
    format!(
        r#"<div class="p-6">
    <div class="flex justify-between items-center mb-6">
        <h1 class="text-2xl font-bold">{title}</h1>
        <a href="/{resource}/new" class="bg-indigo-600 hover:bg-indigo-700 text-white px-4 py-2 rounded-lg transition-colors">
            New {model_title}
        </a>
    </div>

    <div class="bg-slate-800 rounded-xl overflow-hidden">
        <table class="w-full">
            <thead class="bg-slate-700">
                <tr>
                    <th class="px-6 py-3 text-left text-xs font-medium text-slate-300 uppercase tracking-wider">ID</th>
{table_headers}
                    <th class="px-6 py-3 text-left text-xs font-medium text-slate-300 uppercase tracking-wider">Actions</th>
                </tr>
            </thead>
            <tbody class="divide-y divide-slate-700">
                <% if {model_var}s.empty? %>
                <tr>
                    <td colspan="{colspan}" class="px-6 py-8 text-center text-slate-400">
                        No {resource} found. <a href="/{resource}/new" class="text-indigo-400 hover:text-indigo-300">Create one?</a>
                    </td>
                </tr>
                <% end %>
                <% {model_var}s.each do |{model_var}| %>
                <tr class="hover:bg-slate-700/50 transition-colors">
                    <td class="px-6 py-4 whitespace-nowrap text-slate-300"><%= {model_var}["_key"] %></td>
{table_cells}
                    <td class="px-6 py-4 whitespace-nowrap">
                        <div class="flex gap-2">
                            <a href="/{resource}/<%= {model_var}["_key"] %>" class="text-indigo-400 hover:text-indigo-300">Show</a>
                            <a href="/{resource}/<%= {model_var}["_key"] %>/edit" class="text-yellow-400 hover:text-yellow-300">Edit</a>
                            <%- button_to("Delete", "/{resource}/" + {model_var}["_key"].to_s, {{
                                "method": "delete", "confirm": "Are you sure?",
                                "class": "text-red-400 hover:text-red-300", "form_class": "inline"
                            }}) %>
                        </div>
                    </td>
                </tr>
                <% end %>
            </tbody>
        </table>
    </div>
</div>
"#,
        title = title,
        resource = resource_name,
        model_title = model_title,
        model_var = model_var,
        table_headers = table_headers,
        table_cells = table_cells,
        colspan = colspan
    )
}

/// Show view template
pub fn show_view_template(
    resource_name: &str,
    resource_title: &str,
    model_var: &str,
    model_title: &str,
    detail_rows: &str,
) -> String {
    format!(
        r#"<div class="p-6">
    <div class="mb-6">
        <a href="/{resource}" class="text-indigo-400 hover:text-indigo-300">&larr; Back to {resource_title}</a>
    </div>

    <div class="bg-slate-800 rounded-xl overflow-hidden">
        <div class="px-6 py-4 border-b border-slate-700 flex justify-between items-center">
            <h1 class="text-xl font-bold">{model_title} Details</h1>
            <div class="flex gap-2">
                <a href="/{resource}/<%= {model_var}["_key"] %>/edit" class="bg-yellow-600 hover:bg-yellow-700 text-white px-3 py-1 rounded transition-colors">Edit</a>
                <%- button_to("Delete", "/{resource}/" + {model_var}["_key"].to_s, {{
                    "method": "delete", "confirm": "Are you sure?",
                    "class": "bg-red-600 hover:bg-red-700 text-white px-3 py-1 rounded transition-colors",
                    "form_class": "inline"
                }}) %>
            </div>
        </div>
        <div class="p-6">
            <dl class="grid grid-cols-1 gap-x-4 gap-y-6 sm:grid-cols-2">
                <div>
                    <dt class="text-sm font-medium text-slate-400">ID</dt>
                    <dd class="mt-1 text-sm text-white"><%= {model_var}["_key"] %></dd>
                </div>
{detail_rows}
            </dl>
        </div>
    </div>
</div>
"#,
        resource = resource_name,
        resource_title = resource_title,
        model_var = model_var,
        model_title = model_title,
        detail_rows = detail_rows
    )
}

/// Form view template (new/edit). `form_action` is a Soli expression (e.g.
/// `"/posts"` or `"/posts/" + post["_key"].to_s`) evaluated inside the
/// `form_with` code block; `method` is the lowercase verb.
pub fn form_view_template(
    resource_name: &str,
    resource_title: &str,
    model_var: &str,
    title: &str,
    form_action: &str,
    method: &str,
) -> String {
    format!(
        r#"<div class="p-6">
    <div class="mb-6">
        <a href="/{resource}" class="text-indigo-400 hover:text-indigo-300">&larr; Back to {resource_title}</a>
    </div>

    <div class="max-w-2xl">
        <h1 class="text-2xl font-bold mb-6">{title}</h1>

        <% f = form_with({model_var}, {{"url": {form_action}, "method": "{method}", "class": "space-y-6"}}) %>
        <%- f.open() %>
            <%- partial("{resource}/form", {{ "{model_var}": {model_var}, "f": f }}) %>
        <%- f.close() %>
    </div>
</div>
"#,
        resource = resource_name,
        resource_title = resource_title,
        model_var = model_var,
        title = title,
        form_action = form_action,
        method = method
    )
}

/// Form partial template. Receives the record and the `f` FormBuilder from
/// the enclosing new/edit view (partials get a fresh scope, so `f` rides in
/// through the render locals).
pub fn form_partial_template(resource_name: &str, model_title: &str, field_inputs: &str) -> String {
    format!(
        r#"<%- f.error_summary({{"class": "bg-red-500/10 border border-red-500/20 rounded-lg p-4 mb-6 text-red-300 text-sm"}}) %>

{field_inputs}

<div class="flex gap-4">
    <%- f.submit("Submit {model_title}", {{
        "class": "bg-indigo-600 hover:bg-indigo-700 text-white px-6 py-2 rounded-lg transition-colors"
    }}) %>
    <a href="/{resource}" class="bg-slate-600 hover:bg-slate-700 text-white px-6 py-2 rounded-lg transition-colors text-center">
        Cancel
    </a>
</div>
"#,
        resource = resource_name,
        model_title = model_title,
        field_inputs = field_inputs
    )
}

/// Default field input when no fields defined
pub const DEFAULT_FIELD_INPUT: &str = r#"            <div>
                <%- f.label("name", "Name", {"class": "block text-sm font-medium text-slate-300 mb-2"}) %>
                <%- f.text_field("name", {
                    "class": "w-full px-4 py-2 bg-slate-700 border border-slate-600 rounded-lg text-white focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:border-transparent",
                    "placeholder": "Enter name"
                }) %>
                <%- f.errors_for("name") %>
            </div>"#;
