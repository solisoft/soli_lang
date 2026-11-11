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

  # GET /{resource} — every @field reaches the view and the layout, and the
  # matching {resource}/<action> view renders by itself: no render() needed.
  def index
    @{resource} = {model_name}.all
    @title = "{controller_name}"
  end

  # GET /{resource}/:id — Model.find raises on miss, framework maps to 404.
  def show
    @{model_var} = {model_name}.find(params["id"])
    @title = "View {model_name}"
  end

  # GET /{resource}/new
  def new
    @{model_var} = {model_name}.new()
  end

  # GET /{resource}/:id/edit
  def edit
    @{model_var} = {model_name}.find(params["id"])
    @title = "Edit {model_name}"
  end

  # POST /{resource}
  def create
    @{model_var} = {model_name}.create(@_permit_params(params))
    if @{model_var}._errors
      @title = "New {model_name}"
      return render("{resource}/new", {{}}, {{ "status": 422 }})
    end

    redirect("/{resource}")
  end

  # PATCH/PUT /{resource}/:id — an invalid change re-renders the form, 422.
  def update
    @{model_var} = {model_name}.find(params["id"])
    @{model_var}.update(@_permit_params(params))
    if @{model_var}._errors
      @title = "Edit {model_name}"
      return render("{resource}/edit", {{}}, {{ "status": 422 }})
    end

    redirect("/{resource}")
  end

  # DELETE /{resource}/:id
  def delete
    id = params["id"]
    {model_name}.delete(id)
    redirect("/{resource}")
  end

  private

  # Mass-assignment whitelist: permit() keeps exactly this shape and drops
  # everything else a client posts (true = scalar, [] = array of scalars,
  # a nested hash literal = nested whitelist).
  def _permit_params(params)
    return permit(params, {{
{permitted_params}
    }})
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

/// Controller spec template. It makes the records it reads (`_create_<model>`)
/// from `sample_attrs`, a hash literal holding a valid value for every
/// scaffolded field, so it passes on a fresh app with an empty test database.
/// `blank_attrs` (the presence-validated fields, blank) adds the 422 cases of
/// create and update, which only exist when a field is validated.
pub fn controller_test_template(
    controller_name: &str,
    resource_path: &str,
    model_name: &str,
    model_var: &str,
    sample_attrs: &str,
    blank_attrs: Option<&str>,
) -> String {
    let (invalid_test, invalid_update_test) = match blank_attrs {
        Some(blank) => (
            format!(
                r#"

    test("rejects invalid data") do
      response = post("/{resource_path}", {{}})
      assert_eq(res_status(response), 422)
    end"#
            ),
            format!(
                r#"

    test("rejects invalid data") do
      record = _create_{model_var}()
      response = put("/{resource_path}/#{{record._key}}", {blank})
      assert_eq(res_status(response), 422)
    end"#
            ),
        ),
        None => (String::new(), String::new()),
    };
    format!(
        r#"# {controller_name}Controller E2E tests - auto-generated scaffold
#
# This file uses the E2E testing framework with real HTTP requests
# to test controller actions. See www/docs/testing-e2e.md for details.

def _create_{model_var}
  {model_name}.create({sample_attrs})
end

describe("{controller_name}Controller") do
  before_each() do
    as_guest()
  end

  describe("GET /{resource_path}") do
    test("returns list of {resource_path}") do
      response = get("/{resource_path}")
      assert_eq(res_status(response), 200)
    end

    test("renders with correct view assigns") do
      get("/{resource_path}")
      assert(render_template())
      assert_eq(view_path(), "{resource_path}/index.html")
      assert_hash_has_key(assigns(), "{resource_path}")
    end
  end

  describe("GET /{resource_path}/new") do
    test("renders new form") do
      response = get("/{resource_path}/new")
      assert_eq(res_status(response), 200)
      assert(render_template())
    end
  end

  describe("GET /{resource_path}/:id") do
    test("shows a single {model_var}") do
      record = _create_{model_var}()
      response = get("/{resource_path}/#{{record._key}}")
      assert_eq(res_status(response), 200)
      assert_hash_has_key(assigns(), "{model_var}")
    end

    test("returns 404 for missing record") do
      response = get("/{resource_path}/99999")
      assert_eq(res_status(response), 404)
    end
  end

  describe("GET /{resource_path}/:id/edit") do
    test("renders edit form") do
      record = _create_{model_var}()
      response = get("/{resource_path}/#{{record._key}}/edit")
      assert_eq(res_status(response), 200)
      assert(render_template())
    end

    test("returns 404 for missing record") do
      response = get("/{resource_path}/99999/edit")
      assert_eq(res_status(response), 404)
    end
  end

  describe("POST /{resource_path}") do
    test("creates new record with valid data") do
      response = post("/{resource_path}", {sample_attrs})
      assert_eq(res_status(response), 302)
    end{invalid_test}
  end

  describe("PUT /{resource_path}/:id") do
    test("updates record") do
      record = _create_{model_var}()
      response = put("/{resource_path}/#{{record._key}}", {sample_attrs})
      assert_eq(res_status(response), 302)
    end{invalid_update_test}
  end

  describe("DELETE /{resource_path}/:id") do
    test("deletes record") do
      record = _create_{model_var}()
      response = delete("/{resource_path}/#{{record._key}}")
      assert_eq(res_status(response), 302)
    end
  end
end
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The template appends "Controller" to the name it is given; a caller that
    /// also appends produces `describe("UsersControllerController")`.
    #[test]
    fn controller_spec_names_the_controller_once() {
        let spec = controller_test_template(
            "Users",
            "users",
            "User",
            "user",
            r#"{"name": "Sample name"}"#,
            Some(r#"{"name": ""}"#),
        );
        assert!(
            spec.contains(r#"describe("UsersController")"#),
            "unexpected describe name:\n{spec}"
        );
        assert!(!spec.contains("ControllerController"), "doubled suffix");
        // And the generated assertions must be ones the runner actually has.
        for name in ["assert_eq(", "assert(", "assert_hash_has_key("] {
            assert!(spec.contains(name), "missing {name}");
        }
        for bogus in ["assert_true", "assert_equal", "assert_nil"] {
            assert!(
                !spec.contains(bogus),
                "generated a nonexistent assertion: {bogus}"
            );
        }
    }
}
