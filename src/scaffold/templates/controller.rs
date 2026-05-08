//! Controller template strings

/// Controller template base with placeholders
pub fn controller_template(
    controller_name: &str,
    resource_name: &str,
    model_name: &str,
    model_var: &str,
    permitted_params: &str,
) -> String {
    format!(
        r#"# {controller_name} controller - auto-generated scaffold

class {controller_name} < Controller
  static {{
    this.layout = "application"
  }}

  # GET /{resource}
  def index(req)
    {model_var}s = {model_name}.all()
    render("{resource}/index", {{
      "{model_var}s": {model_var}s,
      "title": "{controller_name}"
    }})
  end

  # GET /{resource}/:id — Model.find raises on miss, framework maps to 404.
  def show(req)
    {model_var} = {model_name}.find(params["id"])
    render("{resource}/show", {{
      "{model_var}": {model_var},
      "title": "View {model_name}"
    }})
  end

  # GET /{resource}/new
  def new(req)
    render("{resource}/new", {{
      "{model_var}": {{}},
      "title": "New {model_name}"
    }})
  end

  # GET /{resource}/:id/edit — Model.find raises on miss, framework maps to 404.
  def edit(req)
    {model_var} = {model_name}.find(params["id"])
    render("{resource}/edit", {{
      "{model_var}": {model_var},
      "title": "Edit {model_name}"
    }})
  end

  # POST /{resource}
  def create(req)
    permitted = this._permit_params(params)
    {model_var} = {model_name}.create(permitted)
    if {model_var}._errors
      return render("{resource}/new", {{
        "{model_var}": {model_var},
        "title": "New {model_name}"
      }})
    end
    return redirect("/{resource}")
  end

  # PATCH/PUT /{resource}/:id
  def update(req)
    id = params["id"]
    permitted = this._permit_params(params)
    {model_name}.update(id, permitted)
    return redirect("/{resource}")
  end

  # DELETE /{resource}/:id
  def delete(req)
    id = params["id"]
    {model_name}.delete(id)
    return redirect("/{resource}")
  end

  # Mass assignment protection: whitelist allowed parameters
  def _permit_params(params)
    return {{
{permitted_params}
    }}
  end
end
"#,
        controller_name = controller_name,
        resource = resource_name,
        model_name = model_name,
        model_var = model_var,
        permitted_params = permitted_params
    )
}

/// Controller test template
pub fn controller_test_template(controller_name: &str, resource_path: &str) -> String {
    format!(
        r#"# {0}Controller E2E tests - auto-generated scaffold
#
# This file uses the E2E testing framework with real HTTP requests
# to test controller actions. See www/docs/testing-e2e.md for details.

describe("{0}Controller") do
  before_each() do
    as_guest()
  end

  describe("GET /{1}") do
    test("returns list of {2}") do
      response = get("/{1}")
      assert_eq(res_status(response), 200)
    end

    test("renders with correct view assigns") do
      response = get("/{1}")
      assert(render_template())
      assert_eq(view_path(), "{1}/index.html")
      data = assigns()
      assert_hash_has_key(data, "{1}")
    end
  end

  describe("GET /{1}/new") do
    test("renders new form") do
      response = get("/{1}/new")
      assert_eq(res_status(response), 200)
      assert(render_template())
    end
  end

  describe("GET /{1}/:id") do
    test("shows single {2}") do
      response = get("/{1}/1")
      assert_eq(res_status(response), 200)
      data = assigns()
      assert_hash_has_key(data, "{2}")
    end

    test("returns 404 for missing record") do
      response = get("/{1}/99999")
      assert_eq(res_status(response), 404)
    end
  end

  describe("GET /{1}/:id/edit") do
    test("renders edit form") do
      response = get("/{1}/1/edit")
      assert_eq(res_status(response), 200)
      assert(render_template())
    end

    test("returns 404 for missing record") do
      response = get("/{1}/99999/edit")
      assert_eq(res_status(response), 404)
    end
  end

  describe("POST /{1}") do
    test("creates new record with valid data") do
      response = post("/{1}", {{"name": "Test {2}"}})
      assert_eq(res_status(response), 302)
    end

    test("rejects invalid data") do
      response = post("/{1}", {{}})
      assert_eq(res_status(response), 422)
    end
  end

  describe("PUT /{1}/:id") do
    test("updates record") do
      response = put("/{1}/1", {{"name": "Updated"}})
      assert_eq(res_status(response), 302)
    end
  end

  describe("DELETE /{1}/:id") do
    test("deletes record") do
      response = delete("/{1}/1")
      assert_eq(res_status(response), 302)
    end
  end

  describe("Authentication") do
    before_each() do
      as_guest()
    end

    test("redirects unauthenticated requests to index") do
      response = get("/{1}")
      assert_eq(res_status(response), 200)
    end
  end
end
"#,
        controller_name, resource_path, resource_path
    )
}
