// ============================================================================
// Template Functions Test Suite
// ============================================================================
// Tests for template rendering functions
// ============================================================================

describe("Template Render Functions", fn() {
    test("render() renders template with data", fn() {
        let template = "<h1>{{ name }}</h1>";
        let data = hash();
        data["name"] = "World";

        let result = render(template, data);
        assert_contains(result, "World");
    });

    test("render() with layout", fn() {
        let template = "<h1>{{ title }}</h1>";
        let layout = "<html><body>{{ content }}</body></html>";
        let data = hash();
        data["title"] = "Hello";

        let options = hash();
        options["layout"] = layout;
        let result = render(template, data, options);
        assert_contains(result, "<html>");
        assert_contains(result, "<body>");
    });

    test("render() with partials", fn() {
        let template = "{{> header }}<p>Content</p>";
        let partials = hash();
        partials["header"] = "<header>Header</header>";

        let options = hash();
        options["partials"] = partials;
        let result = render(template, hash(), options);
        assert_contains(result, "<header>Header</header>");
    });

    test("render_partial() renders without layout", fn() {
        let template = "<p>{{ message }}</p>";
        let data = hash();
        data["message"] = "Hello";

        let result = render_partial(template, data);
        assert_contains(result, "Hello");
    });
});

describe("Template Helpers", fn() {
    test("h() is alias for html_escape()", fn() {
        let escaped = h("<script>alert('xss')</script>");
        assert_not_contains(escaped, "<script>");
    });

    test("public_path() adds version hash", fn() {
        let path = public_path("/assets/app.js");
        assert_contains(path, "/assets/app.js");
    });

    test("redirect() creates redirect response", fn() {
        let response = redirect("/new-location");
        assert_contains(response, "Location: /new-location");
    });

    test("render_json() creates JSON response", fn() {
        let data = hash();
        data["status"] = "ok";
        data["value"] = 42;
        let response = render_json(data);
        assert_contains(response, "status");
        assert_contains(response, "ok");
    });

    test("render_text() creates text response", fn() {
        let response = render_text("Plain text");
        assert_contains(response, "Plain text");
    });
});

describe("Template Conditionals", fn() {
    test("{{#if}} renders conditionally", fn() {
        let template = "{{#if show}}Visible{{/if}}";
        let data1 = hash();
        data1["show"] = true;
        let result1 = render(template, data1);

        let data2 = hash();
        data2["show"] = false;
        let result2 = render(template, data2);

        assert_contains(result1, "Visible");
        assert_not_contains(result2, "Visible");
    });
});

describe("Template Loops", fn() {
    test("{{#each}} iterates array", fn() {
        let template = "{{#each items}}<li>{{ this }}</li>{{/each}}";
        let data = hash();
        data["items"] = ["A", "B", "C"];

        let result = render(template, data);
        assert_contains(result, "<li>A</li>");
        assert_contains(result, "<li>B</li>");
        assert_contains(result, "<li>C</li>");
    });
});

describe("Template Variables", fn() {
    test("simple variable interpolation", fn() {
        let template = "Hello, {{ name }}!";
        let data = hash();
        data["name"] = "Alice";

        let result = render(template, data);
        assert_eq(result, "Hello, Alice!");
    });

    test("nested variable access", fn() {
        let template = "{{ user.profile.name }}";
        let data = hash();
        let user = hash();
        let profile = hash();
        profile["name"] = "Bob";
        user["profile"] = profile;
        data["user"] = user;

        let result = render(template, data);
        assert_eq(result, "Bob");
    });

    test("missing variable renders empty", fn() {
        let template = "{{ missing }}";
        let data = hash();

        let result = render(template, data);
        assert_eq(result, "");
    });
});

describe("Template Filters", fn() {
    test("uppercase filter", fn() {
        let template = "{{ name | uppercase }}";
        let data = hash();
        data["name"] = "hello";

        let result = render(template, data);
        assert_eq(result, "HELLO");
    });

    test("lowercase filter", fn() {
        let template = "{{ name | lowercase }}";
        let data = hash();
        data["name"] = "HELLO";

        let result = render(template, data);
        assert_eq(result, "hello");
    });

    test("length filter", fn() {
        let template = "{{ items | length }}";
        let data = hash();
        data["items"] = [1, 2, 3];

        let result = render(template, data);
        assert_eq(result, "3");
    });
});
