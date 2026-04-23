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
    }

    # GET /layout_test/default
    # Render without an explicit layout — expect the registered
    # "custom_layout_e2e" layout to wrap the view.
    fn default(req)
        render("layout_test/default_view")
    end

    # GET /layout_test/explicit_none
    # Explicit `"layout": false` must win over the registered layout —
    # body should be just the view with no layout wrapping.
    fn explicit_none(req)
        render("layout_test/default_view", {"layout": false})
    end
end
