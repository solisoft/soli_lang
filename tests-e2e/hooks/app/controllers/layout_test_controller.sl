# End-to-end fixture for the controller-registered-layout fallback.
#
# The controller declares `this.layout = "custom_layout_e2e"` in its static
# block. When the action calls `render(...)` without passing a `layout`
# key, the framework should fall back to this registered layout. The test
# asserts that both the layout's marker and the view body appear in the
# response, in the expected order.

class LayoutTestController extends Controller
    static {
        this.layout = "custom_layout_e2e";

        # Per-action override: the `print_doc` action uses a different layout
        # without passing `layout:` to its `render(...)` call. Every other
        # action keeps the controller-wide `custom_layout_e2e` default.
        this.layout("print_layout_e2e", only: [:print_doc]);
    }

    # GET /layout_test/default
    # Render without an explicit layout — expect the registered
    # "custom_layout_e2e" layout to wrap the view.
    def default(req)
        render("layout_test/default_view")
    end

    # GET /layout_test/print_doc
    # Render without an explicit layout — the per-action rule must select
    # "print_layout_e2e" instead of the controller default.
    def print_doc(req)
        render("layout_test/default_view")
    end

    # GET /layout_test/explicit_none
    # Explicit `"layout": false` must win over the registered layout —
    # body should be just the view with no layout wrapping.
    def explicit_none(req)
        render("layout_test/default_view", {"layout": false})
    end

    # GET /layout_test/auto_render
    # No explicit `render(...)` call — the action relies on auto-rendering the
    # matching `layout_test/auto_render` view. The registered layout must still
    # be applied on this path (regression: it used to fall back to "application").
    def auto_render(req)
        @marker = "auto"
    end
end
