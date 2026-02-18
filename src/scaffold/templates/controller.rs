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
        r#"// {controller_name} controller - auto-generated scaffold

class {controller_name} extends Controller {{
    static {{
        this.layout = "application";
    }}

    // GET /{resource}
    fn index(req) {{
        let {model_var}s = {model_name}.all();
        return render("{resource}/index", {{
            "{model_var}s": {model_var}s,
            "title": "{controller_name}"
        }});
    }}

    // GET /{resource}/:id
    fn show(req) {{
        let id = req.params["id"];
        let {model_var} = {model_name}.find(id);
        if {model_var} == null {{
            return error(404, "{model_name} not found");
        }}
        return render("{resource}/show", {{
            "{model_var}": {model_var},
            "title": "View {model_name}"
        }});
    }}

    // GET /{resource}/new
    fn new(req) {{
        return render("{resource}/new", {{
            "{model_var}": {{}},
            "title": "New {model_name}"
        }});
    }}

    // GET /{resource}/:id/edit
    fn edit(req) {{
        let id = req.params["id"];
        let {model_var} = {model_name}.find(id);
        if {model_var} == null {{
            return error(404, "{model_name} not found");
        }}
        return render("{resource}/edit", {{
            "{model_var}": {model_var},
            "title": "Edit {model_name}"
        }});
    }}

    // POST /{resource}
    fn create(req) {{
        let permitted = this._permit_params(req.params);
        let result = {model_name}.create(permitted);
        if result["valid"] == true {{
            return redirect("/{resource}");
        }}
        return render("{resource}/new", {{
            "{model_var}": result,
            "title": "New {model_name}"
        }});
    }}

    // PATCH/PUT /{resource}/:id
    fn update(req) {{
        let id = req.params["id"];
        let permitted = this._permit_params(req.params);
        {model_name}.update(id, permitted);
        return redirect("/{resource}");
    }}

    // DELETE /{resource}/:id
    fn delete(req) {{
        let id = req.params["id"];
        {model_name}.delete(id);
        return redirect("/{resource}");
    }}

    // Mass assignment protection: whitelist allowed parameters
    fn _permit_params(params: Any) -> Any {{
        return {{
{permitted_params}
        }};
    }}
}}
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
        r#"// {0}Controller E2E tests - auto-generated scaffold
//
// This file uses the E2E testing framework with real HTTP requests
// to test controller actions. See www/docs/testing-e2e.md for details.

describe("{0}Controller", fn() {{
    before_each(fn() {{
        as_guest();
    }})

    describe("GET /{1}", fn() {{
        test("returns list of {2}", fn() {{
            let response = get("/{1}");
            assert_eq(res_status(response), 200);
        }})

        test("renders with correct view assigns", fn() {{
            let response = get("/{1}");
            assert(render_template());
            assert_eq(view_path(), "{1}/index.html");
            let data = assigns();
            assert_hash_has_key(data, "{1}");
        }})
    }})

    describe("GET /{1}/new", fn() {{
        test("renders new form", fn() {{
            let response = get("/{1}/new");
            assert_eq(res_status(response), 200);
            assert(render_template());
        }})
    }})

    describe("GET /{1}/:id", fn() {{
        test("shows single {2}", fn() {{
            let response = get("/{1}/1");
            assert_eq(res_status(response), 200);
            let data = assigns();
            assert_hash_has_key(data, "{2}");
        }})

        test("returns 404 for missing record", fn() {{
            let response = get("/{1}/99999");
            assert_eq(res_status(response), 404);
        }})
    }})

    describe("GET /{1}/:id/edit", fn() {{
        test("renders edit form", fn() {{
            let response = get("/{1}/1/edit");
            assert_eq(res_status(response), 200);
            assert(render_template());
        }})

        test("returns 404 for missing record", fn() {{
            let response = get("/{1}/99999/edit");
            assert_eq(res_status(response), 404);
        }})
    }})

    describe("POST /{1}", fn() {{
        test("creates new record with valid data", fn() {{
            let response = post("/{1}", {{"name": "Test {2}"}});
            assert_eq(res_status(response), 302);
        }})

        test("rejects invalid data", fn() {{
            let response = post("/{1}", {{}});
            assert_eq(res_status(response), 422);
        }})
    }})

    describe("PUT /{1}/:id", fn() {{
        test("updates record", fn() {{
            let response = put("/{1}/1", {{"name": "Updated"}});
            assert_eq(res_status(response), 302);
        }})
    }})

    describe("DELETE /{1}/:id", fn() {{
        test("deletes record", fn() {{
            let response = delete("/{1}/1");
            assert_eq(res_status(response), 302);
        }})
    }})

    describe("Authentication", fn() {{
        before_each(fn() {{
            as_guest();
        }})

        test("redirects unauthenticated requests to index", fn() {{
            let response = get("/{1}");
            assert_eq(res_status(response), 200);
        }})
    }})
}})
"#,
        controller_name, resource_path, resource_path
    )
}
