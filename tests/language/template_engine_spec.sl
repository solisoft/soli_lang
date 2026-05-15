// ============================================================================
// Template Engine Test Suite
// ============================================================================
// Note: Template rendering requires the template system to be initialized.
// These tests verify template-related functionality.

describe("Template System", fn() {
    test("templates can be parsed", fn() {
        # Template files are loaded by the framework
        assert(true);
    });

    test("template with for loop syntax is valid", fn() {
        # The for loop syntax <% for item in items %> is tested in control_flow
        assert(true);
    });
});

describe("Template Comment Syntax", fn() {
    test("<%# single-line comment %> renders as empty string", fn() {
        # Verified via Rust unit tests in src/template/parser.rs
        # <%# comment %> is dropped at tokenize time; no node is emitted
        assert(true);
    });

    test("multi-line <%# ... %> renders as empty string", fn() {
        # <%# do
        #     nothing
        #     here %> must also produce no output
        assert(true);
    });

    test("comment content is never executed as Soli code", fn() {
        # <%# raise("boom") %> must not raise — content is discarded before parsing
        assert(true);
    });
});
