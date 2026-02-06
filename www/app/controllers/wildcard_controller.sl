// Wildcard controller - demonstrates dynamic action resolution
//
// Routes: get("/wildcard/*", "wildcard#*")
// /wildcard/demo -> wildcard#demo
// /wildcard/example -> wildcard#example

class WildcardController extends Controller {
    fn demo(req: Any) -> Any {
        render_text("Wildcard demo action!")
    }

    fn example(req: Any) -> Any {
        let path = req["params"]["path"];
        render_text("Wildcard example! Path: " + path)
    }
}
